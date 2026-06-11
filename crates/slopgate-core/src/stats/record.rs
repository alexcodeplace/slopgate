//! Append blocked-violation rows to global + project stats stores.
//! Mirrors `src/stats/record.mjs` (`recordIncidents`, `resolveSession`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::ResolvedConfig;
use crate::install::agent_hooks::home_dir;
use crate::report::Violation;
use crate::stats::store::{append_row, global_stats_path_in, project_stats_path};

const SESSION_TTL_MS: f64 = 8.0 * 60.0 * 60.0 * 1000.0;

/// Resolved session/model attribution for a stats row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSession {
    pub model: String,
    pub session_id: Option<String>,
    pub started_at: Option<String>,
}

fn session_key(repo_root: &str) -> String {
    let canonical = fs::canonicalize(repo_root).unwrap_or_else(|_| PathBuf::from(repo_root));
    let mut h = Sha256::new();
    h.update(canonical.to_string_lossy().as_bytes());
    hex_digest(&h.finalize())[..16].to_string()
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn home_slopgate_sessions_dir_in(home: &Path) -> PathBuf {
    home.join(".slopgate").join("sessions")
}

/// Resolve the recording model. Precedence: `model_override` → env `SLOPGATE_MODEL` →
/// SessionStart file (`~/.slopgate/sessions/<key>.json`) → `unknown`. Never panics.
pub fn resolve_session_in(
    home: &Path,
    repo_root: &str,
    model_override: Option<&str>,
) -> ResolvedSession {
    if let Some(model) = model_override
        .map(str::to_string)
        .or_else(|| std::env::var("SLOPGATE_MODEL").ok().filter(|m| !m.is_empty()))
    {
        return ResolvedSession {
            model,
            session_id: None,
            started_at: None,
        };
    }

    let path = home_slopgate_sessions_dir_in(home).join(format!("{}.json", session_key(repo_root)));
    let Ok(raw) = fs::read_to_string(&path) else {
        return unknown_session();
    };
    let Ok(session) = serde_json::from_str::<SessionFile>(&raw) else {
        return unknown_session();
    };

    if let Some(ref started_at) = session.started_at {
        if let Some(started_ms) = iso_to_ms(started_at) {
            let now_ms = unix_ms_now();
            if now_ms - started_ms > SESSION_TTL_MS {
                return unknown_session();
            }
        }
    }

    ResolvedSession {
        model: session
            .model
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
        session_id: session.session_id,
        started_at: session.started_at,
    }
}

/// Resolve session using the real `$HOME` (production callers read home once at the call site).
pub fn resolve_session(repo_root: &str) -> ResolvedSession {
    resolve_session_in(&home_dir(), repo_root, None)
}

fn unknown_session() -> ResolvedSession {
    ResolvedSession {
        model: "unknown".to_string(),
        session_id: None,
        started_at: None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    model: Option<String>,
    session_id: Option<String>,
    started_at: Option<String>,
}

fn unix_ms_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0 + f64::from(d.subsec_millis()))
        .unwrap_or(0.0)
}

