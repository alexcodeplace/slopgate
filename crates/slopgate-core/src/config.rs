//! Native TOML config resolver — mirrors `src/config.mjs` `resolveConfig`.

use crate::rules::packs::{self, Pattern, UxPack};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateAllow {
    pub file: HashSet<String>,
    pub staged: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub repo_root: String,
    pub config_dir: String,
    pub roots: Vec<String>,
    pub roots_rel: Vec<String>,
    pub exts: HashSet<String>,
    pub skip_dirs: HashSet<String>,
    pub patterns: Vec<Pattern>,
    pub ast_rule_dirs: Vec<String>,
    pub checkers: BTreeMap<String, serde_json::Value>,
    pub ast_disable: HashSet<String>,
    pub baseline_path: String,
    pub suppressions_path: String,
    pub fixtures_dirs: Vec<String>,
    pub checker_concurrency: u32,
    pub gate: GateAllow,
    pub ux_ast_severity: BTreeMap<String, String>,
    pub ux_ast_all: HashSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    #[serde(default)]
    roots: Vec<String>,
    exts: Option<Vec<String>>,
    skip_dirs: Option<Vec<String>>,
    #[serde(default)]
    baseline: Vec<String>,
    #[serde(default)]
    stack: Vec<String>,
    #[serde(default)]
    rules: Vec<String>,
    ast_rules: Option<String>,
    #[serde(default)]
    ast_disable: Vec<String>,
    #[serde(default)]
    checkers: BTreeMap<String, toml::Value>,
    gate: Option<RawGate>,
    suppressions: Option<String>,
    fixtures: Option<String>,
    checker_concurrency: Option<u32>,
    #[serde(default)]
    ux: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize)]
struct RawGate {
    #[serde(default)]
    file: Vec<String>,
    #[serde(default)]
    staged: Vec<String>,
}

fn git_root(from_dir: &Path) -> Option<PathBuf> {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .map(PathBuf::from)
}

fn resolve_path(config_dir: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        config_dir.join(p)
    }
}

fn path_to_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

fn toml_to_json(v: toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::json!(i),
        toml::Value::Float(f) => serde_json::json!(f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(toml_to_json).collect())
        }
        toml::Value::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t {
                map.insert(k, toml_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
    }
}

fn process_checkers(raw: BTreeMap<String, toml::Value>) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for (name, v) in raw {
        match &v {
            toml::Value::Boolean(false) => continue,
            toml::Value::Boolean(true) => {
                out.insert(name, serde_json::json!({}));
            }
            _ => {
                out.insert(name, toml_to_json(v));
            }
        }
    }
    out
}

fn resolve_ux_severity(value: &toml::Value, pack: &UxPack) -> Option<String> {
    match value {
        toml::Value::Boolean(false) => None,
        toml::Value::Boolean(true) => Some(pack.default_severity.clone()),
        toml::Value::String(s) => {
            if s == "advisory" || s == "report" {
                Some("medium".to_string())
            } else {
                Some(s.clone())
            }
        }
        _ => None,
    }
}

fn ux_ast_all_ids() -> HashSet<String> {
    packs::ux_packs()
        .values()
        .flat_map(|p| p.ast_ids.iter().cloned())
        .collect()
}

