//! shellcheck adapter for staged/full shell scripts.

use crate::checkers::index::{CheckerRunOpts, CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    git_staged_paths, repo_relative_path, resolve_tool_bin, run_tool, source_line, truncate_chars,
};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SHELL_EXTS: &[&str] = &["sh", "bash", "zsh", "ksh", "dash", "bats"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellcheckFinding {
    pub file: String,
    pub line: u32,
    pub code: String,
    pub level: String,
    pub message: String,
}

fn resolve_shellcheck_bin(config: &ResolvedConfig, cfg: &Value) -> Option<PathBuf> {
    resolve_tool_bin(
        Path::new(&config.repo_root),
        cfg,
        "shellcheck",
        &["--version"],
    )
}

fn shellcheck_code(v: &Value) -> String {
    if let Some(code) = v.as_u64() {
        return format!("SC{code}");
    }
    let raw = v.as_str().unwrap_or("SC0000");
    if raw.starts_with("SC") {
        raw.to_string()
    } else {
        format!("SC{raw}")
    }
}

pub fn parse_shellcheck_output(j: &Value) -> Vec<ShellcheckFinding> {
    let Some(items) = j
        .as_array()
        .or_else(|| j.get("comments").and_then(|c| c.as_array()))
    else {
        return vec![];
    };

    let mut out = Vec::new();
    for item in items {
        let Some(file) = item
            .get("file")
            .and_then(|f| f.as_str())
            .filter(|f| !f.is_empty())
        else {
            continue;
        };
        let code = shellcheck_code(item.get("code").unwrap_or(&Value::Null));
        let message = item
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        if message.is_empty() {
            continue;
        }
        out.push(ShellcheckFinding {
            file: file.to_string(),
            line: item.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32,
            code,
            level: item
                .get("level")
                .and_then(|l| l.as_str())
                .unwrap_or("warning")
                .to_string(),
            message,
        });
    }
    out
}

fn map_level(level: &str) -> &'static str {
    match level {
        "error" => "critical",
        "warning" => "high",
        "info" | "style" => "medium",
        _ => "medium",
    }
}

pub fn shellcheck_violations(findings: &[ShellcheckFinding], repo: &Path) -> Vec<Violation> {
    findings
        .iter()
        .map(|f| {
            let file = repo_relative_path(repo, &f.file);
            Violation {
                id: format!("shellcheck-{}", f.code),
                severity: map_level(&f.level).into(),
                category: "shell".into(),
                file: file.clone(),
                line: f.line,
                full_line: source_line(repo, &file, f.line),
                text: truncate_chars(&format!("{}: {}", f.code, f.message), 90),
                resolution:
                    "Fix the ShellCheck finding; use a narrow shellcheck directive only when intentional."
                        .into(),
                engine: "checker:shellcheck".into(),
            }
        })
        .collect()
}

fn should_skip_dir(config: &ResolvedConfig, name: &str) -> bool {
    name == ".git" || name == "target" || name == "node_modules" || config.skip_dirs.contains(name)
}

fn has_shell_shebang(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let first = contents.lines().next().unwrap_or("");
    first.starts_with("#!")
        && ["sh", "bash", "zsh", "ksh", "dash"]
            .iter()
            .any(|shell| first.contains(shell))
}

fn is_shell_script(repo: &Path, rel: &str) -> bool {
    let path = repo.join(rel);
    if !path.is_file() {
        return false;
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SHELL_EXTS.contains(&e))
    {
        return true;
    }
    has_shell_shebang(&path)
}

fn walk_shell_scripts(config: &ResolvedConfig) -> Vec<String> {
    let repo = Path::new(&config.repo_root);
    let mut files = Vec::new();
    for entry in WalkDir::new(repo).into_iter().filter_entry(|entry| {
        if entry.file_type().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                return !should_skip_dir(config, name);
            }
        }
        true
    }) {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rel) = entry
            .path()
            .strip_prefix(repo)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
        else {
            continue;
        };
        if is_shell_script(repo, &rel) {
            files.push(rel);
        }
    }
    files.sort();
    files
}

