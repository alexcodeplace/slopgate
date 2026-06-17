//! knip adapter — mirrors `src/checkers/knip.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    local_bin, run_json_tool, source_line, truncate_chars, JsonToolResult,
};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::fs;
use std::path::Path;

const ISSUE_TYPES: &[&str] = &[
    "dependencies",
    "devDependencies",
    "unlisted",
    "exports",
    "types",
    "duplicates",
];

const RESOLUTIONS: &[(&str, &str)] = &[
    (
        "files",
        "Delete the unused file (or wire it in if it was meant to be used).",
    ),
    (
        "exports",
        "Remove the unused export (or its consumer was deleted by mistake).",
    ),
    ("types", "Remove the unused exported type."),
    ("dependencies", "Uninstall the unused dependency."),
    ("devDependencies", "Uninstall the unused devDependency."),
    (
        "unlisted",
        "Add the dependency to package.json (it is imported but unlisted).",
    ),
    ("duplicates", "Deduplicate the export."),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnipFinding {
    pub finding_type: String,
    pub file: String,
    pub line: u32,
    pub name: String,
}

pub fn parse_knip_output(j: &Value) -> Vec<KnipFinding> {
    let mut out = Vec::new();
    if let Some(files) = j.get("files").and_then(|f| f.as_array()) {
        for f in files {
            if let Some(file) = f.as_str() {
                out.push(KnipFinding {
                    finding_type: "files".into(),
                    file: file.into(),
                    line: 1,
                    name: file.into(),
                });
            }
        }
    }
    if let Some(issues) = j.get("issues").and_then(|i| i.as_array()) {
        for issue in issues {
            let file = issue
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            for &t in ISSUE_TYPES {
                if let Some(items) = issue.get(t).and_then(|v| v.as_array()) {
                    for item in items {
                        let line = item.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32;
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(String::from)
                            .unwrap_or_else(|| item.to_string());
                        out.push(KnipFinding {
                            finding_type: t.into(),
                            file: file.clone(),
                            line,
                            name,
                        });
                    }
                }
            }
        }
    }
    out
}

fn resolution_for(t: &str) -> &'static str {
    RESOLUTIONS
        .iter()
        .find(|(k, _)| *k == t)
        .map(|(_, v)| *v)
        .unwrap_or("Fix the knip finding.")
}

fn has_knip_config(repo_root: &Path) -> bool {
    for f in [
        "knip.json",
        "knip.jsonc",
        "knip.config.ts",
        "knip.config.js",
    ] {
        if repo_root.join(f).exists() {
            return true;
        }
    }
    let pkg = repo_root.join("package.json");
    if let Ok(content) = fs::read_to_string(&pkg) {
        if let Ok(j) = serde_json::from_str::<Value>(&content) {
            return j.get("knip").is_some();
        }
    }
    false
}

pub fn detect(config: &ResolvedConfig, _cfg: &Value) -> DetectResult {
    let repo = Path::new(&config.repo_root);
    if local_bin(repo, "knip").is_none() {
        return DetectResult {
            available: false,
            reason: Some("no local knip binary".to_string()),
        };
    }
    if !has_knip_config(repo) {
        return DetectResult {
            available: false,
            reason: Some("no knip config".to_string()),
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
    let Some(bin) = local_bin(repo, "knip") else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![],
        };
    };
    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(90)
        .saturating_mul(1000);
    let JsonToolResult { data, errors } = run_json_tool(
        "knip",
        &bin,
        &["--reporter", "json", "--no-exit-code"],
        Some(repo),
        Some(timeout_ms),
    );
    let Some(data) = data else {
        return CheckerRunResult {
            violations: vec![],
            errors,
        };
    };
    let findings = parse_knip_output(&data);
    let violations = findings
        .into_iter()
        .map(|f| {
            let text = if f.finding_type == "files" {
                format!("unused file: {}", f.name)
            } else {
                format!("unused {}: {}", f.finding_type, f.name)
            };
            Violation {
                id: format!("knip-{}", f.finding_type),
                severity: "high".into(),
                category: "dead-code".into(),
                file: f.file.clone(),
                line: f.line,
                full_line: if f.finding_type == "files" {
                    String::new()
                } else {
                    source_line(repo, &f.file, f.line)
                },
                text: truncate_chars(&text, 90),
                resolution: resolution_for(&f.finding_type).into(),
                engine: "checker:knip".into(),
            }
        })
        .collect();
    CheckerRunResult { violations, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_knip_output_matches_fixture() {
        let j: Value = serde_json::from_str(
            r#"{
  "files": ["src/orphan.ts"],
  "issues": [
    {
      "file": "src/util.ts",
      "dependencies": [],
      "devDependencies": [],
      "unlisted": [],
      "exports": [{ "name": "unusedHelper", "line": 14 }],
      "types": [{ "name": "UnusedType", "line": 2 }],
      "duplicates": []
    },
    {
      "file": "package.json",
      "dependencies": [{ "name": "left-pad", "line": 12 }],
      "devDependencies": [],
      "unlisted": [],
      "exports": [],
      "types": [],
      "duplicates": []
    }
  ]
}"#,
        )
        .unwrap();
        let got = parse_knip_output(&j);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].finding_type, "files");
        assert_eq!(got[0].file, "src/orphan.ts");
        assert_eq!(got[1].finding_type, "exports");
        assert_eq!(got[1].line, 14);
        assert_eq!(got[3].finding_type, "dependencies");
        assert_eq!(got[3].name, "left-pad");
    }

    #[test]
    fn detect_false_when_no_local_knip() {
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
            checker_phases: Default::default(),
            phases: crate::config::default_phase_settings(),
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
        assert_eq!(det.reason.as_deref(), Some("no local knip binary"));
    }
}
