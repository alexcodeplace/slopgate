use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern, UxPack};
use fancy_regex::Regex;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct GateAllow {
    pub file: HashSet<String>,
    pub staged: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub repo_root: String,
    pub config_dir: String,
    pub roots: Vec<String>,
    pub roots_rel: Vec<String>,
    pub exts: HashSet<String>,
    pub skip_dirs: HashSet<String>,
    pub patterns: Vec<Pattern>,
    pub ast_rule_dirs: Vec<String>,
    pub checkers: BTreeMap<String, JsonValue>,
    pub ast_disable: HashSet<String>,
    pub baseline_path: String,
    pub suppressions_path: String,
    pub fixtures_dirs: Vec<String>,
    pub checker_concurrency: u32,
    pub gate: GateAllow,
    pub ux_ast_severity: BTreeMap<String, String>,
    pub ux_ast_all: HashSet<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    #[serde(default)]
    roots: Option<Vec<String>>,
    #[serde(default)]
    exts: Option<Vec<String>>,
    #[serde(default)]
    skip_dirs: Option<Vec<String>>,
    #[serde(default)]
    baseline: Option<Vec<String>>,
    #[serde(default)]
    stack: Option<Vec<String>>,
    #[serde(default)]
    rules: Option<Vec<String>>,
    #[serde(default)]
    ast_rules: Option<String>,
    #[serde(default)]
    ast_disable: Option<Vec<String>>,
    #[serde(default)]
    suppressions: Option<String>,
    #[serde(default)]
    fixtures: Option<String>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
    #[serde(default)]
    checkers: BTreeMap<String, toml::Value>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: BTreeMap<String, toml::Value>,
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs = absolute_from_cwd(Path::new(path))?;
    let meta = fs::metadata(&abs).map_err(|_| format!("slopgate: config not found: {}", abs.display()))?;
    if !meta.is_file() {
        return Err(format!("slopgate: config not found: {}", abs.display()));
    }

    let config_dir = abs
        .parent()
        .ok_or_else(|| format!("slopgate: config has no parent directory: {}", abs.display()))?
        .to_path_buf();
    let repo_root = git_root(&config_dir).unwrap_or_else(|| config_dir.parent().unwrap_or(&config_dir).to_path_buf());
    let src = fs::read_to_string(&abs).map_err(|e| format!("slopgate: failed to read config: {e}"))?;
    resolve_config_inner(&repo_root, &config_dir, &src)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let repo_root = std::env::current_dir().map_err(|e| format!("slopgate: current dir unavailable: {e}"))?;
    let config_dir = repo_root.join(".slopgate");
    resolve_config_inner(&repo_root, &config_dir, toml_src)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for (key, value) in [
        ("id", p.id.as_str()),
        ("severity", p.severity.as_str()),
        ("pattern", p.pattern.as_str()),
        ("resolution", p.resolution.as_str()),
    ] {
        if value.is_empty() {
            return Err(format!("slopgate: rule missing \"{key}\" (id={})", if p.id.is_empty() { "?" } else { p.id.as_str() }));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let _ = strip_stateful_flags(flags);
    Regex::new(pattern).map(|_| ()).map_err(|e| e.to_string())
}

fn resolve_config_inner(repo_root: &Path, config_dir: &Path, toml_src: &str) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: config parse error: {e}"))?;
    let baseline = baseline_packs();
    let stacks = stack_packs();
    let ux = ux_packs();

    let mut patterns = Vec::new();

    for name in raw.baseline.unwrap_or_default() {
        let pack = baseline.get(&name).ok_or_else(|| unknown_pack("baseline", &name, baseline.keys()))?;
        push_pack_patterns(&mut patterns, pack, &format!("baseline:{name}"))?;
    }

    for name in raw.stack.unwrap_or_default() {
        let pack = stacks.get(&name).ok_or_else(|| unknown_pack("stack", &name, stacks.keys()))?;
        push_pack_patterns(&mut patterns, pack, &format!("stack:{name}"))?;
    }

    for rel_path in raw.rules.unwrap_or_default() {
        return Err(format!("slopgate: project rule packs unsupported in Rust resolver: {rel_path}"));
    }

    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    for (key, value) in raw.ux {
        let pack = ux.get(&key).ok_or_else(|| unknown_pack("ux sub-module", &key, ux.keys()))?;
        let sev = resolve_ux_severity(&value, pack)?;
        let Some(sev) = sev else { continue };
        for rule in &pack.regex {
            let mut normalized = rule.clone();
            normalized.severity = sev.clone();
            validate_pattern(&normalized)?;
            normalized.flags = strip_stateful_flags(normalized.flags.as_deref());
            patterns.push(normalized);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), sev.clone());
            ux_enabled_ast = true;
        }
    }

    let patterns = dedupe_patterns(patterns);
    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel.iter().map(|r| abs_to_string(abs_path(repo_root, r))).collect();
    let exts = raw.exts.unwrap_or_else(|| vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()]).into_iter().collect();
    let skip_dirs = raw.skip_dirs.unwrap_or_else(|| vec!["node_modules".to_string(), "dist".to_string(), "tests".to_string()]).into_iter().collect();
    let ast_disable = raw.ast_disable.unwrap_or_default().into_iter().collect();
    let ast_rule_dirs = resolve_ast_rule_dirs(repo_root, config_dir, raw.ast_rules.as_deref(), ux_enabled_ast);
    let checkers = resolve_checkers(raw.checkers)?;
    let baseline_path = abs_to_string(config_dir.join("baseline.json"));
    let suppressions_path = abs_to_string(match raw.suppressions {
        Some(path) => abs_path(config_dir, &path),
        None => config_dir.join("suppressions.json"),
    });
    let fixtures_dirs = resolve_fixtures_dirs(repo_root, config_dir, raw.fixtures.as_deref());
    let gate = resolve_gate(raw.gate);

    Ok(ResolvedConfig {
        repo_root: abs_to_string(repo_root),
        config_dir: abs_to_string(config_dir),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path,
        suppressions_path,
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate,
        ux_ast_severity,
        ux_ast_all: ux_all_ast_ids(),
    })
}

