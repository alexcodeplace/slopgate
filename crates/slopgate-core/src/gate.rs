//! Language-neutral policy-gate coordinator (SG-ARCH-001, SG-GATE-001).
use crate::ast_engine::{run_ast_grep_scan, AstGrepScanOpts};
use crate::checkers::index::{Checker, CheckerRunOpts, CheckerRunResult, CheckerScope};
use crate::checkers::shared::{emit_stage_progress, map_limit};
use crate::config::ResolvedConfig;
use crate::enumerate::{
    list_source_files, require_consistent_staged_tree, staged_identity, EnumerateCtx, EnumerateMode,
};
use crate::hash::line_hash;
use crate::protocol::{run_adapter, AdapterTier, ExternalAdapterConfig};
use crate::ratchet::{filter_new, load_baseline, staged_renames};
use crate::regex_engine::scan_regex_checked;
use crate::report::{print_gate_report_to, Violation};
use crate::suppressions::load_suppressions;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    File,
    Staged,
    Full,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fast,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub id: String,
    pub scope: CheckerScope,
    pub status: String,
    pub required: bool,
    pub selected_files: Option<usize>,
    pub elapsed_ms: u128,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CollectResult {
    pub violations: Vec<Violation>,
    pub notices: Vec<String>,
    pub errors: Vec<String>,
    pub coverage: Vec<Coverage>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub violations: Vec<Violation>,
    pub code: i32,
    pub errors: Vec<String>,
    pub coverage: Vec<Coverage>,
}

pub struct GateStderr<'a> {
    pub writer: &'a mut dyn Write,
}
impl GateStderr<'_> {
    fn writeln(&mut self, line: &str) {
        let _ = writeln!(self.writer, "{line}");
    }
    fn notice(&mut self, msg: &str) {
        self.writeln(&format!("SLOPGATE: {msg}"));
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
fn push_ast_violations(
    config: &ResolvedConfig,
    violations: &mut Vec<Violation>,
    mut finding: Violation,
) {
    if config.ast_disable.contains(&finding.id) {
        return;
    }
    if config.ux_ast_all.contains(&finding.id) {
        let Some(severity) = config.ux_ast_severity.get(&finding.id) else {
            return;
        };
        finding.severity = severity.clone();
    }
    violations.push(finding);
}

pub fn validate_registry(config: &ResolvedConfig, checkers: &[Checker]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let mut known = HashSet::new();
    for checker in checkers {
        if !known.insert(checker.id) {
            errors.push(format!("duplicate built-in adapter ID {}", checker.id));
        }
    }
    for (id, cfg) in &config.checkers {
        if !known.contains(id.as_str()) {
            errors.push(format!("unknown checker {id:?}"));
        }
        if cfg == &Value::Bool(false) {
            continue;
        }
        if !cfg.is_object() {
            errors.push(format!("checker {id}: settings must be a table or boolean"));
            continue;
        }
        if cfg.get("required").is_some_and(|v| !v.is_boolean()) {
            errors.push(format!("checker {id}: required must be boolean"));
        }
        if cfg
            .get("timeout")
            .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0 || n > 3600))
        {
            errors.push(format!("checker {id}: timeout must be 1..3600 seconds"));
        }
    }
    for (id, adapter) in &config.external_adapters {
        if known.contains(id.as_str()) || config.checkers.contains_key(id) {
            errors.push(format!("external adapter shadows built-in ID {id:?}"));
        }
        if let Err(error) = adapter.validate(id) {
            errors.push(error);
        }
    }
    if config.checker_concurrency == 0 || config.checker_concurrency > 64 {
        errors.push("checkerConcurrency must be 1..64".into());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn required(cfg: &Value) -> bool {
    cfg.get("required").and_then(Value::as_bool).unwrap_or(true)
}

#[derive(Clone, Copy)]
enum Work<'a> {
    Builtin(&'a Checker, &'a Value),
    External(&'a str, &'a ExternalAdapterConfig),
}
impl Work<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Builtin(c, _) => c.id,
            Self::External(id, _) => id,
        }
    }
    fn scope(&self) -> CheckerScope {
        match self {
            Self::Builtin(c, _) => c.scope,
            Self::External(_, c) => c.scope,
        }
    }
    fn required(&self) -> bool {
        match self {
            Self::Builtin(_, c) => required(c),
            Self::External(_, c) => c.required,
        }
    }
}

