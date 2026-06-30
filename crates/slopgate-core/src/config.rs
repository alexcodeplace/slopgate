//! config — implemented in its task.

use crate::rules::packs::Pattern;
use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, UxPack};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    #[serde(default)]
    roots: Vec<String>,
    #[serde(default)]
    exts: Vec<String>,
    #[serde(default)]
    skip_dirs: Vec<String>,
    #[serde(default)]
    baseline: Vec<String>,
    #[serde(default)]
    stack: Vec<String>,
    #[serde(default)]
    rules: Vec<String>,
    #[serde(default)]
    ast_rules: Option<String>,
    #[serde(default)]
    ast_disable: Vec<String>,
    #[serde(default)]
    suppressions: Option<String>,
    #[serde(default)]
    fixtures: Option<String>,
    #[serde(default)]
    checkers: BTreeMap<String, Value>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: BTreeMap<String, Value>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGate {
    #[serde(default)]
    file: Option<Vec<String>>,
    #[serde(default)]
    staged: Option<Vec<String>>,
}

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let abs_config = absolutize_from(&std::env::current_dir().map_err(|e| e.to_string())?, path);
    let meta = fs::metadata(&abs_config)
        .map_err(|_| format!("slopgate: config not found: {}", abs_config.display()))?;
    if !meta.is_file() {
        return Err(format!("slopgate: config not found: {}", abs_config.display()));
    }
    let config_dir = abs_config
        .parent()
        .ok_or_else(|| format!("slopgate: config has no parent dir: {}", abs_config.display()))?
        .to_path_buf();
    let toml_src = fs::read_to_string(&abs_config)
        .map_err(|e| format!("slopgate: failed to read {}: {e}", abs_config.display()))?;
    resolve_config_inner(&toml_src, config_dir)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let config_dir = std::env::current_dir().map_err(|e| format!("slopgate: current_dir: {e}"))?;
    resolve_config_inner(toml_src, config_dir)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    for key in ["id", "severity", "pattern", "resolution"] {
        let missing = match key {
            "id" => p.id.trim().is_empty(),
            "severity" => p.severity.trim().is_empty(),
            "pattern" => p.pattern.trim().is_empty(),
            "resolution" => p.resolution.trim().is_empty(),
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
        .map_err(|e| format!("slopgate: rule {} {e}", p.id))
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let compiled = compile_pattern(pattern, flags)?;
    let _ = compiled;
    Ok(())
}

fn resolve_config_inner(toml_src: &str, config_dir: PathBuf) -> Result<ResolvedConfig, String> {
    let raw: RawConfig =
        toml::from_str(toml_src).map_err(|e| format!("slopgate: invalid TOML: {e}"))?;
    let repo_root = git_root(&config_dir)
        .or_else(|| config_dir.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| config_dir.clone());

    let mut patterns = Vec::new();
    let baseline = baseline_packs();
    for name in &raw.baseline {
        let pack = baseline.get(name).ok_or_else(|| {
            format!(
                "slopgate: unknown baseline pack \"{name}\" (known: {})",
                baseline.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        for p in pack {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }

    let stack = stack_packs();
    for name in &raw.stack {
        let pack = stack.get(name).ok_or_else(|| {
            format!(
                "slopgate: unknown stack pack \"{name}\" (known: {})",
                stack.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        for p in pack {
            validate_pattern(p)?;
            patterns.push(p.clone());
        }
    }

    for rel_path in &raw.rules {
        if !rel_path.trim().is_empty() {
            return Err(format!(
                "slopgate: unsupported project rule pack in Rust resolver: {rel_path} // PHASE-2: project rule packs"
            ));
        }
    }

    let ux = ux_packs();
    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_enabled_ast = false;
    for (key, raw_value) in &raw.ux {
        let pack = ux.get(key).ok_or_else(|| {
            format!(
                "slopgate: unknown ux sub-module \"{key}\" (known: {})",
                ux.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let severity = resolve_ux_severity(raw_value, pack)?;
        let Some(severity) = severity else {
            continue;
        };
        for p in &pack.regex {
            let mut overridden = p.clone();
            overridden.severity = severity.clone();
            validate_pattern(&overridden)?;
            patterns.push(overridden);
        }
        for id in &pack.ast_ids {
            ux_ast_severity.insert(id.clone(), severity.clone());
            ux_enabled_ast = true;
        }
    }

    let deduped_patterns = dedupe_patterns(patterns);
    let baseline_ast_dir = workspace_root().join("rules/baseline/ast");
    let baseline_fixtures_dir = workspace_root().join("rules/baseline/fixtures");
    let ux_ast_dir = workspace_root().join("rules/ux/ast");

    let mut ast_rule_dirs = vec![baseline_ast_dir.to_string_lossy().into_owned()];
    if let Some(ast_rules) = &raw.ast_rules {
        let abs = absolutize_from(&config_dir, ast_rules);
        if abs.is_dir() {
            ast_rule_dirs.push(abs.to_string_lossy().into_owned());
        }
    }
    if ux_enabled_ast && ux_ast_dir.is_dir() {
        ast_rule_dirs.push(ux_ast_dir.to_string_lossy().into_owned());
    }

    let mut checkers = BTreeMap::new();
    for (name, value) in &raw.checkers {
        match value {
            Value::Null => {}
            Value::Bool(false) => {}
            Value::Bool(true) => {
                checkers.insert(name.clone(), Value::Object(Map::new()));
            }
            other => {
                checkers.insert(name.clone(), other.clone());
            }
        }
    }

    let roots_rel = if raw.roots.is_empty() {
        vec!["src".to_string()]
    } else {
        raw.roots.clone()
    };
    let roots = roots_rel
        .iter()
        .map(|r| absolutize_from(&repo_root, r).to_string_lossy().into_owned())
        .collect();

    let exts = collect_set_or_default(&raw.exts, &[".ts", ".tsx", ".astro"]);
    let skip_dirs = collect_set_or_default(&raw.skip_dirs, &["node_modules", "dist", "tests"]);
    let ast_disable = raw.ast_disable.into_iter().collect();
    let gate = GateAllow {
        file: collect_set_from_opt_vec(raw.gate.as_ref().and_then(|g| g.file.as_ref()), &["critical", "high"]),
        staged: collect_set_from_opt_vec(
            raw.gate.as_ref().and_then(|g| g.staged.as_ref()),
            &["critical", "high"],
        ),
    };
    let suppressions_path = raw
        .suppressions
        .as_deref()
        .map(|p| absolutize_from(&config_dir, p))
        .unwrap_or_else(|| config_dir.join("suppressions.json"))
        .to_string_lossy()
        .into_owned();
    let baseline_path = config_dir.join("baseline.json").to_string_lossy().into_owned();
    let mut fixtures_dirs = vec![baseline_fixtures_dir.to_string_lossy().into_owned()];
    if let Some(fixtures) = raw.fixtures.as_deref() {
        fixtures_dirs.push(absolutize_from(&config_dir, fixtures).to_string_lossy().into_owned());
    }

    Ok(ResolvedConfig {
        repo_root: repo_root.to_string_lossy().into_owned(),
        config_dir: config_dir.to_string_lossy().into_owned(),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns: deduped_patterns,
        ast_rule_dirs,
        checkers,
        ast_disable,
        baseline_path,
        suppressions_path,
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(3),
        gate,
        ux_ast_severity,
        ux_ast_all: ux
            .values()
            .flat_map(|pack| pack.ast_ids.iter().cloned())
            .collect(),
    })
}

fn compile_pattern(pattern: &str, flags: Option<&str>) -> Result<Regex, String> {
    let flags = sanitize_flags(flags)?;
    let wrapped = if flags.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{flags}){pattern}")
    };
    Regex::new(&wrapped).map_err(|e| format!("bad regex: {e}"))
}

fn sanitize_flags(flags: Option<&str>) -> Result<String, String> {
    let mut out = String::new();
    let mut seen = HashSet::new();
    for ch in flags.unwrap_or("").chars() {
        if ch == 'g' || ch == 'y' {
            continue;
        }
        if !matches!(ch, 'i' | 'm' | 's' | 'R' | 'U' | 'u' | 'x') {
            return Err(format!("bad regex: unsupported flag '{ch}'"));
        }
        if seen.insert(ch) {
            out.push(ch);
        }
    }
    Ok(out)
}

fn resolve_ux_severity(value: &Value, pack: &UxPack) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(pack.default_severity.clone())),
        Value::String(s) if s == "advisory" || s == "report" => Ok(Some("medium".to_string())),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err("slopgate: ux severity must be a boolean, string, or null".into()),
    }
}

fn dedupe_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    let mut order = Vec::new();
    let mut by_id = HashMap::new();
    for pattern in patterns {
        if let Some(index) = by_id.get(&pattern.id).copied() {
            order[index] = pattern;
        } else {
            by_id.insert(pattern.id.clone(), order.len());
            order.push(pattern);
        }
    }
    order
}

fn collect_set_or_default(values: &[String], defaults: &[&str]) -> HashSet<String> {
    if values.is_empty() {
        defaults.iter().map(|s| (*s).to_string()).collect()
    } else {
        values.iter().cloned().collect()
    }
}

fn collect_set_from_opt_vec(values: Option<&Vec<String>>, defaults: &[&str]) -> HashSet<String> {
    values
        .map(|v| v.iter().cloned().collect())
        .unwrap_or_else(|| defaults.iter().map(|s| (*s).to_string()).collect())
}

fn absolutize_from(base: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn git_root(from_dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, contents).expect("write");
    }

    #[test]
    fn validate_pattern_str_rejects_invalid_regex() {
        let err = validate_pattern_str("(", None).unwrap_err();
        assert!(err.contains("bad regex"), "{err}");
    }

    #[test]
    fn validate_pattern_allows_stateful_flags_by_stripping_them() {
        let p = Pattern {
            id: "x".into(),
            severity: "high".into(),
            pattern: "abc".into(),
            resolution: "fix".into(),
            title: None,
            description: None,
            category: None,
            flags: Some("gyiu".into()),
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: None,
        };
        validate_pattern(&p).expect("pattern should validate");
    }

    #[test]
    fn resolve_config_rejects_project_rule_packs() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        write_file(&config_path, "rules = ['./rules/custom.mjs']\n");
        let err = resolve_config(config_path.to_str().expect("utf8")).unwrap_err();
        assert!(err.contains("unsupported"), "{err}");
        assert!(err.contains("./rules/custom.mjs"), "{err}");
    }

    #[test]
    fn resolve_config_applies_defaults_and_ux_aliases() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("rules/ast")).expect("mkdir ast");
        fs::create_dir_all(root.join("fixtures")).expect("mkdir fixtures");
        let config_path = root.join("config.toml");
        write_file(
            &config_path,
            r#"
baseline = ["no-stubs"]
astRules = "./rules/ast"
fixtures = "./fixtures"

[ux]
a11y = true
taste = "advisory"

[checkers]
diff-shape = { maxDirs = 5 }
disabled = false
enabled = true
"#,
        );

        let resolved = resolve_config(config_path.to_str().expect("utf8")).expect("resolved");
        assert_eq!(resolved.roots_rel, vec!["src".to_string()]);
        assert!(resolved.exts.contains(".ts"));
        assert!(resolved.skip_dirs.contains("node_modules"));
        assert_eq!(resolved.checker_concurrency, 3);
        assert_eq!(resolved.gate.file.len(), 2);
        assert!(resolved.gate.file.contains("critical"));
        assert!(resolved.gate.staged.contains("high"));
        assert_eq!(
            resolved.ux_ast_severity.get("ux-div-onclick"),
            Some(&"high".to_string())
        );
        assert!(resolved.ux_ast_all.contains("ux-modal-no-close"));
        let regex_rule = resolved
            .patterns
            .iter()
            .find(|p| p.id == "ux-emoji-in-ui")
            .expect("ux taste regex present");
        assert_eq!(regex_rule.severity, "medium");
        assert!(resolved.checkers.contains_key("diff-shape"));
        assert!(resolved.checkers.contains_key("enabled"));
        assert!(!resolved.checkers.contains_key("disabled"));
    }

    #[test]
    fn resolve_config_dedupes_last_value_wins_first_order_kept() {
        let resolved = resolve_config_str(
            r#"
baseline = ["no-stubs"]

[ux]
taste = "high"
"#,
        )
        .expect("resolved");

        let ids: Vec<_> = resolved.patterns.iter().map(|p| p.id.as_str()).collect();
        let first_no_stubs = ids.iter().position(|id| *id == "no-stubs-placeholder");
        let first_taste = ids.iter().position(|id| *id == "ux-emoji-in-ui");
        assert!(first_no_stubs.is_some());
        assert!(first_taste.is_some());
        let taste = resolved
            .patterns
            .iter()
            .find(|p| p.id == "ux-emoji-in-ui")
            .expect("taste rule");
        assert_eq!(taste.severity, "high");
    }
}
