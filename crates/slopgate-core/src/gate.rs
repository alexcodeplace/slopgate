//! Gate conductor — mirrors `src/gate.mjs` (`collectViolations`, `applyGateFilters`, `runGate`, `snapshotViolations`).

use crate::ast_engine::{run_ast_grep_scan, AstGrepScanOpts};
use crate::checkers::health::{is_infra_error, update_checker_health, CheckerOutcome};
use crate::checkers::index::{Checker, CHECKERS};
use crate::checkers::shared::{emit_stage_progress, ensure_cache_dir, map_limit};
use crate::config::ResolvedConfig;
use crate::enumerate::{list_source_files, EnumerateCtx, EnumerateMode};
use crate::hash::line_hash;
use crate::ratchet::{filter_new, load_baseline, staged_renames};
use crate::regex_engine::scan_regex;
use crate::report::{print_gate_report_to, Violation};
use crate::suppressions::{is_suppressed, load_suppressions, SuppressionViolation};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

/// Gate scan scope — mirrors JS `'file'|'staged'|'full'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    File,
    Staged,
    Full,
}

/// Checker tier — mirrors JS `'fast'|'commit'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fast,
    Commit,
}

/// Result of [`collect_violations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectResult {
    pub violations: Vec<Violation>,
    pub notices: Vec<String>,
    pub fatal: bool,
}

/// Result of [`run_gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub violations: Vec<Violation>,
    pub code: i32,
}

/// Result of [`snapshot_violations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotResult {
    Violations(Vec<Violation>),
    Fatal,
}

/// Stderr sink for gate machine-surface output (testable).
pub struct GateStderr<'a> {
    pub writer: &'a mut dyn Write,
}

impl GateStderr<'_> {
    fn writeln(&mut self, line: &str) {
        let _ = writeln!(self.writer, "{line}");
    }

    fn notice(&mut self, msg: &str) {
        self.writeln(&format!("⚠ SLOPGATE: {msg}"));
    }

    fn warning(&mut self, msg: &str) {
        self.notice(msg);
    }
}

fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::File => "file",
        Mode::Staged => "staged",
        Mode::Full => "full",
    }
}

fn enumerate_ctx(config: &ResolvedConfig) -> EnumerateCtx {
    EnumerateCtx {
        repo_root: Path::new(&config.repo_root).to_path_buf(),
        roots: config
            .roots
            .iter()
            .map(Path::new)
            .map(Path::to_path_buf)
            .collect(),
        roots_rel: config.roots_rel.clone(),
        exts: config.exts.clone(),
        skip_dirs: config.skip_dirs.clone(),
    }
}

fn gate_allow(config: &ResolvedConfig, mode: Mode) -> &HashSet<String> {
    match mode {
        Mode::File => &config.gate.file,
        Mode::Staged | Mode::Full => &config.gate.staged,
    }
}

fn iso_now() -> String {
    use std::process::Command;
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%S.000Z"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string())
}

fn push_ast_violations(config: &ResolvedConfig, violations: &mut Vec<Violation>, ast_v: Violation) {
    if config.ast_disable.contains(&ast_v.id) {
        return;
    }
    if config.ux_ast_all.contains(&ast_v.id) {
        let Some(sev) = config.ux_ast_severity.get(&ast_v.id) else {
            return;
        };
        violations.push(Violation {
            severity: sev.clone(),
            ..ast_v
        });
        return;
    }
    violations.push(ast_v);
}

struct EligibleChecker<'a> {
    checker: &'static Checker,
    cfg: &'a Value,
}

struct CheckerRunItemResult {
    id: String,
    res: crate::checkers::index::CheckerRunResult,
    seconds: f64,
}

