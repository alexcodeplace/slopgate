//! Native git pre-commit hook installer. Mirrors `src/install-hooks.mjs`.
//!
//! Marker-delimited block: idempotent rewrite of our block; foreign hook content
//! is always preserved (block inserted before first `exec`, else appended).

use crate::error::SlopError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MARKER_BEGIN: &str = "# slopgate-hook v1 BEGIN";
pub const MARKER_END: &str = "# slopgate-hook v1 END";

/// What changed when installing the pre-commit hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookInstallAction {
    Created,
    Updated,
    Appended,
    Unchanged,
}

/// Result of [`install_pre_commit_hook`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInstallResult {
    pub action: HookInstallAction,
    pub path: PathBuf,
}

fn hook_block(engine_invocation: &str, node_path: &str) -> String {
    [
        MARKER_BEGIN,
        "SLOPGATE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)",
        &format!("SLOPGATE_ENGINE=\"{engine_invocation}\""),
        &format!("SLOPGATE_NODE=\"{node_path}\""),
        "[ -x \"$SLOPGATE_NODE\" ] || SLOPGATE_NODE=node",
        "if [ -n \"$SLOPGATE_ROOT\" ] && [ -f \"$SLOPGATE_ROOT/.slopgate/config.mjs\" ]; then",
        "  if [ -f \"$SLOPGATE_ENGINE\" ]; then",
        "    \"$SLOPGATE_NODE\" \"$SLOPGATE_ENGINE\" --staged --config \"$SLOPGATE_ROOT/.slopgate/config.mjs\" || exit 1",
        "  else",
        "    echo \"slopgate: engine missing at $SLOPGATE_ENGINE — gate SKIPPED\" >&2",
        "  fi",
        "fi",
        MARKER_END,
    ]
    .join("\n")
}

/// Render pre-commit hook file content. Idempotent: a second call on its own output
/// returns the same string (markers are not duplicated).
pub fn render_hook_content(existing: &str, engine_invocation: &str, node_path: &str) -> String {
    let block = hook_block(engine_invocation, node_path);

    if existing.is_empty() {
        return format!("#!/usr/bin/env bash\n{block}\n");
    }

    if existing.contains(MARKER_BEGIN) {
        let start = existing
            .find(MARKER_BEGIN)
            .expect("MARKER_BEGIN present");
        let end = existing
            .find(MARKER_END)
            .map(|i| i + MARKER_END.len())
            .unwrap_or(existing.len());
        return format!("{}{}{}", &existing[..start], block, &existing[end..]);
    }

    let lines: Vec<&str> = existing.split('\n').collect();
    let exec_idx = lines
        .iter()
        .position(|line| line.trim_start().starts_with("exec "));

    if exec_idx.is_none() {
        let trimmed = existing.trim_end_matches('\n');
        return format!("{trimmed}\n{block}\n");
    }

    let exec_idx = exec_idx.expect("exec line index");
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == exec_idx {
            out.push_str(&block);
            out.push('\n');
        }
        out.push_str(line);
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

/// Resolve the git hooks directory for `repo_root`.
pub fn resolve_hooks_dir(repo_root: &Path) -> Result<PathBuf, SlopError> {
    let hooks_path = Command::new("git")
        .args(["config", "core.hooksPath"])
        .current_dir(repo_root)
        .output();

    if let Ok(output) = hooks_path {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                let path = Path::new(trimmed);
                return Ok(if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    repo_root.join(path)
                });
            }
        }
    }

    let git_dir = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| SlopError::Tool(format!("git rev-parse --git-dir: {e}")))?;

    if !git_dir.status.success() {
        return Err(SlopError::Tool(
            "git rev-parse --git-dir failed (not a git repository?)".into(),
        ));
    }

    let raw = String::from_utf8_lossy(&git_dir.stdout);
    let git_dir = raw.trim();
    let path = Path::new(git_dir);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    Ok(resolved.join("hooks"))
}

