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

const MISSING_BIN_MSG: &str = "ast-grep binary not found — AST rules cannot be evaluated";

/// Outcome of [`run_ast_grep_scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepScanResult {
    pub available: bool,
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub inspection: Vec<String>,
}

/// Options for [`run_ast_grep_scan`].
#[derive(Debug, Clone, Default)]
pub struct AstGrepScanOpts {
    /// Allow directory targets for explicitly configured fixture self-tests.
    pub raw_targets: bool,
    /// Total AST execution budget, excluding bounded executable resolution.
    pub timeout_ms: Option<u64>,
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
    if let Some(local) = resolve_local_bin(repo_root) {
        return (Some(local), "local".into());
    }
    let path = path_env
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(if cfg!(windows) {
            "ast-grep.exe"
        } else {
            "ast-grep"
        });
        if !candidate.is_file() {
            continue;
        }
        let output =
            crate::process::run_tool(&candidate, &["--version"], Some(repo_root), Some(2_000));
        if output.ok && output.status == Some(0) {
            return (Some(candidate), "path".into());
        }
    }
    (None, String::new())
}

#[cfg(not(windows))]
fn resolve_local_bin(repo_root: &Path) -> Option<PathBuf> {
    let local = repo_root.join("node_modules/.bin/ast-grep");
    local.exists().then_some(local)
}

