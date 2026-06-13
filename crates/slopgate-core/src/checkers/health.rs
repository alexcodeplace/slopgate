//! Consecutive infra-failure tracking per checker across commit-tier runs.
//! Mirrors `src/checkers/health.mjs`: the gate fails open on checker crash/timeout/
//! missing-binary — correct per-commit, but a checker that infra-fails every run is
//! silently off forever. This counter escalates that into a loud warning without
//! ever flipping the exit code. State lives in the self-gitignored cache dir.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FAILURE_THRESHOLD: u32 = 2;

/// An error string that means "the tool did not actually run/produce results".
pub fn is_infra_error(msg: &str) -> bool {
    msg.contains("failed:")
        || msg.contains("crashed")
        || msg.contains("killed by signal")
        || msg.contains("JSON parse error")
}

/// One checker outcome from a commit-tier run.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckerOutcome {
    pub id: String,
    pub infra_failed: bool,
    pub detail: Option<String>,
    pub seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct CheckerHealthEntry {
    #[serde(rename = "consecutiveFailures", default)]
    consecutive_failures: u32,
    #[serde(rename = "lastOk", default, skip_serializing_if = "Option::is_none")]
    last_ok: Option<String>,
    #[serde(
        rename = "lastFailure",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    last_failure: Option<String>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(
        rename = "lastDurationSeconds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    last_duration_seconds: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HealthFile {
    version: u32,
    checkers: HashMap<String, CheckerHealthEntry>,
}

/// Read JSON state, increment consecutive infra failures, escalate at threshold,
/// reset on success, write back. Never panics; write failures are fail-open.
pub fn update_checker_health(path: &Path, outcomes: &[CheckerOutcome], now: &str) -> Vec<String> {
    let mut state = load_health_state(path);
    let mut warnings = Vec::new();

    for outcome in outcomes {
        let entry = state.entry(outcome.id.clone()).or_default();
        if outcome.infra_failed {
            entry.consecutive_failures += 1;
            entry.last_failure = Some(now.to_string());
            entry.last_error = outcome.detail.clone();
        } else {
            entry.consecutive_failures = 0;
            entry.last_ok = Some(now.to_string());
        }
        if let Some(seconds) = outcome.seconds {
            entry.last_duration_seconds = Some(seconds);
        }
        if entry.consecutive_failures >= FAILURE_THRESHOLD {
            let err = entry.last_error.as_deref().unwrap_or("unknown");
            warnings.push(format!(
                "CHECKER OFF: {} infra-failed {} consecutive commit runs — its checks are NOT gating (fail-open). Last error: {}",
                outcome.id, entry.consecutive_failures, err
            ));
        }
    }

    write_health_state(path, &state);
    warnings
}

fn load_health_state(path: &Path) -> HashMap<String, CheckerHealthEntry> {
    if !path.exists() {
        return HashMap::new();
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    match serde_json::from_str::<HealthFile>(&contents) {
        Ok(file) => file.checkers,
        Err(_) => HashMap::new(),
    }
}

fn write_health_state(path: &Path, checkers: &HashMap<String, CheckerHealthEntry>) {
    let file = HealthFile {
        version: 1,
        checkers: checkers.clone(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, format!("{json}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn is_infra_error_true_on_infra_markers() {
        assert!(is_infra_error("leakscan failed: spawn failed"));
        assert!(is_infra_error("depcruise crashed: oops"));
        assert!(is_infra_error("tool killed by signal 9"));
        assert!(is_infra_error("leakscan JSON parse error: expected value"));
    }

    #[test]
    fn is_infra_error_false_on_normal_violation_message() {
        assert!(!is_infra_error("Route I/O through a service layer"));
        assert!(!is_infra_error("boundary violation in UserCard.tsx"));
    }

    #[test]
    fn update_checker_health_no_notice_on_first_infra_failure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("checker-health.json");
        let now = "2026-06-11T12:00:00.000Z";
        let warnings = update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "leakscan".to_string(),
                infra_failed: true,
                detail: Some("leakscan failed: no binary".to_string()),
                seconds: Some(0.1),
            }],
            now,
        );
        assert!(warnings.is_empty());
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["checkers"]["leakscan"]["consecutiveFailures"], 1);
        assert_eq!(
            saved["checkers"]["leakscan"]["lastError"],
            "leakscan failed: no binary"
        );
        assert_eq!(saved["checkers"]["leakscan"]["lastFailure"], now);
        assert_eq!(saved["checkers"]["leakscan"]["lastDurationSeconds"], 0.1);
    }

    #[test]
    fn update_checker_health_escalates_on_second_consecutive_failure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("checker-health.json");
        let now1 = "2026-06-11T12:00:00.000Z";
        let now2 = "2026-06-11T12:01:00.000Z";
        update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "depcruise".to_string(),
                infra_failed: true,
                detail: Some("depcruise failed: timeout".to_string()),
                seconds: None,
            }],
            now1,
        );
        let warnings = update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "depcruise".to_string(),
                infra_failed: true,
                detail: Some("depcruise failed: timeout".to_string()),
                seconds: None,
            }],
            now2,
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("CHECKER OFF: depcruise"));
        assert!(warnings[0].contains("2 consecutive"));
        assert!(warnings[0].contains("depcruise failed: timeout"));
    }

    #[test]
    fn update_checker_health_resets_after_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("checker-health.json");
        let now1 = "2026-06-11T12:00:00.000Z";
        let now2 = "2026-06-11T12:01:00.000Z";
        let now3 = "2026-06-11T12:02:00.000Z";
        update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "tsc".to_string(),
                infra_failed: true,
                detail: Some("tsc crashed".to_string()),
                seconds: None,
            }],
            now1,
        );
        update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "tsc".to_string(),
                infra_failed: true,
                detail: Some("tsc crashed".to_string()),
                seconds: None,
            }],
            now2,
        );
        let warnings = update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "tsc".to_string(),
                infra_failed: false,
                detail: None,
                seconds: Some(1.5),
            }],
            now3,
        );
        assert!(warnings.is_empty());
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["checkers"]["tsc"]["consecutiveFailures"], 0);
        assert_eq!(saved["checkers"]["tsc"]["lastOk"], now3);
        assert_eq!(saved["checkers"]["tsc"]["lastDurationSeconds"], 1.5);

        let warnings = update_checker_health(
            &path,
            &[CheckerOutcome {
                id: "tsc".to_string(),
                infra_failed: true,
                detail: Some("tsc failed: again".to_string()),
                seconds: None,
            }],
            "2026-06-11T12:03:00.000Z",
        );
        assert!(warnings.is_empty());
    }
}