fn resolve_gate(raw: Option<RawGate>) -> GateAllow {
    let raw = raw.unwrap_or_default();
    GateAllow {
        file: raw
            .file
            .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
            .into_iter()
            .collect(),
        staged: raw
            .staged
            .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
            .into_iter()
            .collect(),
    }
}

fn resolve_checkers(raw: BTreeMap<String, toml::Value>) -> Result<BTreeMap<String, JsonValue>, String> {
    let mut out = BTreeMap::new();
    for (name, value) in raw {
        match value {
            toml::Value::Boolean(false) => {}
            toml::Value::Boolean(true) => {
                out.insert(name, JsonValue::Object(Default::default()));
            }
            other => {
                let json = serde_json::to_value(other).map_err(|e| format!("slopgate: checker {name} could not be converted to JSON: {e}"))?;
                out.insert(name, json);
            }
        }
    }
    Ok(out)
}

fn resolve_fixtures_dirs(repo_root: &Path, config_dir: &Path, fixtures: Option<&str>) -> Vec<String> {
    let mut dirs = vec![abs_to_string(abs_path(repo_root, "rules/baseline/fixtures"))];
    if let Some(fixtures) = fixtures {
        dirs.push(abs_to_string(abs_path(config_dir, fixtures)));
    }
    dirs
}

fn resolve_ast_rule_dirs(repo_root: &Path, config_dir: &Path, ast_rules: Option<&str>, ux_enabled_ast: bool) -> Vec<String> {
    let mut dirs = vec![abs_to_string(abs_path(repo_root, "rules/baseline/ast"))];
    if let Some(ast_rules) = ast_rules {
        let abs = abs_path(config_dir, ast_rules);
        if abs.is_dir() {
            dirs.push(abs_to_string(abs));
        }
    }
    if ux_enabled_ast {
        dirs.push(abs_to_string(abs_path(repo_root, "rules/ux/ast")));
    }
    dirs
}

