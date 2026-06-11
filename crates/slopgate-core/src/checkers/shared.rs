//! Shared checker plumbing. Mirrors `src/checkers/shared.mjs`: local-bin resolution,
//! subprocess wrapper (never panics), source-line lookup, cache dir, and the
//! leakscan-style JSON checker seam (spawn + parse split for unit tests).

use crate::report::Violation;
use crate::severity::map_passthrough;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of `run_tool` — mirrors the JS `{ ok, error, stdout, stderr, status }` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOut {
    pub ok: bool,
    pub error: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub status: Option<i32>,
}

/// Result of `run_json_tool` — never panics; errors collected in `errors`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonToolResult {
    pub data: Option<Value>,
    pub errors: Vec<String>,
}

/// Mapping options for checker JSON → [`Violation`] (leakscan / native-binary adapters).
#[derive(Debug, Clone, Copy)]
pub struct CheckerMapConfig<'a> {
    /// Checker id used as violation id prefix (`{checker_id}-{rule}`).
    pub checker_id: &'a str,
    pub category: &'a str,
    pub resolution: &'a str,
}

/// Outcome of `run_checker_json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerRunResult {
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Resolve `node_modules/.bin/<name>` under `repo_root` when present.
pub fn local_bin(repo_root: &Path, name: &str) -> Option<PathBuf> {
    let p = repo_root.join("node_modules").join(".bin").join(name);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Per-repo slopgate cache dir (`<config_dir>/cache`). Self-gitignoring.
pub fn ensure_cache_dir(config_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = config_dir.join("cache");
    fs::create_dir_all(&dir)?;
    let gi = dir.join(".gitignore");
    if !gi.exists() {
        fs::write(&gi, "*\n")?;
    }
    Ok(dir)
}

/// Spawn a subprocess and capture stdout/stderr. Never panics.
///
/// `timeout_ms` is accepted for API stability; enforcement is deferred:
/// // PHASE-2: subprocess timeout via `wait_timeout` (long-running checkers).
pub fn run_tool(
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    _timeout_ms: Option<u64>,
) -> ToolOut {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }

    match cmd.output() {
        Ok(output) => ToolOut {
            ok: true,
            error: None,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            status: output.status.code(),
        },
        Err(e) => ToolOut {
            ok: false,
            error: Some(e.to_string()),
            stdout: String::new(),
            stderr: String::new(),
            status: None,
        },
    }
}

/// Run a tool that emits JSON on stdout. Never panics.
pub fn run_json_tool(
    label: &str,
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: Option<u64>,
) -> JsonToolResult {
    let res = run_tool(bin, args, cwd, timeout_ms);
    if !res.ok {
        let detail = res.error.unwrap_or_else(|| "spawn failed".to_string());
        return JsonToolResult {
            data: None,
            errors: vec![format!("{label} failed: {detail}")],
        };
    }
    match serde_json::from_str(&res.stdout) {
        Ok(data) => JsonToolResult {
            data: Some(data),
            errors: vec![],
        },
        Err(e) => JsonToolResult {
            data: None,
            errors: vec![format!("{label} JSON parse error: {e}")],
        },
    }
}

/// Read a 1-based source line; returns empty on missing file or out-of-range line.
pub fn source_line(repo_root: &Path, file: &str, line: u32) -> String {
    let path = repo_root.join(file);
    let Ok(content) = fs::read_to_string(&path) else {
        return String::new();
    };
    if line == 0 {
        return String::new();
    }
    content
        .lines()
        .nth((line - 1) as usize)
        .unwrap_or("")
        .to_string()
}

/// Map checker JSON (`violations[]` with `file`, `line`, `rule`, `severity`, …) to engine violations.
/// Unit-testable without spawning a binary (mirrors `leakscanViolations` / `parseDepcruiseOutput` split).
pub fn parse_checker_json(value: &Value, cfg: &CheckerMapConfig<'_>) -> Vec<Violation> {
    let Some(items) = value.get("violations").and_then(|v| v.as_array()) else {
        return vec![];
    };

    let mut out = Vec::new();
    for v in items {
        let Some(file) = v.get("file").and_then(|f| f.as_str()).filter(|f| !f.is_empty()) else {
            continue;
        };
        let raw_sev = v
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("critical");
        let Some(severity) = map_passthrough(raw_sev) else {
            continue;
        };
        let rule = v
            .get("rule")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32;
        let snippet = v
            .get("snippet")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let text_src = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or(rule);
        let text = truncate_chars(text_src, 90);

        out.push(Violation {
            id: format!("{}-{}", cfg.checker_id, rule),
            severity: severity.to_string(),
            category: cfg.category.to_string(),
            file: file.to_string(),
            line,
            full_line: snippet.to_string(),
            text,
            resolution: cfg.resolution.to_string(),
            engine: format!("checker:{}", cfg.checker_id),
        });
    }
    out
}

/// Spawn a checker binary, parse JSON stdout, map to violations. Never panics.
pub fn run_checker_json(
    label: &str,
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_ms: Option<u64>,
    map_cfg: &CheckerMapConfig<'_>,
) -> CheckerRunResult {
    if !bin.exists() {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![format!("no {label} binary")],
        };
    }

    let JsonToolResult { data, mut errors } =
        run_json_tool(label, bin, args, Some(cwd), timeout_ms);

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
        violations: parse_checker_json(&data, map_cfg),
        errors,
    }
}

