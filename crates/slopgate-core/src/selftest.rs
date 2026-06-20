//! Self-test orchestrator — port of `src/selftest.mjs` (`runSelfTest`).

use crate::ast_engine::{run_ast_grep_scan, AstGrepScanOpts};
use crate::checkers::depcruise::parse_depcruise_output;
use crate::checkers::depcruise::DepcruiseParsed;
use crate::checkers::jscpd::parse_jscpd_report;
use crate::checkers::jscpd::JscpdClone;
use crate::checkers::knip::parse_knip_output;
use crate::checkers::knip::KnipFinding;
use crate::checkers::tsc::{parse_tsc_output, TscError};
use crate::checkers::type_coverage::{parse_type_coverage_output, TypeCoverageEntry};
use crate::config::ResolvedConfig;
use crate::regex_engine::compile_line_regex;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn eprint(msg: &str) {
    let _ = writeln!(io::stderr(), "{msg}");
}

fn regex_test(pattern: &str, flags: &str, text: &str) -> Result<bool, String> {
    let re = compile_line_regex(pattern, flags)?;
    Ok(re.is_match(text).unwrap_or(false))
}

fn baseline_ast_dir(repo_root: &str) -> String {
    Path::new(repo_root)
        .join("rules/baseline/ast")
        .to_string_lossy()
        .into_owned()
}

fn checker_fixtures_dir(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join("rules/baseline/fixtures/checker-outputs")
}

