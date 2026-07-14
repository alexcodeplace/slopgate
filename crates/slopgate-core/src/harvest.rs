use crate::config::ResolvedConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefectRecord {
    pub class: String,
    pub file: String,
    pub line: u32,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

pub fn ledger_path(config: &ResolvedConfig) -> PathBuf {
    Path::new(&config.config_dir).join("defects.jsonl")
}

pub fn record(config: &ResolvedConfig, item: &DefectRecord) -> Result<(), String> {
    if item.class.is_empty()
        || !item
            .class
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("defect class must be non-empty kebab-case".into());
    }
    if item.file.is_empty() || item.source.is_empty() || item.line == 0 {
        return Err("defect file/source must be non-empty and line must be positive".into());
    }
    let path = ledger_path(config);
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut out, item).map_err(|e| e.to_string())?;
    writeln!(out).map_err(|e| e.to_string())
}

pub fn check(config: &ResolvedConfig) -> Result<Vec<String>, String> {
    let path = ledger_path(config);
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut classes: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let item: DefectRecord =
            serde_json::from_str(line).map_err(|e| format!("defects.jsonl:{}: {e}", index + 1))?;
        let occurrence = item
            .fingerprint
            .unwrap_or_else(|| format!("{}:{}", item.file, item.line));
        classes.entry(item.class).or_default().insert(occurrence);
    }
    let fixtures = Path::new(&config.config_dir).join("fixtures");
    let mut unmet = vec![];
    for (class, _) in classes.into_iter().filter(|(_, v)| v.len() >= 2) {
        let rule = config
            .ast_rule_dirs
            .iter()
            .any(|dir| Path::new(dir).join(format!("{class}.yml")).is_file());
        let invalid = ["ts", "tsx"]
            .iter()
            .any(|ext| fixtures.join(format!("{class}.invalid.{ext}")).is_file());
        let valid = ["ts", "tsx"]
            .iter()
            .any(|ext| fixtures.join(format!("{class}.valid.{ext}")).is_file());
        if !(rule && invalid && valid) {
            unmet.push(class);
        }
    }
    Ok(unmet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config_str;
    use tempfile::TempDir;

    fn config(dir: &TempDir) -> ResolvedConfig {
        let mut c = resolve_config_str("roots = [\"src\"]\nastRules = \"./rules/ast\"\n").unwrap();
        c.config_dir = dir.path().join(".slopgate").to_string_lossy().into_owned();
        c.repo_root = dir.path().to_string_lossy().into_owned();
        c.ast_rule_dirs = vec![dir
            .path()
            .join(".slopgate/rules/ast")
            .to_string_lossy()
            .into_owned()];
        c
    }

    #[test]
    fn second_distinct_occurrence_requires_rule_and_fixtures() {
        let dir = TempDir::new().unwrap();
        let c = config(&dir);
        for line in [1, 2] {
            record(
                &c,
                &DefectRecord {
                    class: "repeat-bug".into(),
                    file: "src/a.ts".into(),
                    line,
                    source: "review".into(),
                    fingerprint: None,
                },
            )
            .unwrap();
        }
        assert_eq!(check(&c).unwrap(), vec!["repeat-bug"]);
    }
}