/// Validate a pattern's required fields and regex compile-ability.
pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for (k, v) in [
        ("id", p.id.as_str()),
        ("severity", p.severity.as_str()),
        ("pattern", p.pattern.as_str()),
        ("resolution", p.resolution.as_str()),
    ] {
        if v.is_empty() {
            return Err(format!(
                "slopgate: rule missing \"{k}\" (id={})",
                if p.id.is_empty() { "?" } else { &p.id }
            ));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

/// Validate a raw regex pattern + optional flags by compiling it through the SAME
/// engine used for matching (`regex_engine::compile_line_regex` — fancy-regex with
/// JS-compat translation). Validating with the strict `regex` crate wrongly rejected
/// look-ahead patterns that both the matcher and JS `RegExp` accept (e.g. baseline
/// `raw-rgb-color`'s `(?![^)]*var\()`).
pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    crate::regex_engine::compile_line_regex(pattern, flags.unwrap_or(""))
        .map(|_| ())
        .map_err(|e| format!("slopgate: bad regex: {e}"))
}

fn resolve_inner(
    raw: RawConfig,
    config_dir: PathBuf,
    repo_root: PathBuf,
) -> Result<ResolvedConfig, String> {
    let baseline_packs = packs::baseline_packs();
    let stack_packs = packs::stack_packs();
    let ux_packs = packs::ux_packs();

    let mut patterns: Vec<Pattern> = Vec::new();

    for name in &raw.baseline {
        let Some(pack) = baseline_packs.get(name) else {
            let known: Vec<&String> = baseline_packs.keys().collect();
            return Err(format!(
                "slopgate: unknown baseline pack \"{name}\" (known: {})",
                known
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        for p in pack {
            validate_pattern(p).map_err(|e| format!("{e} (from baseline:{name})"))?;
            patterns.push(p.clone());
        }
    }

    for name in &raw.stack {
        let Some(pack) = stack_packs.get(name) else {
            let known: Vec<&String> = stack_packs.keys().collect();
            return Err(format!(
                "slopgate: unknown stack pack \"{name}\" (known: {})",
                known
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        for p in pack {
            validate_pattern(p).map_err(|e| format!("{e} (from stack:{name})"))?;
            patterns.push(p.clone());
        }
    }

    for rel_path in &raw.rules {
        let abs = resolve_path(&config_dir, rel_path);
        let pack = packs::load_project_pack(&abs)?;
        for (pack_name, patterns_in) in &pack {
            for p in patterns_in {
                validate_pattern(p)
                    .map_err(|e| format!("{e} (from project:{pack_name} in {})", abs.display()))?;
                patterns.push(p.clone());
            }
        }
    }

    let mut ux_ast_severity: BTreeMap<String, String> = BTreeMap::new();
    let mut ux_enabled_ast = false;

    for (key, value) in &raw.ux {
        let Some(pack) = ux_packs.get(key) else {
            let known: Vec<&String> = ux_packs.keys().collect();
            return Err(format!(
                "slopgate: unknown ux sub-module \"{key}\" (known: {})",
                known
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let Some(sev) = resolve_ux_severity(value, pack) else {
            continue;
        };
        for p in &pack.regex {
            let mut overridden = p.clone();
            overridden.severity = sev.clone();
            validate_pattern(&overridden).map_err(|e| format!("{e} (from ux:{key})"))?;
            patterns.push(overridden);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), sev.clone());
            ux_enabled_ast = true;
        }
    }

    // Dedupe by id: last value wins, first-occurrence order (JS Map semantics).
    let mut by_id: IndexMap<String, Pattern> = IndexMap::new();
    for p in patterns {
        by_id.insert(p.id.clone(), p);
    }
    let patterns: Vec<Pattern> = by_id.into_values().collect();

    let mut ast_rule_dirs = vec![repo_root.join("rules/baseline/ast")];
    if let Some(ast_rules) = &raw.ast_rules {
        let abs = resolve_path(&config_dir, ast_rules);
        if abs.is_dir() {
            ast_rule_dirs.push(abs);
        }
    }
    if ux_enabled_ast {
        ast_rule_dirs.push(repo_root.join("rules/ux/ast"));
    }

    let checkers = process_checkers(raw.checkers);

    let roots_rel = if raw.roots.is_empty() {
        vec!["src".to_string()]
    } else {
        raw.roots
    };
    let roots: Vec<String> = roots_rel
        .iter()
        .map(|r| path_to_string(repo_root.join(r)))
        .collect();

    let exts: HashSet<String> = raw
        .exts
        .unwrap_or_else(|| vec![".ts".into(), ".tsx".into(), ".astro".into()])
        .into_iter()
        .collect();

    let skip_dirs: HashSet<String> = raw
        .skip_dirs
        .unwrap_or_else(|| vec!["node_modules".into(), "dist".into(), "tests".into()])
        .into_iter()
        .collect();

    let gate_file: HashSet<String> = raw
        .gate
        .as_ref()
        .map(|g| g.file.iter().cloned().collect())
        .unwrap_or_else(|| ["critical", "high"].iter().map(|s| s.to_string()).collect());

    let gate_staged: HashSet<String> = raw
        .gate
        .as_ref()
        .map(|g| g.staged.iter().cloned().collect())
        .unwrap_or_else(|| ["critical", "high"].iter().map(|s| s.to_string()).collect());

    let suppressions_path = raw
        .suppressions
        .as_ref()
        .map(|s| path_to_string(resolve_path(&config_dir, s)))
        .unwrap_or_else(|| path_to_string(config_dir.join("suppressions.json")));

    let mut fixtures_dirs = vec![path_to_string(repo_root.join("rules/baseline/fixtures"))];
    if let Some(fixtures) = &raw.fixtures {
        fixtures_dirs.push(path_to_string(resolve_path(&config_dir, fixtures)));
    }

    let baseline_path = path_to_string(config_dir.join("baseline.json"));
    Ok(ResolvedConfig {
        repo_root: path_to_string(repo_root),
        config_dir: path_to_string(config_dir),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns,
        ast_rule_dirs: ast_rule_dirs.into_iter().map(path_to_string).collect(),
        checkers,
        ast_disable: raw.ast_disable.into_iter().collect(),
        baseline_path,
        suppressions_path,
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate: GateAllow {
            file: gate_file,
            staged: gate_staged,
        },
        ux_ast_severity,
        ux_ast_all: ux_ast_all_ids(),
    })
}

/// Resolve a TOML config file at `path` into a fully expanded `ResolvedConfig`.
pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs_config = {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| format!("slopgate: cwd unavailable: {e}"))?
                .join(p)
        }
    };
    if !abs_config.is_file() {
        return Err(format!(
            "slopgate: config not found: {}",
            abs_config.display()
        ));
    }
    let config_dir = abs_config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let repo_root = git_root(&config_dir)
        .unwrap_or_else(|| config_dir.parent().unwrap_or(&config_dir).to_path_buf());

    let contents = std::fs::read_to_string(&abs_config)
        .map_err(|e| format!("slopgate: read config {}: {e}", abs_config.display()))?;
    if let Some(msg) = legacy_js_config_error(&abs_config, &contents) {
        return Err(msg);
    }
    let raw: RawConfig = toml::from_str(&contents).map_err(|e| {
        if let Some(msg) = legacy_js_config_error_on_parse_failure(&abs_config, &contents) {
            msg
        } else {
            format!("slopgate: parse config {}: {e}", abs_config.display())
        }
    })?;
    resolve_inner(raw, config_dir, repo_root)
}

/// True when `contents` reads as a JavaScript module rather than TOML — the
/// signature of a pre-TOML legacy `config.mjs` (`export default {…}` /
/// `module.exports = {…}`). Used to turn a cryptic TOML parse error into an
/// actionable migration message.
fn looks_like_js_config(contents: &str) -> bool {
    contents.contains("export default") || contents.contains("module.exports")
}

/// Actionable error when the config path is a legacy JavaScript config (by
/// extension). The engine reads TOML only; a `.mjs`/`.cjs`/`.js` config is the
/// pre-rebrand format and a stale pre-commit hook can still point a TOML engine
/// at it (the "parsed as TOML and aborts" failure). Returns `None` for `.toml`.
fn legacy_js_config_error(abs_config: &Path, contents: &str) -> Option<String> {
    let ext = abs_config
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_js_ext = matches!(ext.as_str(), "mjs" | "cjs" | "js");
    if is_js_ext || (ext != "toml" && looks_like_js_config(contents)) {
        return Some(legacy_migration_message(abs_config));
    }
    None
}

/// Fallback for a `.toml`-named file whose body is actually JavaScript (a stale
/// hook renamed the path but the file was never migrated). Only fires when the
/// TOML parse already failed.
fn legacy_js_config_error_on_parse_failure(abs_config: &Path, contents: &str) -> Option<String> {
    if looks_like_js_config(contents) {
        Some(legacy_migration_message(abs_config))
    } else {
        None
    }
}

fn legacy_migration_message(abs_config: &Path) -> String {
    format!(
        "slopgate: {} is a legacy JavaScript config; the engine reads TOML.\n  \
         Run `slopgate init` in the repo to migrate it to .slopgate/config.toml, \
         then remove the stale pre-commit hook block (re-run `slopgate init` rewrites it).",
        abs_config.display()
    )
}

/// Resolve inline TOML (unit tests). Uses `.` as `config_dir` and git/cwd for `repo_root`.
pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = PathBuf::from(".");
    let repo_root = git_root(&config_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let raw: RawConfig =
        toml::from_str(toml_src).map_err(|e| format!("slopgate: parse config: {e}"))?;
    resolve_inner(raw, config_dir, repo_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn cfg_path() -> String {
        format!("{}/tests/fixtures/config.toml", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn validate_pattern_rejects_bad_regex() {
        assert!(validate_pattern_str("a(b", Some("")).is_err());
        assert!(validate_pattern_str(r"\d+", Some("i")).is_ok());
    }

    #[test]
    fn defaults_present() {
        let c = resolve_config(&cfg_path()).unwrap();
        assert!(c.exts.contains(".ts") && c.exts.contains(".tsx"));
        assert!(c.skip_dirs.contains("node_modules"));
        assert!(c.gate.staged.contains("critical") && c.gate.staged.contains("high"));
        assert_eq!(c.checker_concurrency, 3);
    }

    #[test]
    fn resolves_baseline_packs_and_dedupes() {
        let c = resolve_config(&cfg_path()).unwrap();
        let ids: Vec<&str> = c.patterns.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.iter().any(|i| i.starts_with("no-stubs")));
        let mut seen = std::collections::HashSet::new();
        for p in &c.patterns {
            assert!(seen.insert(&p.id), "dup id {}", p.id);
        }
    }

    #[test]
    fn matches_js_resolver_machine_surface() {
        let vp = format!(
            "{}/tests/parity_vectors/resolved_config.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let js: Value = serde_json::from_str(&std::fs::read_to_string(vp).unwrap()).unwrap();
        let rust = resolve_config(&cfg_path()).unwrap();
        let js_ids = sorted_id_sev(&js["patterns"]);
        let mut rust_ids: Vec<String> = rust
            .patterns
            .iter()
            .map(|p| format!("{}:{}", p.id, p.severity))
            .collect();
        rust_ids.sort();
        assert_eq!(
            rust_ids, js_ids,
            "pattern id:severity set must match JS resolver"
        );
        assert_eq!(sorted_strs(&js["exts"]), sorted_set(&rust.exts));
        assert_eq!(
            sorted_strs(&js["gate"]["staged"]),
            sorted_set(&rust.gate.staged)
        );
    }

    #[test]
    fn unknown_baseline_pack_errors() {
        assert!(resolve_config_str("baseline = [\"nope\"]\n").is_err());
    }

    fn write_temp_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn resolves_project_rule_pack() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r#"{"proj":[{"id":"proj-x","severity":"high","pattern":"FORBIDDEN_TOKEN","resolution":"remove it","canary":"FORBIDDEN_TOKEN","excludeGlobs":["**/*.md"]}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/proj.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let proj = cfg
            .patterns
            .iter()
            .find(|p| p.id == "proj-x")
            .expect("proj-x pattern");
        assert_eq!(proj.resolution, "remove it");
        assert_eq!(proj.exclude_globs, Some(vec!["**/*.md".to_string()]));
    }

    #[test]
    fn project_rule_pack_bad_regex_errors_with_pack_tag() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r#"{"proj":[{"id":"bad","severity":"high","pattern":"(","resolution":"fix it"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/proj.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(err.contains("(from project:proj in"), "err={err}");
        assert!(err.contains("proj.json"), "err={err}");
    }

    #[test]
    fn project_rule_pack_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/nope.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(
            err.contains("cannot read project rule pack") && err.contains("nope.json"),
            "err={err}"
        );
    }

    #[test]
    fn resolves_two_project_rule_packs() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/a.json"),
            r#"{"pack-a":[{"id":"proj-a","severity":"high","pattern":"TOKEN_A","resolution":"remove a","canary":"TOKEN_A"}]}"#,
        );
        write_temp_file(
            &config_dir.join("rules/b.json"),
            r#"{"pack-b":[{"id":"proj-b","severity":"medium","pattern":"TOKEN_B","resolution":"remove b","canary":"TOKEN_B"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/a.json", "./rules/b.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let ids: std::collections::HashSet<_> =
            cfg.patterns.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains("proj-a"));
        assert!(ids.contains("proj-b"));
    }

    #[test]
    fn project_rule_pack_second_path_missing_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/a.json"),
            r#"{"pack-a":[{"id":"proj-a","severity":"high","pattern":"TOKEN_A","resolution":"remove a","canary":"TOKEN_A"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/a.json", "./rules/b.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(
            err.contains("cannot read project rule pack") && err.contains("b.json"),
            "err={err}"
        );
    }

    #[test]
    fn project_rule_pack_malformed_json_errors_at_resolve_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        let pack_path = config_dir.join("rules/bad.json");
        write_temp_file(&pack_path, "{not json");
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/bad.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(err.contains("invalid project rule pack"), "err={err}");
        assert!(err.contains("bad.json"), "err={err}");
    }

    #[test]
    fn project_rule_pack_overrides_colliding_baseline_id() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        let r_baseline = {
            write_temp_file(&config_dir.join("config.toml"), r#"baseline = ["raw-hex"]"#);
            let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
            let baseline_rule = cfg
                .patterns
                .iter()
                .find(|p| p.id == "raw-hex-color")
                .expect("raw-hex-color from baseline");
            baseline_rule.resolution.clone()
        };
        let r_project = "PROJECT override resolution";
        assert_ne!(r_project, r_baseline);
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r##"{"proj":[{"id":"raw-hex-color","severity":"high","pattern":"#PROJECT","resolution":"PROJECT override resolution","canary":"#abc"}]}"##,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"baseline = ["raw-hex"]
rules = ["./rules/proj.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let matches: Vec<_> = cfg
            .patterns
            .iter()
            .filter(|p| p.id == "raw-hex-color")
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].resolution, r_project);
    }

    fn sorted_id_sev(v: &Value) -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}:{}",
                    p["id"].as_str().unwrap(),
                    p["severity"].as_str().unwrap()
                )
            })
            .collect();
        out.sort();
        out
    }

    fn sorted_strs(v: &Value) -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        out.sort();
        out
    }

    fn sorted_set(s: &std::collections::HashSet<String>) -> Vec<String> {
        let mut out: Vec<String> = s.iter().cloned().collect();
        out.sort();
        out
    }
}
