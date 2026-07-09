//! Legacy `.mjs` → `.toml` config migration.
//!
//! Before the TOML rewrite, a slopgate project config was JavaScript
//! (`.slopgate/config.mjs`, or `.slop-gate/config.mjs` before the brand rename)
//! that `export default`-ed a plain config object. The current engine reads TOML
//! only, so a stale pre-commit hook pointing the engine at a `.mjs` produces the
//! "parsed as TOML and aborts" failure (see `config::resolve_config`).
//!
//! Migration executes the legacy module with `node` to obtain its resolved export
//! as JSON — the only correct way to read arbitrary JS — then serialises that into
//! a canonical-ordered `config.toml`. Project-owned JS rule packs (`rules: [...]`
//! ending in `.mjs`/`.cjs`/`.js`) cannot be loaded by the native engine; they are
//! dropped from the migrated config and reported so the author re-authors them as
//! JSON regex rule packs (listed in `rules = [...]`).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of attempting to migrate a legacy JS config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateOutcome {
    /// A legacy config was found and written to `to` as TOML. `dropped_rules`
    /// lists any `rules:` entries (legacy JS rule packs) that the native engine
    /// cannot execute — they are emptied from the migrated config and reported so
    /// the author re-authors them as JSON regex rule packs (listed in `rules = [...]`).
    Migrated {
        from: PathBuf,
        to: PathBuf,
        dropped_rules: Vec<String>,
    },
    /// No legacy config present (the common case) — nothing to do.
    NoLegacy,
    /// A legacy config was found but migration failed (e.g. `node` unavailable or
    /// the module threw). The caller should report `reason` and fall back to a
    /// fresh scaffold rather than leave the project ungated.
    Failed { from: PathBuf, reason: String },
}

/// Locate a legacy JS config under `target_dir`. Checks the current config dir
/// first, then the pre-rename `.slop-gate` dir. Returns the first that exists.
pub fn find_legacy_config(target_dir: &Path) -> Option<PathBuf> {
    const CANDIDATES: [&str; 4] = [
        ".slopgate/config.mjs",
        ".slopgate/config.cjs",
        ".slop-gate/config.mjs",
        ".slop-gate/config.cjs",
    ];
    CANDIDATES
        .iter()
        .map(|rel| target_dir.join(rel))
        .find(|p| p.is_file())
}

/// Execute a legacy JS config with `node` and return its default export as JSON.
///
/// Uses dynamic `import()` (handles both ESM `export default` and CJS
/// `module.exports`) via a file URL so paths with spaces survive. Any failure —
/// `node` missing, module throw, non-JSON output — becomes an `Err`.
fn read_legacy_export(path: &Path) -> Result<Value, String> {
    let abs = path
        .canonicalize()
        .map_err(|e| format!("resolve {}: {e}", path.display()))?;
    // Dynamic imports only → runs under the default CJS eval context (no
    // --input-type needed). argv[1] is the config path.
    const SCRIPT: &str = r#"
const p = process.argv[1];
import('node:url')
  .then(({ pathToFileURL }) => import(pathToFileURL(p).href))
  .then((m) => {
    const cfg = m && m.default !== undefined ? m.default : m;
    if (cfg === null || typeof cfg !== 'object') {
      throw new Error('config default export is not an object');
    }
    process.stdout.write(JSON.stringify(cfg));
  })
  .catch((e) => {
    process.stderr.write(String((e && e.message) || e));
    process.exit(3);
  });
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(SCRIPT)
        .arg(&abs)
        .output()
        .map_err(|e| {
            format!("node not available to read legacy config ({e}); install Node or migrate manually")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "node failed to evaluate {}: {}",
            abs.display(),
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|e| format!("legacy config export was not JSON-serialisable: {e}"))
}

/// TOML-escape and quote a string scalar.
fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// TOML array of string scalars.
fn toml_str_array(values: &[String]) -> String {
    let inner: Vec<String> = values.iter().map(|v| toml_string(v)).collect();
    format!("[{}]", inner.join(", "))
}

fn str_array_field(v: &Value, key: &str) -> Option<Vec<String>> {
    v.get(key)?.as_array().map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect()
    })
}