pub fn collect_violations(
    mode: Mode,
    config: &ResolvedConfig,
    tier: Tier,
    file_target: Option<&str>,
    checkers: &[Checker],
) -> CollectResult {
    let mut result = CollectResult::default();
    if let Err(errors) = validate_registry(config, checkers) {
        result.errors = errors;
        return result;
    }
    if mode == Mode::Staged {
        if let Err(error) = require_consistent_staged_tree(Path::new(&config.repo_root)) {
            result.errors.push(error);
            return result;
        }
    }
    let proposed_tree = if mode == Mode::Staged {
        match staged_identity(Path::new(&config.repo_root)) {
            Ok(identity) => Some(identity),
            Err(error) => {
                result.errors.push(error);
                return result;
            }
        }
    } else {
        None
    };
    let started = Instant::now();
    emit_stage_progress("discovery", "start", None);
    let files = match list_source_files(
        &enumerate_ctx(config),
        match mode {
            Mode::File => EnumerateMode::File(file_target.unwrap_or("")),
            Mode::Staged => EnumerateMode::Staged,
            Mode::Full => EnumerateMode::Walk,
        },
    ) {
        Ok(files) => files,
        Err(error) => {
            result.errors.push(error);
            return result;
        }
    };
    emit_stage_progress("discovery", "end", Some(started.elapsed().as_millis()));
    if files.is_empty() {
        result
            .notices
            .push("no source files selected; configured project checks remain applicable".into());
    }

    let started = Instant::now();
    emit_stage_progress("regex", "start", None);
    match scan_regex_checked(config, &files, mode == Mode::File) {
        Ok(findings) => result.violations.extend(findings),
        Err(errors) => result.errors.extend(errors),
    }
    emit_stage_progress("regex", "end", Some(started.elapsed().as_millis()));
    result.coverage.push(Coverage {
        id: "regex".into(),
        scope: CheckerScope::Files,
        status: if config.patterns.is_empty() || files.is_empty() {
            "not-applicable"
        } else if result.errors.is_empty() {
            "complete"
        } else {
            "error"
        }
        .into(),
        required: true,
        selected_files: Some(files.len()),
        elapsed_ms: started.elapsed().as_millis(),
        details: vec![format!(
            "{} configured line-scoped rules",
            config.patterns.len()
        )],
    });
    if config.ast_enabled && !files.is_empty() {
        let started = Instant::now();
        emit_stage_progress("ast", "start", None);
        let ast = run_ast_grep_scan(
            config,
            Some(&files),
            &AstGrepScanOpts {
                timeout_ms: Some(if tier == Tier::Fast { 5_000 } else { 60_000 }),
                ..Default::default()
            },
        );
        emit_stage_progress("ast", "end", Some(started.elapsed().as_millis()));
        let mut details = ast.warnings;
        details.extend(ast.inspection);
        result.notices.extend(details.iter().cloned());
        result.coverage.push(Coverage {
            id: "ast".into(),
            scope: CheckerScope::Files,
            required: true,
            selected_files: Some(files.len()),
            status: if ast.available && ast.errors.is_empty() {
                "complete"
            } else {
                "error"
            }
            .into(),
            elapsed_ms: started.elapsed().as_millis(),
            details,
        });
        if !ast.available && ast.errors.is_empty() {
            result.errors.push("AST checker did not complete".into());
        }
        result.errors.extend(ast.errors);
        for finding in ast.violations {
            push_ast_violations(config, &mut result.violations, finding);
        }
    } else {
        result.coverage.push(Coverage {
            id: "ast".into(),
            scope: CheckerScope::Files,
            required: config.ast_enabled,
            selected_files: Some(files.len()),
            elapsed_ms: 0,
            details: vec![],
            status: if config.ast_enabled {
                "not-applicable"
            } else {
                "disabled"
            }
            .into(),
        });
    }

    let mut work = Vec::new();
    for checker in checkers {
        if let Some(cfg) = config.checkers.get(checker.id) {
            if cfg == &Value::Bool(false) {
                result.coverage.push(Coverage {
                    id: checker.id.into(),
                    scope: checker.scope,
                    status: "disabled".into(),
                    required: false,
                    selected_files: None,
                    elapsed_ms: 0,
                    details: vec![],
                });
                continue;
            }
            if tier == Tier::Commit {
                work.push(Work::Builtin(checker, cfg));
            } else {
                result.coverage.push(Coverage {
                    id: checker.id.into(),
                    scope: checker.scope,
                    status: "out-of-tier".into(),
                    required: required(cfg),
                    selected_files: None,
                    elapsed_ms: 0,
                    details: vec![],
                });
            }
        }
    }
    for (id, cfg) in &config.external_adapters {
        if tier == Tier::Commit || cfg.tier == AdapterTier::Fast {
            work.push(Work::External(id, cfg));
        } else {
            result.coverage.push(Coverage {
                id: id.clone(),
                scope: cfg.scope,
                status: "out-of-tier".into(),
                required: cfg.required,
                selected_files: None,
                elapsed_ms: 0,
                details: vec![],
            });
        }
    }
    // A worker may run a batch of configured subprojects. Partition its inner
    // allowance so nested scheduling cannot exceed the global configured cap.
    let workers = (config.checker_concurrency as usize).min(work.len().max(1));
    let mut worker_config = config.clone();
    worker_config.checker_concurrency =
        (config.checker_concurrency as usize / workers).max(1) as u32;
    let outputs = map_limit(&work, workers, |item| {
        let started = Instant::now();
        if item.scope() == CheckerScope::Files && files.is_empty() {
            return (
                CheckerRunResult::default(),
                started.elapsed().as_millis(),
                true,
            );
        }
        emit_stage_progress(item.id(), "start", None);
        let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match item {
            Work::Builtin(checker, cfg) => {
                let availability = (checker.detect)(&worker_config, cfg);
                if !availability.available {
                    return CheckerRunResult {
                        errors: vec![availability
                            .reason
                            .unwrap_or_else(|| "checker unavailable".into())],
                        ..Default::default()
                    };
                }
                let opts = if checker.scope == CheckerScope::Files {
                    CheckerRunOpts {
                        files: Some(&files),
                        mode: mode_str(mode),
                    }
                } else {
                    CheckerRunOpts {
                        files: None,
                        mode: "full",
                    }
                };
                (checker.run)(&worker_config, cfg, opts)
            }
            Work::External(id, cfg) => run_adapter(
                id,
                cfg,
                &worker_config,
                &files,
                mode_str(mode),
                if tier == Tier::Fast {
                    AdapterTier::Fast
                } else {
                    AdapterTier::Commit
                },
            ),
        }))
        .unwrap_or_else(|panic| CheckerRunResult {
            errors: vec![format!("checker panicked: {}", panic_payload_str(panic))],
            ..Default::default()
        });
        emit_stage_progress(item.id(), "end", Some(started.elapsed().as_millis()));
        (output, started.elapsed().as_millis(), false)
    });
    for (item, (mut output, elapsed_ms, not_applicable)) in work.iter().zip(outputs) {
        let status = if not_applicable {
            "not-applicable"
        } else if output.errors.is_empty() {
            "complete"
        } else if item.required() {
            "error"
        } else {
            "optional-error"
        };
        let mut details = output.warnings;
        if !output.errors.is_empty() {
            let messages = output
                .errors
                .iter()
                .map(|error| format!("{}: {error}", item.id()));
            if item.required() {
                result.errors.extend(messages);
            } else {
                result.notices.extend(messages);
            }
            details.extend(output.errors);
        }
        result.notices.extend(
            details
                .iter()
                .map(|notice| format!("{}: {notice}", item.id())),
        );
        result.coverage.push(Coverage {
            id: item.id().into(),
            scope: item.scope(),
            status: status.into(),
            required: item.required(),
            selected_files: (item.scope() == CheckerScope::Files).then_some(files.len()),
            elapsed_ms,
            details,
        });
        for finding in &mut output.violations {
            // Engine provenance belongs to the coordinator, not arbitrary output.
            finding.engine = match item {
                Work::Builtin(_, _) => format!("checker:{}", item.id()),
                Work::External(_, _) => format!("adapter:{}", item.id()),
            };
        }
        result.violations.extend(output.violations);
    }
    if mode == Mode::Staged {
        if let Err(error) = require_consistent_staged_tree(Path::new(&config.repo_root)) {
            result
                .errors
                .push(format!("staged inputs changed during execution: {error}"));
        }
    }
    if let Some(before) = proposed_tree {
        match staged_identity(Path::new(&config.repo_root)) {
            Ok(after) if before == after => {}
            Ok(_) => result.errors.push(
                "proposed commit tree changed during checking; rerun on stable inputs".into(),
            ),
            Err(error) => result.errors.push(error),
        }
    }
    result.violations.sort_by(|a, b| {
        (&a.engine, &a.file, a.line, &a.id, &a.text)
            .cmp(&(&b.engine, &b.file, b.line, &b.id, &b.text))
    });
    result.notices.sort();
    result.notices.dedup();
    result.errors.sort();
    result.errors.dedup();
    result.coverage.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