fn target_files(config: &ResolvedConfig, opts: CheckerRunOpts<'_>) -> Vec<String> {
    let repo = Path::new(&config.repo_root);
    let raw = match opts.mode {
        "staged" => git_staged_paths(repo),
        "file" => opts.files.unwrap_or(&[]).to_vec(),
        _ => walk_shell_scripts(config),
    };
    raw.into_iter()
        .map(|f| repo_relative_path(repo, &f))
        .filter(|f| is_shell_script(repo, f))
        .collect()
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    if resolve_shellcheck_bin(config, cfg).is_none() {
        return DetectResult {
            available: false,
            reason: Some("no shellcheck binary (configured/local/PATH)".to_string()),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

pub fn run(config: &ResolvedConfig, cfg: &Value, opts: CheckerRunOpts<'_>) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(bin) = resolve_shellcheck_bin(config, cfg) else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec!["no shellcheck binary".to_string()],
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
    let mut args = vec!["--format=json".to_string()];
    if cfg
        .get("externalSources")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        args.push("--external-sources".to_string());
    }
    args.extend(files);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let res = run_tool(&bin, &arg_refs, Some(repo), Some(timeout_ms));
    if !res.ok && res.status.is_none() {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![format!(
                "shellcheck failed: {}",
                res.error.unwrap_or_else(|| "spawn failed".to_string())
            )],
        };
    }

    let stdout = res.stdout.trim();
    if stdout.is_empty() {
        let mut errors = Vec::new();
        if !matches!(res.status, Some(0) | Some(1)) {
            errors.push(format!(
                "shellcheck failed: {}",
                truncate_chars(res.stderr.trim(), 180)
            ));
        }
        return CheckerRunResult {
            violations: vec![],
            errors,
        };
    }

    let data: Value = match serde_json::from_str(stdout) {
        Ok(data) => data,
        Err(e) => {
            return CheckerRunResult {
                violations: vec![],
                errors: vec![format!("shellcheck JSON parse error: {e}")],
            };
        }
    };
    let violations = shellcheck_violations(&parse_shellcheck_output(&data), repo);
    let mut errors = Vec::new();
    if !matches!(res.status, Some(0) | Some(1)) && violations.is_empty() {
        errors.push(format!(
            "shellcheck failed: {}",
            truncate_chars(res.stderr.trim(), 180)
        ));
    }
    CheckerRunResult { violations, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shellcheck_output_supports_json_array() {
        let j: Value = serde_json::from_str(
            r#"[
  { "file": "scripts/deploy.sh", "line": 7, "column": 9, "level": "warning", "code": 2086, "message": "Double quote to prevent globbing and word splitting." },
  { "file": "hooks/commit-hook.sh", "line": 2, "level": "style", "code": "SC2148", "message": "Tips depend on target shell and yours is unknown." }
]"#,
        )
        .unwrap();
        let got = parse_shellcheck_output(&j);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].code, "SC2086");
        assert_eq!(got[1].code, "SC2148");
    }

    #[test]
    fn parse_shellcheck_output_supports_comments_wrapper() {
        let got = parse_shellcheck_output(&serde_json::json!({
            "comments": [{
                "file": "script.sh",
                "line": 3,
                "level": "error",
                "code": 1009,
                "message": "The mentioned syntax error was in this simple command."
            }]
        }));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, "error");
        assert_eq!(got[0].code, "SC1009");
    }

    #[test]
    fn shellcheck_violations_map_levels() {
        let findings = vec![
            ShellcheckFinding {
                file: "script.sh".into(),
                line: 3,
                code: "SC1009".into(),
                level: "error".into(),
                message: "syntax issue".into(),
            },
            ShellcheckFinding {
                file: "script.sh".into(),
                line: 4,
                code: "SC2086".into(),
                level: "warning".into(),
                message: "quote it".into(),
            },
        ];
        let v = shellcheck_violations(&findings, Path::new("/repo"));
        assert_eq!(v[0].severity, "critical");
        assert_eq!(v[1].severity, "high");
        assert_eq!(v[1].id, "shellcheck-SC2086");
    }
}
