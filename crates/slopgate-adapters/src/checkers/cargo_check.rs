//! Rust workspace type/compile checking through Cargo, not a compiler in core.
use serde_json::Value;
use slopgate_core::checkers::index::{CheckerRunOpts, CheckerRunResult, DetectResult};
use slopgate_core::checkers::shared::{
    command_available, repo_relative_path, run_tool, source_line,
};
use slopgate_core::config::ResolvedConfig;
use slopgate_core::report::Violation;
use std::path::{Path, PathBuf};

fn binary(cfg: &Value) -> PathBuf {
    PathBuf::from(cfg.get("bin").and_then(Value::as_str).unwrap_or("cargo"))
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    let root = Path::new(&config.repo_root);
    let manifest = cfg
        .get("manifest")
        .and_then(Value::as_str)
        .unwrap_or("Cargo.toml");
    let reason = if !root.join(manifest).is_file() {
        Some(format!("missing Cargo manifest {manifest}"))
    } else if !command_available(&binary(cfg), &["--version"], Some(root)) {
        Some("Cargo executable is unavailable".to_string())
    } else {
        None
    };
    DetectResult {
        available: reason.is_none(),
        reason,
    }
}

pub fn run(config: &ResolvedConfig, cfg: &Value, _opts: CheckerRunOpts<'_>) -> CheckerRunResult {
    let root = Path::new(&config.repo_root);
    let mut args = vec![
        "check".to_string(),
        "--workspace".into(),
        "--all-targets".into(),
        "--message-format=json".into(),
    ];
    args.extend(["--jobs".into(), config.checker_concurrency.to_string()]);
    // Tool installation and dependency download are CI setup responsibilities.
    args.push("--offline".into());
    args.push("--locked".into());
    if let Some(manifest) = cfg.get("manifest").and_then(Value::as_str) {
        args.extend(["--manifest-path".into(), manifest.to_string()]);
    }
    let limit = cfg
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(120)
        .saturating_mul(1000);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run_tool(&binary(cfg), &arg_refs, Some(root), Some(limit));
    if !output.ok {
        return CheckerRunResult {
            errors: vec![output
                .error
                .unwrap_or_else(|| "Cargo execution incomplete".into())],
            ..Default::default()
        };
    }
    parse_output(&output.stdout, &output.stderr, output.status, root)
}

pub fn parse_output(
    stdout: &str,
    stderr: &str,
    exit: Option<i32>,
    root: &Path,
) -> CheckerRunResult {
    let mut result = CheckerRunResult::default();
    let mut finished = None;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                result
                    .errors
                    .push(format!("Cargo emitted malformed JSON: {error}"));
                continue;
            }
        };
        match message.get("reason").and_then(Value::as_str) {
            Some("build-finished") => finished = message.get("success").and_then(Value::as_bool),
            Some("compiler-message") => {
                let diagnostic = &message["message"];
                if diagnostic["level"].as_str() != Some("error") {
                    continue;
                }
                let Some(span) = diagnostic["spans"].as_array().and_then(|spans| {
                    spans
                        .iter()
                        .find(|span| span["is_primary"].as_bool() == Some(true))
                }) else {
                    result.errors.push(format!(
                        "Cargo diagnostic without source location: {}",
                        diagnostic["message"]
                            .as_str()
                            .unwrap_or("unknown diagnostic")
                    ));
                    continue;
                };
                let Some(file) = span["file_name"].as_str() else {
                    result
                        .errors
                        .push("Cargo error span lacks file_name".into());
                    continue;
                };
                let file = repo_relative_path(root, file);
                let Some(line) = span["line_start"]
                    .as_u64()
                    .and_then(|line| u32::try_from(line).ok())
                    .filter(|line| *line > 0)
                else {
                    result
                        .errors
                        .push("Cargo error span has invalid line_start".into());
                    continue;
                };
                if !slopgate_core::protocol::valid_relative_path(&file) {
                    result
                        .errors
                        .push(format!("Cargo diagnostic outside repository: {file}"));
                    continue;
                }
                let code = diagnostic["code"]["code"]
                    .as_str()
                    .unwrap_or("compile-error");
                result.violations.push(Violation {
                    id: format!("cargo-{code}"),
                    severity: "high".into(),
                    category: "types".into(),
                    full_line: source_line(root, &file, line),
                    file,
                    line,
                    text: diagnostic["message"]
                        .as_str()
                        .unwrap_or("Rust compile error")
                        .to_string(),
                    resolution: "Fix the Rust compile error without weakening project policy."
                        .into(),
                    engine: "checker:cargo-check".into(),
                });
            }
            Some("compiler-artifact" | "build-script-executed") => {}
            _ => result
                .errors
                .push("Cargo emitted an unrecognized message shape".into()),
        }
    }
    match (exit, finished) {
        (Some(0), Some(true)) if result.violations.is_empty() => {}
        (Some(101), Some(false)) if !result.violations.is_empty() => {}
        _ => result.errors.push(format!(
            "Cargo did not complete a verifiable check (exit {exit:?}, finished {finished:?}): {}",
            stderr.chars().take(1000).collect::<String>()
        )),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cargo_requires_terminal_success_not_just_empty_output() {
        let root = tempfile::tempdir().unwrap();
        assert!(!parse_output("", "", Some(0), root.path()).errors.is_empty());
        assert!(!parse_output("garbage", "", Some(0), root.path())
            .errors
            .is_empty());
        assert!(parse_output(
            "{\"reason\":\"build-finished\",\"success\":true}\n",
            "",
            Some(0),
            root.path()
        )
        .errors
        .is_empty());
        assert!(!parse_output(
            "{\"reason\":\"build-finished\",\"success\":false}\n",
            "error",
            Some(101),
            root.path()
        )
        .errors
        .is_empty());
    }
}