fn iso_timestamp_now() -> String {
    let ms = unix_ms_now();
    let secs = (ms / 1000.0).floor() as i64;
    let millis = (ms - f64::from(secs as u32) * 1000.0).round() as u32;
    let (y, mo, d, h, mi, s) = unix_secs_to_utc(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

fn unix_secs_to_utc(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    let mut y = 1970i32;
    let mut day = days;

    loop {
        let year_days = if is_leap_year(y) { 366 } else { 365 };
        if day < year_days {
            break;
        }
        day -= year_days;
        y += 1;
    }

    let leap = is_leap_year(y);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut mo = 1u32;
    for md in month_days {
        if day < md {
            break;
        }
        day -= md;
        mo += 1;
    }

    (y, mo, day as u32 + 1, h, mi, s)
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Parse ISO-8601 timestamp to Unix ms (`new Date()` compatible subset).
fn iso_to_ms(ts: &str) -> Option<f64> {
    let ts = ts.trim();
    if ts.len() < 19 {
        return None;
    }
    let end = ts
        .find('+')
        .or_else(|| ts.rfind('Z'))
        .unwrap_or(ts.len())
        .min(ts.len());
    let datetime = ts[..end].split('.').next()?;
    let (date, time) = datetime.split_once('T')?;
    let mut dp = date.split('-');
    let y: i32 = dp.next()?.parse().ok()?;
    let mo: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let mut tp = time.split(':');
    let h: u32 = tp.next()?.parse().ok()?;
    let mi: u32 = tp.next()?.parse().ok()?;
    let s: u32 = tp.next()?.parse().ok()?;
    let days = civil_to_unix_days(y, mo, d)?;
    Some(
        (days as f64 * 86_400.0 + f64::from(h) * 3600.0 + f64::from(mi) * 60.0 + f64::from(s))
            * 1000.0,
    )
}

fn civil_to_unix_days(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut y = y;
    let mut m = m as i32;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era * 146097 + doe - 719468))
}

fn repo_basename(repo_root: &str) -> String {
    Path::new(repo_root)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

fn build_row(
    ts: &str,
    project: &str,
    config: &ResolvedConfig,
    session: &ResolvedSession,
    mode: &str,
    v: &Violation,
) -> Value {
    let session_id = session
        .session_id
        .as_ref()
        .map(|s| Value::String(s.clone()))
        .unwrap_or(Value::Null);

    json!({
        "ts": ts,
        "project": project,
        "projectPath": config.repo_root,
        "model": session.model,
        "sessionId": session_id,
        "mode": mode,
        "ruleId": v.id,
        "severity": v.severity,
        "category": v.category,
        "engine": v.engine,
        "file": v.file,
        "line": v.line,
    })
}

/// Append one row per blocked violation to the global + project stores.
/// Fail-open: store errors are swallowed (no panic); returns rows successfully written.
pub fn record_incidents(violations: &[Violation], config: &ResolvedConfig, mode: &str) -> usize {
    let home = home_dir();
    record_incidents_in(violations, config, mode, &home)
}

/// Like [`record_incidents`] but uses an explicit `home` for the global stats path and session file.
pub fn record_incidents_in(
    violations: &[Violation],
    config: &ResolvedConfig,
    mode: &str,
    home: &Path,
) -> usize {
    record_incidents_to_paths(
        violations,
        config,
        mode,
        &global_stats_path_in(home),
        &project_stats_path(config),
        home,
        None,
    )
}

fn record_incidents_to_paths(
    violations: &[Violation],
    config: &ResolvedConfig,
    mode: &str,
    global_path: &Path,
    project_path: &Path,
    home: &Path,
    model_override: Option<&str>,
) -> usize {
    if violations.is_empty() {
        return 0;
    }

    let session = resolve_session_in(home, &config.repo_root, model_override);
    let ts = iso_timestamp_now();
    let project = repo_basename(&config.repo_root);

    let mut written = 0usize;
    for v in violations {
        let row = build_row(&ts, &project, config, &session, mode, v);
        if append_row(global_path, &row).is_err() {
            return written;
        }
        if append_row(project_path, &row).is_err() {
            return written;
        }
        written += 1;
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config_str;
    use std::fs;
    use tempfile::TempDir;

    fn test_config(root: &Path) -> ResolvedConfig {
        let toml = fs::read_to_string(format!(
            "{}/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let config_dir = root.join(".slopgate");
        fs::create_dir_all(&config_dir).unwrap();
        let mut config = resolve_config_str(&toml).unwrap();
        config.repo_root = root.to_string_lossy().into_owned();
        config.config_dir = config_dir.to_string_lossy().into_owned();
        config
    }

    fn sample_violation(id: &str, file: &str, line: u32) -> Violation {
        Violation {
            id: id.to_string(),
            severity: "error".to_string(),
            category: "quality".to_string(),
            file: file.to_string(),
            line,
            full_line: "const x = 1".to_string(),
            text: "no any".to_string(),
            resolution: "fix it".to_string(),
            engine: "regex".to_string(),
        }
    }

    #[test]
    fn resolve_session_in_uses_model_override() {
        let dir = TempDir::new().unwrap();
        let session = resolve_session_in(dir.path(), dir.path().to_str().unwrap(), Some("claude-opus"));
        assert_eq!(
            session,
            ResolvedSession {
                model: "claude-opus".to_string(),
                session_id: None,
                started_at: None,
            }
        );
    }

    #[test]
    fn resolve_session_reads_session_file() {
        let dir = TempDir::new().unwrap();
        let sessions = home_slopgate_sessions_dir_in(dir.path());
        fs::create_dir_all(&sessions).unwrap();
        let key = session_key(dir.path().to_str().unwrap());
        fs::write(
            sessions.join(format!("{key}.json")),
            r#"{"model":"gpt-4","sessionId":"sess-1","startedAt":"2099-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();

        let session = resolve_session_in(dir.path(), dir.path().to_str().unwrap(), None);
        assert_eq!(session.model, "gpt-4");
        assert_eq!(session.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn resolve_session_expired_ttl_returns_unknown() {
        let dir = TempDir::new().unwrap();
        let sessions = home_slopgate_sessions_dir_in(dir.path());
        fs::create_dir_all(&sessions).unwrap();
        let key = session_key(dir.path().to_str().unwrap());
        fs::write(
            sessions.join(format!("{key}.json")),
            r#"{"model":"old-model","sessionId":"sess-old","startedAt":"2000-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();

        let session = resolve_session_in(dir.path(), dir.path().to_str().unwrap(), None);
        assert_eq!(session.model, "unknown");
        assert!(session.session_id.is_none());
    }

    #[test]
    fn record_incidents_writes_n_rows_with_expected_fields() {
        let repo = TempDir::new().unwrap();
        let config = test_config(repo.path());
        let global_path = repo.path().join("global-stats.jsonl");
        let project_path = project_stats_path(&config);

        let violations = vec![
            sample_violation("rule-a", "src/a.ts", 10),
            sample_violation("rule-b", "src/b.ts", 20),
            sample_violation("rule-c", "src/c.ts", 30),
        ];

        let written = record_incidents_to_paths(
            &violations,
            &config,
            "staged",
            &global_path,
            &project_path,
            repo.path(),
            Some("test-model"),
        );
        assert_eq!(written, 3);

        let global_rows = crate::stats::store::read_rows(&global_path);
        let project_rows = crate::stats::store::read_rows(&project_path);
        assert_eq!(global_rows.len(), 3);
        assert_eq!(project_rows.len(), 3);

        for (row, v) in project_rows.iter().zip(violations.iter()) {
            assert!(row.get("ts").and_then(Value::as_str).is_some());
            assert_eq!(row["project"], repo.path().file_name().unwrap().to_str().unwrap());
            assert_eq!(row["projectPath"], config.repo_root);
            assert_eq!(row["model"], "test-model");
            assert!(row["sessionId"].is_null());
            assert_eq!(row["mode"], "staged");
            assert_eq!(row["ruleId"], v.id);
            assert_eq!(row["severity"], v.severity);
            assert_eq!(row["category"], v.category);
            assert_eq!(row["engine"], v.engine);
            assert_eq!(row["file"], v.file);
            assert_eq!(row["line"], v.line);
        }
    }

    #[test]
    fn record_incidents_empty_returns_zero() {
        let repo = TempDir::new().unwrap();
        let config = test_config(repo.path());
        assert_eq!(record_incidents_in(&[], &config, "staged", repo.path()), 0);
    }

    #[test]
    fn record_incidents_fail_open_on_store_error() {
        let repo = TempDir::new().unwrap();
        let config = test_config(repo.path());

        let global_path = repo.path().join("global-stats.jsonl");
        let project_path = project_stats_path(&config);
        fs::create_dir_all(project_path.parent().unwrap()).unwrap();
        fs::create_dir(&project_path).unwrap();

        let violations = vec![sample_violation("rule-x", "src/x.ts", 1)];
        let written = record_incidents_to_paths(
            &violations,
            &config,
            "staged",
            &global_path,
            &project_path,
            repo.path(),
            None,
        );
        assert_eq!(written, 0);
    }
}
