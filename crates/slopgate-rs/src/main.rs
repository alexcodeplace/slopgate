use slopgate_core::audit::run::run_audit;
use slopgate_core::config::resolve_config;
use slopgate_core::gate::{run_gate, snapshot_violations, Mode, Tier};
use slopgate_core::init::run::{engine_root, run_init_io};
use slopgate_core::install::agent_hooks::{
    home_dir, install_agent_hooks, remove_agent_hooks, status_agent_hooks, status_symbol, AGENTS,
};
use slopgate_core::install::hooks::{install_pre_commit_hook, HookInstallAction};
use slopgate_core::install::skills::{default_skills_dest_in, install_skills, SkillInstallAction};
use slopgate_core::ratchet::{
    fingerprint_violation, load_baseline, write_baseline, write_baseline_raw, BaselineEntry,
};
use slopgate_core::selftest::run_self_test;
use slopgate_core::help::HELP_TEXT;
use slopgate_core::stats::query::{
    aggregate, aggregate_dashboard, format_dashboard, format_stats, Row, DIMENSIONS,
};
use slopgate_core::stats::record::record_incidents;
use slopgate_core::stats::store::{global_stats_path_in, project_stats_path, read_rows};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn val_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

fn write_slopgate_err(stderr: &mut dyn Write, msg: &str) {
    let _ = writeln!(stderr, "{msg}");
}

fn write_top_level_err(stderr: &mut dyn Write, err: &str) {
    if err.starts_with("slopgate:") {
        let _ = writeln!(stderr, "{err}");
    } else {
        let _ = writeln!(stderr, "slopgate: {err}");
    }
}

fn writeln_stdout(stdout: &mut dyn Write, line: &str) {
    let _ = writeln!(stdout, "{line}");
}

fn pad_end(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn hook_action_str(action: HookInstallAction) -> &'static str {
    match action {
        HookInstallAction::Created => "created",
        HookInstallAction::Updated => "updated",
        HookInstallAction::Appended => "appended",
        HookInstallAction::Unchanged => "unchanged",
    }
}

fn skill_action_str(action: SkillInstallAction) -> &'static str {
    match action {
        SkillInstallAction::Skipped => "skipped",
        SkillInstallAction::Installed => "installed",
        SkillInstallAction::Updated => "updated",
    }
}

fn cwd_string() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string())
}

fn resolve_node_path() -> String {
    Command::new("which")
        .arg("node")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "node".to_string())
}

fn iso_timestamp_now() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    format_timestamp_utc(secs, millis)
}

fn format_timestamp_utc(secs: i64, millis: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    let mut y = 1970i32;
    let mut day = days;

    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if day < year_days {
            break;
        }
        day -= year_days;
        y += 1;
    }

    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [u32; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut mo = 1u32;
    for &md in &month_days {
        if day < i64::from(md) {
            break;
        }
        day -= i64::from(md);
        mo += 1;
    }

    format!(
        "{y:04}-{mo:02}-{:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z",
        day + 1
    )
}

