//! Integration tests: run the built binary over the checked-in fixtures and
//! assert the JSON report. Keeps the rule behaviour pinned end-to-end.

use std::process::Command;

fn run(args: &[&str]) -> serde_json::Value {
    let bin = env!("CARGO_BIN_EXE_leakscan");
    let out = Command::new(bin)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn leakscan");
    assert!(out.status.success(), "leakscan exited non-zero");
    serde_json::from_slice(&out.stdout).expect("valid JSON report")
}

fn rules(v: &serde_json::Value, file_suffix: &str) -> Vec<String> {
    v["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["file"].as_str().unwrap().ends_with(file_suffix))
        .map(|x| x["rule"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn flags_every_leak_in_a_component() {
    let report = run(&["fixtures"]);
    let r = rules(&report, "UserCard.tsx");
    assert!(
        r.contains(&"banned-import-in-component".into()),
        "got {r:?}"
    );
    assert!(
        r.contains(&"raw-global-io-in-component".into()),
        "got {r:?}"
    );
    assert!(r.contains(&"raw-db-call-in-component".into()), "got {r:?}");
    assert!(r.contains(&"inline-query-in-component".into()), "got {r:?}");
}

#[test]
fn clean_component_using_a_service_is_silent() {
    let report = run(&["fixtures/src/components/Clean.tsx"]);
    assert_eq!(report["violations"].as_array().unwrap().len(), 0);
}

#[test]
fn service_layer_is_exempt_even_with_fetch() {
    // services/users.ts uses fetch + axios but is outside the presentation layer.
    let report = run(&["fixtures/src/services/users.ts"]);
    assert_eq!(
        report["scanned"].as_u64().unwrap(),
        0,
        "service file should not be scanned"
    );
    assert_eq!(report["violations"].as_array().unwrap().len(), 0);
}

#[test]
fn locally_bound_fetch_is_not_the_global() {
    let report = run(&["fixtures/src/components/Shadowed.tsx"]);
    assert_eq!(
        report["violations"].as_array().unwrap().len(),
        0,
        "shadowed fetch must not flag"
    );
}