fn stringify_tsc_output(errors: &[TscError]) -> String {
    let arr: Vec<Value> = errors
        .iter()
        .map(|e| {
            json!({
                "file": e.file,
                "line": e.line,
                "code": e.code,
                "message": e.message,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn stringify_knip_output(findings: &[KnipFinding]) -> String {
    let arr: Vec<Value> = findings
        .iter()
        .map(|f| {
            json!({
                "type": f.finding_type,
                "file": f.file,
                "line": f.line,
                "name": f.name,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn stringify_jscpd_output(clones: &[JscpdClone]) -> String {
    let arr: Vec<Value> = clones
        .iter()
        .map(|c| {
            json!({
                "firstFile": c.first_file,
                "firstStart": c.first_start,
                "firstEnd": c.first_end,
                "secondFile": c.second_file,
                "secondStart": c.second_start,
                "secondEnd": c.second_end,
                "lines": c.lines,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn stringify_depcruise_output(parsed: &[DepcruiseParsed]) -> String {
    let arr: Vec<Value> = parsed
        .iter()
        .map(|v| {
            json!({
                "rule": v.rule,
                "severity": v.severity,
                "from": v.from,
                "to": v.to,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn stringify_type_coverage_output(entries: &[TypeCoverageEntry]) -> String {
    let arr: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "file": e.file,
                "line": e.line,
                "name": e.name,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

struct ParserFixture<'a> {
    id: &'a str,
    input: &'a str,
    expected: &'a str,
    parse: fn(&str) -> Result<String, String>,
}

fn parse_tsc_fixture(text: &str) -> Result<String, String> {
    Ok(stringify_tsc_output(&parse_tsc_output(text)))
}

fn parse_knip_fixture(text: &str) -> Result<String, String> {
    let j: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(stringify_knip_output(&parse_knip_output(&j)))
}

fn parse_jscpd_fixture(text: &str) -> Result<String, String> {
    let clones = parse_jscpd_report(text)?;
    Ok(stringify_jscpd_output(&clones))
}

fn parse_depcruise_fixture(text: &str) -> Result<String, String> {
    let j: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(stringify_depcruise_output(&parse_depcruise_output(&j)))
}

fn parse_type_coverage_fixture(text: &str) -> Result<String, String> {
    Ok(stringify_type_coverage_output(&parse_type_coverage_output(
        text,
        Some("/repo"),
    )))
}

const PARSER_FIXTURES: &[ParserFixture<'_>] = &[
    ParserFixture {
        id: "tsc",
        input: "tsc.txt",
        expected: "tsc.expected.json",
        parse: parse_tsc_fixture,
    },
    ParserFixture {
        id: "knip",
        input: "knip.json",
        expected: "knip.expected.json",
        parse: parse_knip_fixture,
    },
    ParserFixture {
        id: "jscpd",
        input: "jscpd.json",
        expected: "jscpd.expected.json",
        parse: parse_jscpd_fixture,
    },
    ParserFixture {
        id: "depcruise",
        input: "depcruise.json",
        expected: "depcruise.expected.json",
        parse: parse_depcruise_fixture,
    },
    ParserFixture {
        id: "type-coverage",
        input: "type-coverage.txt",
        expected: "type-coverage.expected.json",
        parse: parse_type_coverage_fixture,
    },
];

fn extract_ast_rule_id(yml_text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?m)^id:\s*(\S+)").ok()?;
    re.captures(yml_text).map(|c| c[1].to_string())
}

/// Run engine self-test against canaries and fixtures. Mirrors `runSelfTest`. Never panics.
pub fn run_self_test(config: &ResolvedConfig) -> i32 {
    let mut failed = 0i32;

    for p in &config.patterns {
        let Some(canary) = &p.canary else {
            eprint(&format!(
                "WARN {}: no canary — cannot prove rule still fires",
                p.id
            ));
            continue;
        };
        let flags = p.flags.as_deref().unwrap_or("");
        match regex_test(&p.pattern, flags, canary) {
            Err(e) => {
                eprint(&format!("FAIL {}: regex invalid: {e}", p.id));
                failed += 1;
                continue;
            }
            Ok(false) => {
                eprint(&format!("FAIL {}: canary not matched: {canary}", p.id));
                failed += 1;
            }
            Ok(true) => eprint(&format!("OK {}", p.id)),
        }
        for neg in p.negative_canary.as_deref().unwrap_or(&[]) {
            match regex_test(&p.pattern, flags, neg) {
                Err(e) => {
                    eprint(&format!("FAIL {}: regex invalid: {e}", p.id));
                    failed += 1;
                }
                Ok(true) => {
                    eprint(&format!("FAIL {}: negative canary matched: {neg}", p.id));
                    failed += 1;
                }
                Ok(false) => eprint(&format!("OK {} (negative)", p.id)),
            }
        }
    }

    for r in &config.roots {
        if !Path::new(r).exists() {
            eprint(&format!("FAIL config: root missing: {r}"));
            failed += 1;
        }
    }

    let mut fixtures_dirs = Vec::new();
    for d in &config.fixtures_dirs {
        if !Path::new(d).exists() {
            eprint(&format!("FAIL config: fixtures dir missing: {d}"));
            failed += 1;
        } else {
            fixtures_dirs.push(d.clone());
        }
    }

    let ast = run_ast_grep_scan(
        config,
        Some(&fixtures_dirs),
        &AstGrepScanOpts {
            raw_targets: true,
            ..Default::default()
        },
    );

    if !ast.available {
        eprint(&format!(
            "WARN ast-grep unavailable — bucket-B self-test skipped: {}",
            ast.errors.join("; ")
        ));
    } else if !ast.violations.iter().any(|v| v.id == "slopgate-canary") {
        for e in &ast.errors {
            eprint(&format!("FAIL ast: {e}"));
        }
        eprint("FAIL ast-grep canary: slopgate-canary did not fire on fixtures");
        failed += 1;
    } else {
        eprint(&format!(
            "OK ast-grep canary ({} fixture violations)",
            ast.violations.len()
        ));
    }

    let baseline_ast = baseline_ast_dir(&config.repo_root);
    let project_ast_dirs: Vec<&str> = config
        .ast_rule_dirs
        .iter()
        .filter(|d| **d != baseline_ast && Path::new(d).exists())
        .map(String::as_str)
        .collect();

    if !ast.available {
        if !project_ast_dirs.is_empty() {
            eprint("WARN ast-grep unavailable — project ast rules not verified");
        }
    } else {
        for dir in project_ast_dirs {
            let entries = match fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) => {
                    eprint(&format!("FAIL ast {dir}: read dir: {e}"));
                    failed += 1;
                    continue;
                }
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".yml") && !name.ends_with(".yaml") {
                    continue;
                }
                let path = entry.path();
                let text = match fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        eprint(&format!("FAIL ast {name}: read: {e}"));
                        failed += 1;
                        continue;
                    }
                };
                let id = extract_ast_rule_id(&text);
                let Some(id) = id else {
                    eprint(&format!("FAIL ast {name}: no \"id:\" line"));
                    failed += 1;
                    continue;
                };
                if config.ast_disable.contains(&id) {
                    eprint(&format!("SKIP ast {id} (astDisable)"));
                    continue;
                }
                if !ast.violations.iter().any(|v| v.id == id) {
                    eprint(&format!(
                        "FAIL ast {id}: did not fire on fixtures — add a trigger to the project fixtures dir"
                    ));
                    failed += 1;
                } else {
                    eprint(&format!("OK ast {id}"));
                }
            }
        }
    }

    let fix_dir = checker_fixtures_dir(&config.repo_root);
    for f in PARSER_FIXTURES {
        let in_path = fix_dir.join(f.input);
        let exp_path = fix_dir.join(f.expected);
        if !in_path.exists() || !exp_path.exists() {
            eprint(&format!("FAIL parser {}: fixture missing", f.id));
            failed += 1;
            continue;
        }
        let input = match fs::read_to_string(&in_path) {
            Ok(t) => t,
            Err(e) => {
                eprint(&format!("FAIL parser {}: {e}", f.id));
                failed += 1;
                continue;
            }
        };
        let expected_raw = match fs::read_to_string(&exp_path) {
            Ok(t) => t,
            Err(e) => {
                eprint(&format!("FAIL parser {}: {e}", f.id));
                failed += 1;
                continue;
            }
        };
        let want = match serde_json::from_str::<Value>(&expected_raw) {
            Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| expected_raw.clone()),
            Err(_) => expected_raw.clone(),
        };
        match (f.parse)(&input) {
            Ok(got) if got == want => eprint(&format!("OK parser {}", f.id)),
            Ok(_) => {
                eprint(&format!("FAIL parser {}: parsed output != expected", f.id));
                failed += 1;
            }
            Err(e) => {
                eprint(&format!("FAIL parser {}: {e}", f.id));
                failed += 1;
            }
        }
    }

    if failed > 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GateAllow;
    use crate::init::run::engine_root;
    use crate::rules::packs::{baseline_packs, ux_packs, Pattern};
    use indexmap::IndexMap;
    use std::collections::{BTreeMap, HashSet};
    use std::process::Command;

    fn slopgate_repo_with_fixtures() -> PathBuf {
        let engine = engine_root();
        if engine.join("rules/baseline/fixtures").is_dir() {
            return engine;
        }
        let main = engine.join("../../..");
        if main.join("rules/baseline/fixtures").is_dir() {
            return main.canonicalize().unwrap_or(main);
        }
        engine
    }

    fn copy_dir_all(src: &Path, dst: &Path) {
        if !dst.exists() {
            let _ = fs::create_dir_all(dst);
        }
        let Ok(entries) = fs::read_dir(src) else {
            return;
        };
        for entry in entries.flatten() {
            let ty = entry.file_type().ok();
            let dest = dst.join(entry.file_name());
            if ty.as_ref().is_some_and(|t| t.is_dir()) {
                copy_dir_all(&entry.path(), &dest);
            } else if ty.as_ref().is_some_and(|t| t.is_file()) {
                let _ = fs::copy(entry.path(), dest);
            }
        }
    }

    /// Build the selftest.config.mjs equivalent without resolver regex validation
    /// (lookahead patterns compile via fancy-regex at self-test time).
    fn build_selftest_config(repo: &Path) -> ResolvedConfig {
        const BASELINE: &[&str] = &[
            "no-stubs",
            "ts-suppress",
            "as-any",
            "raw-hex",
            "kv-ban",
            "live-secrets",
            "eval-ban",
            "pii-logs",
            "weak-hash",
            "sql-safety",
        ];
        const UX: &[(&str, &str)] = &[
            ("a11y", "high"),
            ("cls", "high"),
            ("feedback", "high"),
            ("taste", "advisory"),
            ("advisory", "advisory"),
        ];

        let baseline = baseline_packs();
        let ux = ux_packs();
        let mut patterns: Vec<Pattern> = Vec::new();
        for name in BASELINE {
            if let Some(pack) = baseline.get(*name) {
                patterns.extend(pack.iter().cloned());
            }
        }
        let mut ux_ast_severity: BTreeMap<String, String> = BTreeMap::new();
        for (key, sev) in UX {
            let Some(pack) = ux.get(*key) else {
                continue;
            };
            for p in &pack.regex {
                let mut overridden = p.clone();
                overridden.severity = (*sev).to_string();
                patterns.push(overridden);
            }
            for id in &pack.ast_ids {
                ux_ast_severity.insert(id.clone(), (*sev).to_string());
            }
        }
        let mut by_id: IndexMap<String, Pattern> = IndexMap::new();
        for p in patterns {
            by_id.insert(p.id.clone(), p);
        }
        let patterns: Vec<Pattern> = by_id.into_values().collect();

        let roots_rel = vec!["rules/baseline/fixtures/src".to_string()];
        let roots: Vec<String> = roots_rel
            .iter()
            .map(|r| repo.join(r).to_string_lossy().into_owned())
            .collect();
        let mut ast_rule_dirs = vec![repo
            .join("rules/baseline/ast")
            .to_string_lossy()
            .into_owned()];
        if repo.join("rules/ux/ast").is_dir() {
            ast_rule_dirs.push(repo.join("rules/ux/ast").to_string_lossy().into_owned());
        }

        ResolvedConfig {
            repo_root: repo.to_string_lossy().into_owned(),
            config_dir: repo.to_string_lossy().into_owned(),
            roots,
            roots_rel,
            exts: [".ts", ".tsx"].iter().map(|s| s.to_string()).collect(),
            skip_dirs: ["node_modules"].iter().map(|s| s.to_string()).collect(),
            patterns,
            ast_rule_dirs,
            checkers: BTreeMap::new(),
            ast_disable: HashSet::new(),
            baseline_path: repo
                .join(".slopgate/baseline.json")
                .to_string_lossy()
                .into_owned(),
            suppressions_path: repo
                .join("rules/baseline/fixtures/suppressions.json")
                .to_string_lossy()
                .into_owned(),
            fixtures_dirs: vec![repo
                .join("rules/baseline/fixtures")
                .to_string_lossy()
                .into_owned()],
            checker_concurrency: 3,
            gate: GateAllow {
                file: ["critical", "high"].iter().map(|s| s.to_string()).collect(),
                staged: ["critical", "high"].iter().map(|s| s.to_string()).collect(),
            },
            ux_ast_severity,
            ux_ast_all: ux_packs()
                .values()
                .flat_map(|p| p.ast_ids.iter().cloned())
                .collect(),
        }
    }

    fn sync_repo_tree(dst: &Path) {
        let src = slopgate_repo_with_fixtures();
        let pairs = [
            ("rules/baseline/fixtures", "rules/baseline/fixtures"),
            ("rules/baseline/ast", "rules/baseline/ast"),
            ("rules/ux/ast", "rules/ux/ast"),
        ];
        for (from, to) in pairs {
            let from_path = src.join(from);
            if from_path.is_dir() {
                copy_dir_all(&from_path, &dst.join(to));
            }
        }
    }

    fn ast_grep_available() -> bool {
        Command::new("ast-grep")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            || slopgate_repo_with_fixtures()
                .join("node_modules/.bin/ast-grep")
                .exists()
    }

    #[test]
    fn self_test_passes_on_real_config() {
        if !ast_grep_available() {
            eprintln!("SKIP self_test_passes_on_real_config: ast-grep not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        sync_repo_tree(tmp.path());
        let config = build_selftest_config(tmp.path());
        assert_eq!(run_self_test(&config), 0, "self-test should pass");
    }

    fn write_temp_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    /// Resolve a project pack via `config.toml`, then isolate the self-test surface:
    /// only resolved project pattern(s), no scan roots, and copied engine fixture paths so
    /// unrelated self-test phases (ast/parser) stay green while canary pass/fail is
    /// attributable to the project rule.
    fn build_project_pack_selftest_config(dir: &Path, proj_json: &str) -> ResolvedConfig {
        use crate::config::resolve_config;

        sync_repo_tree(dir);
        write_temp_file(&dir.join("rules/proj.json"), proj_json);
        write_temp_file(&dir.join("config.toml"), r#"rules = ["./rules/proj.json"]"#);
        let resolved = resolve_config(&dir.join("config.toml").to_string_lossy()).unwrap();

        let fixtures = dir.join("rules/baseline/fixtures");
        let ast_dir = dir.join("rules/baseline/ast");

        ResolvedConfig {
            repo_root: dir.to_string_lossy().into_owned(),
            config_dir: resolved.config_dir,
            roots: vec![],
            roots_rel: vec![],
            exts: resolved.exts,
            skip_dirs: resolved.skip_dirs,
            patterns: resolved.patterns,
            ast_rule_dirs: vec![ast_dir.to_string_lossy().into_owned()],
            checkers: resolved.checkers,
            ast_disable: resolved.ast_disable,
            baseline_path: resolved.baseline_path,
            suppressions_path: resolved.suppressions_path,
            fixtures_dirs: vec![fixtures.to_string_lossy().into_owned()],
            checker_concurrency: resolved.checker_concurrency,
            gate: resolved.gate,
            ux_ast_severity: resolved.ux_ast_severity,
            ux_ast_all: resolved.ux_ast_all,
        }
    }

    const PROJ_SELFTEST_JSON: &str = r#"{"proj":[{"id":"proj-selftest","severity":"high","pattern":"FORBIDDEN_TOKEN","resolution":"remove it","canary":"contains FORBIDDEN_TOKEN here"}]}"#;

    #[test]
    fn project_pack_canary_exercised_by_self_test() {
        if !ast_grep_available() {
            eprintln!("SKIP project_pack_canary_exercised_by_self_test: ast-grep not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let config = build_project_pack_selftest_config(dir.path(), PROJ_SELFTEST_JSON);
        assert!(
            config.patterns.iter().any(|p| p.id == "proj-selftest"),
            "project pattern must flow through resolve into config.patterns"
        );
        assert_eq!(
            run_self_test(&config),
            0,
            "matching project canary must pass self-test"
        );
    }

    #[test]
    fn project_pack_broken_canary_fails_self_test() {
        if !ast_grep_available() {
            eprintln!("SKIP project_pack_broken_canary_fails_self_test: ast-grep not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let proj_json = r#"{"proj":[{"id":"proj-selftest","severity":"high","pattern":"FORBIDDEN_TOKEN","resolution":"remove it","canary":"this is clean text"}]}"#;
        let config = build_project_pack_selftest_config(dir.path(), proj_json);
        assert!(
            config.patterns.iter().any(|p| p.id == "proj-selftest"),
            "project pattern must flow through resolve into config.patterns"
        );
        assert_eq!(
            run_self_test(&config),
            1,
            "only the project canary should fail; ancillary phases stay green"
        );
    }

    #[test]
    fn broken_canary_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().into_owned();
        fs::create_dir_all(tmp.path()).unwrap();
        let config = ResolvedConfig {
            repo_root: root.clone(),
            config_dir: root.clone(),
            roots: vec![root.clone()],
            roots_rel: vec![".".to_string()],
            exts: HashSet::new(),
            skip_dirs: HashSet::new(),
            patterns: vec![Pattern {
                id: "broken-canary".into(),
                severity: "high".into(),
                pattern: r"definitely-wont-match-anything-xyz".into(),
                resolution: "fix".into(),
                title: None,
                description: None,
                category: None,
                flags: None,
                canary: Some("this-should-match-but-wont".into()),
                negative_canary: None,
                include_globs: None,
                exclude_globs: None,
                min_files: None,
            }],
            ast_rule_dirs: vec![],
            checkers: BTreeMap::new(),
            ast_disable: HashSet::new(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![root],
            checker_concurrency: 3,
            gate: GateAllow {
                file: HashSet::new(),
                staged: HashSet::new(),
            },
            ux_ast_severity: BTreeMap::new(),
            ux_ast_all: HashSet::new(),
        };
        let code = run_self_test(&config);
        assert_eq!(code, 1, "broken canary must fail self-test");
    }
}
