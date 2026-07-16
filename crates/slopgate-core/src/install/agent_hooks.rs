//! Agent hooks installer — port of `src/install-agent-hooks.mjs`.
//!
//! Idempotently merges slopgate hook commands into each agent's settings JSON;
//! invalid JSON is left untouched.

use crate::error::SlopError;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One supported agent CLI and its hooks settings file (under `$HOME`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    pub id: &'static str,
    pub label: &'static str,
    pub commands: &'static [&'static str],
    pub rel_path: &'static str,
}

/// All agents slopgate can wire hooks into. Mirrors JS `AGENTS`.
pub const AGENTS: &[AgentDef] = &[
    AgentDef {
        id: "claude",
        label: "claude / cld / cursor-agent",
        commands: &["claude", "cld", "cursor-agent"],
        rel_path: ".claude/settings.json",
    },
    AgentDef {
        id: "codex",
        label: "codex",
        commands: &["codex"],
        rel_path: ".codex/hooks.json",
    },
    AgentDef {
        id: "grok",
        label: "grok",
        commands: &["grok"],
        rel_path: ".grok/hooks/slopgate.json",
    },
    AgentDef {
        id: "gemini",
        label: "gemini",
        commands: &["gemini"],
        rel_path: ".gemini/settings.json",
    },
];

/// Outcome of merging hooks into a settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub action: &'static str,
    pub path: PathBuf,
}

/// Outcome of removing hooks from a settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveResult {
    pub action: &'static str,
    pub path: PathBuf,
}

/// One row from [`install_agent_hooks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRow {
    pub id: String,
    pub label: String,
    pub action: String,
    pub path: PathBuf,
}

/// One row from [`remove_agent_hooks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveRow {
    pub id: String,
    pub label: String,
    pub action: String,
    pub path: PathBuf,
}

/// One row from [`status_agent_hooks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub id: String,
    pub label: String,
    pub detected: bool,
    pub status: String,
    pub path: PathBuf,
}

/// Resolve the user's home directory from `$HOME` (production callers read once at the call site).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Path to an agent's hooks settings file under `home`.
pub fn agent_file_path(home: &Path, agent: &AgentDef) -> PathBuf {
    home.join(agent.rel_path)
}

