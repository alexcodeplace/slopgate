//! Stack detection — port of `src/init/detect-stack.mjs` pure/file-read detectors.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

const EXCLUDE_SCAN: &[&str] = &[
    "node_modules",
    ".next",
    ".open-next",
    ".astro",
    "dist",
    ".worktrees",
];
const SCAN_BASES: &[&str] = &["", "apps", "packages", "workers"];
const EXT_CANDIDATES: &[&str] = &[
    ".ts", ".tsx", ".astro", ".js", ".jsx", ".vue", ".svelte", ".rs",
];
const LEAKSCAN_FRONTEND_EXTS: &[&str] = &[".tsx", ".jsx"];
const OPTIONAL_SKIP: &[&str] = &[
    ".next",
    ".open-next",
    ".astro",
    "build",
    ".turbo",
    "coverage",
];
const BASE_SKIP: &[&str] = &["node_modules", "dist", "tests", ".worktrees"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectRootsResult {
    pub roots: Vec<String>,
    pub warned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConventionSources {
    pub version: u32,
    pub generated_hint: String,
    pub claude_md: Vec<String>,
    pub skills: Vec<String>,
    pub agents: Vec<String>,
    pub commands: Vec<String>,
    pub editor_rules: Vec<String>,
    pub knowledge_docs: Vec<String>,
}

fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/")
}

fn collapse_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for c in s.chars() {
        if c == '/' {
            if !prev_slash {
                out.push('/');
                prev_slash = true;
            }
        } else {
            out.push(c);
            prev_slash = false;
        }
    }
    out
}

fn relative_path(base: &Path, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) => {
            let s = normalize_rel(&rel.to_string_lossy());
            if s.is_empty() {
                ".".to_string()
            } else {
                s
            }
        }
        Err(_) => normalize_rel(&path.to_string_lossy()),
    }
}

fn path_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

fn is_excluded_scan(name: &str) -> bool {
    EXCLUDE_SCAN.contains(&name)
}

/// Read and parse `package.json` from `target_dir`, or `None` if missing/invalid.
pub fn read_package_json(target_dir: &Path) -> Option<Value> {
    let p = target_dir.join("package.json");
    if !path_exists(&p) {
        return None;
    }
    let contents = fs::read_to_string(&p).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Expand workspace glob patterns (single `*` segment) into concrete repo-relative paths.
pub fn expand_workspace_globs(target_dir: &Path, patterns: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for pattern in patterns {
        let norm = normalize_rel(pattern);
        let Some(star) = norm.find('*') else {
            out.push(norm);
            continue;
        };
        let base = &norm[..star];
        let base_path = target_dir.join(base);
        if !path_exists(&base_path) {
            continue;
        }
        let entries = match fs::read_dir(&base_path) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let file_type = match ent.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = ent.file_name();
                let name = name.to_string_lossy();
                if !is_excluded_scan(&name) {
                    out.push(collapse_slashes(&format!("{base}{name}")));
                }
            }
        }
    }
    out
}

/// Extract workspace patterns from a parsed `package.json` value.
pub fn workspace_patterns(pkg: &Value) -> Vec<String> {
    let workspaces = match pkg.get("workspaces") {
        Some(w) => w,
        None => return Vec::new(),
    };
    if let Some(arr) = workspaces.as_array() {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    if let Some(packages) = workspaces.get("packages").and_then(|p| p.as_array()) {
        return packages
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    Vec::new()
}

fn find_src_dirs(dir: &Path, target_dir: &Path, depth: i32, found: &mut HashSet<String>) {
    if depth < 0 || !path_exists(dir) {
        return;
    }
    let rel = relative_path(target_dir, dir);
    if rel != "." && rel.ends_with("/src") {
        found.insert(rel);
    }
    if depth == 0 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if is_excluded_scan(&name) {
            continue;
        }
        find_src_dirs(&ent.path(), target_dir, depth - 1, found);
    }
}

/// Detect source roots under `target_dir`.
pub fn detect_roots(target_dir: &Path) -> DetectRootsResult {
    let mut found = HashSet::new();

    let pkg = read_package_json(target_dir);
    let patterns = pkg.as_ref().map(workspace_patterns).unwrap_or_default();
    for ws in expand_workspace_globs(target_dir, &patterns) {
        let src_rel = collapse_slashes(&format!("{ws}/src"));
        if path_exists(&target_dir.join(&src_rel)) {
            found.insert(src_rel);
        }
    }

    if path_exists(&target_dir.join("src")) {
        found.insert("src".to_string());
    }

    for base in SCAN_BASES {
        let start = if base.is_empty() {
            target_dir.to_path_buf()
        } else {
            target_dir.join(base)
        };
        let max_depth = if base.is_empty() { 3 } else { 2 };
        find_src_dirs(&start, target_dir, max_depth, &mut found);
    }

    let mut roots: Vec<String> = found.into_iter().collect();
    roots.sort();
    if !roots.is_empty() {
        DetectRootsResult {
            roots,
            warned: false,
        }
    } else {
        DetectRootsResult {
            roots: vec!["src".to_string()],
            warned: true,
        }
    }
}

fn walk_ext_counts(dir: &Path, counts: &mut HashMap<&'static str, u32>) {
    if !path_exists(dir) {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if is_excluded_scan(&name) || BASE_SKIP.contains(&name.as_ref()) {
                continue;
            }
            walk_ext_counts(&ent.path(), counts);
        } else {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            let ext = if let Some(dot) = name.rfind('.') {
                format!(".{}", &name[dot + 1..])
            } else {
                String::new()
            };
            if let Some(count) = counts.get_mut(ext.as_str()) {
                *count += 1;
            }
        }
    }
}

