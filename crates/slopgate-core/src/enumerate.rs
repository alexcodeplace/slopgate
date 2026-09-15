//! Deterministic, fallible, language-neutral source discovery (SG-SCOPE-001).
use crate::process::run_tool;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct EnumerateCtx {
    pub repo_root: PathBuf,
    pub roots: Vec<PathBuf>,
    pub roots_rel: Vec<String>,
    /// Empty means all extensions. Discovery is not a claim of parser coverage.
    pub exts: HashSet<String>,
    pub skip_dirs: HashSet<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum EnumerateMode<'a> {
    File(&'a str),
    Staged,
    Walk,
}

pub fn git_paths(repo_root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = run_tool(Path::new("git"), args, Some(repo_root), Some(10_000));
    if !output.ok || output.status != Some(0) {
        return Err(format!(
            "git discovery failed: {} {}",
            output.error.unwrap_or_default(),
            output.stderr.trim()
        ));
    }
    if !output.stdout.is_empty() && !output.stdout.ends_with('\0') {
        return Err("git returned a non-NUL-terminated path list".to_string());
    }
    output
        .stdout
        .split_terminator('\0')
        .map(|file| {
            if file.is_empty()
                || Path::new(file).is_absolute()
                || Path::new(file).components().any(|p| {
                    matches!(
                        p,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                Err(format!(
                    "git returned an invalid repository-relative path: {file:?}"
                ))
            } else {
                Ok(file.to_string())
            }
        })
        .collect()
}

/// Identify the complete proposed tree, including configuration and deletions.
/// Comparing before and after analysis detects an adapter or concurrent writer
/// that changed and staged inputs while leaving a superficially clean worktree.
pub fn staged_identity(repo_root: &Path) -> Result<String, String> {
    let result = run_tool(
        Path::new("git"),
        &["write-tree"],
        Some(repo_root),
        Some(10_000),
    );
    let identity = result.stdout.trim();
    if !result.ok
        || result.status != Some(0)
        || ![40, 64].contains(&identity.len())
        || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "cannot identify proposed commit tree: {} {}",
            result.error.unwrap_or_default(),
            result.stderr
        ));
    }
    Ok(identity.to_string())
}

/// Conservative v1 snapshot policy. Checking an internally inconsistent
/// worktree is not evidence about the index. Never stash or rewrite user files.
pub fn require_consistent_staged_tree(repo_root: &Path) -> Result<(), String> {
    let dirty = git_paths(repo_root, &["diff", "--name-only", "-z", "--no-ext-diff"])?;
    if !dirty.is_empty() {
        return Err(format!("staged snapshot is inconsistent: {} tracked path(s) have unstaged changes; stage or separately commit them before checking", dirty.len()));
    }
    let untracked = git_paths(
        repo_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    if !untracked.is_empty() {
        return Err(format!("staged snapshot contains {} untracked analysis input(s); stage them or explicitly exclude generated files", untracked.len()));
    }
    Ok(())
}

pub fn list_source_files(
    ctx: &EnumerateCtx,
    mode: EnumerateMode<'_>,
) -> Result<Vec<String>, String> {
    let root = ctx
        .repo_root
        .canonicalize()
        .map_err(|error| format!("repository root cannot be resolved: {error}"))?;
    let mut files = match mode {
        EnumerateMode::File(file) => {
            let rel = resolve_rel(ctx, file)?;
            if !applicable(ctx, &rel) {
                return Ok(vec![]);
            }
            validate_source(&root, &rel)?;
            vec![rel]
        }
        EnumerateMode::Staged => {
            let files = git_paths(
                &root,
                &[
                    "diff",
                    "--cached",
                    "--name-only",
                    "--diff-filter=ACMR",
                    "--no-ext-diff",
                    "-z",
                ],
            )?;
            let mut selected = Vec::new();
            for file in files {
                if applicable(ctx, &file) {
                    validate_source(&root, &file)?;
                    selected.push(file);
                }
            }
            selected
        }
        EnumerateMode::Walk => {
            let mut selected = Vec::new();
            for configured in &ctx.roots {
                let scan_root = configured
                    .canonicalize()
                    .map_err(|error| format!("source root {}: {error}", configured.display()))?;
                if !scan_root.starts_with(&root) {
                    return Err(format!(
                        "source root escapes repository: {}",
                        configured.display()
                    ));
                }
                for entry in WalkDir::new(configured).into_iter().filter_entry(|entry| {
                    !(entry.file_type().is_dir()
                        && entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| ctx.skip_dirs.contains(name)))
                }) {
                    let entry =
                        entry.map_err(|error| format!("source enumeration failed: {error}"))?;
                    if entry.file_type().is_dir() {
                        continue;
                    }
                    let rel = entry
                        .path()
                        .strip_prefix(&ctx.repo_root)
                        .map_err(|error| format!("source root mismatch: {error}"))?;
                    let rel = path_to_posix(rel)?;
                    if !applicable(ctx, &rel) {
                        continue;
                    }
                    validate_source(&root, &rel)?;
                    selected.push(rel);
                    if selected.len() > 1_000_000 {
                        return Err("source enumeration exceeds one million files".into());
                    }
                }
            }
            selected
        }
    };
    files.sort();
    files.dedup();
    Ok(files)
}

