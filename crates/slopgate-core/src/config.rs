use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};

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
    checkers: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
    #[serde(default)]
    ux: Option<BTreeMap<String, UxValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum UxValue {
    Bool(bool),
    String(String),
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let raw_config_path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        env::current_dir()
            .map_err(|e| format!("slopgate: failed to get current_dir: {e}"))?
            .join(path)
    };
    if !raw_config_path.exists() || !raw_config_path.is_file() {
        return Err(format!(
            "slopgate: config not found: {}",
            raw_config_path.to_string_lossy()
        ));
    }

    let content = std::fs::read_to_string(&raw_config_path)
        .map_err(|e| format!("slopgate: failed to read config file: {e}"))?;

    let config_dir = raw_config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_root = git_root(&config_dir).unwrap_or_else(|| config_dir.to_string_lossy().to_string());
    let repo_root = PathBuf::from(repo_root);
    let resolved = resolve_from_raw_str(&content, &config_dir, &repo_root)?;
    Ok(resolved)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let cwd = env::current_dir().map_err(|e| format!("slopgate: failed to get current_dir: {e}"))?;
    let repo_root = PathBuf::from(git_root(&cwd).unwrap_or_else(|| cwd.to_string_lossy().to_string()));
    resolve_from_raw_str(toml_src, &cwd, &repo_root)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    if p.id.trim().is_empty() {
        return Err("slopgate: rule missing \"id\"".to_string());
    }
    if p.severity.trim().is_empty() {
        return Err(format!("slopgate: rule {} missing \"severity\"", p.id));
    }
    if p.pattern.trim().is_empty() {
        return Err(format!("slopgate: rule {} missing \"pattern\"", p.id));
    }
    if p.resolution.trim().is_empty() {
        return Err(format!("slopgate: rule {} missing \"resolution\"", p.id));
    }
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let filtered_flags: String = flags
        .unwrap_or("")
        .chars()
        .filter(|c| c != &'g' && c != &'y')
        .collect();
    let full_pattern = if filtered_flags.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{}){}", filtered_flags, pattern)
    };
    Regex::new(&full_pattern).map(|_| ()).map_err(|e| {
        format!("slopgate: bad regex pattern: {}", e)
    })
}

