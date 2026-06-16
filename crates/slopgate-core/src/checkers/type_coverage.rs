//! type-coverage adapter — mirrors `src/checkers/type-coverage.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{local_bin, run_tool, source_line, truncate_chars};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCoverageEntry {
    pub file: String,
    pub line: u32,
    pub name: String,
}

pub fn parse_type_coverage_output(stdout: &str, repo_root: Option<&str>) -> Vec<TypeCoverageEntry> {
    let re = regex::Regex::new(r"^(.+?\.(?:ts|tsx|mts|cts)):(\d+):(\d+):? (.*)$").unwrap();
    let mut out = Vec::new();
    for raw in stdout.lines() {
        let trimmed = raw.trim();
        if let Some(caps) = re.captures(trimmed) {
            let mut file = caps[1].replace('\\', "/");
            if let Some(root) = repo_root {
                let prefix = format!("{root}/");
                if file.starts_with(&prefix) {
                    file = file[prefix.len()..].to_string();
                }
            }
            out.push(TypeCoverageEntry {
                file,
                line: caps[2].parse().unwrap_or(1),
                name: caps[4].to_string(),
            });
        }
    }
    out
}

pub fn detect(config: &ResolvedConfig, _cfg: &Value) -> DetectResult {
    let repo = Path::new(&config.repo_root);
    if !repo.join("tsconfig.json").exists() {
        return DetectResult {
            available: false,
            reason: Some("no tsconfig.json".to_string()),
        };
    }
    if local_bin(repo, "type-coverage").is_none() {
        return DetectResult {
            available: false,
            reason: Some("no local type-coverage binary".to_string()),
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
    let Some(bin) = local_bin(repo, "type-coverage") else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![],
        };
    };
    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .saturating_mul(1000);
    let res = run_tool(&bin, &["--detail"], Some(repo), Some(timeout_ms));
    if !res.ok && res.status.is_none() {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![format!(
                "type-coverage failed: {}",
                res.error.unwrap_or_else(|| "spawn failed".to_string())
            )],
        };
    }
    let violations = parse_type_coverage_output(&res.stdout, Some(&config.repo_root))
        .into_iter()
        .map(|e| Violation {
            id: "type-coverage-uncovered".into(),
            severity: "high".into(),
            category: "types".into(),
            file: e.file.clone(),
            line: e.line,
            full_line: source_line(repo, &e.file, e.line),
            text: truncate_chars(&format!("implicitly any: {}", e.name), 90),
            resolution: "Type this expression precisely.".into(),
            engine: "checker:type-coverage".into(),
        })
        .collect();
    CheckerRunResult {
        violations,
        errors: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_type_coverage_output_matches_fixture() {
        let stdout = "/repo/src/api/handler.ts:42:18: data\n\
/repo/src/api/handler.ts:55:3: response\n\
src/legacy/blob.ts:7:10: payload\n\
2912 / 2930 99.38%\n\
type-coverage success.";
        let got = parse_type_coverage_output(stdout, Some("/repo"));
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].file, "src/api/handler.ts");
        assert_eq!(got[0].line, 42);
        assert_eq!(got[0].name, "data");
        assert_eq!(got[2].file, "src/legacy/blob.ts");
        assert_eq!(got[2].name, "payload");
    }

    #[test]
    fn detect_false_when_no_tsconfig() {
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
        assert_eq!(det.reason.as_deref(), Some("no tsconfig.json"));
    }
}
