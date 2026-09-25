//! config — implemented in its task.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn cfg_path() -> String {
        format!("{}/tests/fixtures/config.toml", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn validate_pattern_rejects_bad_regex() {
        assert!(validate_pattern_str("a(b", Some("")).is_err());
        assert!(validate_pattern_str(r"\d+", Some("i")).is_ok());
    }

    #[test]
    fn defaults_present() {
        let c = resolve_config(&cfg_path()).unwrap();
        assert!(c.exts.contains(".ts") && c.exts.contains(".tsx"));
        assert!(c.skip_dirs.contains("node_modules"));
        assert!(c.gate.staged.contains("critical") && c.gate.staged.contains("high"));
        assert_eq!(c.checker_concurrency, 3);
    }

    #[test]
    fn resolves_baseline_packs_and_dedupes() {
        let c = resolve_config(&cfg_path()).unwrap();
        let ids: Vec<&str> = c.patterns.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.iter().any(|i| i.starts_with("no-stubs")));
        let mut seen = std::collections::HashSet::new();
        for p in &c.patterns {
            assert!(seen.insert(&p.id), "dup id {}", p.id);
        }
    }

    #[test]
    fn matches_js_resolver_machine_surface() {
        let vp = format!(
            "{}/tests/parity_vectors/resolved_config.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let js: Value = serde_json::from_str(&std::fs::read_to_string(vp).unwrap()).unwrap();
        let rust = resolve_config(&cfg_path()).unwrap();
        let js_ids = sorted_id_sev(&js["patterns"]);
        let mut rust_ids: Vec<String> = rust
            .patterns
            .iter()
            .map(|p| format!("{}:{}", p.id, p.severity))
            .collect();
        rust_ids.sort();
        assert_eq!(
            rust_ids, js_ids,
            "pattern id:severity set must match JS resolver"
        );
        assert_eq!(sorted_strs(&js["exts"]), sorted_set(&rust.exts));
        assert_eq!(
            sorted_strs(&js["gate"]["staged"]),
            sorted_set(&rust.gate.staged)
        );
    }

    #[test]
    fn unknown_baseline_pack_errors() {
        assert!(resolve_config_str("baseline = [\"nope\"]\n").is_err());
    }

    fn write_temp_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn resolves_project_rule_pack() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r#"{"proj":[{"id":"proj-x","severity":"high","pattern":"FORBIDDEN_TOKEN","resolution":"remove it","canary":"FORBIDDEN_TOKEN","excludeGlobs":["**/*.md"]}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/proj.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let proj = cfg
            .patterns
            .iter()
            .find(|p| p.id == "proj-x")
            .expect("proj-x pattern");
        assert_eq!(proj.resolution, "remove it");
        assert_eq!(proj.exclude_globs, Some(vec!["**/*.md".to_string()]));
    }

    #[test]
    fn project_rule_pack_bad_regex_errors_with_pack_tag() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r#"{"proj":[{"id":"bad","severity":"high","pattern":"(","resolution":"fix it"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/proj.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(err.contains("(from project:proj in"), "err={err}");
        assert!(err.contains("proj.json"), "err={err}");
    }

    #[test]
    fn project_rule_pack_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/nope.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(
            err.contains("cannot read project rule pack") && err.contains("nope.json"),
            "err={err}"
        );
    }

    #[test]
    fn resolves_two_project_rule_packs() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/a.json"),
            r#"{"pack-a":[{"id":"proj-a","severity":"high","pattern":"TOKEN_A","resolution":"remove a","canary":"TOKEN_A"}]}"#,
        );
        write_temp_file(
            &config_dir.join("rules/b.json"),
            r#"{"pack-b":[{"id":"proj-b","severity":"medium","pattern":"TOKEN_B","resolution":"remove b","canary":"TOKEN_B"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/a.json", "./rules/b.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let ids: std::collections::HashSet<_> =
            cfg.patterns.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains("proj-a"));
        assert!(ids.contains("proj-b"));
    }

    #[test]
    fn project_rule_pack_second_path_missing_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        write_temp_file(
            &config_dir.join("rules/a.json"),
            r#"{"pack-a":[{"id":"proj-a","severity":"high","pattern":"TOKEN_A","resolution":"remove a","canary":"TOKEN_A"}]}"#,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/a.json", "./rules/b.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(
            err.contains("cannot read project rule pack") && err.contains("b.json"),
            "err={err}"
        );
    }

    #[test]
    fn project_rule_pack_malformed_json_errors_at_resolve_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        let pack_path = config_dir.join("rules/bad.json");
        write_temp_file(&pack_path, "{not json");
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"rules = ["./rules/bad.json"]"#,
        );
        let err = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap_err();
        assert!(err.contains("invalid project rule pack"), "err={err}");
        assert!(err.contains("bad.json"), "err={err}");
    }

    #[test]
    fn project_rule_pack_overrides_colliding_baseline_id() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        let r_baseline = {
            write_temp_file(&config_dir.join("config.toml"), r#"baseline = ["raw-hex"]"#);
            let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
            let baseline_rule = cfg
                .patterns
                .iter()
                .find(|p| p.id == "raw-hex-color")
                .expect("raw-hex-color from baseline");
            baseline_rule.resolution.clone()
        };
        let r_project = "PROJECT override resolution";
        assert_ne!(r_project, r_baseline);
        write_temp_file(
            &config_dir.join("rules/proj.json"),
            r##"{"proj":[{"id":"raw-hex-color","severity":"high","pattern":"#PROJECT","resolution":"PROJECT override resolution","canary":"#abc"}]}"##,
        );
        write_temp_file(
            &config_dir.join("config.toml"),
            r#"baseline = ["raw-hex"]
rules = ["./rules/proj.json"]"#,
        );
        let cfg = resolve_config(&config_dir.join("config.toml").to_string_lossy()).unwrap();
        let matches: Vec<_> = cfg
            .patterns
            .iter()
            .filter(|p| p.id == "raw-hex-color")
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].resolution, r_project);
    }

    fn sorted_id_sev(v: &Value) -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}:{}",
                    p["id"].as_str().unwrap(),
                    p["severity"].as_str().unwrap()
                )
            })
            .collect();
        out.sort();
        out
    }

    fn sorted_strs(v: &Value) -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        out.sort();
        out
    }

    fn sorted_set(s: &std::collections::HashSet<String>) -> Vec<String> {
        let mut out: Vec<String> = s.iter().cloned().collect();
        out.sort();
        out
    }
}