/// Whether `cmd` is on PATH (mirrors JS `which`).
pub fn which(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether any of the agent's CLI commands is on PATH.
pub fn agent_detected(agent: &AgentDef) -> bool {
    agent.commands.iter().any(|cmd| which(cmd))
}

/// The canonical slopgate hook set: (event, matcher, command path).
fn required_hook_entries(engine_root: &Path) -> Vec<(&'static str, Option<&'static str>, String)> {
    let path = |name: &str| {
        engine_root
            .join("hooks")
            .join(name)
            .to_string_lossy()
            .into_owned()
    };
    vec![
        ("SessionStart", None, path("session-start.sh")),
        ("PreToolUse", Some("Bash"), path("commit-hook.sh")),
        (
            "PreToolUse",
            Some("Bash|Edit|Write"),
            path("baseline-guard.sh"),
        ),
        ("PostToolUse", Some("Edit|Write"), path("edit-hook.sh")),
    ]
}

/// Path suffixes of the hook scripts slopgate installs. Used to recognize a
/// slopgate hook by *identity* regardless of which install root wrote it.
const SLOPGATE_HOOK_SUFFIXES: [&str; 4] = [
    "hooks/commit-hook.sh",
    "hooks/edit-hook.sh",
    "hooks/session-start.sh",
    "hooks/baseline-guard.sh",
];

/// True when `cmd` is one of slopgate's installed hooks.
///
/// Matches the current install (command lives under `engine_root`) AND — crucially
/// — recognizes hooks left by a *prior* install at a different root (a CI build
/// path, a worktree, `/tmp`) by script identity. Without the identity match,
/// `remove`/`status` only see hooks whose path equals the current root, so stale
/// entries from other roots are never cleaned and stack forever on every
/// `init`/`install` (the dedup-by-exact-path weakness).
fn is_slopgate_cmd(cmd: Option<&str>, engine_root: &Path) -> bool {
    let Some(cmd) = cmd else {
        return false;
    };
    if cmd.contains(engine_root.to_string_lossy().as_ref()) {
        return true;
    }
    SLOPGATE_HOOK_SUFFIXES.iter().any(|s| cmd.ends_with(s))
}

fn ensure_hook_entry(
    settings: &mut Value,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> bool {
    let hooks = settings.as_object_mut().and_then(|o| {
        o.entry("hooks".to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
    });
    let Some(hooks) = hooks else {
        return false;
    };

    let event_arr = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
    let Some(entries) = event_arr.as_array_mut() else {
        return false;
    };

    if matcher.is_none() {
        let present = entries.iter().any(|e| {
            e.get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|arr| {
                    arr.iter()
                        .any(|h| h.get("command") == Some(&json!(command)))
                })
        });
        if present {
            return false;
        }
        entries.push(json!({ "hooks": [{ "type": "command", "command": command }] }));
        return true;
    }

    let matcher = matcher.unwrap();
    let entry = if let Some(idx) = entries
        .iter()
        .position(|e| e.get("matcher") == Some(&json!(matcher)))
    {
        &mut entries[idx]
    } else {
        entries.push(json!({ "matcher": matcher, "hooks": [] }));
        entries.last_mut().unwrap()
    };

    if entry.get("hooks").and_then(|h| h.as_array()).is_none() {
        entry["hooks"] = json!([]);
    }
    let hooks_arr = entry["hooks"].as_array_mut().unwrap();
    let present = hooks_arr
        .iter()
        .any(|h| h.get("command") == Some(&json!(command)));
    if present {
        return false;
    }
    hooks_arr.push(json!({ "type": "command", "command": command }));
    true
}

/// Remove slopgate hooks from a claude-format `root["hooks"]` object, in place.
///
/// A hook is removed when [`is_slopgate_cmd`] matches it (by script *identity*, so
/// foreign-root and prior-version entries are caught) AND its command is not in
/// `keep`. Pass `keep = &[]` to strip every slopgate hook; pass the canonical
/// commands to strip only stale/duplicate ones while leaving the canonical entry
/// untouched (so a no-op merge stays a no-op). Emptied hook arrays, matcher
/// entries, events, and the `hooks` key itself are pruned. Non-slopgate hooks are
/// always preserved. Returns true if anything changed.
fn strip_slopgate_hooks(root: &mut Value, engine_root: &Path, keep: &[&str]) -> bool {
    let Some(hooks_obj) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };

    let mut changed = false;
    let event_keys: Vec<String> = hooks_obj.keys().cloned().collect();

    for event in event_keys {
        let Some(event_arr) = hooks_obj.get_mut(&event).and_then(|v| v.as_array_mut()) else {
            continue;
        };

        let mut new_entries: Vec<Value> = Vec::new();
        for entry in event_arr.drain(..) {
            let Some(hooks_arr) = entry.get("hooks").and_then(|h| h.as_array()) else {
                new_entries.push(entry);
                continue;
            };

            let filtered: Vec<Value> = hooks_arr
                .iter()
                .filter(|h| {
                    let cmd = h.get("command").and_then(|c| c.as_str());
                    let is_stale = is_slopgate_cmd(cmd, engine_root)
                        && !cmd.is_some_and(|c| keep.contains(&c));
                    !is_stale
                })
                .cloned()
                .collect();

            if filtered.len() == hooks_arr.len() {
                new_entries.push(entry);
                continue;
            }

            changed = true;
            if filtered.is_empty() {
                continue;
            }
            let mut new_entry = entry;
            if let Some(obj) = new_entry.as_object_mut() {
                obj.insert("hooks".to_string(), Value::Array(filtered));
            }
            new_entries.push(new_entry);
        }

        if new_entries.is_empty() {
            hooks_obj.remove(&event);
            changed = true;
        } else {
            hooks_obj.insert(event, Value::Array(new_entries));
        }
    }

    if hooks_obj.is_empty() {
        if let Some(obj) = root.as_object_mut() {
            obj.remove("hooks");
            changed = true;
        }
    }

    changed
}

fn write_json_pretty(path: &Path, root: &Value) -> Result<(), SlopError> {
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(root)
            .map_err(|e| SlopError::Parse(format!("serialize {}: {e}", path.display())))?
    );
    fs::write(path, rendered).map_err(|e| SlopError::Io(format!("write {}: {e}", path.display())))
}

