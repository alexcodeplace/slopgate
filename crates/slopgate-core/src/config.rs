//! Native TOML config resolver.

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use toml::Value as TomlValue;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub checkers: BTreeMap<String, Value>,
    pub ast_disable: HashSet<String>,
    pub baseline_path: String,
    pub suppressions_path: String,
    pub fixtures_dirs: Vec<String>,
    pub checker_concurrency: u32,
    pub gate: GateAllow,
    pub ux_ast_severity: BTreeMap<String, String>,
    pub ux_ast_all: HashSet<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
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
    checkers: Option<BTreeMap<String, TomlValue>>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: Option<BTreeMap<String, TomlValue>>,
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn baseline_ast_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| canonical_abs_dir(manifest_dir().join("../../rules/baseline/ast")))
        .clone()
}

fn baseline_fixtures_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| canonical_abs_dir(manifest_dir().join("../../rules/baseline/fixtures")))
        .clone()
}

fn ux_ast_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| canonical_abs_dir(manifest_dir().join("../../rules/ux/ast")))
        .clone()
}

fn canonical_abs_dir(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn config_from_src(toml_src: &str, config_dir: PathBuf) -> Result<ResolvedConfig, String> {
    let raw: RawConfig =
        toml::from_str(toml_src).map_err(|e| format!("slopgate: config parse error: {e}"))?;
    resolve_raw_config(raw, config_dir)
}

fn resolve_raw_config(raw: RawConfig, config_dir: PathBuf) -> Result<ResolvedConfig, String> {
    let repo_root = git_root(&config_dir).unwrap_or_else(|| {
        config_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config_dir.clone())
    });

    let baseline_packs = baseline_packs();
    let stack_packs = stack_packs();
    let ux_packs = ux_packs();

    let mut patterns = Vec::new();

    for name in raw.baseline.unwrap_or_default() {
        let Some(pack) = baseline_packs.get(&name) else {
            return Err(format!(
                "slopgate: unknown baseline pack \"{name}\" (known: {})",
                baseline_packs
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        for pattern in pack {
            validate_pattern(pattern)?;
            patterns.push(pattern.clone());
        }
    }

    for name in raw.stack.unwrap_or_default() {
        let Some(pack) = stack_packs.get(&name) else {
            return Err(format!(
                "slopgate: unknown stack pack \"{name}\" (known: {})",
                stack_packs.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        };
        for pattern in pack {
            validate_pattern(pattern)?;
            patterns.push(pattern.clone());
        }
    }

    if let Some(rules) = raw.rules.as_ref().filter(|rules| !rules.is_empty()) {
        return Err(format!(
            "slopgate: project rule packs are not supported in the Rust resolver (unsupported path: {}) // PHASE-2: project rule packs",
            rules[0]
        ));
    }

    let mut ux_ast_severity = BTreeMap::new();
    for (name, value) in raw.ux.unwrap_or_default() {
        let Some(pack) = ux_packs.get(&name) else {
            return Err(format!(
                "slopgate: unknown ux sub-module \"{name}\" (known: {})",
                ux_packs.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        };
        let Some(severity) = resolve_ux_severity(&value, &pack.default_severity)? else {
            continue;
        };
        for pattern in &pack.regex {
            let mut pattern = pattern.clone();
            pattern.severity = severity.clone();
            validate_pattern(&pattern)?;
            patterns.push(pattern);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), severity.clone());
        }
    }

    let mut seen = HashSet::new();
    let mut deduped_patterns = Vec::new();
    for pattern in patterns {
        if seen.insert(pattern.id.clone()) {
            deduped_patterns.push(pattern);
        } else if let Some(slot) = deduped_patterns.iter_mut().find(|p| p.id == pattern.id) {
            *slot = pattern;
        }
    }

    let mut ast_rule_dirs = vec![path_to_string(baseline_ast_dir())];
    if let Some(ast_rules) = raw.ast_rules.as_ref().filter(|value| !value.is_empty()) {
        let abs = abs_from_config_dir(&config_dir, ast_rules);
        if abs.is_dir() {
            ast_rule_dirs
                .push(path_to_string(fs::canonicalize(abs).unwrap_or_else(|_| {
                    abs_from_config_dir(&config_dir, ast_rules)
                })));
        }
    }
    if !ux_ast_severity.is_empty() {
        ast_rule_dirs.push(path_to_string(ux_ast_dir()));
    }

    let mut checkers = BTreeMap::new();
    for (name, value) in raw.checkers.unwrap_or_default() {
        match value {
            TomlValue::Boolean(false) => continue,
            TomlValue::Boolean(true) => {
                checkers.insert(name, Value::Object(serde_json::Map::new()));
            }
            other => {
                let json = serde_json::to_value(other).map_err(|e| {
                    format!("slopgate: checker config for \"{name}\" is not JSON-compatible: {e}")
                })?;
                checkers.insert(name, json);
            }
        }
    }

    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel
        .iter()
        .map(|root| path_to_string(abs_from_config_dir(&repo_root, root)))
        .collect::<Vec<_>>();

    let exts = raw
        .exts
        .unwrap_or_else(|| vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()])
        .into_iter()
        .collect::<HashSet<_>>();

    let skip_dirs = raw
        .skip_dirs
        .unwrap_or_else(|| {
            vec![
                "node_modules".to_string(),
                "dist".to_string(),
                "tests".to_string(),
            ]
        })
        .into_iter()
        .collect::<HashSet<_>>();

    let gate = raw.gate.unwrap_or(RawGate {
        file: None,
        staged: None,
    });
    let gate = GateAllow {
        file: gate
            .file
            .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
            .into_iter()
            .collect(),
        staged: gate
            .staged
            .unwrap_or_else(|| vec!["critical".to_string(), "high".to_string()])
            .into_iter()
            .collect(),
    };

    let fixtures_dirs = {
        let mut dirs = vec![path_to_string(baseline_fixtures_dir())];
        if let Some(fixtures) = raw.fixtures.as_ref().filter(|value| !value.is_empty()) {
            dirs.push(path_to_string(abs_from_config_dir(&config_dir, fixtures)));
        }
        dirs
    };

    let checker_concurrency = raw.checker_concurrency.unwrap_or(3);
    let ast_disable = raw
        .ast_disable
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let suppressions_path = raw
        .suppressions
        .as_ref()
        .filter(|value| !value.is_empty())
        .map(|value| path_to_string(abs_from_config_dir(&config_dir, value)))
        .unwrap_or_else(|| path_to_string(config_dir.join("suppressions.json")));

    Ok(ResolvedConfig {
        repo_root: path_to_string(repo_root),
        config_dir: path_to_string(config_dir.clone()),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns: deduped_patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path: path_to_string(config_dir.join("baseline.json")),
        suppressions_path,
        fixtures_dirs,
        checker_concurrency,
        gate,
        ux_ast_severity,
        ux_ast_all: ux_all_ast_ids(),
    })
}

fn abs_from_config_dir(base: &Path, rel: &str) -> PathBuf {
    if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        base.join(rel)
    }
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
    let root = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

fn resolve_ux_severity(
    value: &TomlValue,
    default_severity: &str,
) -> Result<Option<String>, String> {
    match value {
        TomlValue::Boolean(false) => Ok(None),
        TomlValue::Boolean(true) => Ok(Some(default_severity.to_string())),
        TomlValue::String(severity) => Ok(Some(match severity.as_str() {
            "advisory" | "report" => "medium".to_string(),
            other => other.to_string(),
        })),
        other => Err(format!(
            "slopgate: ux severity must be a string or boolean, got {other:?}"
        )),
    }
}

fn compile_pattern(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let mut builder = RegexBuilder::new(pattern);
    for flag in flags.unwrap_or("").chars() {
        match flag {
            'g' | 'y' => continue,
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'x' => {
                builder.ignore_whitespace(true);
            }
            'u' => {}
            other => return Err(format!("slopgate: unsupported regex flag '{other}'")),
        }
    }
    builder.build().map(|_| ()).map_err(|e| e.to_string())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    compile_pattern(pattern, flags)
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
            return Err(format!(
                "slopgate: rule missing \"{key}\" (id={})",
                if p.id.is_empty() { "?" } else { &p.id }
            ));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
        .map_err(|e| format!("slopgate: rule {} bad regex: {e}", p.id))
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()
            .map_err(|e| format!("slopgate: failed to resolve current directory: {e}"))?
            .join(path)
    };
    if !abs.is_file() {
        return Err(format!(
            "slopgate: config not found: {}",
            abs.to_string_lossy()
        ));
    }
    let src = fs::read_to_string(&abs).map_err(|e| {
        format!(
            "slopgate: failed to read config {}: {e}",
            abs.to_string_lossy()
        )
    })?;
    let config_dir = abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    config_from_src(&src, config_dir)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = std::env::current_dir()
        .map_err(|e| format!("slopgate: failed to resolve current directory: {e}"))?;
    config_from_src(toml_src, config_dir)
}

fn ux_all_ast_ids() -> HashSet<String> {
    ux_packs()
        .values()
        .flat_map(|pack| pack.ast_ids.iter().cloned())
        .collect()
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