/// Truncate to `max` chars (ASCII fixtures match JS `.slice(0, 90)`).
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Bounded-concurrency map; preserves input order in the result vector.
/// Mirrors `shared.mjs` `mapLimit` — `std::thread::scope` worker pool when `limit > 1`.
pub fn map_limit<T, R, F>(items: &[T], limit: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    if items.is_empty() {
        return vec![];
    }
    let workers = limit.max(1).min(items.len());
    if workers == 1 {
        return items.iter().map(f).collect();
    }

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    slots.lock().unwrap()[i] = Some(f(&items[i]));
                }
            });
        }
    });

    slots
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|s| s.expect("map_limit worker slot"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const LEAKSCAN_MAP: CheckerMapConfig<'static> = CheckerMapConfig {
        checker_id: "leakscan",
        category: "boundary",
        resolution: "Route I/O through a service layer / API client.",
    };

    #[test]
    fn run_tool_captures_echo_stdout() {
        let out = run_tool(Path::new("echo"), &["hello-shared"], None, None);
        assert!(out.ok, "echo should spawn: {:?}", out.error);
        assert!(out.stdout.contains("hello-shared"));
        assert!(out.error.is_none());
    }

    #[test]
    fn run_tool_missing_binary_is_not_ok() {
        let out = run_tool(
            Path::new("/nonexistent/slopgate-checker-binary-xyz"),
            &[],
            None,
            None,
        );
        assert!(!out.ok);
        assert!(out.error.is_some());
        assert!(out.status.is_none());
    }

    #[test]
    fn local_bin_none_when_absent() {
        let dir = TempDir::new().unwrap();
        assert!(local_bin(dir.path(), "depcruise").is_none());
    }

    #[test]
    fn local_bin_some_when_stub_exists() {
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join("node_modules").join(".bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let stub = bin_dir.join("depcruise");
        fs::write(&stub, "#!/bin/sh\n").unwrap();
        assert_eq!(local_bin(dir.path(), "depcruise"), Some(stub));
    }

    #[test]
    fn source_line_reads_one_based_line() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.ts"), "first\nsecond\nthird\n").unwrap();
        assert_eq!(source_line(dir.path(), "a.ts", 2), "second");
    }

    #[test]
    fn source_line_empty_on_out_of_range() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.ts"), "only\n").unwrap();
        assert_eq!(source_line(dir.path(), "a.ts", 99), "");
        assert_eq!(source_line(dir.path(), "missing.ts", 1), "");
    }

    #[test]
    fn ensure_cache_dir_creates_gitignore() {
        let dir = TempDir::new().unwrap();
        let cache = ensure_cache_dir(dir.path()).unwrap();
        assert!(cache.is_dir());
        let gi = cache.join(".gitignore");
        assert!(gi.is_file());
        assert_eq!(fs::read_to_string(&gi).unwrap(), "*\n");
    }

    #[test]
    fn parse_checker_json_maps_canned_report() {
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
        let v = parse_checker_json(&json, &LEAKSCAN_MAP);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "leakscan-global-fetch");
        assert_eq!(v[0].severity, "high");
        assert_eq!(v[0].file, "src/UserCard.tsx");
        assert_eq!(v[0].line, 12);
        assert_eq!(v[0].full_line, "  fetch(url)");
        assert_eq!(v[0].text, "Direct fetch in component");
        assert_eq!(v[0].category, "boundary");
    }

    #[test]
    fn parse_checker_json_drops_unknown_severity_and_missing_file() {
        let json: Value = serde_json::json!({
            "violations": [
                { "file": "", "rule": "x", "severity": "high" },
                { "file": "a.ts", "rule": "y", "severity": "bogus" },
                { "file": "b.ts", "rule": "z", "severity": "medium" }
            ]
        });
        let v = parse_checker_json(&json, &LEAKSCAN_MAP);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "leakscan-z");
    }

    #[test]
    fn run_checker_json_missing_binary_graceful_error() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("no-such-checker");
        let got = run_checker_json(
            "leakscan",
            &missing,
            &[],
            dir.path(),
            None,
            &LEAKSCAN_MAP,
        );
        assert!(got.violations.is_empty());
        assert_eq!(got.errors, vec!["no leakscan binary"]);
    }

    #[test]
    fn map_limit_preserves_input_order() {
        let items: Vec<i32> = (0..10).collect();
        let got: Vec<i32> = map_limit(&items, 3, |&x| x * 2);
        assert_eq!(got, (0..10).map(|x| x * 2).collect::<Vec<_>>());
    }

    #[test]
    fn map_limit_respects_concurrency_cap() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let items: Vec<u32> = (0..12).collect();
        let active_c = Arc::clone(&active);
        let peak_c = Arc::clone(&peak);
        let _ = map_limit(&items, 3, move |_| {
            let now = active_c.fetch_add(1, Ordering::SeqCst) + 1;
            peak_c.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            active_c.fetch_sub(1, Ordering::SeqCst);
            0
        });
        assert!(peak.load(Ordering::SeqCst) <= 3, "peak concurrency was {}", peak.load(Ordering::SeqCst));
    }

    #[test]
    fn map_limit_panic_in_task_propagates_from_worker() {
        let items = [1, 2, 3];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map_limit(&items, 2, |&x| {
                if x == 2 {
                    panic!("boom");
                }
                x
            })
        }));
        assert!(result.is_err());
    }

    #[test]
    fn run_json_tool_invalid_json_reports_parse_error() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("bad-json.sh");
        fs::write(&script, "#!/bin/sh\necho 'not json'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let got = run_json_tool("test-tool", &script, &[], Some(dir.path()), None);
        assert!(got.data.is_none());
        assert!(got.errors.iter().any(|e| e.contains("JSON parse error")));
    }
}
