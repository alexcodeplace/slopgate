//! ast-grep engine wrapper (bucket-B structural rules).
//! Mirrors `src/ast-engine.mjs`: resolve local/PATH binary, spawn scan, map JSON → violations.
//! Missing binary → `available: false` + reason — never panics.

use crate::config::ResolvedConfig;
use crate::report::Violation;
use crate::temp::with_temp_dir_in;
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MISSING_BIN_MSG: &str =
    "ast-grep binary not found (npm i -g @ast-grep/cli) — bucket-B rules SKIPPED";

/// Outcome of [`run_ast_grep_scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepScanResult {
    pub available: bool,
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Options for [`run_ast_grep_scan`].
#[derive(Debug, Clone, Default)]
pub struct AstGrepScanOpts {
    /// When true, use `files` as-is; otherwise keep only `.ts` / `.tsx`.
    pub raw_targets: bool,
    /// Overrides `PATH` for ast-grep resolution only (unit tests).
    #[doc(hidden)]
    pub path_env: Option<String>,
}

/// Result of parsing ast-grep JSON stdout (array of match objects).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepParseResult {
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Resolve `node_modules/.bin/ast-grep` under `repo_root`, then `ast-grep` on PATH.
/// Returns `(None, "")` when neither is available.
pub fn resolve_ast_grep_bin(repo_root: &Path) -> (Option<PathBuf>, String) {
    resolve_ast_grep_bin_inner(repo_root, None)
}

fn resolve_ast_grep_bin_inner(
    repo_root: &Path,
    path_env: Option<&OsStr>,
) -> (Option<PathBuf>, String) {
    let local = repo_root.join("node_modules/.bin/ast-grep");
    if local.exists() {
        return (Some(local), "local".to_string());
    }

    let mut cmd = Command::new("ast-grep");
    cmd.arg("--version");
    if let Some(path) = path_env {
        cmd.env("PATH", path);
    }

    match cmd.output() {
        Ok(output) if output.status.success() => {
            (Some(PathBuf::from("ast-grep")), "path".to_string())
        }
        _ => (None, String::new()),
    }
}

/// Map ast-grep `--json` match array to engine violations (`engine: "ast"`).
/// Unit-testable without spawning a binary.
pub fn parse_ast_grep_json(matches: &Value) -> AstGrepParseResult {
    let Some(items) = matches.as_array() else {
        return AstGrepParseResult {
            violations: vec![],
            errors: vec!["ast-grep output was not an array".to_string()],
        };
    };

    let mut violations = Vec::new();
    let mut errors = Vec::new();

    for m in items {
        let Some(obj) = m.as_object() else {
            continue;
        };

        let rule_id = obj
            .get("ruleId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let meta = match obj.get("note").and_then(|n| n.as_str()).unwrap_or("{}") {
            "" => Value::Object(serde_json::Map::new()),
            note => match serde_json::from_str::<Value>(note) {
                Ok(v) => v,
                Err(_) => {
                    errors.push(format!("rule {rule_id}: note is not valid JSON"));
                    Value::Object(serde_json::Map::new())
                }
            },
        };

        let lines = obj.get("lines").and_then(|l| l.as_str()).unwrap_or("");
        let first_line = lines.split('\n').next().unwrap_or("");

        let ast_severity = obj.get("severity").and_then(|s| s.as_str());
        let severity = meta
            .get("severity")
            .and_then(|s| s.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if ast_severity == Some("error") {
                    "high".to_string()
                } else {
                    "medium".to_string()
                }
            });

        let category = meta
            .get("category")
            .and_then(|c| c.as_str())
            .unwrap_or("convention")
            .to_string();

        let line = obj
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32
            + 1;

        let file = obj
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or("")
            .to_string();

        let message = obj
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        let resolution = meta
            .get("resolution")
            .and_then(|r| r.as_str())
            .map(str::to_string)
            .unwrap_or(message);

        violations.push(Violation {
            id: rule_id,
            severity,
            category,
            file,
            line,
            full_line: first_line.to_string(),
            text: truncate_chars(first_line.trim(), 90),
            resolution,
            engine: "ast".to_string(),
        });
    }

    AstGrepParseResult { violations, errors }
}