fn resolve_ux_severity(value: &toml::Value, pack: &UxPack) -> Result<Option<String>, String> {
    match value {
        toml::Value::Boolean(false) => Ok(None),
        toml::Value::Boolean(true) => Ok(Some(pack.default_severity.clone())),
        toml::Value::String(s) => Ok(Some(match s.as_str() {
            "advisory" | "report" => "medium",
            other => other,
        }
        .to_string())),
        other => Err(format!("slopgate: ux severity must be true/false/string, got {other:?}")),
    }
}

fn push_pack_patterns(patterns: &mut Vec<Pattern>, pack: &[Pattern], src: &str) -> Result<(), String> {
    for rule in pack {
        let mut normalized = rule.clone();
        validate_pattern_with_src(&normalized, src)?;
        normalized.flags = strip_stateful_flags(normalized.flags.as_deref());
        patterns.push(normalized);
    }
    Ok(())
}

fn validate_pattern_with_src(p: &Pattern, src: &str) -> Result<(), String> {
    for (key, value) in [
        ("id", p.id.as_str()),
        ("severity", p.severity.as_str()),
        ("pattern", p.pattern.as_str()),
        ("resolution", p.resolution.as_str()),
    ] {
        if value.is_empty() {
            return Err(format!("slopgate: rule from {src} missing \"{key}\" (id={})", if p.id.is_empty() { "?" } else { p.id.as_str() }));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut order = Vec::new();
    let mut latest = HashMap::new();
    for pattern in patterns {
        if !latest.contains_key(&pattern.id) {
            order.push(pattern.id.clone());
        }
        latest.insert(pattern.id.clone(), pattern);
    }
    order.into_iter().filter_map(|id| latest.remove(&id)).collect()
}

fn strip_stateful_flags(flags: Option<&str>) -> Option<String> {
    let stripped: String = flags.unwrap_or("").chars().filter(|c| *c != 'g' && *c != 'y').collect();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
}

fn unknown_pack<'a, I>(kind: &str, name: &str, known: I) -> String
where
    I: Iterator<Item = &'a String>,
{
    let known = known.cloned().collect::<Vec<_>>().join(", ");
    format!("slopgate: unknown {kind} \"{name}\" (known: {known})")
}

fn git_root(from_dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git").args(["rev-parse", "--show-toplevel"]).current_dir(from_dir).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

fn abs_path(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn absolute_from_cwd(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map_err(|e| format!("slopgate: current dir unavailable: {e}"))
            .map(|cwd| cwd.join(path))
    }
}

fn ux_all_ast_ids() -> HashSet<String> {
    ux_packs().values().flat_map(|p| p.ast_ids.iter().cloned()).collect()
}

fn abs_to_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_fixture_tree(root: &std::path::Path) {
        fs::create_dir_all(root.join("rules/baseline/ast")).unwrap();
        fs::create_dir_all(root.join("rules/ux/ast")).unwrap();
        fs::create_dir_all(root.join(".slopgate/custom/ast-rules")).unwrap();
        fs::create_dir_all(root.join(".slopgate/fixtures")).unwrap();
    }

    #[test]
    fn validate_pattern_str_strips_stateful_flags() {
        validate_pattern_str("foo", Some("giy")).unwrap();
    }

    #[test]
    fn resolve_config_str_resolves_defaults_and_enabled_ux() {
        let tmp = tempdir().unwrap();
        make_fixture_tree(tmp.path());

        let config_dir = tmp.path().join(".slopgate");
        let toml_src = r#"
roots = ["src", "packages/app"]
baseline = ["no-stubs", "as-any"]
stack = ["cloudflare"]
astRules = "custom/ast-rules"
suppressions = "suppressions.json"
fixtures = "fixtures"
checkerConcurrency = 7
astDisable = ["ux-img-no-alt"]

[checkers.diff-shape]
maxDirs = 5

[gate]
file = ["critical", "high", "medium"]
staged = ["critical"]

[ux]
a11y = true
taste = "advisory"
"#;

        let cfg = resolve_config_at(tmp.path(), &config_dir, toml_src).unwrap();

        assert_eq!(cfg.repo_root, tmp.path().to_string_lossy());
        assert_eq!(cfg.config_dir, config_dir.to_string_lossy());
        assert_eq!(cfg.roots_rel, vec!["src".to_string(), "packages/app".to_string()]);
        assert_eq!(cfg.roots[0], tmp.path().join("src").to_string_lossy());
        assert!(cfg.exts.contains(".ts"));
        assert!(cfg.exts.contains(".tsx"));
        assert!(cfg.exts.contains(".astro"));
        assert!(cfg.skip_dirs.contains("node_modules"));
        assert!(cfg.skip_dirs.contains("dist"));
        assert!(cfg.skip_dirs.contains("tests"));
        assert_eq!(cfg.checker_concurrency, 7);
        assert_eq!(cfg.baseline_path, config_dir.join("baseline.json").to_string_lossy());
        assert_eq!(cfg.suppressions_path, config_dir.join("suppressions.json").to_string_lossy());
        assert_eq!(cfg.fixtures_dirs[0], tmp.path().join("rules/baseline/fixtures").to_string_lossy());
        assert_eq!(cfg.fixtures_dirs[1], config_dir.join("fixtures").to_string_lossy());
        let mut gate_file = cfg.gate.file.iter().cloned().collect::<Vec<_>>();
        gate_file.sort();
        assert_eq!(gate_file, vec!["critical".to_string(), "high".to_string(), "medium".to_string()]);
        let mut gate_staged = cfg.gate.staged.iter().cloned().collect::<Vec<_>>();
        gate_staged.sort();
        assert_eq!(gate_staged, vec!["critical".to_string()]);
        assert!(cfg.ast_disable.contains("ux-img-no-alt"));
        assert_eq!(cfg.ast_rule_dirs[0], tmp.path().join("rules/baseline/ast").to_string_lossy());
        assert_eq!(cfg.ast_rule_dirs[1], config_dir.join("custom/ast-rules").to_string_lossy());
        assert_eq!(cfg.ast_rule_dirs[2], tmp.path().join("rules/ux/ast").to_string_lossy());
        assert_eq!(cfg.ux_ast_severity.get("ux-div-onclick").unwrap(), "high");
        assert!(cfg.patterns.iter().any(|p| p.id == "ux-emoji-in-ui" && p.severity == "medium"));
        assert!(cfg.ux_ast_all.contains("ux-modal-no-close"));
        assert!(cfg.checkers.contains_key("diff-shape"));
        assert_eq!(cfg.patterns.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().len(), cfg.patterns.len());
    }

    #[test]
    fn resolve_config_str_rejects_project_rules() {
        let tmp = tempdir().unwrap();
        make_fixture_tree(tmp.path());
        let config_dir = tmp.path().join(".slopgate");

        let toml_src = r#"
rules = ["packs/custom.mjs"]
"#;

        let err = resolve_config_at(tmp.path(), &config_dir, toml_src).unwrap_err();
        assert!(err.contains("packs/custom.mjs"));
    }

    #[test]
    fn dedupe_patterns_prefers_last_value_and_first_order() {
        let a = Pattern {
            id: "dup".to_string(),
            severity: "low".to_string(),
            pattern: "a".to_string(),
            resolution: "one".to_string(),
            title: None,
            description: None,
            category: None,
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: None,
        };
        let b = Pattern {
            id: "keep".to_string(),
            severity: "medium".to_string(),
            pattern: "b".to_string(),
            resolution: "two".to_string(),
            title: None,
            description: None,
            category: None,
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: None,
        };
        let c = Pattern {
            id: "dup".to_string(),
            severity: "critical".to_string(),
            pattern: "c".to_string(),
            resolution: "three".to_string(),
            title: None,
            description: None,
            category: None,
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: None,
        };

        let out = dedupe_patterns(vec![a, b, c]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "dup");
        assert_eq!(out[0].severity, "critical");
        assert_eq!(out[1].id, "keep");
    }
}

#[cfg(test)]
fn resolve_config_at(repo_root: &Path, config_dir: &Path, toml_src: &str) -> Result<ResolvedConfig, String> {
    resolve_config_inner(repo_root, config_dir, toml_src)
}