/// Idempotently merge slopgate hooks into a claude-format hooks JSON file.
pub fn merge_hooks(file_path: &Path, engine_root: &Path) -> Result<MergeResult, SlopError> {
    let required = required_hook_entries(engine_root);
    let existed = file_path.is_file();
    let mut root = if existed {
        let raw = fs::read_to_string(file_path)
            .map_err(|e| SlopError::Io(format!("read {}: {e}", file_path.display())))?;
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(_) => {
                return Ok(MergeResult {
                    action: "invalid-json",
                    path: file_path.to_path_buf(),
                });
            }
        }
    } else {
        json!({})
    };

    if !root.is_object() {
        root = json!({});
    }

    // Self-heal: drop stale/foreign/duplicate slopgate hooks first (keeping the
    // canonical commands for this engine_root so a no-op stays a no-op), then add
    // any canonical entry that is missing. Without the prune, re-running init
    // stacks a fresh entry beside every prior install root (dev checkout + npm),
    // and the hooks double-fire forever (the dedup-by-exact-path weakness).
    let keep: Vec<&str> = required.iter().map(|(_, _, cmd)| cmd.as_str()).collect();
    let pruned = strip_slopgate_hooks(&mut root, engine_root, &keep);

    let mut added_any = false;
    for (event, matcher, command) in &required {
        added_any |= ensure_hook_entry(&mut root, event, *matcher, command);
    }

    if !pruned && !added_any {
        return Ok(MergeResult {
            action: "already-present",
            path: file_path.to_path_buf(),
        });
    }

    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SlopError::Io(format!("mkdir {}: {e}", parent.display())))?;
    }
    if existed {
        let bak = format!("{}.bak", file_path.display());
        fs::copy(file_path, &bak)
            .map_err(|e| SlopError::Io(format!("copy {} -> {bak}: {e}", file_path.display())))?;
    }
    write_json_pretty(file_path, &root)?;

    Ok(MergeResult {
        action: if existed { "merged" } else { "created" },
        path: file_path.to_path_buf(),
    })
}

/// Remove all slopgate hooks from a claude-format hooks JSON file.
pub fn remove_hooks(file_path: &Path, engine_root: &Path) -> Result<RemoveResult, SlopError> {
    if !file_path.is_file() {
        return Ok(RemoveResult {
            action: "not-found",
            path: file_path.to_path_buf(),
        });
    }

    let raw = fs::read_to_string(file_path)
        .map_err(|e| SlopError::Io(format!("read {}: {e}", file_path.display())))?;
    let mut root: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            return Ok(RemoveResult {
                action: "invalid-json",
                path: file_path.to_path_buf(),
            });
        }
    };

    let changed = strip_slopgate_hooks(&mut root, engine_root, &[]);

    if !changed {
        return Ok(RemoveResult {
            action: "not-present",
            path: file_path.to_path_buf(),
        });
    }

    let bak = format!("{}.bak", file_path.display());
    fs::copy(file_path, &bak)
        .map_err(|e| SlopError::Io(format!("copy {} -> {bak}: {e}", file_path.display())))?;
    write_json_pretty(file_path, &root)?;

    Ok(RemoveResult {
        action: "removed",
        path: file_path.to_path_buf(),
    })
}

/// Check how many of the slopgate hooks are present.
pub fn check_status(file_path: &Path, engine_root: &Path) -> &'static str {
    if !file_path.is_file() {
        return "not-installed";
    }

    let raw = match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(_) => return "not-installed",
    };
    let root: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return "invalid-json",
    };

    let Some(h) = root.get("hooks") else {
        return "not-installed";
    };

    let required = required_hook_entries(engine_root);
    let n = required
        .iter()
        .filter(|(event, _, command)| {
            h.get(event).and_then(|v| v.as_array()).is_some_and(|arr| {
                arr.iter().any(|e| {
                    e.get("hooks")
                        .and_then(|hs| hs.as_array())
                        .is_some_and(|hs| {
                            hs.iter().any(|x| x.get("command") == Some(&json!(command)))
                        })
                })
            })
        })
        .count();
    match n {
        n if n == required.len() => "installed",
        0 => "not-installed",
        _ => "partial",
    }
}

