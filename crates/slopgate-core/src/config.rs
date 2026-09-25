//! Native config resolver for the TOML config surface.

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern, UxPack};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
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
    checkers: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: Option<BTreeMap<String, Value>>,
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

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(comp.as_os_str());
            }
        }
    }
    out
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn abs_from_cwd(path: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()
            .map_err(|err| format!("slopgate: current dir: {err}"))?
            .join(path)
    };
    Ok(normalize_path(candidate))
}

fn resolve_from(base: &Path, rel: &str) -> PathBuf {
    normalize_path(base.join(rel))
}

fn resolve_optional_from(base: &Path, rel: Option<&String>) -> Option<PathBuf> {
    let rel = rel?;
    if rel.is_empty() {
        return None;
    }
    Some(resolve_from(base, rel))
}

fn pack_name_list<T>(names: &BTreeMap<String, T>) -> String {
    names.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn validate_required_pattern_fields(p: &Pattern) -> Result<(), String> {
    if p.id.is_empty() {
        return Err("slopgate: rule missing id".to_string());
    }
    if p.severity.is_empty() {
        return Err(format!("slopgate: rule {} missing severity", p.id));
    }
    if p.pattern.is_empty() {
        return Err(format!("slopgate: rule {} missing pattern", p.id));
    }
    if p.resolution.is_empty() {
        return Err(format!("slopgate: rule {} missing resolution", p.id));
    }
    Ok(())
}

fn compile_with_flags(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let mut seen = String::new();
    for flag in flags.unwrap_or("").chars() {
        if flag == 'g' || flag == 'y' {
            continue;
        }
        if !seen.contains(flag) {
            seen.push(flag);
        }
    }

    if seen.is_empty() {
        return Regex::new(pattern)
            .map(|_| ())
            .map_err(|err| format!("slopgate: bad regex: {err}"));
    }

    let mut builder = RegexBuilder::new(pattern);
    for flag in seen.chars() {
        match flag {
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'u' => {
                builder.unicode(true);
            }
            'x' => {
                builder.ignore_whitespace(true);
            }
            other => {
                return Err(format!("slopgate: unsupported regex flag '{other}'"));
            }
        }
    }
    builder
        .build()
        .map(|_| ())
        .map_err(|err| format!("slopgate: bad regex: {err}"))
}

fn validate_ux_severity(value: &Value, pack: &UxPack) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(pack.default_severity.clone())),
        Value::String(s) if s.is_empty() => Ok(None),
        Value::String(s) if s == "advisory" || s == "report" => Ok(Some("medium".to_string())),
        Value::String(s) => Ok(Some(s.clone())),
        other => Err(format!("slopgate: invalid ux severity value: {other}")),
    }
}

fn load_patterns(
    packs: &BTreeMap<String, Vec<Pattern>>,
    names: &[String],
    kind: &str,
) -> Result<Vec<Pattern>, String> {
    let mut out = Vec::new();
    for name in names {
        let Some(patterns) = packs.get(name) else {
            return Err(format!(
                "slopgate: unknown {kind} pack \"{name}\" (known: {})",
                pack_name_list(packs)
            ));
        };
        for pattern in patterns {
            validate_pattern(pattern)?;
            out.push(pattern.clone());
        }
    }
    Ok(out)
}

fn load_ux_patterns(
    raw: &BTreeMap<String, Value>,
    ux_packs: &BTreeMap<String, UxPack>,
) -> Result<(Vec<Pattern>, BTreeMap<String, String>, bool), String> {
    let mut patterns = Vec::new();
    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;

    for (key, value) in raw {
        let Some(pack) = ux_packs.get(key) else {
            return Err(format!(
                "slopgate: unknown ux sub-module \"{key}\" (known: {})",
                pack_name_list(ux_packs)
            ));
        };
        let Some(sev) = validate_ux_severity(value, pack)? else {
            continue;
        };
        for pattern in &pack.regex {
            let mut p = pattern.clone();
            p.severity = sev.clone();
            validate_pattern(&p)?;
            patterns.push(p);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), sev.clone());
            ux_enabled_ast = true;
        }
    }

    Ok((patterns, ux_ast_severity, ux_enabled_ast))
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut ordered: Vec<Pattern> = Vec::new();

    for pattern in patterns {
        if let Some(index) = by_id.get(&pattern.id).copied() {
            ordered[index] = pattern;
        } else {
            let index = ordered.len();
            by_id.insert(pattern.id.clone(), index);
            ordered.push(pattern);
        }
    }

    ordered
}

