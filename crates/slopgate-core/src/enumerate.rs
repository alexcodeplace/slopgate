//! Source file enumeration. Mirrors `src/enumerate.mjs` `listSourceFiles`.

use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use walkdir::WalkDir;

/// Minimal config surface for enumeration (decoupled from `config.rs` until T15).
#[derive(Debug, Clone)]
pub struct EnumerateCtx {
    pub repo_root: PathBuf,
    /// Absolute scan roots (as in resolved config).
    pub roots: Vec<PathBuf>,
    /// Repo-relative root paths as written in config.
    pub roots_rel: Vec<String>,
    /// Allowed extensions including the dot (e.g. `.ts`).
    pub exts: HashSet<String>,
    /// Directory names to skip during full walk (matched on `file_name()` only).
    pub skip_dirs: HashSet<String>,
}

/// How to list source files.
pub enum EnumerateMode<'a> {
    /// Single file: resolve path, apply root/ext/exists filters.
    File(&'a str),
    /// Staged paths from `git diff --cached --name-only`, excluding deletions.
    Staged,
    /// Recurse all `roots`, honoring `skip_dirs` and extension filters.
    Walk,
}

/// Repo-relative source paths matching the JS `listSourceFiles` contract.
pub fn list_source_files(ctx: &EnumerateCtx, mode: EnumerateMode<'_>) -> Vec<String> {
    match mode {
        EnumerateMode::File(file) => list_single_file(ctx, file),
        EnumerateMode::Staged => list_staged(ctx),
        EnumerateMode::Walk => list_walk(ctx),
    }
}

fn list_single_file(ctx: &EnumerateCtx, file: &str) -> Vec<String> {
    let rel = resolve_rel(ctx, file);
    let Some(rel) = rel else {
        return vec![];
    };
    if !under_root(&rel, &ctx.roots_rel) {
        return vec![];
    }
    let ext = ext_with_dot(Path::new(&rel));
    if !ext.as_ref().is_some_and(|e| ctx.exts.contains(e)) {
        return vec![];
    }
    if !ctx.repo_root.join(&rel).exists() {
        return vec![];
    }
    vec![rel]
}

fn list_staged(ctx: &EnumerateCtx) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only", "--diff-filter=d"])
        .current_dir(&ctx.repo_root)
        .output();

    let Ok(output) = output else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter(|f| {
            under_root(f, &ctx.roots_rel)
                && ext_with_dot(Path::new(f))
                    .as_ref()
                    .is_some_and(|e| ctx.exts.contains(e))
        })
        .map(str::to_string)
        .collect()
}

fn list_walk(ctx: &EnumerateCtx) -> Vec<String> {
    let mut files = Vec::new();

    for root in &ctx.roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_entry(|e| {
            if e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    return !ctx.skip_dirs.contains(name);
                }
            }
            true
        }) {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let file_name = entry.file_name().to_str().unwrap_or("");
            let ext = ext_with_dot(Path::new(file_name));
            if !ext.as_ref().is_some_and(|e| ctx.exts.contains(e)) {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&ctx.repo_root)
                .ok()
                .map(path_to_posix);
            let Some(rel) = rel else {
                continue;
            };
            files.push(rel);
        }
    }

    files.sort();
    files
}

fn resolve_rel(ctx: &EnumerateCtx, file: &str) -> Option<String> {
    let path = Path::new(file);
    if path.is_absolute() {
        path.strip_prefix(&ctx.repo_root).ok().map(path_to_posix)
    } else {
        Some(file.replace('\\', "/"))
    }
}

fn under_root(rel: &str, roots_rel: &[String]) -> bool {
    roots_rel
        .iter()
        .any(|r| rel == r || rel.starts_with(&format!("{r}/")))
}

fn ext_with_dot(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
}

