//! Native TOML config resolver — port of `src/config.mjs`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};

const DEFAULT_ROOTS: &[&str] = &["src"];
const DEFAULT_EXTS: &[&str] = &[".ts", ".tsx", ".astro"];
const DEFAULT_SKIP_DIRS: &[&str] = &["node_modules", "dist", "tests"];
const DEFAULT_GATE: &[&str] = &["critical", "high"];
const DEFAULT_CHECKER_CONCURRENCY: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateAllow {
    pub file: HashSet<String>,
    pub staged: HashSet<String>,
}

#[derive(Debug, Deserialize)]
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
    checkers: Option<BTreeMap<String, toml::Value>>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: Option<BTreeMap<String, toml::Value>>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let cwd = current_dir()?;
    let abs_path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };

    if !abs_path.is_file() {
        return Err(format!("slopgate: config not found: {}", path_to_string(&abs_path)));
    }

    let toml_src = fs::read_to_string(&abs_path)
        .map_err(|e| format!("slopgate: failed to read config {}: {e}", path_to_string(&abs_path)))?;
    let config_dir = abs_path.parent().unwrap_or(&cwd).to_path_buf();
    resolve_config_str_internal(&toml_src, &config_dir)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = current_dir()?;
    resolve_config_str_internal(toml_src, &config_dir)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    if p.id.is_empty() || p.severity.is_empty() || p.pattern.is_empty() || p.resolution.is_empty() {
        return Err("slopgate: invalid rule: missing id, severity, pattern, or resolution".to_string());
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let filtered_flags = normalize_flags(flags)?;
    let compiled_pattern = add_inline_flags(pattern, &filtered_flags);
    Regex::new(&compiled_pattern)
        .map(|_| ())
        .map_err(|e| format!("slopgate: invalid regex \"{pattern}\": {e}"))
}