fn resolve_raw(raw: RawConfig, config_dir: &Path, repo_root: &Path) -> Result<ResolvedConfig, String> {
    let baseline = baseline_packs();
    let stack = stack_packs();
    let ux = ux_packs();

    let mut patterns = Vec::new();

    if let Some(names) = raw.baseline.as_deref() {
        patterns.extend(load_patterns(&baseline, names, "baseline")?);
    }
    if let Some(names) = raw.stack.as_deref() {
        patterns.extend(load_patterns(&stack, names, "stack")?);
    }
    if let Some(paths) = raw.rules.as_deref() {
        if let Some(path) = paths.first() {
            return Err(format!(
                "slopgate: project rule packs are unsupported in the native TOML resolver (path: {path})"
            ));
        }
    }

    let (ux_patterns, ux_ast_severity, ux_enabled_ast) = match raw.ux.as_ref() {
        Some(ux_raw) => load_ux_patterns(ux_raw, &ux)?,
        None => (Vec::new(), BTreeMap::new(), false),
    };
    patterns.extend(ux_patterns);

    let patterns = dedupe_patterns(patterns);

    let mut ast_rule_dirs = Vec::new();
    let baseline_ast = resolve_from(repo_root, "rules/baseline/ast");
    if baseline_ast.is_dir() {
        ast_rule_dirs.push(path_string(&baseline_ast));
    }
    if let Some(ast_rules) = raw.ast_rules.as_deref() {
        let abs = resolve_from(config_dir, ast_rules);
        if abs.is_dir() {
            ast_rule_dirs.push(path_string(&abs));
        }
    }
    if ux_enabled_ast {
        let ux_ast = resolve_from(repo_root, "rules/ux/ast");
        if ux_ast.is_dir() {
            ast_rule_dirs.push(path_string(&ux_ast));
        }
    }

    let mut checkers = BTreeMap::new();
    if let Some(raw_checkers) = raw.checkers {
        for (name, value) in raw_checkers {
            if value == Value::Bool(false) || value == Value::Null {
                continue;
            }
            checkers.insert(
                name,
                if value == Value::Bool(true) {
                    Value::Object(serde_json::Map::new())
                } else {
                    value
                },
            );
        }
    }

    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel.iter().map(|r| path_string(&resolve_from(repo_root, r))).collect();
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
    let ast_disable = raw
        .ast_disable
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();

    let mut fixtures_dirs = Vec::new();
    fixtures_dirs.push(path_string(&resolve_from(repo_root, "rules/baseline/fixtures")));
    if let Some(fixtures) = resolve_optional_from(config_dir, raw.fixtures.as_ref()) {
        fixtures_dirs.push(path_string(&fixtures));
    }

    let gate = raw.gate.unwrap_or_default();
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

    let ux_ast_all = ux
        .values()
        .flat_map(|pack| pack.ast_ids.iter().cloned())
        .collect::<HashSet<_>>();

    Ok(ResolvedConfig {
        repo_root: path_string(repo_root),
        config_dir: path_string(config_dir),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path: path_string(&resolve_from(config_dir, "baseline.json")),
        suppressions_path: path_string(
            &resolve_optional_from(config_dir, raw.suppressions.as_ref())
                .unwrap_or_else(|| resolve_from(config_dir, "suppressions.json")),
        ),
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate,
        ux_ast_severity,
        ux_ast_all,
    })
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs = abs_from_cwd(path)?;
    let meta = fs::metadata(&abs)
        .map_err(|_| format!("slopgate: config not found: {}", path_string(&abs)))?;
    if !meta.is_file() {
        return Err(format!("slopgate: config not found: {}", path_string(&abs)));
    }
    let contents =
        fs::read_to_string(&abs).map_err(|err| format!("slopgate: config read error: {err}"))?;
    let raw: RawConfig =
        toml::from_str(&contents).map_err(|err| format!("slopgate: config parse error: {err}"))?;
    let config_dir = abs.parent().unwrap_or(Path::new("."));
    let repo_root = git_root(config_dir)
        .unwrap_or_else(|| config_dir.parent().unwrap_or(config_dir).to_path_buf());
    resolve_raw(raw, config_dir, &repo_root)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let raw: RawConfig =
        toml::from_str(toml_src).map_err(|err| format!("slopgate: config parse error: {err}"))?;
    let config_dir = std::env::current_dir().map_err(|err| format!("slopgate: current dir: {err}"))?;
    let repo_root = git_root(&config_dir).unwrap_or_else(|| config_dir.clone());
    resolve_raw(raw, &config_dir, &repo_root)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    validate_required_pattern_fields(p)?;
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    compile_with_flags(pattern, flags)
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