fn validate_source(root: &Path, rel: &str) -> Result<(), String> {
    let file = root.join(rel);
    let resolved = file
        .canonicalize()
        .map_err(|error| format!("source {rel:?} cannot be read: {error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "source {rel:?} escapes repository through a symlink"
        ));
    }
    if !resolved.is_file() {
        return Err(format!("source {rel:?} is not a regular file"));
    }
    Ok(())
}

fn applicable(ctx: &EnumerateCtx, rel: &str) -> bool {
    let allowed_extension = ctx.exts.is_empty()
        || Path::new(rel)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ctx.exts.contains(&format!(".{ext}")));
    under_root(rel, &ctx.roots_rel)
        && allowed_extension
        && !rel
            .split('/')
            .take(rel.split('/').count().saturating_sub(1))
            .any(|part| ctx.skip_dirs.contains(part))
}

fn resolve_rel(ctx: &EnumerateCtx, file: &str) -> Result<String, String> {
    let file = Path::new(file);
    let rel = if file.is_absolute() {
        file.strip_prefix(&ctx.repo_root)
            .map_err(|_| "source is outside repository".to_string())?
    } else {
        file
    };
    if rel.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("source path contains traversal".into());
    }
    let normalized: PathBuf = rel
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect();
    path_to_posix(&normalized)
}

fn under_root(rel: &str, roots: &[String]) -> bool {
    roots.iter().any(|raw| {
        let root = raw.trim_end_matches('/').trim_start_matches("./");
        root.is_empty() || root == "." || rel == root || rel.starts_with(&format!("{root}/"))
    })
}

fn path_to_posix(path: &Path) -> Result<String, String> {
    let value = path.to_str().ok_or_else(|| {
        "non-UTF-8 source path is not representable in JSON diagnostics".to_string()
    })?;
    #[cfg(windows)]
    let value = value.replace('\\', "/");
    Ok(value.to_string())
}

/// A language-neutral naming convention for rule-owned test scoping. This does
/// not itself exclude tests from discovery or any checker.
pub fn is_test_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.contains(".test.")
        || name.contains(".spec.")
        || path.split('/').any(|part| part == "__tests__")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn list_source_files(ctx: &EnumerateCtx, mode: EnumerateMode<'_>) -> Vec<String> {
        super::list_source_files(ctx, mode).expect("fixture discovery")
    }
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

        assert!(super::list_source_files(&ctx, EnumerateMode::File("src/missing.ts")).is_err());
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

        assert!(super::list_source_files(&ctx, EnumerateMode::Staged).is_err());
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

        assert!(super::list_source_files(&ctx, EnumerateMode::Staged).is_err());
    }

    #[test]
    fn is_test_file_matches_dot_test_ts_and_tsx() {
        assert!(is_test_file("src/a/foo.test.ts"));
        assert!(is_test_file("src/a/foo.test.tsx"));
        assert!(!is_test_file("src/a/foo.ts"));
        assert!(!is_test_file("src/a/foo.testing.ts"));
    }
}