fn panic_payload_str(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(value) = payload.downcast_ref::<&str>() {
        (*value).to_string()
    } else if let Some(value) = payload.downcast_ref::<String>() {
        value.clone()
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

    // Each reviewed suppression entry authorizes one occurrence, not arbitrary
    // newly copied lines with the same content hash. Duplicate approved entries
    // can explicitly authorize multiple occurrences without changing the schema.
    let mut remaining: HashMap<(String, String, String), usize> = HashMap::new();
    for entry in sup.entries {
        *remaining
            .entry((entry.id, entry.file, entry.line_hash))
            .or_default() += 1;
    }
    violations
        .into_iter()
        .filter(|finding| allow.contains(&finding.severity))
        .filter(|finding| {
            let key = (
                finding.id.clone(),
                finding.file.clone(),
                line_hash(&finding.full_line),
            );
            if let Some(count) = remaining.get_mut(&key) {
                if *count > 0 {
                    *count -= 1;
                    return false;
                }
            }
            true
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
    checkers: &[Checker],
) -> GateResult {
    let mut stderr = std::io::stderr();
    let mut gate_stderr = GateStderr {
        writer: &mut stderr,
    };
    run_gate_with_stderr(mode, config, tier, file_target, &mut gate_stderr, checkers)
}

/// Same as [`run_gate`] but writes machine-surface stderr to `gate_stderr` (unit tests).
pub fn run_gate_with_stderr(
    mode: Mode,
    config: &ResolvedConfig,
    tier: Option<Tier>,
    file_target: Option<&str>,
    gate_stderr: &mut GateStderr<'_>,
    checkers: &[Checker],
) -> GateResult {
    let eff_tier = tier.unwrap_or(match mode {
        Mode::Staged => Tier::Commit,
        Mode::File => Tier::Fast,
        Mode::Full => Tier::Commit,
    });

    let CollectResult {
        violations: collected,
        notices,
        errors,
        coverage,
    } = collect_violations(mode, config, eff_tier, file_target, checkers);

    for n in notices {
        gate_stderr.notice(&n);
    }

    if !errors.is_empty() {
        for error in &errors {
            gate_stderr.notice(&format!("INCOMPLETE: {error}"));
        }
        return GateResult {
            violations: collected,
            code: 2,
            errors,
            coverage,
        };
    }
    let suppressions = load_suppressions(Path::new(&config.suppressions_path));
    if let Some(error) = suppressions.error {
        return GateResult {
            violations: collected,
            code: 2,
            errors: vec![format!("invalid suppressions: {error}")],
            coverage,
        };
    }
    let mut violations = apply_gate_filters(collected, config, mode, Some(gate_stderr));

    let mut baselined_count = 0u32;
    if eff_tier == Tier::Commit {
        let bl = load_baseline(Path::new(&config.baseline_path));
        if let Some(err) = &bl.error {
            return GateResult {
                violations,
                code: 2,
                errors: vec![format!("invalid baseline: {err}")],
                coverage,
            };
        }
        if bl.missing && !violations.is_empty() {
            gate_stderr.warning(
                "no baseline — run: slopgate baseline --config <config> to absorb pre-existing violations",
            );
        }
        let renames = if mode == Mode::Staged {
            match staged_renames(Path::new(&config.repo_root)) {
                Ok(renames) => renames,
                Err(error) => {
                    return GateResult {
                        violations,
                        code: 2,
                        errors: vec![error],
                        coverage,
                    }
                }
            }
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
            errors,
            coverage,
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
        errors,
        coverage,
    }
}

/// A baseline may only be derived from a complete project scan.
pub fn snapshot_violations(
    config: &ResolvedConfig,
    checkers: &[Checker],
) -> Result<Vec<Violation>, Vec<String>> {
    let result = collect_violations(Mode::Full, config, Tier::Commit, None, checkers);
    for notice in result.notices {
        eprintln!("SLOPGATE: {notice}");
    }
    if !result.errors.is_empty() {
        return Err(result.errors);
    }
    if let Some(error) = load_suppressions(Path::new(&config.suppressions_path)).error {
        return Err(vec![format!("invalid suppressions: {error}")]);
    }
    Ok(apply_gate_filters_simple(
        result.violations,
        config,
        Mode::Full,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::line_hash;
    use crate::ratchet::write_baseline;
    use crate::rules::packs::Pattern;
    use std::fs;
    use std::io::Cursor;
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
        config.roots = config
            .roots_rel
            .iter()
            .map(|rel| root.join(rel).to_string_lossy().into_owned())
            .collect();
        // Unit tests in this module isolate coordination/regex/ratchet. Concrete
        // adapters and real structural scanning have CLI integration coverage.
        config.checkers.clear();
        config.ast_enabled = false;
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
        test_config(root, &fixture_toml())
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
            run_gate_with_stderr(Mode::File, &config, None, Some("src/clean.ts"), stderr, &[])
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
            run_gate_with_stderr(Mode::File, &config, None, Some("src/bad.ts"), stderr, &[])
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

        let collected =
            collect_violations(Mode::File, &config, Tier::Fast, Some("src/marked.ts"), &[]);
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
            run_gate_with_stderr(Mode::File, &config, None, Some("src/bad.ts"), stderr, &[])
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

        let collected =
            collect_violations(Mode::File, &config, Tier::Fast, Some("src/bad.ts"), &[]);
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
                &[],
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
                &[],
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
            collect_violations(Mode::File, &config, Tier::Fast, Some("src/bad.ts"), &[]).violations,
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

        let snap = snapshot_violations(&config, &[]).unwrap();
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

        let staged = collect_violations(Mode::Staged, &config, Tier::Fast, None, &[]);
        assert!(staged.violations.is_empty());

        let full = collect_violations(Mode::Full, &config, Tier::Fast, None, &[]);
        assert!(full.violations.is_empty());
    }
}
