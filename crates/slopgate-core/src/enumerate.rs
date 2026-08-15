//! Source file enumeration. Mirrors `src/enumerate.mjs` `listSourceFiles`.

use regex::Regex;
use std::collections::HashSet;
use std::fmt;
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

/// Infrastructure failure while discovering source files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumerateError {
    GitSpawn {
        operation: &'static str,
        detail: String,
    },
    GitFailed {
        operation: &'static str,
        status: String,
        detail: String,
    },
    InvalidUtf8 {
        operation: &'static str,
        detail: String,
    },
    MalformedGitOutput {
        operation: &'static str,
        detail: String,
    },
}

impl fmt::Display for EnumerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitSpawn { operation, detail } => {
                write!(f, "cannot run git {operation}: {detail}")
            }
            Self::GitFailed {
                operation,
                status,
                detail,
            } => {
                if detail.is_empty() {
                    write!(f, "git {operation} failed with {status}")
                } else {
                    write!(f, "git {operation} failed with {status}: {detail}")
                }
            }
            Self::InvalidUtf8 { operation, detail } => {
                write!(f, "git {operation} returned invalid UTF-8: {detail}")
            }
            Self::MalformedGitOutput { operation, detail } => {
                write!(f, "git {operation} returned malformed NUL output: {detail}")
            }
        }
    }
}