fn resolve_config_str_internal(toml_src: &str, config_dir: &Path) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: invalid config: {e}"))?;
    let RawConfig {
        roots,
        exts,
        skip_dirs,
        baseline,
        stack,
        rules,
        ast_rules,
        ast_disable,
        suppressions,
        fixtures,
        checkers: raw_checkers,
        gate,
        ux: raw_ux,
        checker_concurrency,
    } = raw;

    let repo_root = git_root(config_dir);
    let repo_root_str = path_to_string(&repo_root);
    let config_dir_str = path_to_string(config_dir);

    let baseline_packs = baseline_packs();
    let stack_packs = stack_packs();
    let ux_packs = ux_packs();
    let mut patterns: Vec<Pattern> = Vec::new();

    for name in baseline.unwrap_or_default() {
        let pack = baseline_packs.get(&name).ok_or_else(|| {
            let known = baseline_packs
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            format!("slopgate: unknown baseline pack \"{name}\" (known: {known})")
        })?;
        for rule in pack {
            validate_pattern(rule)?;
            patterns.push(rule.clone());
        }
    }

    for name in stack.unwrap_or_default() {
        let pack = stack_packs.get(&name).ok_or_else(|| {
            let known = stack_packs.keys().cloned().collect::<Vec<_>>().join(", ");
            format!("slopgate: unknown stack pack \"{name}\" (known: {known})")
        })?;
        for rule in pack {
            validate_pattern(rule)?;
            patterns.push(rule.clone());
        }
    }

    if !rules.is_empty() {
        // PHASE-2: project rule packs
        return Err(format!(
            "slopgate: project rule packs are not supported in native resolver: {}",
            rules.join(", ")
        ));
    }

    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    if let Some(raw_ux) = raw_ux {
        for (name, raw_value) in raw_ux {
            let pack = ux_packs.get(&name).ok_or_else(|| {
                let known = ux_packs.keys().cloned().collect::<Vec<_>>().join(", ");
                format!("slopgate: unknown ux sub-module \"{name}\" (known: {known})")
            })?;

            let Some(severity) = resolve_ux_severity(&raw_value, &pack.default_severity).map_err(|e| {
                format!("slopgate: invalid ux severity for \"{name}\": {e}")
            })? else {
                continue;
            };

            for mut rule in pack.regex.iter().cloned() {
                rule.severity = severity.clone();
                validate_pattern(&rule)?;
                patterns.push(rule);
            }

            if !pack.ast_ids.is_empty() {
                ux_enabled_ast = true;
            }
            for ast_id in &pack.ast_ids {
                ux_ast_severity.insert(ast_id.clone(), severity.clone());
            }
        }
    }

    let mut configured_checkers = BTreeMap::new();
    if let Some(raw_checkers) = raw_checkers {
        for (name, value) in raw_checkers {
            match value {
                toml::Value::Boolean(false) => {}
                toml::Value::Boolean(true) => {
                    configured_checkers.insert(name, json!({}));
                }
                other => {
                    let converted = toml_value_to_json(&other)
                        .map_err(|e| format!("slopgate: invalid checker config for {name}: {e}"))?;
                    configured_checkers.insert(name, converted);
                }
            }
        }
    }

    let patterns = dedupe_patterns(patterns);
    let roots_rel = roots.unwrap_or_else(|| DEFAULT_ROOTS.iter().map(|r| (*r).to_string()).collect());
    let roots = roots_rel
        .iter()
        .map(|raw_root| path_to_string(&resolve_path(&repo_root, raw_root)))
        .collect::<Vec<_>>();

    let mut ast_rule_dirs = vec![path_to_string(
        &repo_root.join("rules").join("baseline").join("ast"),
    )];
    if let Some(ast_rules) = ast_rules {
        let project_ast_rules = resolve_path(config_dir, &ast_rules);
        if project_ast_rules.is_dir() {
            ast_rule_dirs.push(path_to_string(&project_ast_rules));
        }
    }
    if ux_enabled_ast {
        ast_rule_dirs.push(path_to_string(&repo_root.join("rules").join("ux").join("ast")));
    }

    let mut fixtures_dirs = vec![path_to_string(
        &repo_root.join("rules").join("baseline").join("fixtures"),
    )];
    if let Some(raw_fixtures) = fixtures {
        fixtures_dirs.push(path_to_string(&resolve_path(config_dir, &raw_fixtures)));
    }

    let mut ux_ast_all = HashSet::new();
    for pack in ux_packs.values() {
        for id in &pack.ast_ids {
            ux_ast_all.insert(id.clone());
        }
    }

    let gate_file = gate
        .as_ref()
        .and_then(|g| g.file.clone())
        .unwrap_or_else(|| DEFAULT_GATE.iter().map(|v| (*v).to_string()).collect());
    let gate_staged = gate
        .and_then(|g| g.staged)
        .unwrap_or_else(|| DEFAULT_GATE.iter().map(|v| (*v).to_string()).collect());

    Ok(ResolvedConfig {
        repo_root: repo_root_str,
        config_dir: config_dir_str,
        roots,
        roots_rel,
        exts: exts.unwrap_or_else(|| DEFAULT_EXTS.iter().map(|ext| (*ext).to_string()).collect()),
        skip_dirs: skip_dirs.unwrap_or_else(|| DEFAULT_SKIP_DIRS.iter().map(|skip| (*skip).to_string()).collect()),
        patterns,
        ast_rule_dirs,
        checkers: configured_checkers,
        ast_disable: ast_disable.unwrap_or_default().into_iter().collect(),
        baseline_path: path_to_string(&config_dir.join("baseline.json")),
        suppressions_path: path_to_string(
            &suppressions
                .as_ref()
                .map(|path| resolve_path(config_dir, path))
                .unwrap_or_else(|| config_dir.join("suppressions.json")),
        ),
        fixtures_dirs,
        checker_concurrency: checker_concurrency.unwrap_or(DEFAULT_CHECKER_CONCURRENCY),
        gate: GateAllow {
            file: gate_file.into_iter().collect(),
            staged: gate_staged.into_iter().collect(),
        },
        ux_ast_severity,
        ux_ast_all,
    })
}

