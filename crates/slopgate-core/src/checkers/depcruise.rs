//! dependency-cruiser adapter — mirrors `src/checkers/depcruise.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{local_bin, run_json_tool, truncate_chars, JsonToolResult};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use crate::severity::map_depcruise;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepcruiseParsed {
    pub rule: String,
    pub severity: String,
    pub from: String,
    pub to: String,
}

pub fn parse_depcruise_output(j: &Value) -> Vec<DepcruiseParsed> {
    j.get("summary")
        .and_then(|s| s.get("violations"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(DepcruiseParsed {
                        rule: v
                            .get("rule")
                            .and_then(|r| r.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        severity: v
                            .get("rule")
                            .and_then(|r| r.get("severity"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("error")
                            .to_string(),
                        from: v.get("from").and_then(|f| f.as_str())?.to_string(),
                        to: v.get("to").and_then(|t| t.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn depcruise_violations(parsed: &[DepcruiseParsed]) -> Vec<Violation> {
    let mut out = Vec::new();
    for v in parsed {
        let Some(severity) = map_depcruise(&v.severity) else {
            continue;
        };
        if v.from.is_empty() {
            continue;
        }
        out.push(Violation {
            id: format!("depcruise-{}", v.rule),
            severity: severity.to_string(),
            category: "architecture".into(),
            file: v.from.clone(),
            line: 1,
            full_line: String::new(),
            text: truncate_chars(&format!("{} → {} violates {}", v.from, v.to, v.rule), 90),
            resolution:
                "Respect the dependency rule — restructure the import, do not relax the rule."
                    .into(),
            engine: "checker:depcruise".into(),
        });
    }
    out
}

fn rules_file(config: &ResolvedConfig, cfg: &Value) -> Option<PathBuf> {
    let config_dir = Path::new(&config.config_dir);
    let repo = Path::new(&config.repo_root);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(rules) = cfg.get("rules").and_then(|r| r.as_str()) {
        candidates.push(config_dir.join(rules));
    }
    candidates.push(config_dir.join("depcruise.cjs"));
    candidates.push(repo.join(".dependency-cruiser.js"));
    candidates.push(repo.join(".dependency-cruiser.cjs"));
    candidates.push(repo.join(".dependency-cruiser.json"));
    candidates.into_iter().find(|p| p.exists())
}

pub fn run_depcruise_json(config: &ResolvedConfig, cfg: &Value) -> JsonToolResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = local_bin(repo, "depcruise") else {
        return JsonToolResult {
            data: None,
            errors: vec!["no local depcruise binary".to_string()],
        };
    };
    let Some(rules) = rules_file(config, cfg) else {
        return JsonToolResult {
            data: None,
            errors: vec!["no depcruise rules file".to_string()],
        };
    };
    let mut args: Vec<String> = vec![
        "--config".into(),
        rules.to_string_lossy().into_owned(),
        "--output-type".into(),
        "json".into(),
    ];
    args.extend(config.roots_rel.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .saturating_mul(1000);
    run_json_tool("depcruise", &bin, &arg_refs, Some(repo), Some(timeout_ms))
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    let repo = Path::new(&config.repo_root);
    if local_bin(repo, "depcruise").is_none() {
        return DetectResult {
            available: false,
            reason: Some("no local depcruise binary".to_string()),
        };
    }
    if rules_file(config, cfg).is_none() {
        return DetectResult {
            available: false,
            reason: Some("no depcruise rules file".to_string()),
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
    let JsonToolResult { data, errors } = run_depcruise_json(config, cfg);
    let Some(data) = data else {
        return CheckerRunResult {
            violations: vec![],
            errors,
        };
    };
    CheckerRunResult {
        violations: depcruise_violations(&parse_depcruise_output(&data)),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_depcruise_output_matches_fixture() {
        let j: Value = serde_json::from_str(
            r#"{
  "summary": {
    "violations": [
      { "type": "cycle", "from": "src/a.ts", "to": "src/b.ts", "rule": { "severity": "error", "name": "no-circular" } },
      { "type": "module", "from": "src/orphan.ts", "to": "src/orphan.ts", "rule": { "severity": "warn", "name": "no-orphans" } },
      { "type": "dependency", "from": "src/ui/page.ts", "to": "src/db/client.ts", "rule": { "severity": "info", "name": "fyi-only" } }
    ]
  }
}"#,
        )
        .unwrap();
        let parsed = parse_depcruise_output(&j);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].rule, "no-circular");
        assert_eq!(parsed[0].severity, "error");
    }

    #[test]
    fn depcruise_violations_drops_info_severity() {
        let parsed = parse_depcruise_output(&serde_json::json!({
            "summary": { "violations": [
                { "from": "src/a.ts", "to": "src/b.ts", "rule": { "severity": "error", "name": "no-circular" } },
                { "from": "src/ui/page.ts", "to": "src/db/client.ts", "rule": { "severity": "info", "name": "fyi-only" } }
            ]}
        }));
        let v = depcruise_violations(&parsed);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "depcruise-no-circular");
        assert_eq!(v[0].severity, "critical");
    }

    #[test]
    fn detect_false_when_no_local_depcruise() {
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
            agent: crate::config::AgentConfig::default(),
        };
        let det = detect(&config, &serde_json::json!({}));
        assert!(!det.available);
        assert_eq!(det.reason.as_deref(), Some("no local depcruise binary"));
    }
}
