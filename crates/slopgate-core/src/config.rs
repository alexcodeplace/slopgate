use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_ROOTS: &[&str] = &["src"];
const DEFAULT_EXTS: &[&str] = &[".ts", ".tsx", ".astro"];
const DEFAULT_SKIP_DIRS: &[&str] = &["node_modules", "dist", "tests"];
const DEFAULT_GATES: &[&str] = &["critical", "high"];
const DEFAULT_CHECKER_CONCURRENCY: u32 = 3;

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
    checkers: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: Option<BTreeMap<String, String>>,
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
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn abs_path(base: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn existing_dir(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn normalize_flags(flags: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(flags) = flags {
        for ch in flags.chars() {
            match ch {
                'g' | 'y' | 'u' => {}
                other => out.push(other),
            }
        }
    }
    out
}

fn regex_for_flags(pattern: &str, flags: Option<&str>) -> String {
    let normalized = normalize_flags(flags);
    if normalized.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{}){}", normalized, pattern)
    }
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for key in ["id", "severity", "pattern", "resolution"] {
        let missing = match key {
            "id" => p.id.is_empty(),
            "severity" => p.severity.is_empty(),
            "pattern" => p.pattern.is_empty(),
            "resolution" => p.resolution.is_empty(),
            _ => true,
        };
        if missing {
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
    let rewritten = regex_for_flags(pattern, flags);
    Regex::new(&rewritten)
        .map(|_| ())
        .map_err(|e| format!("slopgate: bad regex: {e}"))
}

fn err_unknown_pack(kind: &str, name: &str, known: &[String]) -> String {
    format!(
        "slopgate: unknown {kind} pack \"{name}\" (known: {})",
        known.join(", ")
    )
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut last = HashMap::new();
    for (idx, p) in patterns.iter().enumerate() {
        last.insert(p.id.clone(), idx);
    }
    patterns
        .into_iter()
        .enumerate()
        .filter_map(|(idx, p)| (last.get(&p.id) == Some(&idx)).then_some(p))
        .collect()
}

fn resolve_ux_severity(value: &str, default_severity: &str) -> Option<String> {
    match value {
        "" | "false" => None,
        "true" => Some(default_severity.to_string()),
        "advisory" | "report" => Some("medium".to_string()),
        other => Some(other.to_string()),
    }
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(path)
    };
    let src = fs::read_to_string(&abs)
        .map_err(|e| format!("slopgate: config not found: {} ({e})", abs.to_string_lossy()))?;
    let config_dir = abs.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    resolve_config_inner(&src, &config_dir)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: bad config: {e}"))?;
    let config_dir = std::env::current_dir().map_err(|e| e.to_string())?;
    resolve_config_inner_from_raw(raw, &config_dir)
}

fn resolve_config_inner(toml_src: &str, config_dir: &Path) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: bad config: {e}"))?;
    resolve_config_inner_from_raw(raw, config_dir)
}