/// Run ast-grep against project rule dirs and map findings to violations. Never panics.
pub fn run_ast_grep_scan(
    config: &ResolvedConfig,
    files: Option<&[String]>,
    opts: &AstGrepScanOpts,
) -> AstGrepScanResult {
    run_ast_grep_scan_in(config, files, opts, std::env::temp_dir())
}

fn run_ast_grep_scan_in(
    config: &ResolvedConfig,
    files: Option<&[String]>,
    opts: &AstGrepScanOpts,
    temp_base: impl AsRef<Path>,
) -> AstGrepScanResult {
    let rule_dirs: Vec<&str> = config
        .ast_rule_dirs
        .iter()
        .filter(|d| Path::new(d).exists())
        .map(String::as_str)
        .collect();

    if rule_dirs.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors: vec![],
        };
    }

    let repo_root = Path::new(&config.repo_root);
    let path_env = opts.path_env.as_deref().map(OsStr::new);
    let (bin, source) = resolve_ast_grep_bin_inner(repo_root, path_env);

    let Some(bin) = bin else {
        return AstGrepScanResult {
            available: false,
            violations: vec![],
            errors: vec![MISSING_BIN_MSG.to_string()],
        };
    };

    let mut errors = Vec::new();
    if source == "path" {
        errors.push(
            "ast-grep: using PATH binary (version not pinned — results may differ from CI)"
                .to_string(),
        );
    }

    let targets: Vec<String> = match files {
        None => config.roots_rel.clone(),
        Some(files) => {
            if opts.raw_targets {
                files.to_vec()
            } else {
                files
                    .iter()
                    .filter(|f| f.ends_with(".ts") || f.ends_with(".tsx"))
                    .cloned()
                    .collect()
            }
        }
    };

    if files.is_some() && targets.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors,
        };
    }

    let scan = with_temp_dir_in(temp_base, "slopgate-sg-", |dir| {
        let sg_config = dir.join("sgconfig.yml");
        let yml = format!(
            "ruleDirs:\n{}\n",
            rule_dirs
                .iter()
                .map(|d| format!("  - {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if let Err(e) = fs::write(&sg_config, yml) {
            return AstGrepScanResult {
                available: false,
                violations: vec![],
                errors: vec![format!("ast-grep failed: {e}")],
            };
        }

        let mut args: Vec<String> = vec![
            "scan".into(),
            "--config".into(),
            sg_config.to_string_lossy().into_owned(),
            "--json".into(),
        ];
        args.extend(targets);

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = Command::new(&bin);
        cmd.args(&arg_refs).current_dir(repo_root);

        // PHASE-2: subprocess timeout (60s, mirrors ast-engine.mjs spawnSync timeout)
        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                return AstGrepScanResult {
                    available: false,
                    violations: vec![],
                    errors: vec![format!("ast-grep failed: {e}")],
                };
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let parsed: Value = match serde_json::from_str(stdout.trim()) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("ast-grep JSON parse error: {e}"));
                return AstGrepScanResult {
                    available: true,
                    violations: vec![],
                    errors,
                };
            }
        };

        let AstGrepParseResult {
            violations,
            errors: mut parse_errors,
        } = parse_ast_grep_json(&parsed);
        errors.append(&mut parse_errors);

        if !stderr.is_empty()
            && stderr.to_ascii_lowercase().contains("error")
            && !stderr.contains("error(s) found in code")
        {
            let cap = stderr.chars().take(500).collect::<String>();
            errors.push(format!("ast-grep stderr: {cap}"));
        }

        AstGrepScanResult {
            available: true,
            violations,
            errors,
        }
    });

    match scan {
        Ok(result) => result,
        Err(e) => AstGrepScanResult {
            available: false,
            violations: vec![],
            errors: vec![format!("ast-grep failed: {e}")],
        },
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_none_when_absent() {
        let dir = TempDir::new().unwrap();
        let (bin, source) =
            resolve_ast_grep_bin_inner(dir.path(), Some(OsStr::new("/nonexistent")));
        assert!(bin.is_none());
        assert!(source.is_empty());
    }

    #[test]
    fn resolve_some_when_stub_bin_exists() {
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let stub = bin_dir.join("ast-grep");
        fs::write(&stub, "#!/bin/sh\n").unwrap();

        let (bin, source) = resolve_ast_grep_bin(dir.path());
        assert_eq!(bin, Some(stub));
        assert_eq!(source, "local");
    }

    #[test]
    fn run_scan_no_binary_unavailable() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let opts = AstGrepScanOpts {
            raw_targets: false,
            path_env: Some("/nonexistent".to_string()),
        };
        let got = run_ast_grep_scan(&config, None, &opts);
        assert!(!got.available);
        assert!(got.violations.is_empty());
        assert_eq!(got.errors, vec![MISSING_BIN_MSG.to_string()]);
    }

    #[test]
    fn parse_ast_grep_json_maps_canned_match() {
        let json = json!([{
            "ruleId": "no-console",
            "severity": "error",
            "file": "src/app.ts",
            "lines": "  console.log('x')\n",
            "message": "Avoid console",
            "range": { "start": { "line": 4 } },
            "note": "{\"severity\":\"critical\",\"category\":\"hygiene\",\"resolution\":\"Remove console\"}"
        }]);

        let got = parse_ast_grep_json(&json);
        assert!(got.errors.is_empty());
        assert_eq!(got.violations.len(), 1);

        let v = &got.violations[0];
        assert_eq!(v.id, "no-console");
        assert_eq!(v.severity, "critical");
        assert_eq!(v.category, "hygiene");
        assert_eq!(v.file, "src/app.ts");
        assert_eq!(v.line, 5);
        assert_eq!(v.full_line, "  console.log('x')");
        assert_eq!(v.text, "console.log('x')");
        assert_eq!(v.resolution, "Remove console");
        assert_eq!(v.engine, "ast");
    }

    #[test]
    fn parse_ast_grep_json_defaults_when_note_missing() {
        let json = json!([{
            "ruleId": "bare-rule",
            "severity": "warning",
            "file": "x.tsx",
            "lines": "foo();\n",
            "message": "fix me",
            "range": { "start": { "line": 0 } }
        }]);

        let got = parse_ast_grep_json(&json);
        assert!(got.errors.is_empty());
        assert_eq!(got.violations.len(), 1);
        assert_eq!(got.violations[0].severity, "medium");
        assert_eq!(got.violations[0].category, "convention");
        assert_eq!(got.violations[0].line, 1);
        assert_eq!(got.violations[0].resolution, "fix me");
    }

    #[test]
    fn parse_ast_grep_json_invalid_note_is_error_not_panic() {
        let json = json!([{
            "ruleId": "bad-note",
            "file": "a.ts",
            "lines": "x",
            "note": "not-json"
        }]);

        let got = parse_ast_grep_json(&json);
        assert_eq!(got.violations.len(), 1);
        assert_eq!(got.violations[0].id, "bad-note");
        assert!(got
            .errors
            .iter()
            .any(|e| e.contains("bad-note") && e.contains("note")));
    }

    #[test]
    fn parse_ast_grep_json_non_array_reports_error() {
        let got = parse_ast_grep_json(&json!({ "oops": true }));
        assert!(got.violations.is_empty());
        assert_eq!(
            got.errors,
            vec!["ast-grep output was not an array".to_string()]
        );
    }

    #[test]
    fn run_scan_empty_rule_dirs_available_noop() {
        let dir = TempDir::new().unwrap();
        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![dir
                .path()
                .join("missing-ast-rules")
                .to_string_lossy()
                .into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let got = run_ast_grep_scan(&config, None, &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty());
    }

    #[test]
    fn run_scan_temp_dir_failure_unavailable() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("ast-grep"), "#!/bin/sh\n").unwrap();

        let not_a_dir = dir.path().join("blocking-tmp");
        fs::write(&not_a_dir, "x").unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec!["src".to_string()],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let got = run_ast_grep_scan_in(&config, None, &AstGrepScanOpts::default(), &not_a_dir);

        assert!(!got.available);
        assert!(got.violations.is_empty());
        assert_eq!(got.errors.len(), 1);
        assert!(got.errors[0].starts_with("ast-grep failed:"));
    }

    #[test]
    fn run_scan_non_ts_files_filtered_to_empty() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("ast-grep"), "#!/bin/sh\n").unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let files = vec!["README.md".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty());
    }

    #[test]
    fn run_scan_test_files_are_scanned() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let stub = bin_dir.join("ast-grep");
        fs::write(&stub, "#!/bin/sh\necho '[]'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let files = vec!["src/example.test.ts".to_string(), "src/example.test.tsx".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty());
    }
}