/// Collect raw violations (no suppressions / severity / ratchet filtering).
pub fn collect_violations(
    mode: Mode,
    config: &ResolvedConfig,
    tier: Tier,
    file_target: Option<&str>,
) -> CollectResult {
    let ctx = enumerate_ctx(config);
    let discovery_started = Instant::now();
    emit_stage_progress("discovery", "start", None);
    let enumeration = match mode {
        Mode::Staged => list_source_files(&ctx, EnumerateMode::Staged),
        Mode::File => list_source_files(&ctx, EnumerateMode::File(file_target.unwrap_or(""))),
        Mode::Full => list_source_files(&ctx, EnumerateMode::Walk),
    };
    let files = match enumeration {
        Ok(files) => files,
        Err(error) => {
            emit_stage_progress(
                "discovery",
                "end",
                Some(discovery_started.elapsed().as_millis()),
            );
            return CollectResult {
                violations: vec![],
                notices: vec![format!("source enumeration failed: {error}")],
                fatal: true,
            };
        }
    };
    emit_stage_progress(
        "discovery",
        "end",
        Some(discovery_started.elapsed().as_millis()),
    );

    let mut notices = Vec::new();
    let mut fatal = false;
    if files.is_empty() && mode != Mode::Full {
        return CollectResult {
            violations: vec![],
            notices,
            fatal,
        };
    }

    let regex_started = Instant::now();
    emit_stage_progress("regex", "start", None);
    let mut violations = scan_regex(config, &files, mode == Mode::File);
    emit_stage_progress("regex", "end", Some(regex_started.elapsed().as_millis()));

    let ast_files = if mode == Mode::Full {
        None
    } else {
        Some(files.as_slice())
    };
    let ast_started = Instant::now();
    emit_stage_progress("ast", "start", None);
    let ast = run_ast_grep_scan(config, ast_files, &AstGrepScanOpts::default());
    emit_stage_progress("ast", "end", Some(ast_started.elapsed().as_millis()));
    if !ast.available {
        fatal = true;
        notices.push(ast.errors.join("; "));
    } else {
        for e in &ast.errors {
            if !e.starts_with("ast-grep: using PATH") {
                fatal = true;
            }
            notices.push(format!("ast-grep: {e}"));
        }
    }
    for v in ast.violations {
        push_ast_violations(config, &mut violations, v);
    }

    if tier == Tier::Commit {
        let mut eligible: Vec<EligibleChecker<'_>> = Vec::new();
        let mut outcomes: Vec<CheckerOutcome> = Vec::new();

        for checker in CHECKERS {
            let Some(cfg) = config.checkers.get(checker.id) else {
                continue;
            };
            let det = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (checker.detect)(config, cfg)
            })) {
                Ok(det) => det,
                Err(payload) => {
                    let msg = panic_payload_str(payload);
                    notices.push(format!("{} detect crashed: {msg}", checker.id));
                    outcomes.push(CheckerOutcome {
                        id: checker.id.to_string(),
                        infra_failed: true,
                        detail: Some(format!("detect crashed: {msg}")),
                        seconds: None,
                    });
                    continue;
                }
            };
            if !det.available {
                let reason = det.reason.unwrap_or_else(|| "unavailable".to_string());
                notices.push(format!("skipped: {} ({reason})", checker.id));
                outcomes.push(CheckerOutcome {
                    id: checker.id.to_string(),
                    infra_failed: true,
                    detail: Some(format!("skipped: {reason}")),
                    seconds: None,
                });
                continue;
            }
            eligible.push(EligibleChecker { checker, cfg });
        }

        let started = Instant::now();
        emit_stage_progress("checkers", "start", None);
        let mode_label = mode_str(mode);
        let results: Vec<CheckerRunItemResult> =
            map_limit(&eligible, config.checker_concurrency as usize, |item| {
                let t0 = Instant::now();
                emit_stage_progress(item.checker.id, "start", None);
                let res = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (item.checker.run)(
                        config,
                        item.cfg,
                        crate::checkers::index::CheckerRunOpts {
                            files: if mode == Mode::Full {
                                None
                            } else {
                                Some(&files)
                            },
                            mode: mode_label,
                        },
                    )
                })) {
                    Ok(res) => res,
                    Err(payload) => {
                        let msg = panic_payload_str(payload);
                        crate::checkers::index::CheckerRunResult {
                            violations: vec![],
                            errors: vec![format!("{} crashed: {msg}", item.checker.id)],
                        }
                    }
                };
                let seconds = t0.elapsed().as_secs_f64();
                emit_stage_progress(item.checker.id, "end", Some(t0.elapsed().as_millis()));
                CheckerRunItemResult {
                    id: item.checker.id.to_string(),
                    res,
                    seconds,
                }
            });
        let elapsed = started.elapsed().as_secs_f64();
        emit_stage_progress("checkers", "end", Some(started.elapsed().as_millis()));
        if elapsed > 30.0 {
            notices.push(format!(
                "commit-tier checkers took {:.0}s (budget ~30s) — check tsc incremental cache / disable slow checkers",
                elapsed
            ));
        }

        let aggregation_started = Instant::now();
        emit_stage_progress("aggregation", "start", None);
        for item in results {
            for e in &item.res.errors {
                notices.push(format!("{}: {e}", item.id));
            }
            outcomes.push(CheckerOutcome {
                id: item.id.clone(),
                infra_failed: item.res.errors.iter().any(|e| is_infra_error(e)),
                detail: item.res.errors.iter().find(|e| is_infra_error(e)).cloned(),
                seconds: Some(item.seconds),
            });
            for v in item.res.violations {
                violations.push(Violation {
                    engine: format!("checker:{}", item.id),
                    ..v
                });
            }
        }
        emit_stage_progress(
            "aggregation",
            "end",
            Some(aggregation_started.elapsed().as_millis()),
        );

        if mode == Mode::Staged {
            if let Ok(cache_dir) = ensure_cache_dir(Path::new(&config.config_dir)) {
                let health_path = cache_dir.join("checker-health.json");
                let now = iso_now();
                notices.extend(update_checker_health(&health_path, &outcomes, &now));
            }
        }
    }

    CollectResult {
        violations,
        notices,
        fatal,
    }
}