impl std::error::Error for EnumerateError {}

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
pub fn list_source_files(
    ctx: &EnumerateCtx,
    mode: EnumerateMode<'_>,
) -> Result<Vec<String>, EnumerateError> {
    match mode {
        EnumerateMode::File(file) => Ok(list_single_file(ctx, file)),
        EnumerateMode::Staged => list_staged(ctx),
        EnumerateMode::Walk => Ok(list_walk(ctx)),
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

fn list_staged(ctx: &EnumerateCtx) -> Result<Vec<String>, EnumerateError> {
    const OPERATION: &str = "diff --cached --name-status -z";
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "--diff-filter=ACMRD",
        ])
        .current_dir(&ctx.repo_root)
        .output()
        .map_err(|error| EnumerateError::GitSpawn {
            operation: OPERATION,
            detail: error.to_string(),
        })?;

    let stderr =
        std::str::from_utf8(&output.stderr).map_err(|error| EnumerateError::InvalidUtf8 {
            operation: OPERATION,
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(EnumerateError::GitFailed {
            operation: OPERATION,
            status: output.status.to_string(),
            detail: stderr.trim().to_string(),
        });
    }
    if !stderr.trim().is_empty() {
        return Err(EnumerateError::GitFailed {
            operation: OPERATION,
            status: output.status.to_string(),
            detail: stderr.trim().to_string(),
        });
    }

    let paths = parse_staged_name_status(&output.stdout)?;
    Ok(paths
        .into_iter()
        .filter(|f| {
            under_root(f, &ctx.roots_rel)
                && ext_with_dot(Path::new(f))
                    .as_ref()
                    .is_some_and(|e| ctx.exts.contains(e))
        })
        .collect())
}

fn parse_staged_name_status(raw: &[u8]) -> Result<Vec<String>, EnumerateError> {
    const OPERATION: &str = "diff --cached --name-status -z";
    if raw.is_empty() {
        return Ok(vec![]);
    }
    if !raw.ends_with(&[0]) {
        return Err(EnumerateError::MalformedGitOutput {
            operation: OPERATION,
            detail: "output is not NUL terminated".to_string(),
        });
    }

    let fields: Vec<&[u8]> = raw[..raw.len() - 1].split(|byte| *byte == 0).collect();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = fields[index];
        index += 1;
        if status.is_empty() {
            return Err(EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: "empty status field".to_string(),
            });
        }
        let status = std::str::from_utf8(status).map_err(|error| EnumerateError::InvalidUtf8 {
            operation: OPERATION,
            detail: error.to_string(),
        })?;
        let code = status.as_bytes()[0];
        if !matches!(code, b'A' | b'C' | b'M' | b'R' | b'D') {
            return Err(EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: format!("unsupported status {status:?}"),
            });
        }
        if matches!(code, b'R' | b'C')
            && (status.len() < 2 || !status[1..].chars().all(|c| c.is_ascii_digit()))
        {
            return Err(EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: format!("invalid rename/copy status {status:?}"),
            });
        }
        if matches!(code, b'A' | b'M' | b'D') && status.len() != 1 {
            return Err(EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: format!("invalid status {status:?}"),
            });
        }

        let first = fields
            .get(index)
            .ok_or_else(|| EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: format!("status {status:?} has no path"),
            })?;
        index += 1;
        if first.is_empty() {
            return Err(EnumerateError::MalformedGitOutput {
                operation: OPERATION,
                detail: format!("status {status:?} has an empty path"),
            });
        }

        std::str::from_utf8(first).map_err(|error| EnumerateError::InvalidUtf8 {
            operation: OPERATION,
            detail: error.to_string(),
        })?;

        let path = if matches!(code, b'R' | b'C') {
            let destination =
                fields
                    .get(index)
                    .ok_or_else(|| EnumerateError::MalformedGitOutput {
                        operation: OPERATION,
                        detail: format!("status {status:?} has no destination path"),
                    })?;
            index += 1;
            if destination.is_empty() {
                return Err(EnumerateError::MalformedGitOutput {
                    operation: OPERATION,
                    detail: format!("status {status:?} has an empty destination path"),
                });
            }
            destination
        } else {
            first
        };

        let path = std::str::from_utf8(path).map_err(|error| EnumerateError::InvalidUtf8 {
            operation: OPERATION,
            detail: error.to_string(),
        })?;
        if code != b'D' {
            paths.push(path.to_string());
        }
    }
    Ok(paths)
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

        let files = list_source_files(&ctx, EnumerateMode::Walk).unwrap();

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

        let got = list_source_files(&ctx, EnumerateMode::File("src/a/foo.ts")).unwrap();
        assert_eq!(got, vec!["src/a/foo.ts"]);
    }

    #[test]
    fn file_mode_empty_for_out_of_root() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("lib/out.ts")).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn file_mode_empty_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("src/missing.ts")).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn file_mode_returns_rel_for_test_file() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let got = list_source_files(&ctx, EnumerateMode::File("src/a/foo.test.ts")).unwrap();
        assert_eq!(got, vec!["src/a/foo.test.ts"]);
    }

    #[test]
    fn file_mode_resolves_absolute_path_under_repo() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());
        let abs = dir.path().join("src/a/foo.ts");

        let got = list_source_files(&ctx, EnumerateMode::File(abs.to_str().unwrap())).unwrap();
        assert_eq!(got, vec!["src/a/foo.ts"]);
    }

    #[test]
    fn staged_mode_reports_nonzero_git_as_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let error = list_source_files(&ctx, EnumerateMode::Staged).unwrap_err();
        assert!(matches!(error, EnumerateError::GitFailed { .. }));
    }

    #[test]
    fn staged_mode_reports_corrupt_index_as_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        git(dir.path(), &["init"]);
        fs::write(dir.path().join(".git/index"), b"not a git index").unwrap();

        let error = list_source_files(&ctx, EnumerateMode::Staged).unwrap_err();
        assert!(matches!(error, EnumerateError::GitFailed { .. }));
    }

    #[test]
    fn staged_mode_reports_bad_worktree_as_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        fs::write(dir.path().join(".git"), "gitdir: /missing/worktree").unwrap();
        let ctx = fixture_ctx(dir.path());

        let error = list_source_files(&ctx, EnumerateMode::Staged).unwrap_err();
        assert!(matches!(error, EnumerateError::GitFailed { .. }));
    }

    #[test]
    fn staged_mode_empty_in_non_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path());
        let ctx = fixture_ctx(dir.path());

        let error = list_source_files(&ctx, EnumerateMode::Staged).unwrap_err();
        assert!(matches!(error, EnumerateError::GitFailed { .. }));
        assert!(error.to_string().contains("git diff --cached"));
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

        let mut got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
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

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
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

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/staged-then-gone.tsx"]);
    }

    #[test]
    fn staged_mode_preserves_spaces_in_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/with space.ts"), "// spaced").unwrap();
        let ctx = fixture_ctx(dir.path());

        git(dir.path(), &["init"]);
        git(dir.path(), &["add", "src/with space.ts"]);

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/with space.ts"]);
    }

    #[test]
    fn staged_mode_preserves_spaces_newlines_and_backslashes_in_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/line\nwith\\mark.ts");
        fs::write(&path, "// path").unwrap();
        let ctx = fixture_ctx(dir.path());

        git(dir.path(), &["init"]);
        git(dir.path(), &["add", "src/line\nwith\\mark.ts"]);

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/line\nwith\\mark.ts"]);
    }

    #[test]
    fn staged_mode_reads_linked_worktree_index() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let linked = dir.path().join("linked");
        fs::create_dir_all(main.join("src")).unwrap();
        fs::write(main.join("src/base.ts"), "// base").unwrap();
        git(&main, &["init"]);
        git(&main, &["config", "user.email", "t@t"]);
        git(&main, &["config", "user.name", "t"]);
        git(&main, &["add", "src/base.ts"]);
        git(&main, &["commit", "-m", "init"]);
        git(&main, &["worktree", "add", linked.to_str().unwrap()]);

        fs::write(linked.join("src/linked.ts"), "// linked").unwrap();
        git(&linked, &["add", "src/linked.ts"]);
        let ctx = fixture_ctx(&linked);

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/linked.ts"]);
    }

    #[test]
    fn staged_mode_uses_rename_destination_and_excludes_source() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/old.ts"), "// old").unwrap();
        let ctx = fixture_ctx(dir.path());

        git_init_commit(dir.path(), &["src/old.ts"]);
        git(dir.path(), &["mv", "src/old.ts", "src/new.ts"]);

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/new.ts"]);
    }

    #[test]
    fn staged_mode_reports_git_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path().join("not-a-directory");
        fs::write(&repo_root, "not a directory").unwrap();
        let mut ctx = fixture_ctx(dir.path());
        ctx.repo_root = repo_root;

        let error = list_source_files(&ctx, EnumerateMode::Staged).unwrap_err();
        assert!(matches!(error, EnumerateError::GitSpawn { .. }));
    }

    #[test]
    fn staged_parser_rejects_empty_nul_record() {
        let error = parse_staged_name_status(b"\0").unwrap_err();
        assert!(matches!(error, EnumerateError::MalformedGitOutput { .. }));
    }

    #[test]
    fn staged_parser_rejects_trailing_empty_record() {
        let error = parse_staged_name_status(b"M\0src/file.ts\0\0").unwrap_err();
        assert!(matches!(error, EnumerateError::MalformedGitOutput { .. }));
    }

    #[test]
    fn staged_parser_rejects_missing_nul_terminator() {
        let error = parse_staged_name_status(b"M\0src/file.ts").unwrap_err();
        assert!(matches!(error, EnumerateError::MalformedGitOutput { .. }));
    }

    #[test]
    fn staged_parser_rejects_invalid_utf8_path() {
        let error = parse_staged_name_status(b"M\0src/\xff.ts\0").unwrap_err();
        assert!(matches!(error, EnumerateError::InvalidUtf8 { .. }));
    }

    #[test]
    fn staged_parser_rejects_invalid_utf8_deleted_path() {
        let error = parse_staged_name_status(b"D\0src/\xff.ts\0").unwrap_err();
        assert!(matches!(error, EnumerateError::InvalidUtf8 { .. }));
    }

    #[test]
    fn staged_parser_rejects_invalid_utf8_rename_source() {
        let error = parse_staged_name_status(b"R100\0src/\xff.ts\0src/new.ts\0").unwrap_err();
        assert!(matches!(error, EnumerateError::InvalidUtf8 { .. }));
    }

    #[test]
    fn staged_mode_keeps_eligible_path_when_deletion_is_mixed_in() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/keep.ts"), "// keep").unwrap();
        fs::write(dir.path().join("src/remove.ts"), "// remove").unwrap();
        let ctx = fixture_ctx(dir.path());

        git_init_commit(dir.path(), &["src/keep.ts", "src/remove.ts"]);
        git(dir.path(), &["rm", "src/remove.ts"]);
        fs::write(dir.path().join("src/keep.ts"), "// changed").unwrap();
        git(dir.path(), &["add", "src/keep.ts"]);

        let got = list_source_files(&ctx, EnumerateMode::Staged).unwrap();
        assert_eq!(got, vec!["src/keep.ts"]);
    }

    #[test]
    fn is_test_file_matches_dot_test_ts_and_tsx() {
        assert!(is_test_file("src/a/foo.test.ts"));
        assert!(is_test_file("src/a/foo.test.tsx"));
        assert!(!is_test_file("src/a/foo.ts"));
        assert!(!is_test_file("src/a/foo.testing.ts"));
    }
}
