//! actionlint adapter for GitHub Actions workflow files.

use crate::checkers::index::{CheckerRunOpts, CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    git_staged_paths, repo_relative_path, resolve_tool_bin, run_tool, source_line, truncate_chars,
};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionlintFinding {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub kind: String,
    pub message: String,
    pub snippet: String,
}

fn resolve_actionlint_bin(config: &ResolvedConfig, cfg: &Value) -> Option<PathBuf> {
    resolve_tool_bin(
        Path::new(&config.repo_root),
        cfg,
        "actionlint",
        &["-version"],
    )
}

fn field_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()))
}

fn field_u32(v: &Value, keys: &[&str], default: u32) -> u32 {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_u64()))
        .unwrap_or(default as u64) as u32
}

pub fn parse_actionlint_json(j: &Value) -> Vec<ActionlintFinding> {
    let Some(items) = j.as_array() else {
        return vec![];
    };
    let mut out = Vec::new();
    for item in items {
        let Some(file) =
            field_str(item, &["filepath", "Filepath", "file", "path"]).filter(|f| !f.is_empty())
        else {
            continue;
        };
        let Some(message) = field_str(item, &["message", "Message"]).filter(|m| !m.is_empty())
        else {
            continue;
        };
        out.push(ActionlintFinding {
            file: file.to_string(),
            line: field_u32(item, &["line", "Line"], 1),
            column: field_u32(item, &["column", "Column"], 1),
            kind: field_str(item, &["kind", "Kind"])
                .unwrap_or("issue")
                .to_string(),
            message: message.to_string(),
            snippet: field_str(item, &["snippet", "Snippet"])
                .unwrap_or("")
                .to_string(),
        });
    }
    out
}

fn strip_ansi(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").into_owned()
}

/// Parse actionlint's default one-line errors:
/// `.github/workflows/ci.yml:12:9: message [kind]`.
pub fn parse_actionlint_text(text: &str) -> Vec<ActionlintFinding> {
    let re =
        regex::Regex::new(r"^(.+?):(\d+):(\d+):\s+(.*?)(?:\s+\[([A-Za-z0-9_-]+)\])?$").unwrap();
    let mut out = Vec::new();
    for raw in strip_ansi(text).lines() {
        let Some(caps) = re.captures(raw.trim()) else {
            continue;
        };
        out.push(ActionlintFinding {
            file: caps[1].replace('\\', "/"),
            line: caps[2].parse().unwrap_or(1),
            column: caps[3].parse().unwrap_or(1),
            message: caps[4].to_string(),
            kind: caps
                .get(5)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "issue".to_string()),
            snippet: String::new(),
        });
    }
    out
}

fn safe_kind(kind: &str) -> String {
    let mut out = String::new();
    for c in kind.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

pub fn actionlint_violations(findings: &[ActionlintFinding], repo: &Path) -> Vec<Violation> {
    findings
        .iter()
        .map(|f| {
            let file = repo_relative_path(repo, &f.file);
            let kind = safe_kind(&f.kind);
            Violation {
                id: format!(
                    "actionlint-{}",
                    if kind.is_empty() { "issue" } else { &kind }
                ),
                severity: "high".into(),
                category: "workflow".into(),
                file: file.clone(),
                line: f.line,
                full_line: source_line(repo, &file, f.line),
                text: truncate_chars(&f.message, 90),
                resolution: "Fix the GitHub Actions workflow issue reported by actionlint.".into(),
                engine: "checker:actionlint".into(),
            }
        })
        .collect()
}

fn is_workflow_file(repo: &Path, rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    if !rel.starts_with(".github/workflows/") {
        return false;
    }
    let ext_ok = Path::new(&rel)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "yml" || e == "yaml");
    ext_ok && repo.join(&rel).is_file()
}

fn walk_workflows(repo: &Path) -> Vec<String> {
    let dir = repo.join(".github/workflows");
    let Ok(entries) = fs::read_dir(&dir) else {
        return vec![];
    };
    let mut files: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            path.strip_prefix(repo)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .filter(|rel| is_workflow_file(repo, rel))
        .collect();
    files.sort();
    files
}

fn target_files(config: &ResolvedConfig, opts: CheckerRunOpts<'_>) -> Vec<String> {
    let repo = Path::new(&config.repo_root);
    let raw = match opts.mode {
        "staged" => git_staged_paths(repo),
        "file" => opts.files.unwrap_or(&[]).to_vec(),
        _ => walk_workflows(repo),
    };
    raw.into_iter()
        .map(|f| repo_relative_path(repo, &f))
        .filter(|f| is_workflow_file(repo, f))
        .collect()
}

