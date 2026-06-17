//! Init orchestrator — port of `src/init.mjs` (`runInit`).

use crate::error::SlopError;
use crate::init::detect_stack::{
    build_convention_sources, detect_checkers, detect_exts, detect_roots, detect_skip_dirs,
};
use crate::init::scaffold::{
    convention_sources_json, format_config_toml, format_suppressions_json, merge_settings_json,
    DetectedConfig, DEPCRUISE_STARTER,
};
use crate::install::agent_hooks::{home_dir, install_agent_hooks as install_detected_agent_hooks};
use crate::install::hooks::{install_pre_commit_hook, HookInstallAction};
use crate::install::skills::{default_skills_dest_in, install_skills};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One agent-hooks row in the init summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHookResult {
    pub label: String,
    pub action: String,
}

/// Engine repo root (`parent of src/`), resolved from this crate's manifest dir.
pub fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn resolve_node_path() -> String {
    Command::new("which")
        .arg("node")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "node".to_string())
}

fn hook_action_label(action: HookInstallAction) -> &'static str {
    match action {
        HookInstallAction::Created => "created",
        HookInstallAction::Updated => "updated",
        HookInstallAction::Appended => "appended",
        HookInstallAction::Unchanged => "unchanged",
    }
}

fn install_agent_hooks(home: &Path, engine_root: &Path) -> Vec<AgentHookResult> {
    install_detected_agent_hooks(home, engine_root, None)
        .into_iter()
        .map(|r| AgentHookResult {
            label: r.label,
            action: r.action,
        })
        .collect()
}

fn checker_has_depcruise(checkers: &serde_json::Value) -> bool {
    checkers
        .get("depcruise")
        .is_some_and(|v| v == &serde_json::json!(true))
}

/// Run `slopgate init` scaffolding for `dir`. Mirrors `runInit` stdout/stderr + exit code.
pub fn run_init(dir: &str) -> i32 {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    run_init_io(dir, false, &mut stdout, &mut stderr)
}

/// Testable init entry with explicit writers (JS `options.quiet` supported).
pub fn run_init_io(dir: &str, quiet: bool, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let home = home_dir();
    run_init_io_with_home(dir, quiet, stdout, stderr, &home)
}

/// Like [`run_init_io`] but uses an explicit `home` for agent hooks and skills install paths.
pub fn run_init_io_with_home(
    dir: &str,
    quiet: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    home: &Path,
) -> i32 {
    match run_init_inner(dir, quiet, stdout, stderr, home) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(stderr, "slopgate: init failed: {e}");
            1
        }
    }
}