/// Convert a legacy export JSON object into a canonical-ordered `config.toml`
/// string. Returns the TOML plus any dropped JS rule-pack paths.
///
/// Field order is fixed so the emitted TOML is always valid (scalars/arrays
/// before `[table]` sections) and matches `scaffold::format_config_toml`.
pub fn legacy_json_to_toml(export: &Value) -> Result<(String, Vec<String>), String> {
    if !export.is_object() {
        return Err("legacy config export is not an object".into());
    }
    let mut out = String::new();

    // roots — always present (engine needs it); default to empty if absent.
    let roots = str_array_field(export, "roots").unwrap_or_default();
    out.push_str(&format!("roots = {}\n", toml_str_array(&roots)));

    // exts / skipDirs — only emit when present (engine has defaults otherwise).
    if let Some(exts) = str_array_field(export, "exts") {
        out.push_str(&format!("exts = {}\n", toml_str_array(&exts)));
    }
    if let Some(skip) = str_array_field(export, "skipDirs") {
        out.push_str(&format!("skipDirs = {}\n", toml_str_array(&skip)));
    }

    // baseline / stack opt-ins — preserve exactly.
    let baseline = str_array_field(export, "baseline").unwrap_or_default();
    out.push_str(&format!("baseline = {}\n", toml_str_array(&baseline)));
    if let Some(stack) = str_array_field(export, "stack") {
        if !stack.is_empty() {
            out.push_str(&format!("stack = {}\n", toml_str_array(&stack)));
        }
    }

    // rules — legacy project JS rule packs (.mjs/.cjs/.js). The native loader
    // reads JSON regex rule packs, not executable JS, so these legacy entries
    // cannot be carried over. Always emit empty and report every entry for
    // re-authoring as JSON packs.
    let dropped_rules = str_array_field(export, "rules").unwrap_or_default();
    out.push_str("rules = []\n");

    let ast_rules = export
        .get("astRules")
        .and_then(Value::as_str)
        .unwrap_or("./rules/ast");
    out.push_str(&format!("astRules = {}\n", toml_string(ast_rules)));

    let ast_disable = str_array_field(export, "astDisable").unwrap_or_default();
    out.push_str(&format!("astDisable = {}\n", toml_str_array(&ast_disable)));

    let suppressions = export
        .get("suppressions")
        .and_then(Value::as_str)
        .unwrap_or("./suppressions.json");
    out.push_str(&format!("suppressions = {}\n", toml_string(suppressions)));

    let fixtures = export
        .get("fixtures")
        .and_then(Value::as_str)
        .unwrap_or("./fixtures");
    out.push_str(&format!("fixtures = {}\n", toml_string(fixtures)));

    if let Some(cc) = export.get("checkerConcurrency").and_then(Value::as_u64) {
        out.push_str(&format!("checkerConcurrency = {cc}\n"));
    }

    // [ux] — string severities.
    if let Some(Value::Object(ux)) = export.get("ux") {
        if !ux.is_empty() {
            out.push_str("\n[ux]\n");
            let mut keys: Vec<&String> = ux.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(sev) = ux.get(k).and_then(Value::as_str) {
                    out.push_str(&format!("{k} = {}\n", toml_string(sev)));
                }
            }
        }
    }

    // [checkers.*] — mirrors scaffold::format_config_toml emit.
    if let Some(Value::Object(checkers)) = export.get("checkers") {
        let mut names: Vec<&String> = checkers.keys().collect();
        names.sort();
        for name in names {
            match checkers.get(name) {
                Some(Value::Bool(true)) => out.push_str(&format!("\n[checkers.{name}]\n")),
                Some(Value::Object(fields)) => {
                    out.push_str(&format!("\n[checkers.{name}]\n"));
                    let mut keys: Vec<&String> = fields.keys().collect();
                    keys.sort();
                    for key in keys {
                        if let Some(val) = fields.get(key) {
                            out.push_str(&format!("{key} = {}\n", json_scalar_to_toml(val)));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // [gate] — file/staged severity allowlists; preserve or default.
    out.push_str("\n[gate]\n");
    let gate = export.get("gate");
    let gate_file = gate
        .and_then(|g| str_array_field(g, "file"))
        .unwrap_or_else(|| vec!["critical".into(), "high".into()]);
    let gate_staged = gate
        .and_then(|g| str_array_field(g, "staged"))
        .unwrap_or_else(|| vec!["critical".into(), "high".into()]);
    out.push_str(&format!("file = {}\n", toml_str_array(&gate_file)));
    out.push_str(&format!("staged = {}\n", toml_str_array(&gate_staged)));

    Ok((out, dropped_rules))
}

/// Render a JSON scalar (checker field value) as a TOML scalar.
fn json_scalar_to_toml(v: &Value) -> String {
    match v {
        Value::String(s) => toml_string(s),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(json_scalar_to_toml).collect();
            format!("[{}]", parts.join(", "))
        }
        // Objects/null at this depth are unexpected for checker fields; emit as a
        // string so the migration never produces invalid TOML.
        other => toml_string(&other.to_string()),
    }
}

/// Attempt to migrate a legacy JS config in `target_dir` to `.slopgate/config.toml`.
///
/// Caller invokes this only when no `.slopgate/config.toml` exists yet. Never
/// overwrites an existing TOML config.
pub fn migrate_legacy_config(target_dir: &Path) -> MigrateOutcome {
    let toml_path = target_dir.join(".slopgate/config.toml");
    if toml_path.is_file() {
        return MigrateOutcome::NoLegacy;
    }
    let Some(legacy) = find_legacy_config(target_dir) else {
        return MigrateOutcome::NoLegacy;
    };

    let export = match read_legacy_export(&legacy) {
        Ok(v) => v,
        Err(reason) => return MigrateOutcome::Failed { from: legacy, reason },
    };
    let (toml_src, dropped_rules) = match legacy_json_to_toml(&export) {
        Ok(r) => r,
        Err(reason) => return MigrateOutcome::Failed { from: legacy, reason },
    };

    // Prove the migrated TOML actually resolves before writing it. The legacy
    // config may name a baseline/stack pack the native engine doesn't ship, or a
    // field that no longer maps — writing it blindly would leave the project
    // bricked (now loudly). On failure, surface the resolver's message (it names
    // the offending field) and fall back to a fresh scaffold rather than persist a
    // broken config.
    if let Err(e) = crate::config::resolve_config_str(&toml_src) {
        return MigrateOutcome::Failed {
            from: legacy,
            reason: format!("migrated config did not resolve ({e})"),
        };
    }

    if let Some(parent) = toml_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return MigrateOutcome::Failed {
                from: legacy,
                reason: format!("mkdir {}: {e}", parent.display()),
            };
        }
    }
    if let Err(e) = std::fs::write(&toml_path, toml_src) {
        return MigrateOutcome::Failed {
            from: legacy,
            reason: format!("write {}: {e}", toml_path.display()),
        };
    }

    MigrateOutcome::Migrated {
        from: legacy,
        to: toml_path,
        dropped_rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config;
    use serde_json::json;
    use std::fs;

    fn node_available() -> bool {
        Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn legacy_json_to_toml_round_trips_through_resolver() {
        let export = json!({
            "roots": ["src", "app"],
            "exts": [".ts", ".tsx"],
            "skipDirs": ["node_modules", "dist"],
            "baseline": ["no-stubs", "ts-suppress"],
            "rules": ["./rules/legacy.mjs", "./rules/custom.js"],
            "astRules": "./rules/ast",
            "astDisable": ["some-id"],
            "checkers": { "tsc": true, "jscpd": { "threshold": 5 } },
            "gate": { "file": ["critical"], "staged": ["critical", "high"] },
            "suppressions": "./suppressions.json",
            "fixtures": "./fixtures"
        });
        let (toml_src, dropped) = legacy_json_to_toml(&export).unwrap();
        // The native engine has no project-rule-pack loader → all entries dropped.
        assert_eq!(
            dropped,
            vec!["./rules/legacy.mjs".to_string(), "./rules/custom.js".to_string()]
        );
        assert!(toml_src.contains("rules = []"));

        // Must parse + resolve as a real config.
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join(".slopgate");
        fs::create_dir_all(base.join("rules/ast")).unwrap();
        let cfg_path = base.join("config.toml");
        fs::write(&cfg_path, &toml_src).unwrap();
        let cfg = resolve_config(&cfg_path.to_string_lossy()).unwrap();
        assert!(cfg.roots_rel.iter().any(|r| r == "src"));
        assert!(cfg.exts.contains(".ts"));
        assert!(cfg.checkers.contains_key("tsc"));
        assert!(cfg.gate.file.contains("critical"));
        assert!(!cfg.gate.file.contains("high"));
    }

    #[test]
    fn legacy_json_to_toml_defaults_optional_fields() {
        let export = json!({ "roots": ["src"], "baseline": [] });
        let (toml_src, dropped) = legacy_json_to_toml(&export).unwrap();
        assert!(dropped.is_empty());
        assert!(toml_src.contains("astRules = \"./rules/ast\""));
        assert!(toml_src.contains("suppressions = \"./suppressions.json\""));
        assert!(toml_src.contains("file = [\"critical\", \"high\"]"));
    }

    #[test]
    fn find_legacy_config_prefers_current_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".slop-gate")).unwrap();
        fs::write(root.join(".slop-gate/config.mjs"), "export default {}\n").unwrap();
        let found = find_legacy_config(root).unwrap();
        assert!(found.ends_with(".slop-gate/config.mjs"));
    }

    #[test]
    fn find_legacy_config_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_legacy_config(dir.path()).is_none());
    }

    #[test]
    fn migrate_end_to_end_executes_node() {
        if !node_available() {
            eprintln!("skipping: node not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".slop-gate")).unwrap();
        fs::write(
            root.join(".slop-gate/config.mjs"),
            "export default {\n  roots: ['src'],\n  exts: ['.ts'],\n  skipDirs: ['node_modules'],\n  baseline: ['no-stubs'],\n  rules: [],\n  checkers: { tsc: true },\n  gate: { file: ['critical', 'high'], staged: ['critical', 'high'] },\n};\n",
        )
        .unwrap();

        let outcome = migrate_legacy_config(root);
        match outcome {
            MigrateOutcome::Migrated { to, dropped_rules, .. } => {
                assert!(to.is_file());
                assert!(dropped_rules.is_empty());
                let cfg = resolve_config(&to.to_string_lossy()).unwrap();
                assert!(cfg.roots_rel.iter().any(|r| r == "src"));
                assert!(cfg.checkers.contains_key("tsc"));
            }
            other => panic!("expected Migrated, got {other:?}"),
        }
    }

    #[test]
    fn migrate_noop_when_toml_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".slopgate")).unwrap();
        fs::write(root.join(".slopgate/config.toml"), "roots = [\"src\"]\n").unwrap();
        fs::create_dir_all(root.join(".slop-gate")).unwrap();
        fs::write(root.join(".slop-gate/config.mjs"), "export default {}\n").unwrap();
        assert_eq!(migrate_legacy_config(root), MigrateOutcome::NoLegacy);
    }
}
