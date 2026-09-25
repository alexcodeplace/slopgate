use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct GateAllow {
    pub file: HashSet<String>,
    pub staged: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
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
    checkers: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: BTreeMap<String, String>,
}

fn git_root(from_dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(PathBuf::from(s)) }
}

fn abs_from(base: &Path, rel: &str) -> String {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_string_lossy().into_owned()
    } else {
        base.join(p).to_string_lossy().into_owned()
    }
}

fn repo_root_for(config_dir: &Path) -> PathBuf {
    git_root(config_dir).unwrap_or_else(|| config_dir.parent().unwrap_or(config_dir).to_path_buf())
}

fn ux_ast_dir(repo_root: &Path) -> PathBuf {
    repo_root.join("rules/ux/ast")
}

fn validate_regex(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let mut prefix = String::new();
    if let Some(flags) = flags {
        let cleaned: String = flags.chars().filter(|c| *c != 'g' && *c != 'y').collect();
        if !cleaned.is_empty() {
            prefix = format!("(?{})", cleaned);
        }
    }
    regex::Regex::new(&format!("{prefix}{pattern}")).map(|_| ()).map_err(|e| e.to_string())
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for key in ["id", "severity", "pattern", "resolution"] {
        let missing = match key {
            "id" => p.id.is_empty(),
            "severity" => p.severity.is_empty(),
            "pattern" => p.pattern.is_empty(),
            "resolution" => p.resolution.is_empty(),
            _ => false,
        };
        if missing {
            return Err(format!("rule missing \"{key}\" (id={})", if p.id.is_empty() { "?" } else { &p.id }));
        }
    }
    validate_regex(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    validate_regex(pattern, flags)
}

fn resolve_patterns(names: &[String], packs: &BTreeMap<String, Vec<Pattern>>, kind: &str) -> Result<Vec<Pattern>, String> {
    let mut out = Vec::new();
    for name in names {
        let pack = packs.get(name).ok_or_else(|| {
            format!(
                "slopgate: unknown {kind} pack \"{name}\" (known: {})",
                packs.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        for p in pack {
            validate_pattern(p).map_err(|e| format!("slopgate: {e}"))?;
            out.push(p.clone());
        }
    }
    Ok(out)
}

fn resolve_config_inner(config_dir: &Path, toml_src: &str) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: config parse error: {e}"))?;
    let repo_root = repo_root_for(config_dir);
    let baseline_dir = repo_root.join("rules/baseline/ast");
    let baseline_fixtures = repo_root.join("rules/baseline/fixtures");

    let mut patterns = Vec::new();
    patterns.extend(resolve_patterns(raw.baseline.as_deref().unwrap_or(&[]), &baseline_packs(), "baseline")?);
    patterns.extend(resolve_patterns(raw.stack.as_deref().unwrap_or(&[]), &stack_packs(), "stack")?);

    if let Some(rules) = raw.rules.as_ref() {
        if !rules.is_empty() {
            return Err(format!("slopgate: project rule packs not supported yet: {} // PHASE-2: project rule packs", rules.join(", ")));
        }
    }

    let ux = ux_packs();
    let ux_ast_all = ux.values().flat_map(|p| p.ast_ids.clone()).collect::<HashSet<_>>();
    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    for (key, value) in raw.ux.iter() {
        let pack = ux.get(key).ok_or_else(|| {
            format!(
                "slopgate: unknown ux sub-module \"{key}\" (known: {})",
                ux.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let sev = match value.as_str() {
            "advisory" | "report" => "medium".to_string(),
            "true" => pack.default_severity.clone(),
            other => other.to_string(),
        };
        if sev.is_empty() {
            continue;
        }
        for p in &pack.regex {
            let mut next = p.clone();
            next.severity = sev.clone();
            validate_pattern(&next).map_err(|e| format!("slopgate: {e}"))?;
            patterns.push(next);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), sev.clone());
            ux_enabled_ast = true;
        }
    }

    let mut order = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let mut by_id = BTreeMap::<String, Pattern>::new();
    for p in patterns {
        if seen.insert(p.id.clone()) {
            order.push(p.id.clone());
        }
        by_id.insert(p.id.clone(), p);
    }
    let patterns = order.into_iter().filter_map(|id| by_id.remove(&id)).collect::<Vec<_>>();

    let mut ast_rule_dirs = vec![baseline_dir.to_string_lossy().into_owned()];
    if let Some(ast_rules) = raw.ast_rules.as_deref() {
        let abs = PathBuf::from(abs_from(config_dir, ast_rules));
        if abs.is_dir() {
            ast_rule_dirs.push(abs.to_string_lossy().into_owned());
        }
    }
    if ux_enabled_ast {
        ast_rule_dirs.push(ux_ast_dir(&repo_root).to_string_lossy().into_owned());
    }

    let mut fixtures_dirs = vec![baseline_fixtures.to_string_lossy().into_owned()];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(abs_from(config_dir, fixtures));
    }

    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel.iter().map(|r| abs_from(&repo_root, r)).collect::<Vec<_>>();
    let exts = raw
        .exts
        .unwrap_or_else(|| vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()])
        .into_iter()
        .collect::<HashSet<_>>();
    let skip_dirs = raw
        .skip_dirs
        .unwrap_or_else(|| vec!["node_modules".to_string(), "dist".to_string(), "tests".to_string()])
        .into_iter()
        .collect::<HashSet<_>>();
    let ast_disable = raw.ast_disable.unwrap_or_default().into_iter().collect::<HashSet<_>>();
    let gate_file = raw
        .gate
        .as_ref()
        .and_then(|g| g.file.clone())
        .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
        .into_iter()
        .collect::<HashSet<_>>();
    let gate_staged = raw
        .gate
        .as_ref()
        .and_then(|g| g.staged.clone())
        .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
        .into_iter()
        .collect::<HashSet<_>>();

    Ok(ResolvedConfig {
        repo_root: repo_root.to_string_lossy().into_owned(),
        config_dir: config_dir.to_string_lossy().into_owned(),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns,
        ast_rule_dirs,
        checkers: raw.checkers,
        ast_disable,
        baseline_path: config_dir.join("baseline.json").to_string_lossy().into_owned(),
        suppressions_path: raw
            .suppressions
            .map(|s| abs_from(config_dir, s.as_str()))
            .unwrap_or_else(|| config_dir.join("suppressions.json").to_string_lossy().into_owned()),
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate: GateAllow {
            file: gate_file,
            staged: gate_staged,
        },
        ux_ast_severity,
        ux_ast_all,
    })
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let path = Path::new(path);
    let src = fs::read_to_string(path).map_err(|e| format!("slopgate: config read error: {e}"))?;
    let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
    resolve_config_inner(config_dir, &src)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = std::env::current_dir().map_err(|e| format!("slopgate: current_dir error: {e}"))?;
    resolve_config_inner(&config_dir, toml_src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn cfg_path() -> String {
        format!(
            "{}/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        )
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
        assert_eq!(rust_ids, js_ids, "pattern id:severity set must match JS resolver");
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

    #[test]
    fn project_rule_pack_is_typed_error() {
        let err = resolve_config_str("rules = [\"./my-pack.mjs\"]\n").unwrap_err();
        assert!(err.contains("my-pack.mjs"));
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