fn resolve_from_raw_str(toml_src: &str, config_dir: &Path, repo_root: &Path) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src).map_err(|e| format!("slopgate: bad TOML config: {e}"))?;
    let repo_root = if repo_root.is_absolute() {
        repo_root.to_path_buf()
    } else {
        env::current_dir().map_err(|e| format!("slopgate: failed to get current_dir: {e}"))?
    };

    let mut patterns = Vec::new();
    let baseline_names = raw.baseline.unwrap_or_default();
    let baseline = baseline_packs();
    for name in &baseline_names {
        let Some(baseline_patterns) = baseline.get(name) else {
            let known = baseline.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(format!(
                "slopgate: unknown baseline pack \"{name}\" (known: {known})"
            ));
        };
        for p in baseline_patterns {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }

    let stack_names = raw.stack.unwrap_or_default();
    let stack = stack_packs();
    for name in &stack_names {
        let Some(stack_patterns) = stack.get(name) else {
            let known = stack.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(format!("slopgate: unknown stack pack \"{name}\" (known: {known})"));
        };
        for p in stack_patterns {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }

    if let Some(rules) = raw.rules.as_ref() {
        if !rules.is_empty() {
            let first = &rules[0];
            return Err(format!(
                "slopgate: PHASE-2: project rule packs are unsupported in native resolver: {first}"
            ));
        }
    }

    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_ast_all = HashSet::new();
    let mut ux_has_ast = false;

    let ux_by_name = ux_packs();
    let mut all_ux_ids = Vec::new();
    for pack in ux_by_name.values() {
        all_ux_ids.extend(pack.ast_ids.iter().cloned());
    }
    for id in all_ux_ids {
        ux_ast_all.insert(id);
    }

    for (name, value) in raw.ux.unwrap_or_default() {
        let pack = ux_by_name
            .get(&name)
            .ok_or_else(|| {
                let known = ux_by_name.keys().cloned().collect::<Vec<_>>().join(", ");
                format!("slopgate: unknown ux sub-module \"{name}\" (known: {known})")
            })?;

        let resolved_severity = match value {
            UxValue::Bool(enabled) if !enabled => None,
            UxValue::Bool(_) => Some(pack.default_severity.clone()),
            UxValue::String(value) => {
                if value.trim().is_empty() {
                    return Err(format!(
                        "slopgate: ux sub-module \"{name}\" has empty severity"
                    ));
                }
                let sev = if value == "advisory" { "medium".to_string() } else { value };
                Some(sev)
            }
        };

        if let Some(sev) = resolved_severity {
            for pattern in &pack.regex {
                let mut regex_pattern = pattern.clone();
                regex_pattern.severity = sev.clone();
                validate_pattern(&regex_pattern)?;
                patterns.push(regex_pattern);
            }
            for id in &pack.ast_ids {
                ux_ast_severity.insert(id.clone(), sev.clone());
                ux_has_ast = true;
            }
        }
    }

    let mut seen = HashMap::new();
    for (index, pattern) in patterns.iter().enumerate() {
        seen.insert(pattern.id.clone(), index);
    }
    let mut pattern_ids = HashSet::new();
    let mut deduped_patterns = Vec::new();
    for p in patterns {
        if pattern_ids.insert(p.id.clone()) {
            deduped_patterns.push(p);
        } else if let Some(&idx) = seen.get(&p.id) {
            deduped_patterns[idx] = p;
        }
    }

    let roots_rel = raw.roots.unwrap_or_else(|| vec!["src".to_string()]);
    let roots = roots_rel
        .iter()
        .map(|r| abs_join(repo_root, r))
        .collect::<Vec<_>>();

    let exts = raw
        .exts
        .unwrap_or_else(|| vec![".ts".into(), ".tsx".into(), ".astro".into()])
        .into_iter()
        .collect::<HashSet<_>>();

    let skip_dirs = raw
        .skip_dirs
        .unwrap_or_else(|| vec!["node_modules".into(), "dist".into(), "tests".into()])
        .into_iter()
        .collect::<HashSet<_>>();

    let mut ast_rule_dirs = vec![baseline_ast_dir().to_string_lossy().to_string()];
    if let Some(raw_ast_rules) = raw.ast_rules {
        let resolved_ast_rules = abs_join_to_pathbuf(config_dir, &raw_ast_rules);
        if resolved_ast_rules.exists() && resolved_ast_rules.is_dir() {
            ast_rule_dirs.push(resolved_ast_rules.to_string_lossy().to_string());
        }
    }
    if ux_has_ast {
        ast_rule_dirs.push(ux_ast_dir().to_string_lossy().to_string());
    }

    let mut checkers = BTreeMap::new();
    if let Some(raw_checkers) = raw.checkers {
        for (name, value) in raw_checkers {
            if matches!(value, Value::Bool(false) | Value::Null) {
                continue;
            }
            let resolved = if value == Value::Bool(true) {
                Value::Object(Default::default())
            } else {
                value
            };
            checkers.insert(name, resolved);
        }
    }

    let ast_disable = raw
        .ast_disable
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();

    let mut gate_file = ["critical", "high"]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let mut gate_staged = ["critical", "high"]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if let Some(gate) = raw.gate {
        if let Some(file) = gate.file {
            gate_file = file.into_iter().collect::<HashSet<_>>();
        }
        if let Some(staged) = gate.staged {
            gate_staged = staged.into_iter().collect::<HashSet<_>>();
        }
    }

    let suppressions_path = abs_join(
        config_dir,
        raw.suppressions
            .as_deref()
            .unwrap_or("./suppressions.json"),
    );
    let mut fixtures_dirs = vec![baseline_fixtures_dir().to_string_lossy().to_string()];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(abs_join(config_dir, fixtures));
    }

    Ok(ResolvedConfig {
        repo_root: repo_root.to_string_lossy().to_string(),
        config_dir: config_dir.to_string_lossy().to_string(),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns: deduped_patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path: abs_join(config_dir, "./baseline.json"),
        suppressions_path,
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

fn abs_join(base: &Path, rel: &str) -> String {
    abs_join_to_pathbuf(base, rel).to_string_lossy().to_string()
}

fn abs_join_to_pathbuf(base: &Path, rel: &str) -> PathBuf {
    let candidate = Path::new(rel);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    }
}

fn baseline_ast_dir() -> PathBuf {
    repo_root_from_manifest()
        .join("rules")
        .join("baseline")
        .join("ast")
}

fn baseline_fixtures_dir() -> PathBuf {
    repo_root_from_manifest()
        .join("rules")
        .join("baseline")
        .join("fixtures")
}

fn ux_ast_dir() -> PathBuf {
    repo_root_from_manifest()
        .join("rules")
        .join("ux")
        .join("ast")
}

fn repo_root_from_manifest() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

fn git_root(from_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(from_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let trimmed = root.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