fn run_actionlint_json(
    bin: &Path,
    repo: &Path,
    files: &[String],
    timeout_ms: u64,
) -> Result<Vec<ActionlintFinding>, String> {
    let mut args = vec!["-format".to_string(), "{{json .}}".to_string()];
    args.extend(files.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = run_tool(bin, &arg_refs, Some(repo), Some(timeout_ms));
    if !res.ok && res.status.is_none() {
        return Err(format!(
            "actionlint failed: {}",
            res.error.unwrap_or_else(|| "spawn failed".to_string())
        ));
    }
    if !matches!(res.status, Some(0) | Some(1)) {
        return Err(format!(
            "actionlint failed: {}",
            truncate_chars(
                if res.stderr.trim().is_empty() {
                    res.stdout.trim()
                } else {
                    res.stderr.trim()
                },
                180
            )
        ));
    }
    let stdout = res.stdout.trim();
    if stdout.is_empty() {
        return Ok(vec![]);
    }
    let data: Value =
        serde_json::from_str(stdout).map_err(|e| format!("actionlint JSON parse error: {e}"))?;
    Ok(parse_actionlint_json(&data))
}

fn run_actionlint_text(
    bin: &Path,
    repo: &Path,
    files: &[String],
    timeout_ms: u64,
) -> Result<Vec<ActionlintFinding>, String> {
    let arg_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let res = run_tool(bin, &arg_refs, Some(repo), Some(timeout_ms));
    if !res.ok && res.status.is_none() {
        return Err(format!(
            "actionlint failed: {}",
            res.error.unwrap_or_else(|| "spawn failed".to_string())
        ));
    }
    if !matches!(res.status, Some(0) | Some(1)) {
        return Err(format!(
            "actionlint failed: {}",
            truncate_chars(
                if res.stderr.trim().is_empty() {
                    res.stdout.trim()
                } else {
                    res.stderr.trim()
                },
                180
            )
        ));
    }
    Ok(parse_actionlint_text(&format!(
        "{}\n{}",
        res.stdout, res.stderr
    )))
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    if resolve_actionlint_bin(config, cfg).is_none() {
        return DetectResult {
            available: false,
            reason: Some("no actionlint binary (configured/local/PATH)".to_string()),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

pub fn run(config: &ResolvedConfig, cfg: &Value, opts: CheckerRunOpts<'_>) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = resolve_actionlint_bin(config, cfg) else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec!["no actionlint binary".to_string()],
        };
    };
    let files = target_files(config, opts);
    if files.is_empty() {
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

    match run_actionlint_json(&bin, repo, &files, timeout_ms) {
        Ok(findings) => CheckerRunResult {
            violations: actionlint_violations(&findings, repo),
            errors: vec![],
        },
        Err(json_err) => match run_actionlint_text(&bin, repo, &files, timeout_ms) {
            Ok(findings) => CheckerRunResult {
                violations: actionlint_violations(&findings, repo),
                errors: vec![],
            },
            Err(text_err) => CheckerRunResult {
                violations: vec![],
                errors: vec![json_err, text_err],
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_actionlint_json_reads_documented_fields() {
        let j: Value = serde_json::from_str(
            r#"[
  {
    "message": "property \"platform\" is not defined in object type {os: string}",
    "snippet": "          key: ${{ matrix.platform }}",
    "kind": "expression",
    "filepath": ".github/workflows/ci.yaml",
    "line": 21,
    "column": 20,
    "end_column": 35
  }
]"#,
        )
        .unwrap();
        let got = parse_actionlint_json(&j);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file, ".github/workflows/ci.yaml");
        assert_eq!(got[0].kind, "expression");
        assert_eq!(got[0].line, 21);
    }

    #[test]
    fn parse_actionlint_text_reads_default_output() {
        let text = ".github/workflows/ci.yml:12:9: label \"linux-latest\" is unknown [runner-label]\n  |\n";
        let got = parse_actionlint_text(text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file, ".github/workflows/ci.yml");
        assert_eq!(got[0].line, 12);
        assert_eq!(got[0].column, 9);
        assert_eq!(got[0].kind, "runner-label");
    }

    #[test]
    fn actionlint_violations_are_high_workflow_findings() {
        let findings = vec![ActionlintFinding {
            file: ".github/workflows/ci.yml".into(),
            line: 12,
            column: 9,
            kind: "runner-label".into(),
            message: "label is unknown".into(),
            snippet: String::new(),
        }];
        let v = actionlint_violations(&findings, Path::new("/repo"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "actionlint-runner-label");
        assert_eq!(v[0].severity, "high");
        assert_eq!(v[0].category, "workflow");
    }
}