fn panic_payload_str(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Severity-allow + suppression filter shared by the gate and the baseline snapshot.
/// Emits the malformed-suppressions warning once. Does NOT apply ratchet/baseline.
pub fn apply_gate_filters(
    violations: Vec<Violation>,
    config: &ResolvedConfig,
    mode: Mode,
    stderr: Option<&mut GateStderr<'_>>,
) -> Vec<Violation> {
    let allow = gate_allow(config, mode);

    let sup = load_suppressions(Path::new(&config.suppressions_path));
    if let Some(err) = &sup.error {
        let msg = format!("suppressions.json malformed ({err}) — treating as EMPTY");
        if let Some(stderr) = stderr {
            stderr.warning(&msg);
        } else {
            let _ = writeln!(std::io::stderr(), "⚠ SLOPGATE: {msg}");
        }
    }

    violations
        .into_iter()
        .filter(|v| allow.contains(&v.severity))
        .filter(|v| {
            !is_suppressed(
                &sup.entries,
                &SuppressionViolation {
                    id: v.id.clone(),
                    file: v.file.clone(),
                    line_hash: line_hash(&v.full_line),
                },
            )
        })
        .collect()
}

/// Convenience wrapper: filters without stderr side effects (except malformed-suppressions warning).
pub fn apply_gate_filters_simple(
    violations: Vec<Violation>,
    config: &ResolvedConfig,
    mode: Mode,
) -> Vec<Violation> {
    apply_gate_filters(violations, config, mode, None)
}

/// Run the gate for `file` or `staged` mode. Default tier: staged→commit, file→fast.
pub fn run_gate(
    mode: Mode,
    config: &ResolvedConfig,
    tier: Option<Tier>,
    file_target: Option<&str>,
) -> GateResult {
    let mut stderr = std::io::stderr();
    let mut gate_stderr = GateStderr {
        writer: &mut stderr,
    };
    run_gate_with_stderr(mode, config, tier, file_target, &mut gate_stderr)
}

/// Same as [`run_gate`] but writes machine-surface stderr to `gate_stderr` (unit tests).
pub fn run_gate_with_stderr(
    mode: Mode,
    config: &ResolvedConfig,
    tier: Option<Tier>,
    file_target: Option<&str>,
    gate_stderr: &mut GateStderr<'_>,
) -> GateResult {
    let eff_tier = tier.unwrap_or(match mode {
        Mode::Staged => Tier::Commit,
        Mode::File => Tier::Fast,
        Mode::Full => Tier::Commit,
    });

    let CollectResult {
        violations: collected,
        notices,
        fatal,
    } = collect_violations(mode, config, eff_tier, file_target);

    for n in notices {
        gate_stderr.notice(&n);
    }
    if fatal {
        gate_stderr.writeln("SLOPGATE: blocked (scanner infrastructure error)");
        return GateResult {
            violations: vec![],
            code: 2,
        };
    }

    let mut violations = apply_gate_filters(collected, config, mode, Some(gate_stderr));

    let mut baselined_count = 0u32;
    if eff_tier == Tier::Commit {
        let bl = load_baseline(Path::new(&config.baseline_path));
        if let Some(err) = &bl.error {
            gate_stderr.warning(&format!(
                "baseline.json malformed ({err}) — treating as EMPTY (everything is new)"
            ));
        }
        if bl.missing && !violations.is_empty() {
            gate_stderr.warning(
                "no baseline — run: slopgate baseline --config <config> to absorb pre-existing violations",
            );
        }
        let renames = if mode == Mode::Staged {
            staged_renames(Path::new(&config.repo_root))
        } else {
            HashMap::new()
        };
        let filtered = filter_new(&violations, &bl.entries, &renames);
        violations = filtered.fresh;
        baselined_count = filtered.baselined_count;
    }

    if violations.is_empty() {
        if baselined_count > 0 {
            let _ = writeln!(
                gate_stderr.writer,
                "SLOPGATE: clean ({baselined_count} pre-existing baselined violation(s) ignored)"
            );
        }
        return GateResult {
            violations,
            code: 0,
        };
    }

    let _ = print_gate_report_to(
        &violations,
        mode_str(mode),
        baselined_count,
        gate_stderr.writer,
    );
    GateResult {
        violations,
        code: 1,
    }
}

/// Full-repo commit-tier snapshot, filtered like the gate (severity + suppressions).
pub fn snapshot_violations(config: &ResolvedConfig) -> SnapshotResult {
    let CollectResult {
        violations,
        notices,
        fatal,
    } = collect_violations(Mode::Full, config, Tier::Commit, None);
    for n in notices {
        let _ = writeln!(std::io::stderr(), "⚠ SLOPGATE: {n}");
    }
    if fatal {
        return SnapshotResult::Fatal;
    }
    SnapshotResult::Violations(apply_gate_filters_simple(violations, config, Mode::Staged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::line_hash;
    use crate::ratchet::write_baseline;
    use crate::rules::packs::Pattern;
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use tempfile::TempDir;

    fn fixture_toml() -> String {
        fs::read_to_string(format!(
            "{}/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn test_config(root: &Path, toml: &str) -> ResolvedConfig {
        use crate::config::resolve_config_str;
        let config_dir = root.join(".slopgate");
        fs::create_dir_all(&config_dir).unwrap();
        let mut config = resolve_config_str(toml).unwrap();
        config.repo_root = root.to_string_lossy().into_owned();
        config.config_dir = config_dir.to_string_lossy().into_owned();
        config.baseline_path = config_dir
            .join("baseline.json")
            .to_string_lossy()
            .into_owned();
        config.suppressions_path = config_dir
            .join("suppressions.json")
            .to_string_lossy()
            .into_owned();
        config
    }

    fn setup_repo(root: &Path) -> ResolvedConfig {
        fs::create_dir_all(root.join("src")).unwrap();
        let mut config = test_config(root, &fixture_toml());
        config.ast_rule_dirs.clear();
        config
    }

    #[cfg(unix)]
    fn setup_repo_with_ast(root: &Path) -> ResolvedConfig {
        let mut config = setup_repo(root);
        let rule_dir = root.join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        config
            .ast_rule_dirs
            .push(rule_dir.to_string_lossy().into_owned());
        config
    }

    #[cfg(unix)]
    fn git_ok(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn write_ast_stub(root: &Path, status: i32, stderr: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("node_modules/.bin/ast-grep");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stderr = stderr.replace('\\', "\\\\").replace('\'', "'\\''");
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s' '{stderr}' >&2\nprintf '[]'\nexit {status}\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staged_path_resolution_error_blocks_gate_instead_of_reporting_clean() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo_with_ast(root);
        fs::write(root.join("src/staged.ts"), "export const staged = 1;\n").unwrap();
        git_ok(root, &["init"]);
        git_ok(root, &["add", "src/staged.ts"]);
        fs::remove_file(root.join("src/staged.ts")).unwrap();

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 2, "stderr:\n{stderr}");
        assert!(stderr.contains("staged.ts"), "stderr:\n{stderr}");
        assert!(!stderr.contains("SLOPGATE: clean"), "stderr:\n{stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn staged_enumeration_failure_blocks_gate_with_code_two() {
        let dir = TempDir::new().unwrap();
        let config = setup_repo(dir.path());

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 2, "stderr:\n{stderr}");
        assert!(
            stderr.contains("source enumeration failed"),
            "stderr:\n{stderr}"
        );
        assert!(stderr.contains("git diff --cached"), "stderr:\n{stderr}");
        assert!(!stderr.contains("SLOPGATE: clean"), "stderr:\n{stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn truly_empty_staged_set_is_clean() {
        let dir = TempDir::new().unwrap();
        let config = setup_repo(dir.path());
        git_ok(dir.path(), &["init"]);

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 0, "stderr:\n{stderr}");
        assert!(
            !stderr.contains("source enumeration failed"),
            "stderr:\n{stderr}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn scanner_error_blocks_staged_gate_with_nonzero_status() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo_with_ast(root);
        fs::write(root.join("src/clean.ts"), "export const clean = 1;\n").unwrap();
        git_ok(root, &["init"]);
        git_ok(root, &["config", "user.email", "test@example.invalid"]);
        git_ok(root, &["config", "user.name", "test"]);
        git_ok(root, &["add", "src/clean.ts"]);
        write_ast_stub(root, 7, "scanner exploded\n");

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 2, "stderr:\n{stderr}");
        assert!(stderr.contains("scanner exploded"), "stderr:\n{stderr}");
        assert!(!stderr.contains("SLOPGATE: clean"), "stderr:\n{stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn scanner_stub_that_ignores_targets_blocks_clean_staged_gate() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo_with_ast(root);
        fs::write(root.join("src/clean.ts"), "export const clean = 1;\n").unwrap();
        git_ok(root, &["init"]);
        git_ok(root, &["add", "src/clean.ts"]);
        write_ast_stub(root, 0, "");

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 2, "stderr:\n{stderr}");
        assert!(
            stderr.contains("path participation") || stderr.contains("path canary"),
            "stderr:\n{stderr}"
        );
        assert!(!stderr.contains("SLOPGATE: clean"), "stderr:\n{stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn scanner_stderr_blocks_staged_gate_even_with_zero_status() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo_with_ast(root);
        fs::write(root.join("src/clean.ts"), "export const clean = 1;\n").unwrap();
        git_ok(root, &["init"]);
        git_ok(root, &["add", "src/clean.ts"]);
        write_ast_stub(root, 0, "No such file or directory\n");

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::Staged, &config, Some(Tier::Fast), None, stderr)
        });
        assert_eq!(result.code, 2, "stderr:\n{stderr}");
        assert!(
            stderr.contains("No such file or directory"),
            "stderr:\n{stderr}"
        );
        assert!(!stderr.contains("SLOPGATE: clean"), "stderr:\n{stderr}");
    }

    fn capture_stderr<F>(f: F) -> (GateResult, String)
    where
        F: FnOnce(&mut GateStderr<'_>) -> GateResult,
    {
        let mut buf = Cursor::new(Vec::new());
        let mut gate_stderr = GateStderr { writer: &mut buf };
        let result = f(&mut gate_stderr);
        let stderr = String::from_utf8(buf.into_inner()).unwrap();
        (result, stderr)
    }

    #[test]
    fn stage_progress_line_has_stable_machine_format() {
        assert_eq!(
            crate::checkers::shared::stage_progress_line(
                "tsc:apps/zync-app/tsconfig.json",
                "start",
                None,
            ),
            "SLOPGATE_PROGRESS stage=tsc:apps/zync-app/tsconfig.json event=start"
        );
        assert_eq!(
            crate::checkers::shared::stage_progress_line("aggregation", "end", Some(17)),
            "SLOPGATE_PROGRESS stage=aggregation event=end elapsed_ms=17"
        );
    }

    #[test]
    fn clean_file_returns_code_zero() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let (result, _) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::File, &config, None, Some("src/clean.ts"), stderr)
        });
        assert_eq!(result.code, 0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn violating_file_returns_code_one_with_expected_violation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        let bad = "const x = foo as any;\n";
        fs::write(root.join("src/bad.ts"), bad).unwrap();

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::File, &config, None, Some("src/bad.ts"), stderr)
        });
        assert_eq!(result.code, 1, "stderr:\n{stderr}");
        let v = result
            .violations
            .iter()
            .find(|v| v.id == "as-any-cast")
            .expect("as-any-cast violation");
        assert_eq!(v.line, 1);
        assert_eq!(v.severity, "high");
        assert!(stderr.contains("src/bad.ts"));
    }

    #[test]
    fn severity_filter_drops_info() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut config = setup_repo(root);
        config.patterns.push(Pattern {
            id: "info-rule".into(),
            severity: "info".into(),
            pattern: "INFO_MARKER".into(),
            resolution: "remove".into(),
            title: None,
            description: None,
            category: Some("test".into()),
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: None,
            scan_test_files: None,
        });
        fs::write(root.join("src/marked.ts"), "const INFO_MARKER = 1;\n").unwrap();

        let collected = collect_violations(Mode::File, &config, Tier::Fast, Some("src/marked.ts"));
        assert!(collected.violations.iter().any(|v| v.id == "info-rule"));

        let filtered = apply_gate_filters_simple(collected.violations, &config, Mode::File);
        assert!(filtered.is_empty());
    }

    #[test]
    fn suppression_suppresses_matching_violation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        let line = "const x = foo as any;\n";
        fs::write(root.join("src/bad.ts"), line).unwrap();
        let lh = line_hash("const x = foo as any;");
        fs::write(
            Path::new(&config.suppressions_path),
            format!(
                r#"{{
  "version": 1,
  "entries": [{{"id": "as-any-cast", "file": "src/bad.ts", "lineHash": "{lh}"}}]
}}
"#
            ),
        )
        .unwrap();

        let (result, _) = capture_stderr(|stderr| {
            run_gate_with_stderr(Mode::File, &config, None, Some("src/bad.ts"), stderr)
        });
        assert_eq!(result.code, 0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn baselined_violation_hidden_with_clean_notice() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        let line = "const x = foo as any;\n";
        fs::write(root.join("src/bad.ts"), line).unwrap();

        let collected = collect_violations(Mode::File, &config, Tier::Fast, Some("src/bad.ts"));
        write_baseline(
            Path::new(&config.baseline_path),
            &collected.violations,
            "test",
        )
        .unwrap();

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(
                Mode::File,
                &config,
                Some(Tier::Commit),
                Some("src/bad.ts"),
                stderr,
            )
        });
        assert_eq!(result.code, 0, "stderr:\n{stderr}");
        assert!(result.violations.is_empty());
        assert!(stderr.contains("pre-existing baselined"));
    }

    #[test]
    fn no_panic_on_missing_baseline_and_suppressions() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let (result, stderr) = capture_stderr(|stderr| {
            run_gate_with_stderr(
                Mode::File,
                &config,
                Some(Tier::Commit),
                Some("src/bad.ts"),
                stderr,
            )
        });
        assert_eq!(result.code, 1);
        assert!(stderr.contains("no baseline"));
    }

    #[test]
    fn malformed_suppressions_emits_warning_and_treats_as_empty() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);
        fs::write(&config.suppressions_path, "{ not json").unwrap();
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let mut buf = Cursor::new(Vec::new());
        let mut gate_stderr = GateStderr { writer: &mut buf };
        let filtered = apply_gate_filters(
            collect_violations(Mode::File, &config, Tier::Fast, Some("src/bad.ts")).violations,
            &config,
            Mode::File,
            Some(&mut gate_stderr),
        );
        let stderr = String::from_utf8(buf.into_inner()).unwrap();
        assert!(stderr.contains("suppressions.json malformed"));
        assert!(!filtered.is_empty());
    }

    #[test]
    fn snapshot_violations_uses_staged_gate_filter() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut config = setup_repo(root);
        config.gate.staged = ["critical"].iter().map(|s| s.to_string()).collect();
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let SnapshotResult::Violations(snap) = snapshot_violations(&config) else {
            panic!("snapshot should succeed");
        };
        assert!(
            snap.is_empty(),
            "high-severity as-any should be filtered by critical-only staged gate"
        );
    }

    #[test]
    fn collect_empty_files_early_return_except_full() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let config = setup_repo(root);

        let staged = collect_violations(Mode::Staged, &config, Tier::Fast, None);
        assert!(staged.violations.is_empty());

        let full = collect_violations(Mode::Full, &config, Tier::Fast, None);
        assert!(full.violations.is_empty());
    }
}
