//! leakscan adapter — mirrors `src/checkers/leakscan.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{run_json_tool, truncate_chars, JsonToolResult};
use crate::config::ResolvedConfig;
use crate::init::run::engine_root;
use crate::report::Violation;
use crate::severity::map_passthrough;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn resolve_bin(config: &ResolvedConfig, cfg: &Value) -> Option<PathBuf> {
    let repo = Path::new(&config.repo_root);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(bin) = cfg.get("bin").and_then(|b| b.as_str()) {
        candidates.push(repo.join(bin));
    }
    if let Ok(env_bin) = std::env::var("LEAKSCAN_BIN") {
        if !env_bin.is_empty() {
            candidates.push(PathBuf::from(env_bin));
        }
    }
    candidates.push(engine_root().join("bin/leakscan"));
    candidates.push(repo.join("tools/leakscan/target/release/leakscan"));
    candidates.push(repo.join("tools/leakscan/target/debug/leakscan"));
    candidates.into_iter().find(|p| p.exists())
}

fn config_file_args(config: &ResolvedConfig, cfg: &Value) -> Vec<String> {
    let config_dir = Path::new(&config.config_dir);
    let p = if let Some(rules) = cfg.get("rules").and_then(|r| r.as_str()) {
        config_dir.join(rules)
    } else {
        config_dir.join("leakscan.json")
    };
    if p.exists() {
        vec!["--config".into(), p.to_string_lossy().into_owned()]
    } else {
        vec![]
    }
}

pub fn leakscan_violations(report: &Value) -> Vec<Violation> {
    let mut out = Vec::new();
    let Some(items) = report.get("violations").and_then(|v| v.as_array()) else {
        return out;
    };
    for v in items {
        let raw_sev = v
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("critical");
        let Some(severity) = map_passthrough(raw_sev) else {
            continue;
        };
        let Some(file) = v
            .get("file")
            .and_then(|f| f.as_str())
            .filter(|f| !f.is_empty())
        else {
            continue;
        };
        let rule = v.get("rule").and_then(|r| r.as_str()).unwrap_or("unknown");
        let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32;
        let snippet = v.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
        let text_src = v.get("message").and_then(|m| m.as_str()).unwrap_or(rule);
        out.push(Violation {
            id: format!("leakscan-{rule}"),
            severity: severity.to_string(),
            category: "boundary".into(),
            file: file.to_string(),
            line,
            full_line: snippet.to_string(),
            text: truncate_chars(text_src, 90),
            resolution: "Route I/O through a service layer / API client — the component depends on the abstraction, not the transport.".into(),
            engine: "checker:leakscan".into(),
        });
    }
    out
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    if resolve_bin(config, cfg).is_none() {
        return DetectResult {
            available: false,
            reason: Some(
                "no leakscan binary (build tools/leakscan: cargo build --release)".to_string(),
            ),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

pub fn run(
    config: &ResolvedConfig,
    cfg: &Value,
    _opts: crate::checkers::index::CheckerRunOpts<'_>,
) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = resolve_bin(config, cfg) else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec!["no leakscan binary".to_string()],
        };
    };
    let mut args = config_file_args(config, cfg);
    args.extend(config.roots_rel.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .saturating_mul(1000);
    let JsonToolResult { data, mut errors } =
        run_json_tool("leakscan", &bin, &arg_refs, Some(repo), Some(timeout_ms));
    let Some(data) = data else {
        return CheckerRunResult {
            violations: vec![],
            errors,
        };
    };
    if let Some(extra) = data.get("errors").and_then(|e| e.as_array()) {
        for e in extra {
            if let Some(s) = e.as_str() {
                errors.push(s.to_string());
            }
        }
    }
    CheckerRunResult {
        violations: leakscan_violations(&data),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leakscan_violations_maps_canned_report() {
        let json: Value = serde_json::json!({
            "violations": [{
                "file": "src/UserCard.tsx",
                "line": 12,
                "rule": "global-fetch",
                "severity": "high",
                "message": "Direct fetch in component",
                "snippet": "  fetch(url)"
            }],
            "scanned": 1,
            "errors": []
        });
        let v = leakscan_violations(&json);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "leakscan-global-fetch");
        assert_eq!(v[0].severity, "high");
        assert_eq!(v[0].file, "src/UserCard.tsx");
        assert_eq!(v[0].line, 12);
        assert_eq!(v[0].full_line, "  fetch(url)");
        assert_eq!(v[0].text, "Direct fetch in component");
        assert!(v[0].resolution.contains("abstraction"));
    }

    #[test]
    fn detect_false_when_no_binary() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().join(".slopgate").to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 3,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };
        let det = detect(&config, &serde_json::json!({}));
        assert!(!det.available);
        assert!(det.reason.as_ref().unwrap().contains("no leakscan binary"));
    }
}