fn path_to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Matches `*.test.ts`/`*.test.tsx`. Enumeration itself no longer excludes these —
/// callers that need the historical "skip test files" default (checkers consuming
/// the shared file list) apply this explicitly; regex-pack patterns may opt in via
/// `Pattern.scan_test_files`.
pub fn is_test_file(path: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.test\.(ts|tsx)$").unwrap())
        .is_match(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as ProcCommand;

    fn fixture_ctx(dir: &Path) -> EnumerateCtx {
        let src = dir.join("src");
        EnumerateCtx {
            repo_root: dir.to_path_buf(),
            roots: vec![src.clone()],
            roots_rel: vec!["src".into()],
            exts: [".ts", ".tsx"].iter().map(|s| s.to_string()).collect(),
            skip_dirs: ["node_modules", "dist"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    fn write_tree(dir: &Path) {
        fs::create_dir_all(dir.join("src/a")).unwrap();
        fs::create_dir_all(dir.join("src/node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("src/a/foo.ts"), "// foo").unwrap();
        fs::write(dir.join("src/a/foo.test.ts"), "// test").unwrap();
        fs::write(dir.join("src/node_modules/pkg/hidden.ts"), "// hidden").unwrap();
        fs::write(dir.join("src/b.tsx"), "// b").unwrap();
        fs::write(dir.join("lib/out.ts"), "// out of root").unwrap();
    }

    #[test]
    fn walk_finds_ts_and_skips_node_modules_but_includes_test_files() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let files = list_source_files(&ctx, EnumerateMode::Walk);

        assert_eq!(
            files,
            vec![
                "src/a/foo.test.ts".to_string(),
                "src/a/foo.ts".to_string(),
                "src/b.tsx".to_string(),
            ]
        );
    }

    #[test]
    fn file_mode_returns_rel_for_valid_in_root_file() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("src/a/foo.ts"));
        assert_eq!(got, vec!["src/a/foo.ts"]);
    }

    #[test]
    fn file_mode_empty_for_out_of_root() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("lib/out.ts"));
        assert!(got.is_empty());
    }

    #[test]
    fn file_mode_empty_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("src/missing.ts"));
        assert!(got.is_empty());
    }

    #[test]
    fn file_mode_returns_rel_for_test_file() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("src/a/foo.test.ts"));
        assert_eq!(got, vec!["src/a/foo.test.ts"]);
    }

    #[test]
    fn file_mode_resolves_absolute_path_under_repo() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());
        let abs = dir.path().join("src/a/foo.ts");

        let got = list_source_files(&ctx, EnumerateMode::File(abs.to_str().unwrap()));
        assert_eq!(got, vec!["src/a/foo.ts"]);
    }

    #[test]
    fn staged_mode_empty_in_non_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::Staged);
        assert!(got.is_empty());
    }

    #[test]
    fn staged_mode_lists_cached_files() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        ProcCommand::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        ProcCommand::new("git")
            .args(["add", "src/a/foo.ts", "src/a/foo.test.ts", "lib/out.ts"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let mut got = list_source_files(&ctx, EnumerateMode::Staged);
        got.sort();
        assert_eq!(got, vec!["src/a/foo.test.ts", "src/a/foo.ts"]);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = ProcCommand::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_init_commit(dir: &Path, add: &[&str]) {
        git(dir, &["init"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        let mut add_args = vec!["add"];
        add_args.extend_from_slice(add);
        git(dir, &add_args);
        git(dir, &["commit", "-m", "init"]);
    }

    #[test]
    fn staged_mode_skips_deleted_paths() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        git_init_commit(dir.path(), &["src/a/foo.ts", "src/b.tsx"]);
        git(dir.path(), &["rm", "src/b.tsx"]);

        let got = list_source_files(&ctx, EnumerateMode::Staged);
        assert!(got.is_empty(), "staged deletion leaked into scan: {got:?}");
    }

    /// A path staged with content but removed from the working tree is still
    /// listed: the index carries content that is about to be committed, so
    /// dropping it here would gate nothing while reporting success.
    #[test]
    fn staged_mode_lists_added_path_removed_from_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        git_init_commit(dir.path(), &["src/a/foo.ts"]);
        fs::write(dir.path().join("src/staged-then-gone.tsx"), "// content").unwrap();
        git(dir.path(), &["add", "src/staged-then-gone.tsx"]);
        fs::remove_file(dir.path().join("src/staged-then-gone.tsx")).unwrap();

        let got = list_source_files(&ctx, EnumerateMode::Staged);
        assert_eq!(got, vec!["src/staged-then-gone.tsx"]);
    }

    #[test]
    fn is_test_file_matches_dot_test_ts_and_tsx() {
        assert!(is_test_file("src/a/foo.test.ts"));
        assert!(is_test_file("src/a/foo.test.tsx"));
        assert!(!is_test_file("src/a/foo.ts"));
        assert!(!is_test_file("src/a/foo.testing.ts"));
    }
}
