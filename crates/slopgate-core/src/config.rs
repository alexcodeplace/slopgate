//! Native TOML config resolver.

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
struct RawConfig {
    roots: Option<Vec<String>>,
    exts: Option<Vec<String>>,
    skip_dirs: Option<Vec<String>>,
    baseline: Option<Vec<String>>,
    stack: Option<Vec<String>>,
    rules: Option<Vec<String>>,
    ast_rules: Option<String>,
    checkers: Option<BTreeMap<String, toml::Value>>,
    ast_disable: Option<Vec<String>>,
    suppressions: Option<String>,
    fixtures: Option<String>,
    checker_concurrency: Option<u32>,
    gate: Option<RawGate>,
    ux: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    file: Option<Vec<String>>,
    staged: Option<Vec<String>>,
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs_config = absolute_from_current(Path::new(path))?;
    let meta = fs::metadata(&abs_config)
        .map_err(|_| format!("slopgate: config not found: {}", path_string(&abs_config)))?;
    if !meta.is_file() {
        return Err(format!(
            "slopgate: config not found: {}",
            path_string(&abs_config)
        ));
    }

    let config_dir = abs_config
        .parent()
        .ok_or_else(|| {
            format!(
                "slopgate: config has no parent: {}",
                path_string(&abs_config)
            )
        })?
        .to_path_buf();
    let src = fs::read_to_string(&abs_config).map_err(|e| {
        format!(
            "slopgate: failed to read config {}: {e}",
            path_string(&abs_config)
        )
    })?;
    resolve_config_inner(&src, config_dir)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = std::env::current_dir()
        .map_err(|e| format!("slopgate: failed to read current directory: {e}"))?;
    resolve_config_inner(toml_src, config_dir)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for (key, value) in [
        ("id", p.id.as_str()),
        ("severity", p.severity.as_str()),
        ("pattern", p.pattern.as_str()),
        ("resolution", p.resolution.as_str()),
    ] {
        if value.is_empty() {
            return Err(format!(
                "slopgate: rule missing \"{key}\" (id={})",
                if p.id.is_empty() { "?" } else { &p.id }
            ));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
        .map_err(|e| format!("slopgate: rule {} {e}", p.id))
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let _flags_without_state = flags.map(|f| {
        f.chars()
            .filter(|c| *c != 'g' && *c != 'y')
            .collect::<String>()
    });
    Regex::new(pattern)
        .map(|_| ())
        .map_err(|e| format!("bad regex: {e}"))
}

fn resolve_config_inner(toml_src: &str, config_dir: PathBuf) -> Result<ResolvedConfig, String> {
    let raw: RawConfig =
        toml::from_str(toml_src).map_err(|e| format!("slopgate: invalid TOML config: {e}"))?;
    let repo_root = git_root(&config_dir).unwrap_or_else(|| {
        config_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config_dir.clone())
    });

    let mut patterns = Vec::new();
    let baseline = baseline_packs();
    for name in raw.baseline.unwrap_or_default() {
        let pack = baseline.get(&name).ok_or_else(|| {
            format!(
                "slopgate: unknown baseline pack \"{}\" (known: {})",
                name,
                known_keys(&baseline)
            )
        })?;
        for pattern in pack {
            validate_pattern(pattern).map_err(|e| format!("{e} from baseline:{name}"))?;
            patterns.push(pattern.clone());
        }
    }

    let stack = stack_packs();
    for name in raw.stack.unwrap_or_default() {
        let pack = stack.get(&name).ok_or_else(|| {
            format!(
                "slopgate: unknown stack pack \"{}\" (known: {})",
                name,
                known_keys(&stack)
            )
        })?;
        for pattern in pack {
            validate_pattern(pattern).map_err(|e| format!("{e} from stack:{name}"))?;
            patterns.push(pattern.clone());
        }
    }

    // PHASE-2: project rule packs
    for rel_path in raw.rules.unwrap_or_default() {
        return Err(format!(
            "slopgate: project rule packs are unsupported in native Rust resolver until PHASE-2: {rel_path}"
        ));
    }

    let ux = ux_packs();
    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    for (key, value) in raw.ux.unwrap_or_default() {
        let pack = ux.get(&key).ok_or_else(|| {
            format!(
                "slopgate: unknown ux sub-module \"{}\" (known: {})",
                key,
                known_keys(&ux)
            )
        })?;
        let Some(severity) = resolve_ux_severity(&value, &pack.default_severity)? else {
            continue;
        };
        for pattern in &pack.regex {
            let mut overridden = pattern.clone();
            overridden.severity = severity.clone();
            validate_pattern(&overridden).map_err(|e| format!("{e} from ux:{key}"))?;
            patterns.push(overridden);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), severity.clone());
            ux_enabled_ast = true;
        }
    }

    let patterns = dedupe_patterns(patterns);
    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let ast_rule_dirs = ast_rule_dirs(&config_dir, raw.ast_rules.as_deref(), ux_enabled_ast);
    let checkers = resolve_checkers(raw.checkers)?;
    let gate = raw.gate.unwrap_or_default();
    let suppressions_path = raw
        .suppressions
        .as_deref()
        .map(|p| resolve_path(&config_dir, p))
        .unwrap_or_else(|| config_dir.join("suppressions.json"));
    let mut fixtures_dirs = vec![baseline_fixtures_dir()];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(resolve_path(&config_dir, fixtures));
    }

    Ok(ResolvedConfig {
        repo_root: path_string(&repo_root),
        config_dir: path_string(&config_dir),
        roots: roots_rel
            .iter()
            .map(|root| path_string(&repo_root.join(root)))
            .collect(),
        roots_rel,
        exts: to_set(
            raw.exts.unwrap_or_else(|| {
                vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()]
            }),
        ),
        skip_dirs: to_set(raw.skip_dirs.unwrap_or_else(|| {
            vec![
                "node_modules".to_string(),
                "dist".to_string(),
                "tests".to_string(),
            ]
        })),
        patterns,
        ast_rule_dirs: ast_rule_dirs.into_iter().map(|p| path_string(&p)).collect(),
        checkers,
        ast_disable: to_set(raw.ast_disable.unwrap_or_default()),
        baseline_path: path_string(&config_dir.join("baseline.json")),
        suppressions_path: path_string(&suppressions_path),
        fixtures_dirs: fixtures_dirs.into_iter().map(|p| path_string(&p)).collect(),
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate: GateAllow {
            file: to_set(gate.file.unwrap_or_else(default_gate)),
            staged: to_set(gate.staged.unwrap_or_else(default_gate)),
        },
        ux_ast_severity,
        ux_ast_all: ux
            .values()
            .flat_map(|pack| pack.ast_ids.iter().cloned())
            .collect(),
    })
}

