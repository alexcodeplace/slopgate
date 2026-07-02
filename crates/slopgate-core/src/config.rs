//! Native config resolver for slopgate-core.

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern, UxPack};
use serde::Deserialize;
use serde_json::Value;
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

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawConfig {
    roots: Option<Vec<String>>,
    exts: Option<Vec<String>>,
    skip_dirs: Option<Vec<String>>,
    baseline: Option<Vec<String>>,
    stack: Option<Vec<String>>,
    rules: Option<Vec<String>>,
    ast_rules: Option<String>,
    ast_disable: Option<Vec<String>>,
    suppressions: Option<String>,
    fixtures: Option<String>,
    checkers: Option<BTreeMap<String, Value>>,
    checker_concurrency: Option<u32>,
    gate: Option<RawGate>,
    ux: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawGate {
    file: Option<Vec<String>>,
    staged: Option<Vec<String>>,
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs = abs_path(Path::new(path))?;
    let src = fs::read_to_string(&abs)
        .map_err(|e| format!("slopgate: failed to read config {}: {}", abs.display(), e))?;
    resolve_from_source(&src, Some(&abs))
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    resolve_from_source(toml_src, None)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for key in ["id", "severity", "pattern", "resolution"] {
        if missing(&p, key) {
            return Err(format!(
                "slopgate: rule missing \"{}\" (id={})",
                key,
                if p.id.is_empty() { "?" } else { &p.id }
            ));
        }
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let _ = strip_stateful_flags(flags);
    regex::Regex::new(pattern)
        .map(|_| ())
        .map_err(|e| format!("slopgate: bad regex: {}", e))
}

fn resolve_from_source(src: &str, config_path: Option<&Path>) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(src).map_err(|e| format!("slopgate: config parse error: {}", e))?;
    let config_dir = config_path
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(current_dir);
    let repo_root = git_root(&config_dir).unwrap_or_else(|| parent_or_self(&config_dir));

    let baseline_packs = baseline_packs();
    let stack_packs = stack_packs();
    let ux_packs = ux_packs();

    let mut patterns = Vec::new();
    for name in raw.baseline.unwrap_or_default() {
        let pack = baseline_packs
            .get(&name)
            .ok_or_else(|| unknown_pack_err("baseline pack", &name, baseline_packs.keys().cloned().collect()))?;
        push_validated_pack(&mut patterns, pack, &format!("baseline:{}", name))?;
    }
    for name in raw.stack.unwrap_or_default() {
        let pack = stack_packs
            .get(&name)
            .ok_or_else(|| unknown_pack_err("stack pack", &name, stack_packs.keys().cloned().collect()))?;
        push_validated_pack(&mut patterns, pack, &format!("stack:{}", name))?;
    }
    if let Some(rules) = raw.rules.as_ref() {
        if !rules.is_empty() {
            return Err(format!(
                "slopgate: project rule packs are not supported natively yet: {:?} // PHASE-2: project rule packs",
                rules
            ));
        }
    }

    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    if let Some(ux) = raw.ux.as_ref() {
        for (key, value) in ux {
            let pack = ux_packs
                .get(key)
                .ok_or_else(|| unknown_pack_err("ux sub-module", key, ux_packs.keys().cloned().collect()))?;
            let sev = resolve_ux_severity(value, pack)?;
            if let Some(sev) = sev {
                for p in &pack.regex {
                    let mut p = p.clone();
                    p.severity = sev.clone();
                    validate_pattern(&p)?;
                    patterns.push(p);
                }
                for id in &pack.ast_ids {
                    ux_ast_severity.insert(id.clone(), sev.clone());
                    ux_enabled_ast = true;
                }
            }
        }
    }

    let patterns = dedupe_patterns(patterns);

    let mut ast_rule_dirs = vec![abs_join(&repo_root, "rules/baseline/ast")];
    if let Some(ast_rules) = raw.ast_rules.as_deref() {
        let abs = resolve_rel(&config_dir, ast_rules);
        if is_dir(&abs) {
            ast_rule_dirs.push(abs.to_string_lossy().into_owned());
        }
    }
    if ux_enabled_ast {
        ast_rule_dirs.push(abs_join(&repo_root, "rules/ux/ast"));
    }

    let mut fixtures_dirs = vec![abs_join(&repo_root, "rules/baseline/fixtures")];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(resolve_rel(&config_dir, fixtures).to_string_lossy().into_owned());
    }

    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel
        .iter()
        .map(|r| resolve_rel(&repo_root, r).to_string_lossy().into_owned())
        .collect();

    let exts = raw
        .exts
        .unwrap_or_else(|| vec![".ts".into(), ".tsx".into(), ".astro".into()])
        .into_iter()
        .collect();
    let skip_dirs = raw
        .skip_dirs
        .unwrap_or_else(|| vec!["node_modules".into(), "dist".into(), "tests".into()])
        .into_iter()
        .collect();
    let ast_disable = raw.ast_disable.unwrap_or_default().into_iter().collect();
    let checkers = raw
        .checkers
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let gate = raw.gate.unwrap_or_default();

    Ok(ResolvedConfig {
        repo_root,
        config_dir: config_dir.to_string_lossy().into_owned(),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path: abs_join(&config_dir, "baseline.json"),
        suppressions_path: resolve_optional_or_default(&config_dir, raw.suppressions.as_deref(), "suppressions.json"),
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate: GateAllow {
            file: gate
                .file
                .unwrap_or_else(|| vec!["critical".into(), "high".into()])
                .into_iter()
                .collect(),
            staged: gate
                .staged
                .unwrap_or_else(|| vec!["critical".into(), "high".into()])
                .into_iter()
                .collect(),
        },
        ux_ast_severity,
        ux_ast_all: ux_ast_all_ids(),
    })
}

