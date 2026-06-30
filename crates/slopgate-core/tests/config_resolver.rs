use slopgate_core::config::{resolve_config, validate_pattern_str};
use std::fs;
use std::process::Command;

#[test]
fn resolves_toml_config_with_packs_paths_checkers_and_ux() {
    let dir = tempfile::tempdir().unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    fs::create_dir_all(dir.path().join(".slopgate/rules/ast")).unwrap();
    fs::create_dir_all(dir.path().join(".slopgate/fixtures")).unwrap();
    let config_path = dir.path().join(".slopgate/slopgate.toml");
    fs::write(
        &config_path,
        r#"
roots = ["app", "src"]
skipDirs = ["node_modules", "dist", ".worktrees"]
baseline = ["no-stubs"]
astRules = "rules/ast"
suppressions = "custom-suppressions.json"
fixtures = "fixtures"

[checkers.diff-shape]
maxDirs = 5

[checkers.health]
enabled = true

[gate]
file = ["critical"]

[ux]
taste = "advisory"
a11y = true
"#,
    )
    .unwrap();

    let cfg = resolve_config(config_path.to_str().unwrap()).unwrap();

    assert_eq!(cfg.repo_root, dir.path().to_string_lossy());
    assert_eq!(
        cfg.config_dir,
        dir.path().join(".slopgate").to_string_lossy()
    );
    assert_eq!(cfg.roots_rel, vec!["app", "src"]);
    assert_eq!(
        cfg.roots,
        vec![
            dir.path().join("app").to_string_lossy().to_string(),
            dir.path().join("src").to_string_lossy().to_string()
        ]
    );
    assert!(cfg.skip_dirs.contains(".worktrees"));
    assert!(cfg.patterns.iter().any(|p| p.id == "no-stubs-placeholder"));
    assert!(cfg
        .patterns
        .iter()
        .any(|p| p.id == "ux-emoji-in-ui" && p.severity == "medium"));
    assert_eq!(cfg.checkers["diff-shape"]["maxDirs"], 5);
    assert_eq!(
        cfg.gate.file,
        ["critical"].into_iter().map(String::from).collect()
    );
    assert_eq!(
        cfg.gate.staged,
        ["critical", "high"].into_iter().map(String::from).collect()
    );
    assert_eq!(
        cfg.suppressions_path,
        dir.path()
            .join(".slopgate/custom-suppressions.json")
            .to_string_lossy()
    );
    assert!(cfg.ast_rule_dirs.contains(
        &dir.path()
            .join(".slopgate/rules/ast")
            .to_string_lossy()
            .to_string()
    ));
    assert_eq!(cfg.ux_ast_severity["ux-div-onclick"], "high");
    assert!(cfg.ux_ast_all.contains("ux-modal-no-close"));
}

#[test]
fn rejects_project_rule_packs_until_phase_two() {
    let err =
        slopgate_core::config::resolve_config_str(r#"rules = ["rules/custom.mjs"]"#).unwrap_err();

    assert!(err.contains("project rule packs"));
    assert!(err.contains("rules/custom.mjs"));
}

#[test]
fn validates_regex_after_stripping_javascript_stateful_flags() {
    validate_pattern_str("a+", Some("gyim")).unwrap();

    let err = validate_pattern_str("(", Some("g")).unwrap_err();
    assert!(err.contains("bad regex"));
}