fn resolve_ux_severity(value: &toml::Value, default: &str) -> Result<Option<String>, String> {
    match value {
        toml::Value::Boolean(false) => Ok(None),
        toml::Value::Boolean(true) => Ok(Some(default.to_string())),
        toml::Value::String(s) if s.is_empty() => Ok(None),
        toml::Value::String(s) if s == "advisory" || s == "report" => {
            Ok(Some("medium".to_string()))
        }
        toml::Value::String(s) => Ok(Some(s.clone())),
        other => Err(format!(
            "slopgate: ux severity must be boolean or string, got {other}"
        )),
    }
}

fn resolve_checkers(
    raw: Option<BTreeMap<String, toml::Value>>,
) -> Result<BTreeMap<String, JsonValue>, String> {
    let mut checkers = BTreeMap::new();
    for (name, value) in raw.unwrap_or_default() {
        match value {
            toml::Value::Boolean(false) => {}
            toml::Value::Boolean(true) => {
                checkers.insert(name, JsonValue::Object(JsonMap::new()));
            }
            other => {
                let json = serde_json::to_value(other).map_err(|e| {
                    format!("slopgate: checker {name} config is not JSON-compatible: {e}")
                })?;
                checkers.insert(name, json);
            }
        }
    }
    Ok(checkers)
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut order = Vec::new();
    let mut by_id = HashMap::new();
    for pattern in patterns {
        if !by_id.contains_key(&pattern.id) {
            order.push(pattern.id.clone());
        }
        by_id.insert(pattern.id.clone(), pattern);
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

fn ast_rule_dirs(
    config_dir: &Path,
    project_ast_rules: Option<&str>,
    ux_enabled_ast: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let baseline = baseline_ast_dir();
    if baseline.is_dir() {
        dirs.push(baseline);
    }
    if let Some(raw) = project_ast_rules {
        let path = resolve_path(config_dir, raw);
        if path.is_dir() {
            dirs.push(path);
        }
    }
    let ux = ux_ast_dir();
    if ux_enabled_ast && ux.is_dir() {
        dirs.push(ux);
    }
    dirs
}

fn git_root(from_dir: &Path) -> Option<PathBuf> {
    for bin in ["git", "/usr/bin/git"] {
        let output = Command::new(bin)
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(from_dir)
            .output();
        let Ok(output) = output else {
            continue;
        };
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                return Some(PathBuf::from(root));
            }
        }
    }
    None
}

fn absolute_from_current(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| format!("slopgate: failed to read current directory: {e}"))?
            .join(path))
    }
}

fn resolve_path(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn baseline_ast_dir() -> PathBuf {
    workspace_root().join("rules/baseline/ast")
}

fn baseline_fixtures_dir() -> PathBuf {
    workspace_root().join("rules/baseline/fixtures")
}

fn ux_ast_dir() -> PathBuf {
    workspace_root().join("rules/ux/ast")
}

fn known_keys<T>(map: &BTreeMap<String, T>) -> String {
    map.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn to_set(values: Vec<String>) -> HashSet<String> {
    values.into_iter().collect()
}

fn default_gate() -> Vec<String> {
    vec!["critical".to_string(), "high".to_string()]
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