fn resolve_config_inner_from_raw(raw: RawConfig, config_dir: &Path) -> Result<ResolvedConfig, String> {
    let config_dir = config_dir.to_path_buf();
    let repo_root = git_root(&config_dir).unwrap_or_else(|| {
        config_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config_dir.clone())
    });

    let baseline = baseline_packs();
    let stack = stack_packs();
    let ux = ux_packs();

    let mut patterns = Vec::new();
    for name in raw.baseline.iter().flatten() {
        let pack = baseline
            .get(name)
            .ok_or_else(|| err_unknown_pack("baseline", name, &baseline.keys().cloned().collect::<Vec<_>>()))?;
        for p in pack {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }
    for name in raw.stack.iter().flatten() {
        let pack = stack
            .get(name)
            .ok_or_else(|| err_unknown_pack("stack", name, &stack.keys().cloned().collect::<Vec<_>>()))?;
        for p in pack {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }
    if let Some(rules) = raw.rules.as_ref() {
        if !rules.is_empty() {
            return Err(format!(
                "slopgate: project rule packs are not supported yet: {}",
                rules.join(", ")
            ));
        }
    }

    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    if let Some(ux_cfg) = raw.ux.as_ref() {
        for (key, value) in ux_cfg {
            let pack = ux
                .get(key)
                .ok_or_else(|| err_unknown_pack("ux sub-module", key, &ux.keys().cloned().collect::<Vec<_>>()))?;
            let Some(sev) = resolve_ux_severity(value, &pack.default_severity) else { continue };
            for p in &pack.regex {
                let mut rule = p.clone();
                rule.severity = sev.clone();
                validate_pattern(&rule)?;
                patterns.push(rule);
            }
            for id in &pack.ast_ids {
                ux_ast_severity.insert(id.clone(), sev.clone());
                ux_enabled_ast = true;
            }
        }
    }

    let patterns = dedupe_patterns(patterns);

    let roots_rel = raw.roots.unwrap_or_else(|| DEFAULT_ROOTS.iter().map(|s| s.to_string()).collect());
    let roots = roots_rel.iter().map(|r| path_to_string(abs_path(&repo_root, r))).collect();
    let exts = raw
        .exts
        .unwrap_or_else(|| DEFAULT_EXTS.iter().map(|s| s.to_string()).collect())
        .into_iter()
        .collect();
    let skip_dirs = raw
        .skip_dirs
        .unwrap_or_else(|| DEFAULT_SKIP_DIRS.iter().map(|s| s.to_string()).collect())
        .into_iter()
        .collect();
    let ast_disable = raw.ast_disable.unwrap_or_default().into_iter().collect();
    let checkers = raw
        .checkers
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, v)| {
            if v.is_null() || v == Value::Bool(false) {
                None
            } else if v == Value::Bool(true) {
                Some((name, Value::Object(Map::new())))
            } else {
                Some((name, v))
            }
        })
        .collect();
    let checker_concurrency = raw.checker_concurrency.unwrap_or(DEFAULT_CHECKER_CONCURRENCY);
    let gate = raw.gate.unwrap_or_default();

    let baseline_path = path_to_string(abs_path(&config_dir, "baseline.json"));
    let suppressions_path = path_to_string(abs_path(
        &config_dir,
        raw.suppressions.as_deref().unwrap_or("suppressions.json"),
    ));

    let mut fixtures_dirs = vec![path_to_string(abs_path(&repo_root, "rules/baseline/fixtures"))];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(path_to_string(abs_path(&config_dir, fixtures)));
    }

    let mut ast_rule_dirs = vec![path_to_string(abs_path(&repo_root, "rules/baseline/ast"))];
    if let Some(ast_rules) = raw.ast_rules.as_deref() {
        let abs = abs_path(&config_dir, ast_rules);
        if existing_dir(&abs) {
            ast_rule_dirs.push(path_to_string(abs));
        }
    }
    if ux_enabled_ast {
        ast_rule_dirs.push(path_to_string(abs_path(&repo_root, "rules/ux/ast")));
    }

    let ux_ast_all = ux
        .values()
        .flat_map(|pack| pack.ast_ids.iter().cloned())
        .collect();

    Ok(ResolvedConfig {
        repo_root: path_to_string(repo_root),
        config_dir: path_to_string(config_dir),
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
        checker_concurrency,
        gate: GateAllow {
            file: gate
                .file
                .unwrap_or_else(|| DEFAULT_GATES.iter().map(|s| s.to_string()).collect())
                .into_iter()
                .collect(),
            staged: gate
                .staged
                .unwrap_or_else(|| DEFAULT_GATES.iter().map(|s| s.to_string()).collect())
                .into_iter()
                .collect(),
        },
        ux_ast_severity,
        ux_ast_all,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pattern_str_strips_stateful_flags() {
        assert!(validate_pattern_str("foo", Some("giy")).is_ok());
    }
}
