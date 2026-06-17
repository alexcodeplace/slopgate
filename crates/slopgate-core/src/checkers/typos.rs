//! typos adapter for source/docs/rules/skills spelling checks.

use crate::checkers::index::{CheckerRunOpts, CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    git_staged_paths, repo_relative_path, resolve_tool_bin, run_tool, source_line, truncate_chars,
};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const EXTRA_DIR_SCOPES: &[&str] = &["docs", "rules", "skills"];
const ROOT_DOC_SCOPES: &[&str] = &[
    "README.md",
    "CONTRIBUTING.md",
    "CHANGELOG.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyposFinding {
    pub file: String,
    pub line: u32,
    pub typo: String,
    pub corrections: Vec<String>,
}

fn resolve_typos_bin(config: &ResolvedConfig, cfg: &Value) -> Option<PathBuf> {
    resolve_tool_bin(Path::new(&config.repo_root), cfg, "typos", &["--version"])
}

pub fn parse_typos_json_lines(stdout: &str) -> Result<Vec<TyposFinding>, String> {
    let mut out = Vec::new();
    for (idx, raw) in stdout.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let j: Value = serde_json::from_str(line)
            .map_err(|e| format!("line {}: {}", idx.saturating_add(1), e))?;
        if j.get("type").and_then(|t| t.as_str()) != Some("typo") {
            continue;
        }
        let Some(file) = j
            .get("path")
            .and_then(|p| p.as_str())
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let Some(typo) = j
            .get("typo")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let corrections = j
            .get("corrections")
            .and_then(|c| c.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.push(TyposFinding {
            file: file.to_string(),
            line: j.get("line_num").and_then(|l| l.as_u64()).unwrap_or(1) as u32,
            typo: typo.to_string(),
            corrections,
        });
    }
    Ok(out)
}

pub fn typos_violations(findings: &[TyposFinding], repo: &Path) -> Vec<Violation> {
    findings
        .iter()
        .map(|f| {
            let file = repo_relative_path(repo, &f.file);
            let text = if f.corrections.is_empty() {
                format!("possible typo: `{}`", f.typo)
            } else {
                format!("`{}` should be `{}`", f.typo, f.corrections.join("`, `"))
            };
            Violation {
                id: "typos-typo".into(),
                severity: "high".into(),
                category: "spelling".into(),
                file: file.clone(),
                line: f.line,
                full_line: source_line(repo, &file, f.line),
                text: truncate_chars(&text, 90),
                resolution: "Fix the typo or add a project-specific typos allowlist entry.".into(),
                engine: "checker:typos".into(),
            }
        })
        .collect()
}

fn under_scope(rel: &str, scope: &str) -> bool {
    rel == scope || rel.starts_with(&format!("{scope}/"))
}

fn default_scopes(config: &ResolvedConfig) -> Vec<String> {
    let repo = Path::new(&config.repo_root);
    let mut seen = HashSet::new();
    let mut scopes = Vec::new();
    for scope in config
        .roots_rel
        .iter()
        .map(String::as_str)
        .chain(EXTRA_DIR_SCOPES.iter().copied())
        .chain(ROOT_DOC_SCOPES.iter().copied())
    {
        if seen.insert(scope.to_string()) && repo.join(scope).exists() {
            scopes.push(scope.to_string());
        }
    }
    scopes
}

fn target_paths(config: &ResolvedConfig, opts: CheckerRunOpts<'_>) -> Vec<String> {
    let repo = Path::new(&config.repo_root);
    let scopes = default_scopes(config);
    if scopes.is_empty() {
        return vec![];
    }

    if opts.mode == "staged" {
        return git_staged_paths(repo)
            .into_iter()
            .map(|f| repo_relative_path(repo, &f))
            .filter(|f| repo.join(f).is_file())
            .filter(|f| scopes.iter().any(|scope| under_scope(f, scope)))
            .collect();
    }

    if opts.mode == "file" {
        return opts
            .files
            .unwrap_or(&[])
            .iter()
            .map(|f| repo_relative_path(repo, f))
            .filter(|f| repo.join(f).is_file())
            .filter(|f| scopes.iter().any(|scope| under_scope(f, scope)))
            .collect();
    }

    scopes
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    if resolve_typos_bin(config, cfg).is_none() {
        return DetectResult {
            available: false,
            reason: Some("no typos binary (configured/local/PATH)".to_string()),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

pub fn run(config: &ResolvedConfig, cfg: &Value, opts: CheckerRunOpts<'_>) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = resolve_typos_bin(config, cfg) else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec!["no typos binary".to_string()],
        };
    };
    let targets = target_paths(config, opts);
    if targets.is_empty() {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![],
        };
    }

    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .saturating_mul(1000);
    let mut args = vec!["--format".to_string(), "json".to_string()];
    args.extend(targets);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = run_tool(&bin, &arg_refs, Some(repo), Some(timeout_ms));
    if !res.ok && res.status.is_none() {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![format!(
                "typos failed: {}",
                res.error.unwrap_or_else(|| "spawn failed".to_string())
            )],
        };
    }
    if !matches!(res.status, Some(0) | Some(2)) {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![format!(
                "typos failed: {}",
                truncate_chars(
                    if res.stderr.trim().is_empty() {
                        res.stdout.trim()
                    } else {
                        res.stderr.trim()
                    },
                    180
                )
            )],
        };
    }

    let findings = match parse_typos_json_lines(&res.stdout) {
        Ok(findings) => findings,
        Err(e) => {
            return CheckerRunResult {
                violations: vec![],
                errors: vec![format!("typos JSON parse error: {e}")],
            };
        }
    };
    CheckerRunResult {
        violations: typos_violations(&findings, repo),
        errors: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_typos_json_lines_maps_typo_records_only() {
        let stdout = concat!(
            r#"{"type":"typo","path":"./src/lib.rs","line_num":3,"byte_offset":8,"typo":"retrive","corrections":["retrieve"]}"#,
            "\n",
            r#"{"type":"binary-file","path":"assets/logo.png"}"#,
            "\n",
            r#"{"type":"typo","path":"docs/guide.md","line_num":9,"byte_offset":4,"typo":"succesfully","corrections":["successfully"]}"#,
            "\n",
        );
        let got = parse_typos_json_lines(stdout).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].file, "./src/lib.rs");
        assert_eq!(got[0].typo, "retrive");
        assert_eq!(got[1].corrections, vec!["successfully"]);
    }

    #[test]
    fn typos_violations_are_high_spelling_findings() {
        let findings = vec![TyposFinding {
            file: "./docs/guide.md".into(),
            line: 9,
            typo: "succesfully".into(),
            corrections: vec!["successfully".into()],
        }];
        let v = typos_violations(&findings, Path::new("/repo"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "typos-typo");
        assert_eq!(v[0].severity, "high");
        assert_eq!(v[0].file, "docs/guide.md");
        assert!(v[0].text.contains("successfully"));
    }
}