/// Status column symbol for CLI output (mirrors `cli.mjs` `SYMBOL` table).
pub fn status_symbol(status: &str) -> &'static str {
    match status {
        "installed" => "✓",
        "partial" => "~",
        "not-installed" => "✗",
        "not-detected" => "-",
        "invalid-json" => "!",
        _ => "?",
    }
}

/// Install slopgate hooks for all detected (or specified) agents.
pub fn install_agent_hooks(
    home: &Path,
    engine_root: &Path,
    agent_ids: Option<&[String]>,
) -> Vec<InstallRow> {
    let targets: Vec<&AgentDef> = match agent_ids {
        Some(ids) => AGENTS
            .iter()
            .filter(|a| ids.iter().any(|id| id == a.id))
            .collect(),
        None => AGENTS.iter().filter(|a| agent_detected(a)).collect(),
    };

    targets
        .into_iter()
        .filter_map(|a| {
            let file_path = agent_file_path(home, a);
            match merge_hooks(&file_path, engine_root) {
                Ok(r) => Some(InstallRow {
                    id: a.id.to_string(),
                    label: a.label.to_string(),
                    action: r.action.to_string(),
                    path: r.path,
                }),
                Err(_) => None,
            }
        })
        .collect()
}

/// Remove slopgate hooks for all (or specified) agents.
pub fn remove_agent_hooks(
    home: &Path,
    engine_root: &Path,
    agent_ids: Option<&[String]>,
) -> Vec<RemoveRow> {
    let targets: Vec<&AgentDef> = match agent_ids {
        Some(ids) => AGENTS
            .iter()
            .filter(|a| ids.iter().any(|id| id == a.id))
            .collect(),
        None => AGENTS.iter().collect(),
    };

    targets
        .into_iter()
        .filter_map(|a| {
            let file_path = agent_file_path(home, a);
            match remove_hooks(&file_path, engine_root) {
                Ok(r) => Some(RemoveRow {
                    id: a.id.to_string(),
                    label: a.label.to_string(),
                    action: r.action.to_string(),
                    path: r.path,
                }),
                Err(_) => None,
            }
        })
        .collect()
}