fn resolve_ux_severity(value: &toml::Value, default: &str) -> Result<Option<String>, String> {
    match value {
        toml::Value::Boolean(false) => Ok(None),
        toml::Value::Boolean(true) => Ok(Some(default.to_string())),
        toml::Value::String(v) => Ok(Some(match v.as_str() {
            "advisory" => "medium".to_string(),
            _ => v.to_string(),
        })),
        _ => Err(format!("unsupported value: {value:?}")),
    }
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut index_by_id: HashMap<String, usize> = HashMap::new();
    let mut deduped: Vec<Pattern> = Vec::new();

    for pattern in patterns {
        if let Some(i) = index_by_id.get(&pattern.id).copied() {
            deduped[i] = pattern;
        } else {
            index_by_id.insert(pattern.id.clone(), deduped.len());
            deduped.push(pattern);
        }
    }

    deduped
}

fn resolve_path(base: &Path, raw_path: &str) -> PathBuf {
    let candidate = Path::new(raw_path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(raw_path)
    }
}

fn git_root(from_dir: &Path) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from_dir)
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                return PathBuf::from(root);
            }
        }
    }

    from_dir.to_path_buf()
}

fn toml_value_to_json(value: &toml::Value) -> Result<JsonValue, String> {
    match value {
        toml::Value::String(v) => Ok(JsonValue::String(v.clone())),
        toml::Value::Integer(v) => Ok(JsonValue::Number((*v).into())),
        toml::Value::Float(v) => serde_json::Number::from_f64(*v)
            .map(JsonValue::Number)
            .ok_or_else(|| "unsupported float value".to_string()),
        toml::Value::Boolean(v) => Ok(JsonValue::Bool(*v)),
        toml::Value::Datetime(v) => Ok(JsonValue::String(v.to_string())),
        toml::Value::Array(list) => list
            .iter()
            .map(toml_value_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array),
        toml::Value::Table(map) => {
            let mut out = JsonMap::new();
            for (k, v) in map {
                out.insert(k.clone(), toml_value_to_json(v)?);
            }
            Ok(JsonValue::Object(out))
        }
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn add_inline_flags(pattern: &str, flags: &str) -> String {
    if flags.is_empty() {
        return pattern.to_string();
    }

    let mut prefix = String::new();
    if flags.contains('i') {
        prefix.push_str("(?i)");
    }
    if flags.contains('m') {
        prefix.push_str("(?m)");
    }
    if flags.contains('s') {
        prefix.push_str("(?s)");
    }
    if flags.contains('x') {
        prefix.push_str("(?x)");
    }
    if prefix.is_empty() {
        pattern.to_string()
    } else {
        format!("{prefix}{pattern}")
    }
}

fn normalize_flags(flags: Option<&str>) -> Result<String, String> {
    let mut out = String::new();
    let mut seen = HashSet::new();

    for flag in flags.unwrap_or("").chars() {
        if matches!(flag, 'g' | 'y') {
            continue;
        }
        if matches!(flag, 'i' | 'm' | 's' | 'x' | 'u') {
            if seen.insert(flag) {
                out.push(flag);
            }
            continue;
        }
        return Err(format!("slopgate: unsupported regex flag: {flag}"));
    }

    Ok(out)
}

fn current_dir() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("slopgate: failed to resolve current directory: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_pattern_flags() {
        assert!(validate_pattern_str("(?:for now|in a real app)", Some("im")).is_ok());
        assert!(validate_pattern_str("(?:for now|in a real app)", Some("g")).is_ok());
    }

    #[test]
    fn test_project_rules_error() {
        let err = resolve_config_str("rules = [\"./rules/local.mjs\"]").unwrap_err();
        assert!(err.contains("project rule packs are not supported"));
    }
}
