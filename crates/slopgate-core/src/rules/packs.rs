//! Built-in rule packs, compiled into the engine from the committed JSON in
//! `crates/slopgate-core/src/rules/*.json` via `include_str!`. These JSON files
//! are the canonical source of truth for baseline/stack/ux pack patterns.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub id: String,
    pub severity: String,
    pub pattern: String,
    pub resolution: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub flags: Option<String>,
    #[serde(default)]
    pub canary: Option<String>,
    #[serde(default)]
    pub negative_canary: Option<Vec<String>>,
    #[serde(default)]
    pub include_globs: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_globs: Option<Vec<String>>,
    #[serde(default)]
    pub min_files: Option<u32>,
}

pub type Packs = BTreeMap<String, Vec<Pattern>>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UxPack {
    pub default_severity: String,
    pub ast_ids: Vec<String>,
    pub regex: Vec<Pattern>,
}

pub type UxPacks = BTreeMap<String, UxPack>;

const BASELINE_JSON: &str = include_str!("baseline.json");
const STACK_JSON: &str = include_str!("stack.json");
const UX_JSON: &str = include_str!("ux.json");

pub fn baseline_packs() -> Packs {
    serde_json::from_str(BASELINE_JSON).expect("baseline.json")
}

pub fn stack_packs() -> Packs {
    serde_json::from_str(STACK_JSON).expect("stack.json")
}

pub fn ux_packs() -> UxPacks {
    serde_json::from_str(UX_JSON).expect("ux.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_packs_load() {
        let p = baseline_packs();
        assert!(p.contains_key("no-stubs"));
        let rule = &p["no-stubs"][0];
        assert_eq!(rule.id, "no-stubs-placeholder");
        assert_eq!(rule.severity, "critical");
        assert!(!rule.pattern.is_empty());
        assert!(!rule.resolution.is_empty());
    }

    #[test]
    fn stack_packs_load() {
        let p = stack_packs();
        assert!(!p.is_empty());
        for patterns in p.values() {
            for rule in patterns {
                assert!(!rule.id.is_empty());
                assert!(!rule.severity.is_empty());
                assert!(!rule.pattern.is_empty());
                assert!(!rule.resolution.is_empty());
            }
        }
    }

    #[test]
    fn ux_packs_load() {
        let p = ux_packs();
        assert!(!p.is_empty());
        let a11y = &p["a11y"];
        assert!(!a11y.ast_ids.is_empty());
        assert!(a11y.ast_ids.contains(&"ux-div-onclick".to_string()));
        assert!(!a11y.regex.is_empty());
        assert_eq!(a11y.regex[0].id, "ux-positive-tabindex");
    }

    #[test]
    fn unknown_pack_absent() {
        assert!(!baseline_packs().contains_key("does-not-exist"));
    }
}
