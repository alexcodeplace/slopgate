//! Exact positive/negative policy fixtures through the production file pipeline.
//! A fixture contract is data, not an alternate scanner implementation.
use crate::checkers::index::Checker;
use crate::config::ResolvedConfig;
use crate::gate::{collect_violations, Mode, Tier};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Contract {
    version: u32,
    cases: BTreeMap<String, Case>,
    rules: BTreeMap<String, RuleProof>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    file: String,
    expected: Vec<Expected>,
}
#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Expected {
    id: String,
    engine: String,
    line: u32,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleProof {
    positive: String,
    negative: String,
}

/// All required rule IDs need a positive witness and a separate negative case.
/// Every case compares the entire finding multiset, including source locations.
pub fn verify(
    config: &ResolvedConfig,
    contract_path: &Path,
    required_rules: &BTreeSet<String>,
    checkers: &[Checker],
) -> Result<usize, Vec<String>> {
    let contents = std::fs::read(contract_path).map_err(|error| {
        vec![format!(
            "fixture contract {}: {error}",
            contract_path.display()
        )]
    })?;
    if contents.len() > 4 * 1024 * 1024 {
        return Err(vec!["fixture contract exceeds 4 MiB".into()]);
    }
    let contract: Contract = serde_json::from_slice(&contents)
        .map_err(|error| vec![format!("invalid fixture contract: {error}")])?;
    if contract.version != 1 || contract.cases.is_empty() || contract.cases.len() > 1000 {
        return Err(vec![
            "fixture contract must be version 1 with 1..1000 cases".into(),
        ]);
    }
    let root = Path::new(&config.repo_root)
        .canonicalize()
        .map_err(|error| vec![error.to_string()])?;
    let fixture_roots: Result<Vec<_>, _> = config
        .fixtures_dirs
        .iter()
        .map(|directory| Path::new(directory).canonicalize())
        .collect();
    let fixture_roots = fixture_roots.map_err(|error| vec![format!("fixture root: {error}")])?;
    let mut fixture_config = config.clone();
    fixture_config.roots = fixture_roots
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    fixture_config.roots_rel = fixture_roots
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(|error| {
                    vec![format!(
                        "fixture directory must be inside repository: {error}"
                    )]
                })
        })
        .collect::<Result<_, _>>()?;
    // Respect configured extensions and per-rule globs, rather than proving a
    // rule only through the historical raw-target fixture bypass.
    let mut actual = BTreeMap::new();
    let mut errors = Vec::new();
    for (name, case) in &contract.cases {
        if !crate::protocol::valid_relative_path(&case.file) {
            errors.push(format!("case {name}: invalid path"));
            continue;
        }
        let source = root
            .join(&case.file)
            .canonicalize()
            .map_err(|error| vec![format!("fixture {name}: {error}")])?;
        if !fixture_roots
            .iter()
            .any(|directory| source.starts_with(directory))
        {
            errors.push(format!(
                "case {name}: input is outside configured fixture directories"
            ));
            continue;
        }
        let result = collect_violations(
            Mode::File,
            &fixture_config,
            Tier::Fast,
            Some(&case.file),
            checkers,
        );
        if !result.errors.is_empty() {
            errors.extend(
                result
                    .errors
                    .into_iter()
                    .map(|error| format!("fixture {name}: {error}")),
            );
            continue;
        }
        if result
            .coverage
            .iter()
            .all(|coverage| coverage.selected_files == Some(0))
        {
            errors.push(format!(
                "fixture {name}: configured file selection excluded the fixture"
            ));
            continue;
        }
        let mut observed: Vec<_> = result
            .violations
            .into_iter()
            .map(|finding| Expected {
                id: finding.id,
                engine: finding.engine,
                line: finding.line,
            })
            .collect();
        observed.sort();
        let mut expected: Vec<_> = case.expected.iter().collect();
        expected.sort();
        if observed.iter().collect::<Vec<_>>() != expected {
            errors.push(format!(
                "fixture {name}: expected {expected:?}, observed {observed:?}"
            ));
        }
        actual.insert(name, observed);
    }
    for required in required_rules {
        if !contract.rules.contains_key(required) {
            errors.push(format!(
                "rule {required}: missing positive/negative fixture proof"
            ));
        }
    }
    for (id, proof) in &contract.rules {
        if proof.positive == proof.negative {
            errors.push(format!(
                "rule {id}: positive and negative cases must differ"
            ));
            continue;
        }
        let positive = actual.get(&proof.positive);
        let negative = actual.get(&proof.negative);
        if !positive.is_some_and(|findings| findings.iter().any(|finding| finding.id == *id)) {
            errors.push(format!(
                "rule {id}: positive fixture did not produce the rule"
            ));
        }
        if !negative.is_some_and(|findings| findings.iter().all(|finding| finding.id != *id)) {
            errors.push(format!(
                "rule {id}: negative fixture failed or produced the rule"
            ));
        }
    }
    if errors.is_empty() {
        Ok(contract.cases.len())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_contract_detects_wrong_line_count_and_false_positive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("fixtures")).unwrap();
        std::fs::write(root.join("fixtures/bad.data"), "BAD\n").unwrap();
        std::fs::write(root.join("fixtures/good.data"), "GOOD\n").unwrap();
        let mut config = crate::config::resolve_config_text_at(
            "roots=['fixtures']\nastEnabled=false\nfixtures='./fixtures'",
            root,
            root,
        )
        .unwrap();
        let pattern:crate::rules::packs::Pattern=serde_json::from_value(serde_json::json!({"id":"no-bad","severity":"high","pattern":"BAD","resolution":"Remove BAD"})).unwrap();
        config.patterns.push(pattern);
        let path = root.join("fixtures/expectations.json");
        let mut contract = serde_json::json!({"version":1,"cases":{"bad":{"file":"fixtures/bad.data","expected":[{"id":"no-bad","engine":"regex","line":1}]},"good":{"file":"fixtures/good.data","expected":[]}},"rules":{"no-bad":{"positive":"bad","negative":"good"}}});
        let required = ["no-bad".into()].into_iter().collect();
        std::fs::write(&path, contract.to_string()).unwrap();
        assert_eq!(verify(&config, &path, &required, &[]).unwrap(), 2);
        contract["cases"]["bad"]["expected"][0]["line"] = serde_json::json!(2);
        std::fs::write(&path, contract.to_string()).unwrap();
        assert!(verify(&config, &path, &required, &[]).is_err());
        contract["cases"]["bad"]["expected"][0]["line"] = serde_json::json!(1);
        std::fs::write(&path, contract.to_string()).unwrap();
        std::fs::write(root.join("fixtures/good.data"), "BAD\n").unwrap();
        assert!(verify(&config, &path, &required, &[]).is_err());
    }
}