fn resolve_ux_severity(value: &toml::Value, pack: &UxPack) -> Result<Option<String>, String> {
    match value {
        toml::Value::Boolean(false) => Ok(None),
        toml::Value::Boolean(true) => Ok(Some(pack.default_severity.clone())),
        toml::Value::String(s) => Ok(Some(match s.as_str() {
            "advisory" | "report" => "medium".to_string(),
            other => other.to_string(),
        })),
        _ => Err("slopgate: ux severity must be a string or boolean".into()),
    }
}

fn resolve_optional_or_default(base: &Path, rel: Option<&str>, default: &str) -> String {
    rel.map(|v| resolve_rel(base, v).to_string_lossy().into_owned())
        .unwrap_or_else(|| abs_join(base, default))
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut slots: Vec<Option<Pattern>> = Vec::new();
    let mut index = HashMap::<String, usize>::new();
    for pattern in patterns {
        if let Some(&slot) = index.get(&pattern.id) {
            slots[slot] = Some(pattern);
        } else {
            let slot = slots.len();
            index.insert(pattern.id.clone(), slot);
            slots.push(Some(pattern));
        }
    }
    slots.into_iter().flatten().collect()
}

fn push_validated_pack(out: &mut Vec<Pattern>, pack: &[Pattern], src: &str) -> Result<(), String> {
    for pattern in pack {
        validate_pattern(pattern).map_err(|e| format!("{} from {}", e, src))?;
        out.push(pattern.clone());
    }
    Ok(())
}

fn unknown_pack_err(kind: &str, name: &str, known: Vec<String>) -> String {
    format!(
        "slopgate: unknown {} \"{}\" (known: {})",
        kind,
        name,
        known.join(", ")
    )
}

fn missing(p: &Pattern, key: &str) -> bool {
    match key {
        "id" => p.id.is_empty(),
        "severity" => p.severity.is_empty(),
        "pattern" => p.pattern.is_empty(),
        "resolution" => p.resolution.is_empty(),
        _ => false,
    }
}

fn strip_stateful_flags(flags: Option<&str>) -> String {
    flags.unwrap_or("").chars().filter(|c| *c != 'g' && *c != 'y').collect()
}

fn abs_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let joined = current_dir().join(path);
        Ok(joined.canonicalize().unwrap_or(joined))
    }
}

fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn git_root(from_dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from_dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn parent_or_self(path: &Path) -> String {
    path.parent()
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn resolve_rel(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn abs_join(base: &Path, rel: &str) -> String {
    resolve_rel(base, rel).to_string_lossy().into_owned()
}

fn is_dir(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn ux_ast_all_ids() -> HashSet<String> {
    ux_packs()
        .values()
        .flat_map(|p| p.ast_ids.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_stateful_flags_removes_g_and_y_only() {
        assert_eq!(strip_stateful_flags(Some("igyms")), "ims");
    }

    #[test]
    fn dedupe_last_wins_first_order() {
        let mut a = Pattern {
            id: "x".into(),
            severity: "high".into(),
            pattern: "a".into(),
            resolution: "r".into(),
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
        let mut b = a.clone();
        b.pattern = "b".into();
        let out = dedupe_patterns(vec![a.clone(), b.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pattern, "b");
    }
}
