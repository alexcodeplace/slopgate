use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{env, fs, process::Command};

use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::rules::packs::{baseline_packs, stack_packs, ux_packs, Pattern};

const DEFAULT_ROOTS: [&str; 1] = ["src"];
const DEFAULT_EXTS: [&str; 3] = [".ts", ".tsx", ".astro"];
const DEFAULT_SKIP_DIRS: [&str; 3] = ["node_modules", "dist", "tests"];
const DEFAULT_GATE_LEVELS: [&str; 2] = ["critical", "high"];
const DEFAULT_CHECKER_CONCURRENCY: u32 = 3;

const BASELINE_AST_DIR: &str = "rules/baseline/ast";
const BASELINE_FIXTURES_DIR: &str = "rules/baseline/fixtures";
const BASELINE_PATH: &str = "baseline.json";
const SUPPRESSIONS_PATH: &str = "suppressions.json";
const UX_AST_DIR: &str = "rules/ux/ast";

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
    checkers: Option<BTreeMap<String, JsonValue>>,
    #[serde(default)]
    checker_concurrency: Option<u32>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    ux: Option<BTreeMap<String, JsonValue>>,
}

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

pub fn resolve_config(path: &str) -> Result<ResolvedConfig, String> {
    let path = Path::new(path);
    let abs_config = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = env::current_dir().map_err(|e| format!("slopgate: cannot resolve cwd: {e}"))?;
        cwd.join(path)
    };

    if !abs_config.is_file() {
        return Err(format!("slopgate: config not found: {}", abs_config.display()));
    }

    let toml_src = fs::read_to_string(&abs_config)
        .map_err(|e| format!("slopgate: cannot read config {}: {e}", abs_config.display()))?;
    resolve_config_from_source(&toml_src, &abs_config)
}

pub fn resolve_config_str(toml_src: &str) -> Result<ResolvedConfig, String> {
    let cwd = env::current_dir().map_err(|e| format!("slopgate: cannot resolve cwd: {e}"))?;
    let fake_path = cwd.join("slopgate.config.toml");
    resolve_config_from_source(toml_src, &fake_path)
}

pub fn validate_pattern(p: &Pattern) -> Result<(), String> {
    ensure_pattern_fields(p)?;
    validate_pattern_str(&p.pattern, p.flags.as_deref())
}

pub fn validate_pattern_str(pattern: &str, flags: Option<&str>) -> Result<(), String> {
    let mut builder = RegexBuilder::new(pattern);
    apply_regex_flags(&mut builder, flags)?;
    builder
        .build()
        .map_err(|e| format!("slopgate: bad regex: {e}"))?;
    Ok(())
}