/// Windows: `node_modules/.bin/ast-grep` is a POSIX sh shim that CreateProcess
/// rejects (os error 193). Prefer the platform package's `ast-grep.exe` —
/// a sibling of the resolved `@ast-grep/cli` dir under both npm hoisting and
/// pnpm's virtual store — then an explicitly provisioned native `.exe`. Command shims are not executed.
#[cfg(windows)]
fn resolve_local_bin(repo_root: &Path) -> Option<PathBuf> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        "x86" => Some("ia32"),
        _ => None,
    };
    if let Some(arch) = arch {
        let cli = repo_root.join("node_modules/@ast-grep/cli");
        if let Ok(real) = fs::canonicalize(&cli) {
            if let Some(scope) = real.parent() {
                let exe = scope
                    .join(format!("cli-win32-{arch}-msvc"))
                    .join("ast-grep.exe");
                if exe.exists() {
                    return Some(exe);
                }
            }
        }
    }
    let shim = repo_root.join("node_modules/.bin/ast-grep.exe");
    shim.exists().then_some(shim)
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

    if items.len() > crate::protocol::MAX_DIAGNOSTICS {
        return AstGrepParseResult {
            violations: vec![],
            errors: vec!["AST diagnostic count exceeds the 10000 finding limit".into()],
        };
    }
    let mut violations = Vec::new();
    let mut errors = Vec::new();

    for m in items {
        let Some(obj) = m.as_object() else {
            errors.push("AST match is not an object".into());
            continue;
        };

        if obj
            .get("ruleId")
            .and_then(Value::as_str)
            .is_none_or(|value| value.is_empty())
            || obj
                .get("file")
                .and_then(Value::as_str)
                .is_none_or(|value| value.is_empty())
            || obj
                .get("range")
                .and_then(|range| range.get("start"))
                .and_then(|start| start.get("line"))
                .and_then(Value::as_u64)
                .is_none_or(|line| line >= u32::MAX as u64)
        {
            errors.push("AST finding lacks a valid rule, file or location".into());
            continue;
        }
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

        if !matches!(
            severity.as_str(),
            "critical" | "high" | "medium" | "low" | "info"
        ) {
            errors.push(format!("rule {rule_id}: invalid severity {severity:?}"));
            continue;
        }
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
    let failed = |message: String| AstGrepScanResult {
        available: false,
        violations: vec![],
        errors: vec![message],
        warnings: vec![],
        inspection: vec![],
    };
    if !config.ast_enabled {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors: vec![],
            warnings: vec![],
            inspection: vec!["disabled by explicit project configuration".into()],
        };
    }
    for directory in &config.ast_rule_dirs {
        if !Path::new(directory).is_dir() {
            return failed(format!(
                "required AST rule directory is missing: {directory}"
            ));
        }
    }
    if config.ast_rule_dirs.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors: vec![],
            warnings: vec![],
            inspection: vec!["no AST rules configured; no structural coverage".into()],
        };
    }
    // No extension whitelist: the engine reports actual rule/language coverage.
    // Normal gate entry points supply the exact discovery snapshot. Directory
    // targets are retained only for the explicit fixture self-test interface.
    let targets = files.map_or_else(|| config.roots_rel.clone(), <[String]>::to_vec);
    if targets.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors: vec![],
            warnings: vec![],
            inspection: vec!["no AST targets selected".into()],
        };
    }
    if !opts.raw_targets && files.is_some() {
        for target in &targets {
            if !Path::new(&config.repo_root).join(target).is_file() {
                return failed(format!("AST target is not a readable file: {target:?}"));
            }
        }
    }
    let repo_root = Path::new(&config.repo_root);
    let (bin, source) = if let Some(binary) = &config.ast_binary {
        (Some(PathBuf::from(binary)), "configured".into())
    } else {
        resolve_ast_grep_bin_inner(repo_root, opts.path_env.as_deref().map(OsStr::new))
    };
    let Some(bin) = bin else {
        return failed(MISSING_BIN_MSG.to_string());
    };
    let mut result = AstGrepScanResult {
        available: true,
        violations: vec![],
        errors: vec![],
        warnings: vec![],
        inspection: vec![],
    };
    if source == "path" {
        result
            .warnings
            .push("ast-grep executable was resolved from PATH; pin its version in CI setup".into());
    }
    let operation = with_temp_dir_in(temp_base, "slopgate-sg-", |directory| {
        let sg_config = directory.join("sgconfig.yml");
        // JSON strings/arrays are valid YAML and cannot inject configuration via
        // a quote, colon or newline in the rule-directory path.
        let yaml = format!(
            "ruleDirs: {}\n",
            serde_json::to_string(&config.ast_rule_dirs).expect("string array serialization")
        );
        if let Err(error) = fs::write(&sg_config, yaml) {
            return failed(format!("AST configuration: {error}"));
        }
        let mut batches: Vec<Vec<String>> = vec![vec![]];
        let mut bytes = 0usize;
        for target in &targets {
            if target.len() > 12_000 {
                return failed("AST path exceeds argument size budget".into());
            }
            if bytes + target.len() + 1 > 16_000 {
                batches.push(vec![]);
                bytes = 0;
            }
            batches
                .last_mut()
                .expect("nonempty batches")
                .push(target.clone());
            bytes += target.len() + 1;
        }
        let started = std::time::Instant::now();
        for batch in batches {
            let budget = opts.timeout_ms.unwrap_or(60_000);
            let remaining =
                budget.saturating_sub(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            if remaining == 0 {
                result.errors.push(format!(
                    "AST scan exceeded its total {budget} ms execution deadline"
                ));
                break;
            }
            let mut args = vec![
                "scan".to_string(),
                "--config".into(),
                sg_config.to_string_lossy().into_owned(),
                "--json=compact".into(),
                "--inspect=summary".into(),
                "--threads".into(),
                config.checker_concurrency.to_string(),
            ];
            for ignored in ["hidden", "dot", "exclude", "global", "parent", "vcs"] {
                args.push("--no-ignore".into());
                args.push(ignored.into());
            }
            args.push("--".into());
            args.extend(batch);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let output = crate::process::run_tool(&bin, &refs, Some(repo_root), Some(remaining));
            if !output.ok {
                result.errors.push(format!(
                    "AST execution incomplete: {}",
                    output.error.unwrap_or_default()
                ));
                break;
            }
            if !matches!(output.status, Some(0 | 1)) {
                result.errors.push(format!(
                    "AST execution failed with exit {:?}: {}",
                    output.status,
                    output.stderr.chars().take(1000).collect::<String>()
                ));
                break;
            }
            let value: Value = match serde_json::from_str(output.stdout.trim()) {
                Ok(value) => value,
                Err(error) => {
                    result
                        .errors
                        .push(format!("AST emitted malformed JSON: {error}"));
                    break;
                }
            };
            let parsed = parse_ast_grep_json(&value);
            if output.status == Some(1) && parsed.violations.is_empty() {
                result
                    .errors
                    .push("AST reported rule errors but returned no findings".into());
            }
            result.violations.extend(parsed.violations);
            result.errors.extend(parsed.errors);
            for line in output.stderr.lines().filter(|line| !line.trim().is_empty()) {
                if line.starts_with("sg: ") {
                    result.inspection.push(line.to_string());
                } else if line.starts_with("Error:") && line.contains("error(s) found in code") { /* a successful scan with findings */
                } else if line.starts_with("Help: Scan succeeded") { /* explanatory success diagnostic */
                } else {
                    result.errors.push(format!("AST diagnostic: {line}"));
                }
            }
            if !result.errors.is_empty() {
                break;
            }
        }
        result
            .violations
            .sort_by(|a, b| (&a.file, a.line, &a.id).cmp(&(&b.file, b.line, &b.id)));
        result
    });
    match operation {
        Ok(result) => result,
        Err(error) => failed(format!("AST temporary workspace: {error}")),
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

    /// Write a runnable `ast-grep` stub that prints `stdout`: `.cmd` batch on
    /// Windows (spawnable), executable sh script elsewhere.
    fn write_stub(bin_dir: &Path, stdout: &str) -> PathBuf {
        #[cfg(windows)]
        {
            // The production core deliberately rejects .cmd scripts. Exercise
            // the same native executable boundary here instead of bypassing it.
            let stub = bin_dir.join("ast-grep.exe");
            let source = bin_dir.join("ast_fixture.rs");
            fs::write(
                &source,
                format!("fn main() {{ println!(\"{{}}\", {stdout:?}); }}"),
            )
            .unwrap();
            let compiled = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-name", "slopgate_ast_fixture"])
                .arg(&source)
                .arg("-o")
                .arg(&stub)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "native AST fixture compilation: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            stub
        }
        #[cfg(not(windows))]
        {
            let stub = bin_dir.join("ast-grep");
            fs::write(&stub, format!("#!/bin/sh\necho '{stdout}'\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
            stub
        }
    }

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
        let stub = write_stub(&bin_dir, "[]");

        let (bin, source) = resolve_ast_grep_bin(dir.path());
        assert_eq!(bin, Some(stub));
        assert_eq!(source, "local");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_prefers_platform_exe_on_windows() {
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            _ => return,
        };
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        write_stub(&bin_dir, "[]");

        let scope = dir.path().join("node_modules/@ast-grep");
        fs::create_dir_all(scope.join("cli")).unwrap();
        let exe_dir = scope.join(format!("cli-win32-{arch}-msvc"));
        fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("ast-grep.exe");
        fs::write(&exe, "").unwrap();

        let (bin, source) = resolve_ast_grep_bin(dir.path());
        assert_eq!(bin, Some(fs::canonicalize(&exe).unwrap()));
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
            roots_rel: vec!["src".into()],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
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
            ..Default::default()
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
        assert!(got.errors.is_empty(), "AST errors: {:?}", got.errors);
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
        assert!(got.errors.is_empty(), "AST errors: {:?}", got.errors);
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
            "range": {"start": {"line": 0}},
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
    fn run_scan_missing_rule_directory_is_incomplete() {
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
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
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
        assert!(!got.available);
        assert!(got.violations.is_empty());
        assert!(got
            .errors
            .iter()
            .any(|error| error.contains("rule directory is missing")));
    }

    #[test]
    fn run_scan_temp_dir_failure_unavailable() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        write_stub(&bin_dir, "[]");

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
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
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
        assert!(got.errors[0].starts_with("AST temporary workspace:"));
    }

    #[test]
    fn run_scan_arbitrary_extensions_are_forwarded() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        write_stub(&bin_dir, "[]");

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
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
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

        fs::write(dir.path().join("README.md"), "arbitrary configured source").unwrap();
        let files = vec!["README.md".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty(), "AST errors: {:?}", got.errors);
    }

    #[test]
    fn run_scan_test_files_are_scanned() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        write_stub(&bin_dir, "[]");

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
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
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

        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/example.test.ts"), "test source").unwrap();
        fs::write(dir.path().join("src/example.test.tsx"), "test source").unwrap();
        let files = vec![
            "src/example.test.ts".to_string(),
            "src/example.test.tsx".to_string(),
        ];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available, "ast scan unavailable: {:?}", got.errors);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty(), "AST errors: {:?}", got.errors);
    }
}