/// Detect file extensions present under the given source roots.
pub fn detect_exts(target_dir: &Path, roots: &[String]) -> Vec<String> {
    let mut counts: HashMap<&'static str, u32> =
        EXT_CANDIDATES.iter().copied().map(|e| (e, 0)).collect();

    for root in roots {
        walk_ext_counts(&target_dir.join(root), &mut counts);
    }

    let detected: Vec<String> = EXT_CANDIDATES
        .iter()
        .filter(|e| counts[*e] > 0)
        .map(|e| e.to_string())
        .collect();

    if detected.is_empty() {
        vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()]
    } else {
        detected
    }
}

/// Detect directories that should be skipped during scans.
pub fn detect_skip_dirs(target_dir: &Path) -> Vec<String> {
    let mut skip: Vec<String> = BASE_SKIP.iter().map(|s| s.to_string()).collect();
    for d in OPTIONAL_SKIP {
        if path_exists(&target_dir.join(d)) && !skip.iter().any(|s| s == d) {
            skip.push(d.to_string());
        }
    }
    skip
}

fn bin_exists(target_dir: &Path, name: &str) -> bool {
    path_exists(&target_dir.join("node_modules/.bin").join(name))
}

fn path_bin_exists(target_dir: &Path, name: &str, args: &[&str]) -> bool {
    Command::new(name)
        .args(args)
        .current_dir(target_dir)
        .output()
        .ok()
        .is_some_and(|o| o.status.success())
}

fn tool_exists(target_dir: &Path, name: &str, args: &[&str]) -> bool {
    bin_exists(target_dir, name) || path_bin_exists(target_dir, name, args)
}

fn leakscan_binary_exists(target_dir: &Path, engine_root: &Path) -> bool {
    [
        engine_root.join("bin/leakscan"),
        target_dir.join("tools/leakscan/target/release/leakscan"),
        target_dir.join("tools/leakscan/target/debug/leakscan"),
    ]
    .iter()
    .any(|p| path_exists(p))
}

fn has_shell_shebang(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let first = contents.lines().next().unwrap_or("");
    first.starts_with("#!")
        && ["sh", "bash", "zsh", "ksh", "dash"]
            .iter()
            .any(|shell| first.contains(shell))
}

fn is_shell_script_path(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["sh", "bash", "zsh", "ksh", "dash", "bats"].contains(&e))
    {
        return true;
    }
    has_shell_shebang(path)
}

fn has_shell_scripts(dir: &Path, depth: i32) -> bool {
    if depth < 0 || !path_exists(dir) {
        return false;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if file_type.is_file() && is_shell_script_path(&path) {
            return true;
        }
        if file_type.is_dir() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if is_excluded_scan(&name) || name == ".git" || name == "target" {
                continue;
            }
            if has_shell_scripts(&path, depth - 1) {
                return true;
            }
        }
    }
    false
}

fn walk_has_leakscan_frontend_file(dir: &Path) -> bool {
    if !path_exists(dir) {
        return false;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for ent in entries.flatten() {
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if is_excluded_scan(&name) || BASE_SKIP.contains(&name.as_ref()) {
                continue;
            }
            if walk_has_leakscan_frontend_file(&ent.path()) {
                return true;
            }
        } else {
            let path = ent.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"));
            if ext
                .as_deref()
                .is_some_and(|e| LEAKSCAN_FRONTEND_EXTS.contains(&e))
            {
                return true;
            }
        }
    }
    false
}