fn run_init_inner(
    dir: &str,
    quiet: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    home: &Path,
) -> Result<i32, SlopError> {
    let target_dir = Path::new(dir);
    let base = target_dir.join(".slopgate");
    let config_path = base.join("config.toml");
    let config_exists = config_path.is_file();

    let roots_result = detect_roots(target_dir);
    let roots = roots_result.roots;
    let warned = roots_result.warned;
    let exts = detect_exts(target_dir, &roots);
    let skip_dirs = detect_skip_dirs(target_dir);
    let engine = engine_root();

    fs::create_dir_all(base.join("rules/ast"))
        .map_err(|e| SlopError::Io(format!("mkdir {}: {e}", base.join("rules/ast").display())))?;

    let checkers = detect_checkers(target_dir, &roots, &exts, &engine);

    if !config_exists {
        fs::create_dir_all(base.join("fixtures/src")).map_err(|e| {
            SlopError::Io(format!(
                "mkdir {}: {e}",
                base.join("fixtures/src").display()
            ))
        })?;
        let detected = DetectedConfig {
            roots: roots.clone(),
            exts: exts.clone(),
            skip_dirs: skip_dirs.clone(),
            checkers: checkers.clone(),
        };
        fs::write(&config_path, format_config_toml(&detected))
            .map_err(|e| SlopError::Io(format!("write {}: {e}", config_path.display())))?;
        fs::write(base.join("suppressions.json"), format_suppressions_json())
            .map_err(|e| SlopError::Io(format!("write suppressions.json: {e}")))?;
    }

    let depcruise_path = base.join("depcruise.cjs");
    if checker_has_depcruise(&checkers) && !depcruise_path.is_file() {
        fs::write(&depcruise_path, DEPCRUISE_STARTER)
            .map_err(|e| SlopError::Io(format!("write {}: {e}", depcruise_path.display())))?;
    }

    let engine_invocation = engine.join("bin/slopgate").to_string_lossy().into_owned();
    let node_path = resolve_node_path();

    let hook_action = match install_pre_commit_hook(target_dir, &engine_invocation, &node_path) {
        Ok(result) => hook_action_label(result.action).to_string(),
        Err(_) => "skipped (not a git repo)".to_string(),
    };

    let skills_src = engine.join("skills");
    let _ = install_skills(&skills_src, &default_skills_dest_in(home), false);

    let agent_results = install_agent_hooks(home, &engine);

    let sources = build_convention_sources(target_dir);
    fs::write(
        base.join("convention-sources.json"),
        convention_sources_json(&sources),
    )
    .map_err(|e| SlopError::Io(format!("write convention-sources.json: {e}")))?;

    let settings_action = merge_settings_json(target_dir, &engine, stderr)?;

    if !quiet {
        if config_exists {
            let _ = writeln!(
                stderr,
                "slopgate: {} already exists — preserved (not overwritten)",
                config_path.display()
            );
        } else {
            let _ = writeln!(stdout, "slopgate: scaffolded {}/", base.display());
        }
        if warned {
            let _ = writeln!(
                stderr,
                "slopgate: WARNING — no source roots detected; defaulting to [\"src\"] — review manually"
            );
        }
        let _ = writeln!(stdout, "\n--- slopgate init summary ---");
        let _ = writeln!(
            stdout,
            "roots:     {}",
            serde_json::to_string(&roots).unwrap()
        );
        let _ = writeln!(
            stdout,
            "exts:      {}",
            serde_json::to_string(&exts).unwrap()
        );
        let _ = writeln!(
            stdout,
            "skipDirs:  {}",
            serde_json::to_string(&skip_dirs).unwrap()
        );
        let _ = writeln!(
            stdout,
            "settings:  {} (.claude/settings.json)",
            settings_action.as_str()
        );
        let checker_keys: Vec<&str> = checkers
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let _ = writeln!(
            stdout,
            "checkers:  {}",
            serde_json::to_string(&checker_keys).unwrap()
        );
        let _ = writeln!(stdout, "pre-commit hook: {hook_action}");
        if agent_results.is_empty() {
            let _ = writeln!(stdout, "agent hooks:     no agent CLIs detected");
        } else {
            for r in &agent_results {
                let _ = writeln!(stdout, "agent hooks:     {} — {}", r.label, r.action);
            }
        }
        let _ = writeln!(stdout, "\nNEXT STEPS:");
        let _ = writeln!(
            stdout,
            "  1. Review .slopgate/convention-sources.json for project rule candidates"
        );
        let _ = writeln!(
            stdout,
            "  2. Author project rules as ast-grep YAML in .slopgate/rules/ast/"
        );
        let _ = writeln!(
            stdout,
            "  3. Run a dry-run gate pass before enabling blocking mode"
        );
        let _ = writeln!(
            stdout,
            "  4. Drive each new rule to zero hits before enabling blocking mode"
        );
        let _ = writeln!(
            stdout,
            "  5. Run: slopgate baseline --config .slopgate/config.toml (absorb pre-existing violations)"
        );
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config;
    use std::io::Cursor;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn capture_init(dir: &Path) -> (i32, String, String) {
        let mut stdout = Cursor::new(Vec::new());
        let mut stderr = Cursor::new(Vec::new());
        let code = run_init_io(dir.to_str().unwrap(), false, &mut stdout, &mut stderr);
        (
            code,
            String::from_utf8(stdout.into_inner()).unwrap(),
            String::from_utf8(stderr.into_inner()).unwrap(),
        )
    }

    #[test]
    fn fresh_init_creates_expected_files_and_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("src/index.ts"), "export const x = 1;\n");

        let (code, stdout, stderr) = capture_init(root);
        assert_eq!(code, 0, "stderr={stderr}");
        assert!(stdout.contains("slopgate: scaffolded"));
        assert!(stdout.contains("--- slopgate init summary ---"));
        assert!(!stderr.contains("already exists"));

        let base = root.join(".slopgate");
        assert!(base.join("config.toml").is_file());
        assert!(base.join("suppressions.json").is_file());
        assert!(base.join("rules/ast").is_dir());
        assert!(base.join("fixtures/src").is_dir());
        assert!(base.join("convention-sources.json").is_file());

        let suppressions = fs::read_to_string(base.join("suppressions.json")).unwrap();
        assert_eq!(
            suppressions,
            crate::init::scaffold::format_suppressions_json()
        );

        let cfg = resolve_config(&base.join("config.toml").to_string_lossy()).unwrap();
        assert!(cfg.roots_rel.iter().any(|r| r == "src"));
        assert!(cfg.exts.contains(".ts"));
        assert!(cfg.skip_dirs.contains("node_modules"));
    }

    #[test]
    fn reinit_preserves_config_and_reports_on_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("src/index.ts"), "export const x = 1;\n");

        let (code1, _, _) = capture_init(root);
        assert_eq!(code1, 0);

        let config_path = root.join(".slopgate/config.toml");
        let original = fs::read_to_string(&config_path).unwrap();
        write_file(&config_path, "# sentinel — must survive re-init\n");

        let (code2, stdout2, stderr2) = capture_init(root);
        assert_eq!(code2, 0);
        assert!(
            stderr2.contains("already exists — preserved (not overwritten)"),
            "stderr={stderr2}"
        );
        assert!(!stdout2.contains("slopgate: scaffolded"));
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "# sentinel — must survive re-init\n"
        );
        assert_ne!(original, "# sentinel — must survive re-init\n");
    }

    #[test]
    fn init_without_src_warns_and_defaults_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let (code, _, stderr) = capture_init(root);
        assert_eq!(code, 0);
        assert!(stderr.contains("WARNING — no source roots detected"));
    }

    #[test]
    fn depcruise_starter_written_when_checker_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("src/index.ts"), "export const x = 1;\n");
        write_file(
            &root.join("node_modules/.bin/depcruise"),
            "#!/usr/bin/env node\n",
        );

        let (code, _, _) = capture_init(root);
        assert_eq!(code, 0);
        let dep = root.join(".slopgate/depcruise.cjs");
        assert!(dep.is_file());
        assert_eq!(fs::read_to_string(dep).unwrap(), DEPCRUISE_STARTER);
    }
}