/// Return status for all agents (detected or not).
pub fn status_agent_hooks(home: &Path, engine_root: &Path) -> Vec<StatusRow> {
    AGENTS
        .iter()
        .map(|a| {
            let path = agent_file_path(home, a);
            StatusRow {
                id: a.id.to_string(),
                label: a.label.to_string(),
                detected: agent_detected(a),
                status: check_status(&path, engine_root).to_string(),
                path,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn setup_engine() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("hooks")).unwrap();
        fs::write(dir.path().join("hooks/commit-hook.sh"), "").unwrap();
        fs::write(dir.path().join("hooks/edit-hook.sh"), "").unwrap();
        fs::write(dir.path().join("hooks/session-start.sh"), "").unwrap();
        fs::write(dir.path().join("hooks/baseline-guard.sh"), "").unwrap();
        dir
    }

    /// (commit, edit, session, guard) command paths for `engine`.
    fn hook_cmds(engine: &Path) -> (String, String, String, String) {
        let find = |suffix: &str| {
            required_hook_entries(engine)
                .into_iter()
                .map(|(_, _, cmd)| cmd)
                .find(|cmd| cmd.ends_with(suffix))
                .unwrap()
        };
        (
            find("commit-hook.sh"),
            find("edit-hook.sh"),
            find("session-start.sh"),
            find("baseline-guard.sh"),
        )
    }

    #[test]
    fn merge_creates_fresh_settings_with_all_hooks() {
        let engine = setup_engine();
        let (commit, edit, session, guard) = hook_cmds(engine.path());
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");

        let result = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "created");
        assert!(file.is_file());

        let root: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let hooks = root.get("hooks").unwrap();
        assert!(hooks["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["hooks"][0]["command"] == session));
        assert!(hooks["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["matcher"] == "Bash" && e["hooks"][0]["command"] == commit));
        assert!(hooks["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["matcher"] == "Bash|Edit|Write" && e["hooks"][0]["command"] == guard));
        assert!(hooks["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["matcher"] == "Edit|Write" && e["hooks"][0]["command"] == edit));
    }

    #[test]
    fn merge_is_idempotent_already_present() {
        let engine = setup_engine();
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");

        let first = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(first.action, "created");
        let before = fs::read_to_string(&file).unwrap();

        let second = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(second.action, "already-present");
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn merge_collapses_stale_foreign_root_into_single_canonical() {
        // A global settings.json double-stacked with a prior install root (a dev
        // checkout) beside the current engine_root, plus an unrelated non-slopgate
        // hook. Re-merge must prune the foreign-root slopgate entry, keep exactly
        // one canonical entry per event, and preserve the unrelated hook.
        let engine = setup_engine();
        let (commit, _edit, session, _guard) = hook_cmds(engine.path());
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");

        let seeded = serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [
                        { "type": "command", "command": "/foreign/dev/checkout/hooks/commit-hook.sh" },
                        { "type": "command", "command": commit },
                        { "type": "command", "command": "/repo/scripts/code-quality/commit-hook.sh" }
                    ] }
                ],
                "SessionStart": [
                    { "hooks": [ { "type": "command", "command": "/foreign/dev/checkout/hooks/session-start.sh" } ] }
                ]
            }
        });
        fs::write(
            &file,
            format!("{}\n", serde_json::to_string_pretty(&seeded).unwrap()),
        )
        .unwrap();

        let result = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "merged");

        let root: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let pre = root["hooks"]["PreToolUse"].as_array().unwrap();
        let bash = pre.iter().find(|e| e["matcher"] == "Bash").unwrap();
        let cmds: Vec<&str> = bash["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["command"].as_str().unwrap())
            .collect();
        // foreign-root slopgate commit hook pruned; canonical kept exactly once.
        assert_eq!(cmds.iter().filter(|c| **c == commit).count(), 1);
        assert!(!cmds.iter().any(|c| c.starts_with("/foreign")));
        // non-slopgate project hook (code-quality/) preserved.
        assert!(cmds.contains(&"/repo/scripts/code-quality/commit-hook.sh"));
        // SessionStart foreign entry pruned, canonical present exactly once.
        let sess = root["hooks"]["SessionStart"].as_array().unwrap();
        let session_cmds: Vec<&str> = sess
            .iter()
            .flat_map(|e| e["hooks"].as_array().unwrap())
            .map(|h| h["command"].as_str().unwrap())
            .collect();
        assert_eq!(session_cmds.iter().filter(|c| **c == session).count(), 1);
        assert!(!session_cmds.iter().any(|c| c.starts_with("/foreign")));
    }

    #[test]
    fn merge_into_existing_file_merged_action_and_backup() {
        let engine = setup_engine();
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");
        fs::write(&file, "{}\n").unwrap();

        let result = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "merged");
        let bak = format!("{}.bak", file.display());
        assert!(!bak.is_empty());
        assert!(Path::new(&bak).is_file());
    }

    #[test]
    fn merge_invalid_json_left_untouched() {
        let engine = setup_engine();
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");
        let corrupt = "{ not json";
        fs::write(&file, corrupt).unwrap();

        let result = merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "invalid-json");
        assert_eq!(fs::read_to_string(&file).unwrap(), corrupt);
        assert!(!format!("{}.bak", file.display())
            .parse::<PathBuf>()
            .map(|p| p.is_file())
            .unwrap_or(false));
    }

    #[test]
    fn remove_strips_slopgate_hooks_only() {
        let engine = setup_engine();
        let (commit, edit, session, guard) = hook_cmds(engine.path());
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");

        merge_hooks(&file, engine.path()).unwrap();
        let result = remove_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "removed");

        let root: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert!(root.get("hooks").is_none());
        let _ = (commit, edit, session, guard);
    }

    #[test]
    fn remove_not_found_and_not_present() {
        let engine = setup_engine();
        let missing = tempfile::tempdir().unwrap().path().join("nope.json");
        assert_eq!(
            remove_hooks(&missing, engine.path()).unwrap().action,
            "not-found"
        );

        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");
        fs::write(&file, "{}\n").unwrap();
        assert_eq!(
            remove_hooks(&file, engine.path()).unwrap().action,
            "not-present"
        );
    }

    #[test]
    fn remove_invalid_json_left_untouched() {
        let engine = setup_engine();
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");
        let corrupt = "{ bad";
        fs::write(&file, corrupt).unwrap();

        let result = remove_hooks(&file, engine.path()).unwrap();
        assert_eq!(result.action, "invalid-json");
        assert_eq!(fs::read_to_string(&file).unwrap(), corrupt);
    }

    #[test]
    fn check_status_installed_partial_and_not_installed() {
        let engine = setup_engine();
        let (commit, edit, session, _guard) = hook_cmds(engine.path());
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");

        assert_eq!(check_status(&file, engine.path()), "not-installed");

        merge_hooks(&file, engine.path()).unwrap();
        assert_eq!(check_status(&file, engine.path()), "installed");

        let partial = json!({
            "hooks": {
                "SessionStart": [{ "hooks": [{ "type": "command", "command": session }] }]
            }
        });
        fs::write(
            &file,
            format!("{}\n", serde_json::to_string_pretty(&partial).unwrap()),
        )
        .unwrap();
        assert_eq!(check_status(&file, engine.path()), "partial");

        let _ = (commit, edit);
    }

    #[test]
    fn check_status_invalid_json() {
        let engine = setup_engine();
        let settings = tempfile::tempdir().unwrap();
        let file = settings.path().join("settings.json");
        fs::write(&file, "not-json").unwrap();
        assert_eq!(check_status(&file, engine.path()), "invalid-json");
    }

    #[test]
    fn status_agent_hooks_rows_and_symbols() {
        let engine = setup_engine();
        let home = tempfile::tempdir().unwrap();
        let rows = status_agent_hooks(home.path(), engine.path());
        assert_eq!(rows.len(), AGENTS.len());
        for row in &rows {
            assert!(!row.id.is_empty());
            assert!(!row.label.is_empty());
            assert!(!row.path.as_os_str().is_empty());
            let sym = status_symbol(&row.status);
            assert!(matches!(sym, "✓" | "~" | "✗" | "!"));
            let _ = sym;
        }
    }

    #[test]
    fn install_agent_hooks_with_explicit_agent_id() {
        let engine = setup_engine();
        let home = tempfile::tempdir().unwrap();
        let settings_path = home.path().join(AGENTS[0].rel_path);
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let ids = vec!["claude".to_string()];
        let rows = install_agent_hooks(home.path(), engine.path(), Some(&ids));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "created");
        assert_eq!(rows[0].id, "claude");
        assert!(settings_path.is_file());
    }

    #[test]
    fn remove_agent_hooks_with_explicit_agent_id() {
        let engine = setup_engine();
        let home = tempfile::tempdir().unwrap();
        let settings_path = home.path().join(AGENTS[0].rel_path);
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let ids = vec!["claude".to_string()];
        install_agent_hooks(home.path(), engine.path(), Some(&ids));
        let rows = remove_agent_hooks(home.path(), engine.path(), Some(&ids));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "removed");
        assert!(settings_path.is_file());
    }

    #[test]
    fn is_slopgate_cmd_matches_engine_root_path() {
        let engine = setup_engine();
        let cmd = engine.path().join("hooks/commit-hook.sh");
        assert!(is_slopgate_cmd(Some(cmd.to_str().unwrap()), engine.path()));
        assert!(!is_slopgate_cmd(Some("/other/hook.sh"), engine.path()));
    }

    #[test]
    fn is_slopgate_cmd_matches_foreign_root_by_identity() {
        // Hooks installed by a prior build at a DIFFERENT root (CI path, worktree,
        // /tmp) must still be recognized so remove/status self-heal stale entries.
        let engine = setup_engine();
        for suffix in SLOPGATE_HOOK_SUFFIXES {
            let foreign = format!("/home/runner/work/slopgate/slopgate/{suffix}");
            assert!(
                is_slopgate_cmd(Some(&foreign), engine.path()),
                "foreign-root hook not recognized: {foreign}"
            );
        }
        // A node-launcher invocation embedding the script path is still ours.
        assert!(is_slopgate_cmd(
            Some("/tmp/slopgate-verify/hooks/edit-hook.sh"),
            engine.path()
        ));
        // Unrelated commands and None stay false.
        assert!(!is_slopgate_cmd(Some("/usr/bin/prettier"), engine.path()));
        assert!(!is_slopgate_cmd(None, engine.path()));
    }
}