fn has_workflow_files(target_dir: &Path) -> bool {
    let dir = target_dir.join(".github/workflows");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "yml" || e == "yaml")
    })
}

fn has_typos_scope(target_dir: &Path) -> bool {
    if ["docs", "rules", "skills"]
        .iter()
        .any(|p| path_exists(&target_dir.join(p)))
    {
        return true;
    }
    if [
        "README.md",
        "CONTRIBUTING.md",
        "CHANGELOG.md",
        "SECURITY.md",
        "CODE_OF_CONDUCT.md",
    ]
    .iter()
    .any(|p| path_exists(&target_dir.join(p)))
    {
        return true;
    }
    detect_roots(target_dir)
        .roots
        .iter()
        .any(|root| path_exists(&target_dir.join(root)))
}

fn has_leakscan_frontend_roots(target_dir: &Path, roots: &[String], exts: &[String]) -> bool {
    if !exts
        .iter()
        .any(|e| LEAKSCAN_FRONTEND_EXTS.contains(&e.as_str()))
    {
        return false;
    }
    roots
        .iter()
        .any(|root| walk_has_leakscan_frontend_file(&target_dir.join(root)))
}

/// Detect which checkers are available in the project.
pub fn detect_checkers(
    target_dir: &Path,
    roots: &[String],
    exts: &[String],
    engine_root: &Path,
) -> Value {
    let mut checkers = serde_json::Map::new();

    if path_exists(&target_dir.join("tsconfig.json")) && bin_exists(target_dir, "tsc") {
        checkers.insert("tsc".to_string(), json!(true));
    }
    if bin_exists(target_dir, "knip") {
        checkers.insert("knip".to_string(), json!(true));
    }
    if bin_exists(target_dir, "jscpd") {
        checkers.insert("jscpd".to_string(), json!({ "minTokens": 50 }));
    }
    if bin_exists(target_dir, "depcruise") {
        checkers.insert("depcruise".to_string(), json!(true));
    }
    if bin_exists(target_dir, "type-coverage") {
        checkers.insert("type-coverage".to_string(), json!(true));
    }
    if tool_exists(target_dir, "shellcheck", &["--version"]) && has_shell_scripts(target_dir, 5) {
        checkers.insert("shellcheck".to_string(), json!(true));
    }
    if tool_exists(target_dir, "actionlint", &["-version"]) && has_workflow_files(target_dir) {
        checkers.insert("actionlint".to_string(), json!(true));
    }
    if tool_exists(target_dir, "typos", &["--version"]) && has_typos_scope(target_dir) {
        checkers.insert("typos".to_string(), json!(true));
    }
    if leakscan_binary_exists(target_dir, engine_root)
        && has_leakscan_frontend_roots(target_dir, roots, exts)
    {
        checkers.insert("leakscan".to_string(), json!(true));
    }
    checkers.insert("diff-shape".to_string(), json!({ "maxDirs": 5 }));

    Value::Object(checkers)
}

fn collect_named_files(
    dir: &Path,
    target_dir: &Path,
    depth: i32,
    name: &str,
    out: &mut Vec<String>,
) {
    if depth < 0 || !path_exists(dir) {
        return;
    }
    let candidate = dir.join(name);
    if path_exists(&candidate) {
        out.push(relative_path(target_dir, &candidate));
    }
    if depth == 0 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let ent_name = ent.file_name();
        let ent_name = ent_name.to_string_lossy();
        if ent_name == "node_modules" || is_excluded_scan(&ent_name) {
            continue;
        }
        collect_named_files(&ent.path(), target_dir, depth - 1, name, out);
    }
}

fn collect_dir_files(dir: &Path, target_dir: &Path, out: &mut Vec<String>) {
    if !path_exists(dir) {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let Ok(file_type) = ent.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_dir_files(&path, target_dir, out);
        } else {
            out.push(relative_path(target_dir, &path));
        }
    }
}

