//! jscpd adapter — mirrors `src/checkers/jscpd.mjs`.

use crate::checkers::index::{CheckerRunOpts, CheckerRunResult, DetectResult};
use crate::checkers::shared::{local_bin, run_tool, source_line, truncate_chars};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use crate::temp::with_temp_dir;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JscpdClone {
    pub first_file: String,
    pub first_start: u32,
    pub first_end: u32,
    pub second_file: String,
    pub second_start: u32,
    pub second_end: u32,
    pub lines: u32,
}

pub fn parse_jscpd_report(json_text: &str) -> Result<Vec<JscpdClone>, String> {
    let j: Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(dups) = j.get("duplicates").and_then(|d| d.as_array()) {
        for d in dups {
            let first = d.get("firstFile").and_then(|f| f.as_object());
            let second = d.get("secondFile").and_then(|f| f.as_object());
            if let (Some(ff), Some(sf)) = (first, second) {
                let first_start = ff
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        ff.get("startLoc")
                            .and_then(|l| l.get("line"))
                            .and_then(|l| l.as_u64())
                    })
                    .unwrap_or(1) as u32;
                let first_end = ff
                    .get("end")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        ff.get("endLoc")
                            .and_then(|l| l.get("line"))
                            .and_then(|l| l.as_u64())
                    })
                    .unwrap_or(1) as u32;
                let second_start = sf
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        sf.get("startLoc")
                            .and_then(|l| l.get("line"))
                            .and_then(|l| l.as_u64())
                    })
                    .unwrap_or(1) as u32;
                let second_end = sf
                    .get("end")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        sf.get("endLoc")
                            .and_then(|l| l.get("line"))
                            .and_then(|l| l.as_u64())
                    })
                    .unwrap_or(1) as u32;
                out.push(JscpdClone {
                    first_file: ff
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string(),
                    first_start,
                    first_end,
                    second_file: sf
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string(),
                    second_start,
                    second_end,
                    lines: d.get("lines").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                });
            }
        }
    }
    Ok(out)
}

pub fn clone_violations(
    clones: &[JscpdClone],
    staged_files: Option<&[String]>,
    repo_root: Option<&Path>,
) -> Vec<Violation> {
    let staged: Option<HashSet<&str>> =
        staged_files.map(|fs| fs.iter().map(String::as_str).collect());
    let mut out = Vec::new();
    for c in clones {
        let staged_set = staged.as_ref();
        let (mine, other, line) = if staged_set.is_none_or(|s| s.contains(c.first_file.as_str())) {
            (
                c.first_file.as_str(),
                format!("{}:{}-{}", c.second_file, c.second_start, c.second_end),
                c.first_start,
            )
        } else if staged_set.is_some_and(|s| s.contains(c.second_file.as_str())) {
            (
                c.second_file.as_str(),
                format!("{}:{}-{}", c.first_file, c.first_start, c.first_end),
                c.second_start,
            )
        } else {
            continue;
        };
        out.push(Violation {
            id: "jscpd-clone".into(),
            severity: "high".into(),
            category: "duplication".into(),
            file: mine.to_string(),
            line,
            full_line: repo_root
                .map(|r| source_line(r, mine, line))
                .unwrap_or_default(),
            text: truncate_chars(&format!("duplicates {other} ({} lines)", c.lines), 90),
            resolution: "Extract a shared util / import the existing implementation.".into(),
            engine: "checker:jscpd".into(),
        });
    }
    out
}

pub fn detect(config: &ResolvedConfig, _cfg: &Value) -> DetectResult {
    if local_bin(Path::new(&config.repo_root), "jscpd").is_none() {
        return DetectResult {
            available: false,
            reason: Some("no local jscpd binary".to_string()),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

pub fn run(config: &ResolvedConfig, cfg: &Value, opts: CheckerRunOpts<'_>) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = local_bin(repo, "jscpd") else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![],
        };
    };
    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .saturating_mul(1000);
    let min_tokens = cfg
        .get("minTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .to_string();

    let staged = opts.files;
    let mode_files = staged;

    match with_temp_dir("slopgate-jscpd-", |out_dir| {
        let mut args: Vec<String> = config.roots_rel.clone();
        args.extend([
            "--min-tokens".into(),
            min_tokens,
            "--reporters".into(),
            "json".into(),
            "--output".into(),
            out_dir.to_string_lossy().into_owned(),
            "--silent".into(),
        ]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let res = run_tool(&bin, &arg_refs, Some(repo), Some(timeout_ms));
        if !res.ok {
            return CheckerRunResult {
                violations: vec![],
                errors: vec![format!(
                    "jscpd failed: {}",
                    res.error.unwrap_or_else(|| "spawn failed".to_string())
                )],
            };
        }
        let report_path = out_dir.join("jscpd-report.json");
        if !report_path.exists() {
            return CheckerRunResult {
                violations: vec![],
                errors: vec!["jscpd produced no report".to_string()],
            };
        }
        let content = match fs::read_to_string(&report_path) {
            Ok(c) => c,
            Err(e) => {
                return CheckerRunResult {
                    violations: vec![],
                    errors: vec![format!("jscpd read error: {e}")],
                };
            }
        };
        let clones = match parse_jscpd_report(&content) {
            Ok(c) => c,
            Err(e) => {
                return CheckerRunResult {
                    violations: vec![],
                    errors: vec![format!("jscpd JSON parse error: {e}")],
                };
            }
        };
        CheckerRunResult {
            violations: clone_violations(&clones, mode_files, Some(repo)),
            errors: vec![],
        }
    }) {
        Ok(r) => r,
        Err(e) => CheckerRunResult {
            violations: vec![],
            errors: vec![format!("jscpd temp dir failed: {e}")],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jscpd_report_matches_fixture() {
        let json = r#"{
  "duplicates": [{
    "format": "typescript",
    "lines": 18,
    "firstFile": { "name": "src/features/a.ts", "start": 10, "end": 27, "startLoc": { "line": 10 }, "endLoc": { "line": 27 } },
    "secondFile": { "name": "src/features/b.ts", "start": 40, "end": 57, "startLoc": { "line": 40 }, "endLoc": { "line": 57 } }
  }]
}"#;
        let clones = parse_jscpd_report(json).unwrap();
        assert_eq!(clones.len(), 1);
        assert_eq!(clones[0].first_file, "src/features/a.ts");
        assert_eq!(clones[0].first_start, 10);
        assert_eq!(clones[0].lines, 18);
    }

    #[test]
    fn clone_violations_staged_side_selection() {
        let clones = vec![JscpdClone {
            first_file: "src/features/a.ts".into(),
            first_start: 10,
            first_end: 27,
            second_file: "src/features/b.ts".into(),
            second_start: 40,
            second_end: 57,
            lines: 18,
        }];
        let staged = vec!["src/features/b.ts".into()];
        let v = clone_violations(&clones, Some(&staged), None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].file, "src/features/b.ts");
        assert_eq!(v[0].line, 40);
        assert!(v[0].text.contains("src/features/a.ts:10-27"));
    }

    #[test]
    fn clone_violations_skips_non_staged_in_staged_mode() {
        let clones = vec![JscpdClone {
            first_file: "src/a.ts".into(),
            first_start: 1,
            first_end: 2,
            second_file: "src/b.ts".into(),
            second_start: 3,
            second_end: 4,
            lines: 2,
        }];
        let staged = vec!["src/other.ts".into()];
        assert!(clone_violations(&clones, Some(&staged), None).is_empty());
    }

    #[test]
    fn detect_false_when_no_local_jscpd() {
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
        assert_eq!(det.reason.as_deref(), Some("no local jscpd binary"));
    }
}