fn classify_action(existing: Option<&str>, rendered: &str) -> HookInstallAction {
    match existing {
        None => HookInstallAction::Created,
        Some(prev) if prev == rendered => HookInstallAction::Unchanged,
        Some(prev) if prev.contains(MARKER_BEGIN) => HookInstallAction::Updated,
        Some(_) => HookInstallAction::Appended,
    }
}

/// Install or refresh the native pre-commit hook under `repo_root`.
///
/// `engine_invocation` is the absolute path to the slopgate engine binary
/// (JS: `${ENGINE_ROOT}/bin/slopgate`). Node path is taken from the current
/// process executable.
pub fn install_pre_commit_hook(
    repo_root: &Path,
    engine_invocation: &str,
) -> Result<HookInstallResult, SlopError> {
    let node_path = std::env::current_exe()
        .map_err(|e| SlopError::Io(format!("current_exe: {e}")))?
        .display()
        .to_string();

    let hooks_dir = resolve_hooks_dir(repo_root)?;
    fs::create_dir_all(&hooks_dir)
        .map_err(|e| SlopError::Io(format!("create hooks dir {}: {e}", hooks_dir.display())))?;

    let hook_path = hooks_dir.join("pre-commit");
    let existing = if hook_path.is_file() {
        Some(
            fs::read_to_string(&hook_path)
                .map_err(|e| SlopError::Io(format!("read {}: {e}", hook_path.display())))?,
        )
    } else {
        None
    };

    let rendered = render_hook_content(
        existing.as_deref().unwrap_or(""),
        engine_invocation,
        &node_path,
    );
    let action = classify_action(existing.as_deref(), &rendered);

    if action != HookInstallAction::Unchanged {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&hook_path)
            .map_err(|e| SlopError::Io(format!("open {}: {e}", hook_path.display())))?;
        file.write_all(rendered.as_bytes())
            .map_err(|e| SlopError::Io(format!("write {}: {e}", hook_path.display())))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)
            .map_err(|e| SlopError::Io(format!("metadata {}: {e}", hook_path.display())))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)
            .map_err(|e| SlopError::Io(format!("chmod {}: {e}", hook_path.display())))?;
    }

    Ok(HookInstallResult {
        action,
        path: hook_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    const ENGINE: &str = "/opt/slopgate/bin/slopgate";
    const NODE: &str = "/usr/bin/node";

    fn marker_count(content: &str, marker: &str) -> usize {
        content.matches(marker).count()
    }

    #[test]
    fn render_empty_has_both_markers_exactly_once() {
        let content = render_hook_content("", ENGINE, NODE);
        assert!(content.starts_with("#!/usr/bin/env bash\n"));
        assert_eq!(marker_count(&content, MARKER_BEGIN), 1);
        assert_eq!(marker_count(&content, MARKER_END), 1);
        assert!(content.contains(ENGINE));
        assert!(content.contains(NODE));
    }

    #[test]
    fn render_is_idempotent_on_own_output() {
        let first = render_hook_content("", ENGINE, NODE);
        let second = render_hook_content(&first, ENGINE, NODE);
        assert_eq!(first, second, "second pass must not duplicate markers");
        assert_eq!(marker_count(&second, MARKER_BEGIN), 1);
        assert_eq!(marker_count(&second, MARKER_END), 1);
    }

    #[test]
    fn render_preserves_unrelated_content_around_markers() {
        let existing = format!(
            "#!/usr/bin/env bash\n\
             echo before\n\
             {MARKER_BEGIN}\n\
             old slopgate block\n\
             {MARKER_END}\n\
             echo after\n"
        );
        let rendered = render_hook_content(&existing, ENGINE, NODE);
        assert!(rendered.starts_with("#!/usr/bin/env bash\n"));
        assert!(rendered.contains("echo before\n"));
        assert!(rendered.contains("echo after\n"));
        assert!(!rendered.contains("old slopgate block"));
        assert_eq!(marker_count(&rendered, MARKER_BEGIN), 1);
        assert_eq!(marker_count(&rendered, MARKER_END), 1);
        assert!(rendered.contains(ENGINE));
    }

    #[test]
    fn render_appends_before_exec_line() {
        let existing = "#!/usr/bin/env bash\n\
            echo setup\n\
            exec \"$@\"\n";
        let rendered = render_hook_content(existing, ENGINE, NODE);
        let exec_pos = rendered.find("exec \"$@\"").expect("exec line");
        let begin_pos = rendered.find(MARKER_BEGIN).expect("marker begin");
        assert!(begin_pos < exec_pos);
        assert!(rendered.contains("echo setup\n"));
    }

    #[test]
    fn render_appends_when_no_exec_line() {
        let existing = "#!/usr/bin/env bash\n\
            echo only\n";
        let rendered = render_hook_content(existing, ENGINE, NODE);
        assert!(rendered.starts_with(existing.trim_end()));
        assert!(rendered.ends_with('\n'));
        assert!(rendered.contains(MARKER_BEGIN));
        assert!(rendered.contains("echo only\n"));
    }

    #[test]
    fn render_unhappy_marker_begin_without_end_replaces_from_begin() {
        let existing = format!("preamble\n{MARKER_BEGIN}\norphan\n");
        let rendered = render_hook_content(&existing, ENGINE, NODE);
        assert!(rendered.starts_with("preamble\n"));
        assert_eq!(marker_count(&rendered, MARKER_BEGIN), 1);
        assert_eq!(marker_count(&rendered, MARKER_END), 1);
        assert!(!rendered.contains("orphan"));
    }

    fn init_git_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("spawn git");
            assert!(status.success(), "git {:?} failed in {}", args, dir.display());
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "hooks@test.local"]);
        run(&["config", "user.name", "hooks test"]);
    }

    #[test]
    fn resolve_hooks_dir_default_git_hooks() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let hooks = resolve_hooks_dir(dir.path()).unwrap();
        assert!(hooks.ends_with(".git/hooks") || hooks.ends_with("hooks"));
        assert!(hooks.is_absolute() || dir.path().join(&hooks).is_absolute());
    }

    #[test]
    fn resolve_hooks_dir_honors_core_hooks_path() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let custom = dir.path().join("custom-hooks");
        fs::create_dir_all(&custom).unwrap();
        Command::new("git")
            .args([
                "config",
                "core.hooksPath",
                custom.to_str().unwrap(),
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let hooks = resolve_hooks_dir(dir.path()).unwrap();
        assert_eq!(hooks, custom);
    }

    #[test]
    fn resolve_hooks_dir_unhappy_non_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_hooks_dir(dir.path()).unwrap_err();
        assert!(matches!(err, SlopError::Tool(_)));
    }

    #[test]
    fn install_pre_commit_hook_creates_executable_hook() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let result = install_pre_commit_hook(dir.path(), ENGINE).unwrap();
        assert_eq!(result.action, HookInstallAction::Created);
        assert!(result.path.is_file());
        let content = fs::read_to_string(&result.path).unwrap();
        assert_eq!(marker_count(&content, MARKER_BEGIN), 1);
        assert_eq!(marker_count(&content, MARKER_END), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&result.path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
    }

    #[test]
    fn install_pre_commit_hook_unchanged_on_second_run() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let first = install_pre_commit_hook(dir.path(), ENGINE).unwrap();
        assert_eq!(first.action, HookInstallAction::Created);
        let second = install_pre_commit_hook(dir.path(), ENGINE).unwrap();
        assert_eq!(second.action, HookInstallAction::Unchanged);
        assert_eq!(first.path, second.path);
    }

    #[test]
    fn install_pre_commit_hook_unhappy_non_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = install_pre_commit_hook(dir.path(), ENGINE).unwrap_err();
        assert!(matches!(err, SlopError::Tool(_)));
    }
}