/// Collect convention source files an agent should read for rule derivation.
pub fn build_convention_sources(target_dir: &Path) -> ConventionSources {
    let mut claude_md = Vec::new();
    collect_named_files(target_dir, target_dir, 3, "CLAUDE.md", &mut claude_md);

    let mut skills = Vec::new();
    collect_dir_files(&target_dir.join(".claude/skills"), target_dir, &mut skills);

    let mut agents = Vec::new();
    collect_dir_files(&target_dir.join(".claude/agents"), target_dir, &mut agents);

    let mut commands = Vec::new();
    collect_dir_files(
        &target_dir.join(".claude/commands"),
        target_dir,
        &mut commands,
    );

    let mut editor_rules = Vec::new();
    for f in [".cursorrules", ".windsurfrules", ".clinerules"] {
        if path_exists(&target_dir.join(f)) {
            editor_rules.push(f.to_string());
        }
    }

    let mut knowledge_docs = Vec::new();
    for f in [".project_knowledge.md", "AGENTS.md"] {
        if path_exists(&target_dir.join(f)) {
            knowledge_docs.push(f.to_string());
        }
    }

    let sort = |v: &mut Vec<String>| v.sort();

    sort(&mut claude_md);
    sort(&mut skills);
    sort(&mut agents);
    sort(&mut commands);
    sort(&mut editor_rules);
    sort(&mut knowledge_docs);

    ConventionSources {
        version: 1,
        generated_hint: "inputs an agent should read to derive project-specific rule candidates"
            .to_string(),
        claude_md,
        skills,
        agents,
        commands,
        editor_rules,
        knowledge_docs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn setup_workspace_project(root: &Path) {
        write_file(
            &root.join("package.json"),
            r#"{
  "name": "test-monorepo",
  "workspaces": ["packages/*"],
  "dependencies": { "typescript": "^5.0.0", "knip": "^3.0.0" }
}"#,
        );
        write_file(
            &root.join("tsconfig.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        write_file(
            &root.join("packages/pkg-a/src/index.ts"),
            "export const a = 1;\n",
        );
        write_file(
            &root.join("packages/pkg-b/src/component.tsx"),
            "export const B = () => null;\n",
        );
        write_file(&root.join("node_modules/.bin/tsc"), "#!/usr/bin/env node\n");
        write_file(
            &root.join("node_modules/.bin/knip"),
            "#!/usr/bin/env node\n",
        );
    }

    #[test]
    fn read_package_json_parses_workspaces() {
        let tmp = TempDir::new().unwrap();
        setup_workspace_project(tmp.path());

        let pkg = read_package_json(tmp.path()).expect("package.json should parse");
        assert_eq!(pkg["name"], "test-monorepo");
        let patterns = workspace_patterns(&pkg);
        assert_eq!(patterns, vec!["packages/*"]);
    }

    #[test]
    fn expand_workspace_globs_resolves_packages() {
        let tmp = TempDir::new().unwrap();
        setup_workspace_project(tmp.path());

        let expanded = expand_workspace_globs(tmp.path(), &["packages/*".to_string()]);
        assert!(expanded.contains(&"packages/pkg-a".to_string()));
        assert!(expanded.contains(&"packages/pkg-b".to_string()));
    }

    #[test]
    fn detect_roots_finds_workspace_src_dirs() {
        let tmp = TempDir::new().unwrap();
        setup_workspace_project(tmp.path());

        let result = detect_roots(tmp.path());
        assert!(!result.warned);
        assert!(result.roots.contains(&"packages/pkg-a/src".to_string()));
        assert!(result.roots.contains(&"packages/pkg-b/src".to_string()));
    }

    #[test]
    fn detect_exts_includes_ts_and_tsx() {
        let tmp = TempDir::new().unwrap();
        setup_workspace_project(tmp.path());

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        assert!(exts.contains(&".ts".to_string()));
        assert!(exts.contains(&".tsx".to_string()));
    }

    #[test]
    fn detect_checkers_flags_tsc_when_binary_present() {
        let tmp = TempDir::new().unwrap();
        setup_workspace_project(tmp.path());

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        let checkers = detect_checkers(tmp.path(), &roots, &exts, tmp.path());
        assert_eq!(checkers.get("tsc"), Some(&json!(true)));
        assert_eq!(checkers.get("knip"), Some(&json!(true)));
        assert_eq!(checkers.get("diff-shape"), Some(&json!({ "maxDirs": 5 })));
    }

    #[test]
    fn detect_checkers_flags_optional_external_adapters_when_relevant() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("node_modules/.bin/shellcheck"),
            "#!/usr/bin/env sh\n",
        );
        write_file(
            &tmp.path().join("node_modules/.bin/actionlint"),
            "#!/usr/bin/env sh\n",
        );
        write_file(
            &tmp.path().join("node_modules/.bin/typos"),
            "#!/usr/bin/env sh\n",
        );
        write_file(&tmp.path().join("scripts/lint.sh"), "#!/usr/bin/env sh\n");
        write_file(&tmp.path().join(".github/workflows/ci.yml"), "name: ci\n");
        write_file(&tmp.path().join("docs/guide.md"), "# Guide\n");

        let checkers = detect_checkers(tmp.path());

        assert_eq!(checkers.get("shellcheck"), Some(&json!(true)));
        assert_eq!(checkers.get("actionlint"), Some(&json!(true)));
        assert_eq!(checkers.get("typos"), Some(&json!(true)));
    }

    #[test]
    fn detect_checkers_omits_tsc_without_binary() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("tsconfig.json"), "{}");
        // no node_modules/.bin/tsc

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        let checkers = detect_checkers(tmp.path(), &roots, &exts, tmp.path());
        assert!(checkers.get("tsc").is_none());
        assert_eq!(checkers.get("diff-shape"), Some(&json!({ "maxDirs": 5 })));
    }

    #[test]
    fn detect_exts_includes_rs_for_rust_roots() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("crates/core/src/lib.rs"),
            "pub fn x() {}\n",
        );

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);

        assert!(roots.contains(&"crates/core/src".to_string()));
        assert!(exts.contains(&".rs".to_string()));
    }

    #[test]
    fn detect_checkers_enables_leakscan_for_bundled_binary_and_frontend_sources() {
        let tmp = TempDir::new().unwrap();
        let engine = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("src/components/Button.tsx"),
            "export function Button() { return null; }\n",
        );
        write_file(&engine.path().join("bin/leakscan"), "#!/bin/sh\n");

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        let checkers = detect_checkers(tmp.path(), &roots, &exts, engine.path());

        assert_eq!(checkers.get("leakscan"), Some(&json!(true)));
    }

    #[test]
    fn detect_checkers_enables_leakscan_for_dev_binary_and_frontend_sources() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("src/components/Button.jsx"),
            "export function Button() { return null; }\n",
        );
        write_file(
            &tmp.path().join("tools/leakscan/target/debug/leakscan"),
            "#!/bin/sh\n",
        );

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        let checkers = detect_checkers(tmp.path(), &roots, &exts, tmp.path());

        assert_eq!(checkers.get("leakscan"), Some(&json!(true)));
    }

    #[test]
    fn detect_checkers_omits_leakscan_for_rust_only_roots_even_with_binary() {
        let tmp = TempDir::new().unwrap();
        let engine = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("crates/core/src/lib.rs"),
            "pub fn x() {}\n",
        );
        write_file(&engine.path().join("bin/leakscan"), "#!/bin/sh\n");

        let roots = detect_roots(tmp.path()).roots;
        let exts = detect_exts(tmp.path(), &roots);
        let checkers = detect_checkers(tmp.path(), &roots, &exts, engine.path());

        assert!(checkers.get("leakscan").is_none());
    }

    #[test]
    fn missing_package_json_returns_none_and_defaults() {
        let tmp = TempDir::new().unwrap();
        // empty dir — no package.json, no src

        assert!(read_package_json(tmp.path()).is_none());
        assert!(workspace_patterns(&json!({})).is_empty());

        let result = detect_roots(tmp.path());
        assert!(result.warned);
        assert_eq!(result.roots, vec!["src".to_string()]);

        let exts = detect_exts(tmp.path(), &result.roots);
        assert_eq!(
            exts,
            vec![".ts".to_string(), ".tsx".to_string(), ".astro".to_string()]
        );
    }

    #[test]
    fn detect_skip_dirs_includes_optional_when_present() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".next")).unwrap();

        let skip = detect_skip_dirs(tmp.path());
        assert!(skip.contains(&"node_modules".to_string()));
        assert!(skip.contains(&".next".to_string()));
    }

    #[test]
    fn build_convention_sources_collects_files() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("CLAUDE.md"), "# project\n");
        write_file(&tmp.path().join(".claude/skills/foo/SKILL.md"), "# skill\n");
        write_file(&tmp.path().join("AGENTS.md"), "# agents\n");

        let sources = build_convention_sources(tmp.path());
        assert_eq!(sources.version, 1);
        assert!(sources.claude_md.contains(&"CLAUDE.md".to_string()));
        assert!(sources.skills.iter().any(|p| p.contains(".claude/skills")));
        assert!(sources.knowledge_docs.contains(&"AGENTS.md".to_string()));
    }

    #[test]
    fn invalid_package_json_returns_none() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("package.json"), "{ not json");

        assert!(read_package_json(tmp.path()).is_none());
    }
}
