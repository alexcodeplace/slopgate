use slopgate_core::config::resolve_config;
use slopgate_core::gate::{run_gate, Mode, Tier};
use std::io::Write;

const USAGE: &str = "slopgate: no mode (use --staged | --file <p> | --self-test | init [dir] | baseline [--update|--prune] | install-hooks | install-skills [--force] | agent-hooks [status|install|reinstall|remove] [--agent <id>] | audit [--since-days N] [--json] | stats [--by rule|model|project|severity|engine|category] [--since <iso>] [--json] [--config <p>])";

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn val_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

fn write_slopgate_err(stderr: &mut dyn Write, msg: &str) {
    let _ = writeln!(stderr, "{msg}");
}

fn write_top_level_err(stderr: &mut dyn Write, err: &str) {
    if err.starts_with("slopgate:") {
        let _ = writeln!(stderr, "{err}");
    } else {
        let _ = writeln!(stderr, "slopgate: {err}");
    }
}

/// CLI entry point — testable without spawning the binary.
pub fn run(args: &[String]) -> i32 {
    let mut stderr = std::io::stderr();
    run_with_stderr(args, &mut stderr)
}

fn run_with_stderr(args: &[String], stderr: &mut dyn Write) -> i32 {
    match dispatch(args, stderr) {
        Ok(code) => code,
        Err(e) => {
            write_top_level_err(stderr, &e);
            1
        }
    }
}

fn dispatch(args: &[String], stderr: &mut dyn Write) -> Result<i32, String> {
    if args.get(1).is_some_and(|a| a == "--version") {
        println!("slopgate-rs {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    let config_path = match val_of(args, "--config") {
        Some(p) => p,
        None => {
            write_slopgate_err(stderr, "slopgate: --config <path> required");
            return Ok(2);
        }
    };

    let config = resolve_config(config_path)?;

    if has(args, "--self-test") {
        write_slopgate_err(stderr, "slopgate: --self-test not yet implemented in rust");
        return Ok(2);
    }

    let tier = match val_of(args, "--tier") {
        None => None,
        Some("fast") => Some(Tier::Fast),
        Some("commit") => Some(Tier::Commit),
        Some(_) => {
            write_slopgate_err(stderr, "slopgate: --tier must be fast|commit");
            return Ok(2);
        }
    };

    if has(args, "--staged") {
        let result = run_gate(Mode::Staged, &config, tier, None);
        if result.code == 1 {
            // PHASE-3: recordIncidents stats side-effect
        }
        return Ok(result.code);
    }

    if let Some(file_target) = val_of(args, "--file") {
        let result = run_gate(Mode::File, &config, tier, Some(file_target));
        return Ok(result.code);
    }

    write_slopgate_err(stderr, USAGE);
    Ok(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(run(&args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use tempfile::TempDir;

    fn fixture_toml() -> String {
        fs::read_to_string(format!(
            "{}/../slopgate-core/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn setup_tmp_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("git init");
        fs::create_dir_all(root.join(".slopgate")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".slopgate/config.toml"), fixture_toml()).unwrap();
        dir
    }

    fn run_capture(args: Vec<String>) -> (i32, String) {
        let mut stderr = Cursor::new(Vec::new());
        let code = run_with_stderr(&args, &mut stderr);
        let err = String::from_utf8(stderr.into_inner()).unwrap();
        (code, err)
    }

    fn base_args(root: &std::path::Path) -> Vec<String> {
        vec![
            "slopgate-rs".into(),
            "--config".into(),
            root.join(".slopgate/config.toml")
                .to_string_lossy()
                .into_owned(),
        ]
    }

    #[test]
    fn file_clean_returns_zero() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut args = base_args(root);
        args.extend(["--file".into(), "src/clean.ts".into()]);
        let (code, _) = run_capture(args);
        assert_eq!(code, 0);
    }

    #[test]
    fn file_violating_returns_one() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let mut args = base_args(root);
        args.extend(["--file".into(), "src/bad.ts".into()]);
        let (code, _) = run_capture(args);
        assert_eq!(code, 1);
    }

    #[test]
    fn missing_config_returns_two() {
        let (code, err) = run_capture(vec!["slopgate-rs".into(), "--file".into(), "x.ts".into()]);
        assert_eq!(code, 2);
        assert_eq!(err, "slopgate: --config <path> required\n");
    }

    #[test]
    fn bad_tier_returns_two() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut args = base_args(root);
        args.extend([
            "--tier".into(),
            "slow".into(),
            "--file".into(),
            "src/clean.ts".into(),
        ]);
        let (code, err) = run_capture(args);
        assert_eq!(code, 2);
        assert_eq!(err, "slopgate: --tier must be fast|commit\n");
    }

    #[test]
    fn no_mode_returns_two_with_usage() {
        let dir = setup_tmp_repo();
        let args = base_args(dir.path());
        let (code, err) = run_capture(args);
        assert_eq!(code, 2);
        assert_eq!(err, format!("{USAGE}\n"));
    }
}