fn parse_rows(values: &[Value]) -> Vec<Row> {
    values
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

enum ConfigResult {
    Ok(slopgate_core::config::ResolvedConfig),
    Exit(i32),
}

fn require_config(args: &[String], stderr: &mut dyn Write) -> ConfigResult {
    let Some(config_path) = val_of(args, "--config") else {
        write_slopgate_err(
            stderr,
            "slopgate: --config <path> required — run 'slopgate --help'",
        );
        return ConfigResult::Exit(2);
    };
    match resolve_config(config_path) {
        Ok(config) => ConfigResult::Ok(config),
        Err(e) => {
            write_top_level_err(stderr, &e);
            return ConfigResult::Exit(1);
        }
    }
}

/// CLI entry point — testable without spawning the binary.
pub fn run(args: &[String]) -> i32 {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    run_with_io(args, &mut stdout, &mut stderr)
}

fn run_with_io(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let home = home_dir();
    run_with_io_and_home(args, stdout, stderr, &home)
}

fn run_with_io_and_home(
    args: &[String],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    home: &Path,
) -> i32 {
    match dispatch(args, stdout, stderr, home) {
        Ok(code) => code,
        Err(e) => {
            write_top_level_err(stderr, &e);
            1
        }
    }
}

fn dispatch(
    args: &[String],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    home: &Path,
) -> Result<i32, String> {
    let user_args = args.get(1..).unwrap_or(&[]);
    if user_args.is_empty()
        || has(args, "--help")
        || has(args, "-h")
        || user_args.first().is_some_and(|a| a == "help")
    {
        write!(stdout, "{HELP_TEXT}\n").map_err(|e| e.to_string())?;
        return Ok(0);
    }

    if args.get(1).is_some_and(|a| a == "--version") {
        writeln_stdout(stdout, &format!("slopgate-rs {}", env!("CARGO_PKG_VERSION")));
        return Ok(0);
    }

    if has(args, "init") {
        let dir = val_of(args, "init")
            .map(str::to_string)
            .unwrap_or_else(cwd_string);
        return Ok(run_init_io(&dir, false, stdout, stderr));
    }

    if has(args, "stats") {
        let by_present = has(args, "--by");
        let by_flag = val_of(args, "--by");
        let since = val_of(args, "--since");
        let json = has(args, "--json");
        let rows = if let Some(config_path) = val_of(args, "--config") {
            let config = resolve_config(config_path)?;
            parse_rows(&read_rows(&project_stats_path(&config)))
        } else {
            parse_rows(&read_rows(&global_stats_path_in(home)))
        };
        if !by_present {
            let dashboard = aggregate_dashboard(&rows, since)?;
            writeln_stdout(stdout, &format_dashboard(&dashboard, json));
            return Ok(0);
        }
        if !by_flag.is_some_and(|by| DIMENSIONS.contains(&by)) {
            write_slopgate_err(
                stderr,
                &format!("slopgate: --by must be {}", DIMENSIONS.join("|")),
            );
            return Ok(2);
        }
        let aggregated = aggregate(&rows, by_flag, since)?;
        writeln_stdout(stdout, &format_stats(&aggregated, json));
        return Ok(0);
    }

    if has(args, "install-hooks") {
        let config = match require_config(args, stderr) {
            ConfigResult::Ok(c) => c,
            ConfigResult::Exit(code) => return Ok(code),
        };
        let engine = engine_root();
        let engine_invocation = engine.join("bin/slopgate").to_string_lossy().into_owned();
        let node_path = resolve_node_path();
        let result = install_pre_commit_hook(
            Path::new(&config.repo_root),
            &engine_invocation,
            &node_path,
        )
        .map_err(|e| e.to_string())?;
        writeln_stdout(
            stdout,
            &format!(
                "slopgate: pre-commit hook {} ({})",
                hook_action_str(result.action),
                result.path.display()
            ),
        );
        return Ok(0);
    }

    if has(args, "agent-hooks") {
        let sub = args
            .iter()
            .position(|a| a == "agent-hooks")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        let valid_subs = ["install", "reinstall", "remove", "status"];
        let agent_ids: Option<Vec<String>> = val_of(args, "--agent").map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        });

        if let Some(ref ids) = agent_ids {
            let unknown: Vec<&str> = ids
                .iter()
                .filter(|id| !AGENTS.iter().any(|a| a.id == id.as_str()))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                let valid = AGENTS
                    .iter()
                    .map(|a| a.id)
                    .collect::<Vec<_>>()
                    .join(", ");
                write_slopgate_err(
                    stderr,
                    &format!(
                        "slopgate: unknown agent(s): {} — valid: {valid}",
                        unknown.join(", ")
                    ),
                );
                return Ok(2);
            }
        }

        let engine = engine_root();
        let agent_ids_ref = agent_ids.as_deref();

        if sub.is_none() || sub == Some("status") || !valid_subs.contains(&sub.unwrap()) {
            let rows = status_agent_hooks(&home, &engine);
            for r in rows {
                let sym = status_symbol(&r.status);
                let det = if r.detected {
                    "detected"
                } else {
                    "not detected"
                };
                writeln_stdout(
                    stdout,
                    &format!(
                        "  {sym}  {}  {}  ({det})  {}",
                        pad_end(&r.label, 28),
                        pad_end(&r.status, 13),
                        r.path.display()
                    ),
                );
            }
            return Ok(if sub.is_none() || sub == Some("status") {
                0
            } else {
                2
            });
        }

        if sub == Some("install") || sub == Some("reinstall") {
            if sub == Some("reinstall") {
                let rem = remove_agent_hooks(&home, &engine, agent_ids_ref);
                for r in rem {
                    if r.action == "removed" {
                        writeln_stdout(
                            stdout,
                            &format!(
                                "slopgate: agent-hooks {} — removed (reinstalling)",
                                r.label
                            ),
                        );
                    }
                }
            }
            let results = install_agent_hooks(&home, &engine, agent_ids_ref);
            if results.is_empty() {
                writeln_stdout(
                    stdout,
                    "slopgate: no agent CLIs detected — pass --agent <id> to install for a specific agent",
                );
            }
            for r in results {
                if r.action == "invalid-json" {
                    write_slopgate_err(
                        stderr,
                        &format!(
                            "slopgate: agent-hooks {} — {} is not valid JSON, left untouched",
                            r.label,
                            r.path.display()
                        ),
                    );
                } else {
                    writeln_stdout(
                        stdout,
                        &format!(
                            "slopgate: agent-hooks {} — {} ({})",
                            r.label,
                            r.action,
                            r.path.display()
                        ),
                    );
                }
            }
            return Ok(0);
        }

        if sub == Some("remove") {
            let results = remove_agent_hooks(&home, &engine, agent_ids_ref);
            for r in results {
                writeln_stdout(
                    stdout,
                    &format!(
                        "slopgate: agent-hooks {} — {} ({})",
                        r.label,
                        r.action,
                        r.path.display()
                    ),
                );
            }
            return Ok(0);
        }

        write_slopgate_err(
            stderr,
            "slopgate: agent-hooks usage: agent-hooks [status|install|reinstall|remove] [--agent id1,id2]",
        );
        return Ok(2);
    }

    if has(args, "install-skills") {
        let force = has(args, "--force");
        let engine = engine_root();
        let skills_src = engine.join("skills");
        let results = install_skills(&skills_src, &default_skills_dest_in(home), force)
            .map_err(|e| e.to_string())?;
        let empty = results.is_empty();
        for r in results {
            writeln_stdout(
                stdout,
                &format!(
                    "slopgate: skill {} — {}",
                    r.name,
                    skill_action_str(r.action)
                ),
            );
        }
        if empty {
            writeln_stdout(stdout, "slopgate: no skills to install");
        }
        return Ok(0);
    }

    if has(args, "audit") {
        let config = match require_config(args, stderr) {
            ConfigResult::Ok(c) => c,
            ConfigResult::Exit(code) => return Ok(code),
        };
        let since_days_raw = val_of(args, "--since-days").unwrap_or("90");
        let since_days = match since_days_raw.parse::<f64>() {
            Ok(n) if n.is_finite() && n > 0.0 => n as u32,
            _ => {
                write_slopgate_err(stderr, "slopgate: --since-days must be a positive number");
                return Ok(2);
            }
        };
        writeln_stdout(
            stdout,
            &run_audit(&config, since_days, has(args, "--json")),
        );
        return Ok(0);
    }

    if has(args, "baseline") {
        let config = match require_config(args, stderr) {
            ConfigResult::Ok(c) => c,
            ConfigResult::Exit(code) => return Ok(code),
        };
        let baseline_path = Path::new(&config.baseline_path);
        let exists = baseline_path.exists();

        if has(args, "--prune") && !has(args, "--update") {
            let bl = load_baseline(baseline_path);
            if bl.error.is_some() || bl.missing {
                write_slopgate_err(stderr, "slopgate: no valid baseline to prune");
                return Ok(2);
            }
            let snap = snapshot_violations(&config);
            let current: HashSet<String> = snap
                .iter()
                .map(|v| fingerprint_violation(v, None))
                .collect();
            let old_count = bl.entries.len();
            let kept: HashMap<String, BaselineEntry> = bl
                .entries
                .into_iter()
                .filter(|(fp, _)| current.contains(fp))
                .collect();
            let dropped = old_count - kept.len();
            let kept_count = kept.len();
            write_baseline_raw(baseline_path, &kept, &iso_timestamp_now())?;
            let entry_word = if dropped == 1 { "y" } else { "ies" };
            writeln_stdout(
                stdout,
                &format!(
                    "slopgate: baseline pruned — {dropped} resolved entr{entry_word} removed, {kept_count} kept"
                ),
            );
            return Ok(0);
        }

        if exists && !has(args, "--update") {
            write_slopgate_err(
                stderr,
                "slopgate: baseline.json exists — use `baseline --update` to re-snapshot (this re-absorbs ALL current violations) or `baseline --prune` to drop resolved entries",
            );
            return Ok(2);
        }

        let old = if exists {
            load_baseline(baseline_path)
        } else {
            slopgate_core::ratchet::LoadedBaseline {
                entries: HashMap::new(),
                missing: true,
                error: None,
            }
        };
        let snap = snapshot_violations(&config);
        let n = write_baseline(baseline_path, &snap, &iso_timestamp_now())?;
        if exists {
            let fps: HashSet<String> = snap
                .iter()
                .map(|v| fingerprint_violation(v, None))
                .collect();
            let mut seen = HashSet::new();
            let added: Vec<_> = snap
                .iter()
                .filter(|v| {
                    let fp = fingerprint_violation(v, None);
                    if old.entries.contains_key(&fp) || seen.contains(&fp) {
                        return false;
                    }
                    seen.insert(fp);
                    true
                })
                .collect();
            let removed = old
                .entries
                .keys()
                .filter(|fp| !fps.contains(*fp))
                .count();
            let mut by_rule: HashMap<String, u32> = HashMap::new();
            for v in &added {
                *by_rule.entry(v.id.clone()).or_insert(0) += 1;
            }
            let mut rule_entries: Vec<_> = by_rule.into_iter().collect();
            rule_entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let top: String = rule_entries
                .into_iter()
                .take(5)
                .map(|(id, c)| format!("{id}×{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            let top_suffix = if top.is_empty() {
                String::new()
            } else {
                format!(" — absorbed: {top}")
            };
            writeln_stdout(
                stdout,
                &format!(
                    "slopgate: baseline updated — {n} entries (+{} newly absorbed, −{removed} resolved){top_suffix}",
                    added.len()
                ),
            );
            if !added.is_empty() {
                writeln_stdout(
                    stdout,
                    "slopgate: ⚠ newly absorbed entries are violations being LEGITIMIZED — review before committing baseline.json",
                );
            }
        } else {
            let entry_word = if n == 1 { "y" } else { "ies" };
            writeln_stdout(
                stdout,
                &format!(
                    "slopgate: baseline written — {n} entr{entry_word} → {}",
                    config.baseline_path
                ),
            );
        }
        return Ok(0);
    }

    let config = match require_config(args, stderr) {
        ConfigResult::Ok(c) => c,
        ConfigResult::Exit(code) => return Ok(code),
    };

    if has(args, "--self-test") {
        return Ok(run_self_test(&config));
    }

    let tier = match val_of(args, "--tier") {
        None => None,
        Some("fast") => Some(Tier::Fast),
        Some("commit") => Some(Tier::Commit),
        Some(_) => {
            write_slopgate_err(stderr, "slopgate: --tier must be fast|commit");
            return Ok(2);
        }
    };

    if has(args, "--staged") {
        let result = run_gate(Mode::Staged, &config, tier, None);
        if result.code == 1 {
            let record_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                record_incidents(&result.violations, &config, "staged");
            }));
            if let Err(payload) = record_result {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown error".to_string()
                };
                write_slopgate_err(
                    stderr,
                    &format!("⚠ SLOPGATE: stats record failed ({msg}) — ignored"),
                );
            }
        }
        return Ok(result.code);
    }

    if let Some(file_target) = val_of(args, "--file") {
        let result = run_gate(Mode::File, &config, tier, Some(file_target));
        return Ok(result.code);
    }

    write_slopgate_err(
        stderr,
        "slopgate: unknown command — run 'slopgate --help'",
    );
    Ok(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(run(&args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use tempfile::TempDir;

    fn fixture_toml() -> String {
        fs::read_to_string(format!(
            "{}/../slopgate-core/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn setup_tmp_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("git init");
        fs::create_dir_all(root.join(".slopgate")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".slopgate/config.toml"), fixture_toml()).unwrap();
        dir
    }

    fn run_capture(args: Vec<String>) -> (i32, String, String) {
        run_capture_with_home(args, &home_dir())
    }

    fn run_capture_with_home(args: Vec<String>, home: &std::path::Path) -> (i32, String, String) {
        let mut stdout = Cursor::new(Vec::new());
        let mut stderr = Cursor::new(Vec::new());
        let code = run_with_io_and_home(&args, &mut stdout, &mut stderr, home);
        let out = String::from_utf8(stdout.into_inner()).unwrap();
        let err = String::from_utf8(stderr.into_inner()).unwrap();
        (code, out, err)
    }

    fn base_args(root: &std::path::Path) -> Vec<String> {
        vec![
            "slopgate-rs".into(),
            "--config".into(),
            root.join(".slopgate/config.toml")
                .to_string_lossy()
                .into_owned(),
        ]
    }

    #[test]
    fn file_clean_returns_zero() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut args = base_args(root);
        args.extend(["--file".into(), "src/clean.ts".into()]);
        let (code, _, _) = run_capture(args);
        assert_eq!(code, 0);
    }

    #[test]
    fn file_violating_returns_one() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let mut args = base_args(root);
        args.extend(["--file".into(), "src/bad.ts".into()]);
        let (code, _, _) = run_capture(args);
        assert_eq!(code, 1);
    }

    #[test]
    fn missing_config_returns_two() {
        let (code, _, err) = run_capture(vec!["slopgate-rs".into(), "--file".into(), "x.ts".into()]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "slopgate: --config <path> required — run 'slopgate --help'\n"
        );
    }

    #[test]
    fn bad_tier_returns_two() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut args = base_args(root);
        args.extend([
            "--tier".into(),
            "slow".into(),
            "--file".into(),
            "src/clean.ts".into(),
        ]);
        let (code, _, err) = run_capture(args);
        assert_eq!(code, 2);
        assert_eq!(err, "slopgate: --tier must be fast|commit\n");
    }

    #[test]
    fn no_mode_returns_two_with_unknown_command() {
        let dir = setup_tmp_repo();
        let args = base_args(dir.path());
        let (code, _, err) = run_capture(args);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "slopgate: unknown command — run 'slopgate --help'\n"
        );
    }

    #[test]
    fn help_exits_zero_and_prints_help() {
        let (code, out, _) = run_capture(vec!["slopgate-rs".into(), "--help".into()]);
        assert_eq!(code, 0);
        assert_eq!(out, format!("{HELP_TEXT}\n"));
    }

    #[test]
    fn bare_stats_prints_dashboard_sections() {
        let home = TempDir::new().unwrap();

        let (code, out, _) =
            run_capture_with_home(vec!["slopgate-rs".into(), "stats".into()], home.path());
        assert_eq!(code, 0);
        assert!(out.contains("0 incident(s) stopped"));
        assert!(!out.contains("BY RULE"));
    }

    #[test]
    fn stats_with_sample_rows_prints_dashboard_sections() {
        use slopgate_core::stats::store::append_row;

        let home = TempDir::new().unwrap();
        let stats_path = home.path().join(".slopgate/stats.jsonl");
        fs::create_dir_all(stats_path.parent().unwrap()).unwrap();
        let row = Row {
            ts: Some("2026-01-01T10:00:00.000Z".into()),
            rule_id: Some("no-stubs".into()),
            project: Some("slopgate".into()),
            model: Some("claude".into()),
            severity: None,
            engine: None,
            category: None,
            file: None,
            line: None,
        };
        append_row(
            &stats_path,
            &serde_json::to_value(&row).expect("row json"),
        )
        .unwrap();

        let (code, out, _) =
            run_capture_with_home(vec!["slopgate-rs".into(), "stats".into()], home.path());
        assert_eq!(code, 0);
        assert!(out.contains("BY RULE"));
        assert!(out.contains("BY MODEL"));
        assert!(out.contains("BY PROJECT"));
    }

    #[test]
    fn stats_by_single_dimension() {
        use slopgate_core::stats::store::append_row;

        let home = TempDir::new().unwrap();
        let stats_path = home.path().join(".slopgate/stats.jsonl");
        fs::create_dir_all(stats_path.parent().unwrap()).unwrap();
        let row = Row {
            ts: Some("2026-01-01T10:00:00.000Z".into()),
            rule_id: Some("no-stubs".into()),
            project: Some("slopgate".into()),
            model: Some("claude".into()),
            severity: None,
            engine: None,
            category: None,
            file: None,
            line: None,
        };
        append_row(
            &stats_path,
            &serde_json::to_value(&row).expect("row json"),
        )
        .unwrap();

        let (code, out, _) = run_capture_with_home(
            vec![
                "slopgate-rs".into(),
                "stats".into(),
                "--by".into(),
                "model".into(),
            ],
            home.path(),
        );
        assert_eq!(code, 0);
        assert!(out.contains("1 incident(s) stopped"));
        assert!(!out.contains("BY RULE"));
        assert!(out.contains("MODEL"));
        assert!(out.contains("claude"));
    }

    #[test]
    fn stats_by_without_value_returns_two() {
        let (code, _, err) = run_capture(vec!["slopgate-rs".into(), "stats".into(), "--by".into()]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "slopgate: --by must be rule|model|project|severity|engine|category\n"
        );
    }

    #[test]
    fn unknown_command_returns_two() {
        let dir = setup_tmp_repo();
        let mut args = base_args(dir.path());
        args.push("bogus".into());
        let (code, _, err) = run_capture(args);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "slopgate: unknown command — run 'slopgate --help'\n"
        );
    }

    #[test]
    fn init_exits_zero_with_scaffold_message() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/index.ts"), "export const x = 1;\n").unwrap();

        let (code, out, _) = run_capture(vec!["slopgate-rs".into(), "init".into(), root.to_string_lossy().into_owned()]);
        assert_eq!(code, 0);
        assert!(out.contains("slopgate: scaffolded"));
    }

    #[test]
    fn stats_empty_returns_zero_incidents() {
        let home = TempDir::new().unwrap();

        let (code, out, _) =
            run_capture_with_home(vec!["slopgate-rs".into(), "stats".into()], home.path());
        assert_eq!(code, 0);
        assert!(out.contains("0 incident(s) stopped"));
    }

    #[test]
    fn stats_bad_by_returns_two() {
        let (code, _, err) = run_capture(vec![
            "slopgate-rs".into(),
            "stats".into(),
            "--by".into(),
            "bogus".into(),
        ]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "slopgate: --by must be rule|model|project|severity|engine|category\n"
        );
    }

    #[test]
    fn baseline_create_then_guard_returns_two() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut create_args = base_args(root);
        create_args.push("baseline".into());
        let (code1, out1, _) = run_capture(create_args);
        assert_eq!(code1, 0);
        assert!(out1.contains("slopgate: baseline written"));

        let mut guard_args = base_args(root);
        guard_args.push("baseline".into());
        let (code2, _, err2) = run_capture(guard_args);
        assert_eq!(code2, 2);
        assert!(err2.contains("baseline.json exists"));
    }

    #[test]
    fn audit_exits_zero_with_header() {
        let dir = setup_tmp_repo();
        let root = dir.path();
        fs::write(root.join("src/clean.ts"), "export const x = 1;\n").unwrap();

        let mut args = base_args(root);
        args.push("audit".into());
        let (code, out, _) = run_capture(args);
        assert_eq!(code, 0);
        assert!(out.contains("SLOPGATE AUDIT —"));
    }

    #[test]
    fn install_skills_exits_zero() {
        let home = TempDir::new().unwrap();

        let (code, out, _) = run_capture_with_home(
            vec!["slopgate-rs".into(), "install-skills".into()],
            home.path(),
        );
        assert_eq!(code, 0);
        assert!(
            out.contains("slopgate: skill") || out.contains("slopgate: no skills to install")
        );
    }

    #[test]
    fn self_test_exits_zero_on_real_config() {
        let config_path = engine_root().join(".slopgate/config.toml");
        if !config_path.is_file() {
            return;
        }
        let Ok(config) = resolve_config(&config_path.to_string_lossy()) else {
            return;
        };
        if config
            .fixtures_dirs
            .iter()
            .any(|d| !Path::new(d).is_dir())
        {
            return;
        }
        if Command::new("ast-grep")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            return;
        }

        let (code, _, _) = run_capture(vec![
            "slopgate-rs".into(),
            "--config".into(),
            config_path.to_string_lossy().into_owned(),
            "--self-test".into(),
        ]);
        assert_eq!(code, 0);
    }
}