fn resolve_config_from_source(
    toml_src: &str,
    config_path: &Path,
) -> Result<ResolvedConfig, String> {
    let raw: RawConfig = toml::from_str(toml_src)
        .map_err(|e| format!("slopgate: invalid config: {e}"))?;

    let config_dir = config_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_dir_str = config_dir.to_string_lossy().into_owned();
    let repo_root = git_root(&config_dir).unwrap_or_else(|| config_dir_str.clone());
    let repo_root_path = Path::new(&repo_root);

    let mut patterns: Vec<Pattern> = Vec::new();
    let baseline = baseline_packs();
    let baseline_known = known_names(&baseline);
    for name in raw.baseline.unwrap_or_default() {
        let pack = baseline.get(&name).ok_or_else(|| {
            format!(
                "slopgate: unknown baseline pack \"{name}\" (known: {baseline_known})"
            )
        })?;
        for pattern in pack {
            validate_pattern(pattern)?;
            patterns.push(pattern.clone());
        }
    }

    let stack = stack_packs();
    let stack_known = known_names(&stack);
    for name in raw.stack.unwrap_or_default() {
        let pack = stack.get(&name).ok_or_else(|| {
            format!("slopgate: unknown stack pack \"{name}\" (known: {stack_known})")
        })?;
        for pattern in pack {
            validate_pattern(pattern)?;
            patterns.push(pattern.clone());
        }
    }

    // PHASE-2: project rule packs
    if let Some(rules) = raw.rules {
        if let Some(first) = rules.first() {
            return Err(format!("slopgate: project rule pack {first} is unsupported in Rust resolver"));
        }
    }

    let ux_map = ux_packs();
    let mut ux_ast_severity = BTreeMap::new();
    let mut ux_ast_all = HashSet::new();
    for pack in ux_map.values() {
        for id in &pack.ast_ids {
            ux_ast_all.insert(id.clone());
        }
    }
    let ux_known = known_names(&ux_map);
    for (key, raw_severity) in raw.ux.unwrap_or_default() {
        let pack = ux_map.get(&key).ok_or_else(|| {
            format!("slopgate: unknown ux sub-module \"{key}\" (known: {ux_known})")
        })?;
        let severity = resolve_ux_severity(&raw_severity, &pack.default_severity)?;
        let Some(severity) = severity else {
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

    let roots_rel = raw.roots.unwrap_or_else(|| {
        DEFAULT_ROOTS
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    });
    let roots = roots_rel
        .iter()
        .map(|root| absolute_path_string(repo_root_path, root))
        .collect();

    let exts = HashSet::from_iter(
        raw.exts
            .unwrap_or_else(|| {
                DEFAULT_EXTS
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            })
            .into_iter(),
    );
    let skip_dirs = HashSet::from_iter(
        raw.skip_dirs
            .unwrap_or_else(|| {
                DEFAULT_SKIP_DIRS
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            })
            .into_iter(),
    );

    let ast_rule_dirs = resolve_ast_rule_dirs(
        repo_root_path,
        &config_dir,
        raw.ast_rules,
        !ux_ast_severity.is_empty(),
    )?;

    let mut checkers = BTreeMap::new();
    for (name, cfg) in raw.checkers.unwrap_or_default() {
        match cfg {
            JsonValue::Bool(false) | JsonValue::Null => {}
            JsonValue::Bool(true) => {
                checkers.insert(name, JsonValue::Object(Default::default()));
            }
            other => {
                checkers.insert(name, other);
            }
        }
    }

    let gate = raw.gate.unwrap_or_default();
    let gate_file = gate.file.unwrap_or_else(|| {
        DEFAULT_GATE_LEVELS
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    });
    let gate_staged = gate.staged.unwrap_or_else(|| {
        DEFAULT_GATE_LEVELS
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    });

    let mut deduped = patterns;
    dedupe_patterns_by_id(&mut deduped);

    let mut fixtures_dirs = vec![absolute_path_string(repo_root_path, BASELINE_FIXTURES_DIR)];
    if let Some(fixtures) = raw.fixtures {
        fixtures_dirs.push(absolute_path_string(&config_dir, &fixtures));
    }

    Ok(ResolvedConfig {
        repo_root,
        config_dir: config_dir.to_string_lossy().into_owned(),
        roots,
        roots_rel,
        exts,
        skip_dirs,
        patterns: deduped,
        ast_rule_dirs: ast_rule_dirs
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        checkers,
        ast_disable: raw.ast_disable.unwrap_or_default().into_iter().collect(),
        baseline_path: absolute_path_string(&config_dir, BASELINE_PATH),
        suppressions_path: absolute_path_string(
            &config_dir,
            raw.suppressions
                .as_deref()
                .unwrap_or(SUPPRESSIONS_PATH),
        ),
        fixtures_dirs,
        checker_concurrency: raw.checker_concurrency.unwrap_or(DEFAULT_CHECKER_CONCURRENCY),
        gate: GateAllow {
            file: gate_file.into_iter().collect(),
            staged: gate_staged.into_iter().collect(),
        },
        ux_ast_severity,
        ux_ast_all,
    })
}

fn resolve_ux_severity(
    raw: &JsonValue,
    default: &str,
) -> Result<Option<String>, String> {
    match raw {
        JsonValue::Bool(true) => Ok(Some(default.to_string())),
        JsonValue::Bool(false) | JsonValue::Null => Ok(None),
        JsonValue::String(value) => {
            if value == "advisory" {
                Ok(Some("medium".to_string()))
            } else {
                Ok(Some(value.to_string()))
            }
        }
        _ => Err(format!("slopgate: invalid ux severity: {raw}")),
    }
}

fn resolve_ast_rule_dirs(
    repo_root: &Path,
    config_dir: &Path,
    ast_rules: Option<String>,
    ux_enabled: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut dirs = vec![absolute_path(repo_root, BASELINE_AST_DIR)];

    if let Some(raw_ast_rules) = ast_rules {
        let abs = absolute_path(config_dir, &raw_ast_rules);
        let path = Path::new(&abs);
        if path.is_dir() {
            dirs.push(abs);
        }
    }

    if ux_enabled {
        dirs.push(absolute_path(repo_root, UX_AST_DIR));
    }

    Ok(dirs)
}

fn absolute_path(base: &Path, value: &str) -> PathBuf {
    if Path::new(value).is_absolute() {
        PathBuf::from(value)
    } else {
        base.join(value)
    }
}

fn absolute_path_string(base: &Path, value: &str) -> String {
    absolute_path(base, value).to_string_lossy().into_owned()
}

fn git_root(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let out = String::from_utf8(output.stdout).ok()?;
    let out = out.trim().to_string();
    if out.is_empty() { None } else { Some(out) }
}

fn known_names<T>(packs: &BTreeMap<String, T>) -> String {
    packs.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn dedupe_patterns_by_id(patterns: &mut Vec<Pattern>) {
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut deduped: Vec<Pattern> = Vec::with_capacity(patterns.len());

    for pattern in patterns.drain(..) {
        if let Some(index) = indices.get(&pattern.id) {
            deduped[*index] = pattern;
            continue;
        }

        let index = deduped.len();
        indices.insert(pattern.id.clone(), index);
        deduped.push(pattern);
    }

    *patterns = deduped;
}

fn ensure_pattern_fields(pattern: &Pattern) -> Result<(), String> {
    if pattern.id.is_empty() {
        return Err(String::from("slopgate: rule missing \"id\""));
    }
    if pattern.severity.is_empty() {
        return Err(format!("slopgate: rule {} missing \"severity\"", pattern.id));
    }
    if pattern.pattern.is_empty() {
        return Err(format!("slopgate: rule {} missing \"pattern\"", pattern.id));
    }
    if pattern.resolution.is_empty() {
        return Err(format!(
            "slopgate: rule {} missing \"resolution\"",
            pattern.id
        ));
    }
    Ok(())
}

fn apply_regex_flags(builder: &mut RegexBuilder, flags: Option<&str>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for flag in flags.unwrap_or("").chars().filter(|flag| !matches!(flag, 'g' | 'y')) {
        if !seen.insert(flag) {
            continue;
        }
        match flag {
            'i' => builder.case_insensitive(true),
            'm' => builder.multi_line(true),
            's' => builder.dot_matches_newline(true),
            'u' => builder.unicode(true),
            'x' => builder.ignore_whitespace(true),
            _ => return Err(format!("slopgate: unsupported regex flag: {flag}")),
        }
    }
    Ok(())
}
