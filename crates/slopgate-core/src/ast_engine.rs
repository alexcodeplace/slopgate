//! ast-grep engine wrapper (bucket-B structural rules).
//! Mirrors `src/ast-engine.mjs`: resolve local/PATH binary, spawn scan, map JSON → violations.
//! Missing binary → `available: false` + reason — never panics.

use crate::config::ResolvedConfig;
use crate::report::Violation;
use crate::temp::with_temp_dir_in;
use serde_json::Value;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MISSING_BIN_MSG: &str =
    "ast-grep binary not found (npm i -g @ast-grep/cli) — bucket-B rules SKIPPED";

/// Outcome of [`run_ast_grep_scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepScanResult {
    pub available: bool,
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Options for [`run_ast_grep_scan`].
#[derive(Debug, Clone, Default)]
pub struct AstGrepScanOpts {
    /// When true, allow directory and external fixture targets without extension filtering.
    pub raw_targets: bool,
    /// Overrides `PATH` for ast-grep resolution only (unit tests).
    #[doc(hidden)]
    pub path_env: Option<String>,
}

/// Result of parsing ast-grep JSON stdout (array of match objects).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepParseResult {
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Resolve `node_modules/.bin/ast-grep` under `repo_root`, then `ast-grep` on PATH.
/// Returns `(None, "")` when neither is available.
pub fn resolve_ast_grep_bin(repo_root: &Path) -> (Option<PathBuf>, String) {
    resolve_ast_grep_bin_inner(repo_root, None)
}

fn resolve_ast_grep_bin_inner(
    repo_root: &Path,
    path_env: Option<&OsStr>,
) -> (Option<PathBuf>, String) {
    if let Some(local) = resolve_local_bin(repo_root) {
        return (Some(local), "local".to_string());
    }

    let mut cmd = Command::new("ast-grep");
    cmd.arg("--version");
    if let Some(path) = path_env {
        cmd.env("PATH", path);
    }

    match cmd.output() {
        Ok(output) if output.status.success() => {
            (Some(PathBuf::from("ast-grep")), "path".to_string())
        }
        _ => (None, String::new()),
    }
}

#[cfg(not(windows))]
fn resolve_local_bin(repo_root: &Path) -> Option<PathBuf> {
    let local = repo_root.join("node_modules/.bin/ast-grep");
    local.exists().then_some(local)
}

/// Windows: `node_modules/.bin/ast-grep` is a POSIX sh shim that CreateProcess
/// rejects (os error 193). Prefer the platform package's `ast-grep.exe` —
/// a sibling of the resolved `@ast-grep/cli` dir under both npm hoisting and
/// pnpm's virtual store — then the `.cmd` shim, which std spawns via cmd.exe.
#[cfg(windows)]
fn resolve_local_bin(repo_root: &Path) -> Option<PathBuf> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        "x86" => Some("ia32"),
        _ => None,
    };
    if let Some(arch) = arch {
        let cli = repo_root.join("node_modules/@ast-grep/cli");
        if let Ok(real) = fs::canonicalize(&cli) {
            if let Some(scope) = real.parent() {
                let exe = scope
                    .join(format!("cli-win32-{arch}-msvc"))
                    .join("ast-grep.exe");
                if exe.exists() {
                    return Some(exe);
                }
            }
        }
    }
    let shim = repo_root.join("node_modules/.bin/ast-grep.cmd");
    shim.exists().then_some(shim)
}

/// Map ast-grep `--json` match array to engine violations (`engine: "ast"`).
/// Unit-testable without spawning a binary.
pub fn parse_ast_grep_json(matches: &Value) -> AstGrepParseResult {
    let Some(items) = matches.as_array() else {
        return AstGrepParseResult {
            violations: vec![],
            errors: vec!["ast-grep output was not an array".to_string()],
        };
    };

    let mut violations = Vec::new();
    let mut errors = Vec::new();

    for m in items {
        let Some(obj) = m.as_object() else {
            errors.push("ast-grep output contained a non-object match".to_string());
            continue;
        };

        let Some(rule_id) = obj.get("ruleId").and_then(|v| v.as_str()) else {
            errors.push("ast-grep match had no ruleId".to_string());
            continue;
        };
        let rule_id = rule_id.to_string();
        let Some(file) = obj.get("file").and_then(|f| f.as_str()) else {
            errors.push(format!("rule {rule_id}: match had no file path"));
            continue;
        };
        let Some(lines) = obj.get("lines").and_then(|l| l.as_str()) else {
            errors.push(format!("rule {rule_id}: match had no source lines"));
            continue;
        };

        let meta = match obj.get("note") {
            None | Some(Value::Null) => Value::Object(serde_json::Map::new()),
            Some(Value::String(note)) if note.is_empty() => Value::Object(serde_json::Map::new()),
            Some(Value::String(note)) => match serde_json::from_str::<Value>(note) {
                Ok(value) if value.is_object() => value,
                Ok(_) => {
                    errors.push(format!("rule {rule_id}: note JSON is not an object"));
                    Value::Object(serde_json::Map::new())
                }
                Err(_) => {
                    errors.push(format!("rule {rule_id}: note is not valid JSON"));
                    Value::Object(serde_json::Map::new())
                }
            },
            Some(_) => {
                errors.push(format!("rule {rule_id}: note is not a JSON string"));
                Value::Object(serde_json::Map::new())
            }
        };

        let first_line = lines.split('\n').next().unwrap_or("");

        let ast_severity = obj.get("severity").and_then(|s| s.as_str());
        let severity = meta
            .get("severity")
            .and_then(|s| s.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if ast_severity == Some("error") {
                    "high".to_string()
                } else {
                    "medium".to_string()
                }
            });

        let category = meta
            .get("category")
            .and_then(|c| c.as_str())
            .unwrap_or("convention")
            .to_string();

        let line = obj
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32
            + 1;

        let message = obj
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        let resolution = meta
            .get("resolution")
            .and_then(|r| r.as_str())
            .map(str::to_string)
            .unwrap_or(message);

        violations.push(Violation {
            id: rule_id,
            severity,
            category,
            file: file.to_string(),
            line,
            full_line: first_line.to_string(),
            text: truncate_chars(first_line.trim(), 90),
            resolution,
            engine: "ast".to_string(),
        });
    }

    AstGrepParseResult { violations, errors }
}

fn normalize_relative_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/").replace('\\', "/"))
}

fn input_relative_path(repo_root: &Path, canonical_root: &Path, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.strip_prefix(canonical_root)
            .or_else(|_| path.strip_prefix(repo_root))
            .ok()
            .and_then(normalize_relative_path)
    } else {
        normalize_relative_path(path)
    }
}

fn staged_deleted_paths(repo_root: &Path) -> Result<HashSet<String>, String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "--diff-filter=DR",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("cannot inspect staged deletions: {e}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("git diff exited with {}", output.status)
        } else {
            format!("git diff failed: {detail}")
        });
    }

    let fields: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();
    let mut deleted = HashSet::new();
    let mut index = 0;
    while index < fields.len() {
        let status = fields[index];
        index += 1;
        if status.is_empty() {
            continue;
        }
        let Some(status_code) = status.first().copied() else {
            continue;
        };
        let Some(first_path) = fields.get(index) else {
            break;
        };
        index += 1;
        let first_path = String::from_utf8_lossy(first_path).replace('\\', "/");
        if status_code == b'D' || status_code == b'R' {
            deleted.insert(first_path);
        }
        if status_code == b'R' {
            index += 1;
        }
    }
    Ok(deleted)
}

fn resolve_changed_targets(
    repo_root: &Path,
    files: &[String],
    raw_targets: bool,
) -> (Vec<String>, Vec<String>) {
    let canonical_root = match fs::canonicalize(repo_root) {
        Ok(root) => root,
        Err(error) => {
            return (
                vec![],
                vec![format!(
                    "ast-grep path resolution failed: cannot resolve worktree root {}: {error}",
                    repo_root.display()
                )],
            )
        }
    };

    let mut deleted_paths: Option<HashSet<String>> = None;
    let mut targets = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    for raw in files {
        if !raw_targets && !raw.ends_with(".ts") && !raw.ends_with(".tsx") {
            continue;
        }

        let path = Path::new(raw);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };

        let metadata = match fs::metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let rel = input_relative_path(repo_root, &canonical_root, raw);
                if let Some(deleted) = deleted_paths.as_ref() {
                    if rel.as_deref().is_some_and(|path| deleted.contains(path)) {
                        continue;
                    }
                } else {
                    match staged_deleted_paths(repo_root) {
                        Ok(paths) => deleted_paths = Some(paths),
                        Err(detail) => {
                            errors.push(format!(
                                "ast-grep path resolution failed for '{raw}': {detail}"
                            ));
                            continue;
                        }
                    }
                    if rel.as_deref().is_some_and(|path| {
                        deleted_paths.as_ref().is_some_and(|set| set.contains(path))
                    }) {
                        continue;
                    }
                }
                errors.push(format!(
                    "ast-grep path resolution failed for '{raw}': file does not exist under worktree root {}",
                    canonical_root.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "ast-grep path resolution failed for '{raw}': cannot inspect path: {error}"
                ));
                continue;
            }
        };

        if !(metadata.is_file() || (raw_targets && metadata.is_dir())) {
            errors.push(format!(
                "ast-grep path resolution failed for '{raw}': path is not a regular file"
            ));
            continue;
        }

        let canonical = match fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(error) => {
                errors.push(format!(
                    "ast-grep path resolution failed for '{raw}': cannot canonicalize path: {error}"
                ));
                continue;
            }
        };
        let relative = match canonical.strip_prefix(&canonical_root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) if raw_targets => canonical.to_string_lossy().replace('\\', "/"),
            Err(_) => {
                errors.push(format!(
                    "ast-grep path resolution failed for '{raw}': path escapes worktree root {}",
                    canonical_root.display()
                ));
                continue;
            }
        };
        if seen.insert(relative.clone()) {
            targets.push(relative);
        }
    }

    (targets, errors)
}

fn is_blank_or_comment_only(source: &str) -> bool {
    let mut remaining = source.strip_prefix('\u{feff}').unwrap_or(source);
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            return true;
        }
        if let Some(after_line_comment) = remaining.strip_prefix("//") {
            remaining = after_line_comment
                .find('\n')
                .map(|end| &after_line_comment[end + 1..])
                .unwrap_or("");
            continue;
        }
        if let Some(after_block_start) = remaining.strip_prefix("/*") {
            let Some(end) = after_block_start.find("*/") else {
                return false;
            };
            remaining = &after_block_start[end + 2..];
            continue;
        }
        return false;
    }
}

fn equivalent_scanned_path(repo_root: &Path, expected: &str, actual: &str) -> bool {
    fn absolute(root: &Path, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    }

    fs::canonicalize(absolute(repo_root, expected)).ok()
        == fs::canonicalize(absolute(repo_root, actual)).ok()
}

const PATH_CANARY_ID: &str = "slopgate-path-participation";

fn prepare_path_participation_canary(
    dir: &Path,
    targets: &[String],
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), Vec<String>> {
    let mut errors = Vec::new();
    let mut groups: Vec<(&str, Vec<&String>)> =
        vec![("TypeScript", Vec::new()), ("Tsx", Vec::new())];
    for target in targets {
        match Path::new(target).extension().and_then(OsStr::to_str) {
            Some("ts") => groups[0].1.push(target),
            Some("tsx") => groups[1].1.push(target),
            _ => errors.push(format!(
                "ast-grep path participation failed for '{target}': unsupported source extension"
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut rule_dirs = Vec::new();
    let mut sentinels = Vec::new();
    for (language, group) in groups {
        if group.is_empty() {
            continue;
        }
        let suffix = if language == "Tsx" { "tsx" } else { "ts" };
        let rule_dir = dir.join(format!("path-canary-rules-{suffix}"));
        let rule = rule_dir.join("path-canary.yml");
        let sentinel = dir.join(format!("path-canary-sentinel.{suffix}"));
        let rule_text = format!(
            "id: {PATH_CANARY_ID}\nlanguage: {language}\nseverity: info\nmessage: path parsed\nrule:\n  kind: program\n"
        );
        if let Err(error) = fs::create_dir_all(&rule_dir) {
            errors.push(format!(
                "ast-grep path participation failed: cannot create canary rule directory: {error}"
            ));
            continue;
        }
        if let Err(error) = fs::write(&rule, rule_text) {
            errors.push(format!(
                "ast-grep path participation failed: cannot write canary rule: {error}"
            ));
            continue;
        }
        if let Err(error) = fs::write(&sentinel, "export const __slopgate_path_canary = 1;\n") {
            errors.push(format!(
                "ast-grep path participation failed: cannot write canary sentinel: {error}"
            ));
            continue;
        }
        rule_dirs.push(rule_dir);
        sentinels.push(sentinel);
    }

    if errors.is_empty() {
        Ok((rule_dirs, sentinels))
    } else {
        Err(errors)
    }
}

fn validate_path_participation(
    parsed: &Value,
    repo_root: &Path,
    targets: &[String],
    sentinels: &[PathBuf],
) -> Vec<String> {
    let Some(items) = parsed.as_array() else {
        return vec!["ast-grep path participation returned JSON that was not an array".to_string()];
    };

    let mut errors = Vec::new();
    let mut covered = HashSet::new();
    let mut seen_sentinels = HashSet::new();
    for item in items {
        let Some(object) = item.as_object() else {
            errors.push("ast-grep returned a non-object match".to_string());
            continue;
        };
        if object.get("ruleId").and_then(Value::as_str) != Some(PATH_CANARY_ID) {
            continue;
        }
        let Some(file) = object.get("file").and_then(Value::as_str) else {
            errors.push("ast-grep path participation match had no file path".to_string());
            continue;
        };
        for (index, sentinel) in sentinels.iter().enumerate() {
            if equivalent_scanned_path(repo_root, &sentinel.to_string_lossy(), file) {
                seen_sentinels.insert(index);
            }
        }
        for target in targets {
            if equivalent_scanned_path(repo_root, target, file) {
                covered.insert(target.clone());
            }
        }
    }

    if seen_sentinels.len() != sentinels.len() {
        errors.push(
            "ast-grep path participation canary was not observed; scanner may have ignored its arguments"
                .to_string(),
        );
    }
    for target in targets {
        if covered.contains(target) {
            continue;
        }
        let path = Path::new(target);
        let source = if path.is_absolute() {
            fs::read_to_string(path)
        } else {
            fs::read_to_string(repo_root.join(path))
        };
        match source {
            Ok(source) if is_blank_or_comment_only(&source) => {}
            Ok(_) => errors.push(format!(
                "ast-grep path participation missing structural canary for '{target}'"
            )),
            Err(error) => errors.push(format!(
                "ast-grep path participation cannot inspect '{target}': {error}"
            )),
        }
    }
    errors
}

/// Run ast-grep against project rule dirs and map findings to violations. Never panics.
pub fn run_ast_grep_scan(
    config: &ResolvedConfig,
    files: Option<&[String]>,
    opts: &AstGrepScanOpts,
) -> AstGrepScanResult {
    run_ast_grep_scan_in(config, files, opts, std::env::temp_dir())
}

fn run_ast_grep_scan_in(
    config: &ResolvedConfig,
    files: Option<&[String]>,
    opts: &AstGrepScanOpts,
    temp_base: impl AsRef<Path>,
) -> AstGrepScanResult {
    let rule_dirs: Vec<&str> = config
        .ast_rule_dirs
        .iter()
        .filter(|d| Path::new(d).exists())
        .map(String::as_str)
        .collect();

    if rule_dirs.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors: vec![],
        };
    }

    let repo_root = Path::new(&config.repo_root);
    let mut errors = Vec::new();
    let targets: Vec<String> = match files {
        None => config.roots_rel.clone(),
        Some(files) => {
            let (resolved, resolution_errors) =
                resolve_changed_targets(repo_root, files, opts.raw_targets);
            errors.extend(resolution_errors);
            resolved
        }
    };

    if files.is_some() && !errors.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors,
        };
    }

    if files.is_some() && targets.is_empty() {
        return AstGrepScanResult {
            available: true,
            violations: vec![],
            errors,
        };
    }

    let path_env = opts.path_env.as_deref().map(OsStr::new);
    let (bin, source) = resolve_ast_grep_bin_inner(repo_root, path_env);

    let Some(bin) = bin else {
        return AstGrepScanResult {
            available: false,
            violations: vec![],
            errors: vec![MISSING_BIN_MSG.to_string()],
        };
    };

    if source == "path" {
        errors.push(
            "ast-grep: using PATH binary (version not pinned — results may differ from CI)"
                .to_string(),
        );
    }

    let scan = with_temp_dir_in(temp_base, "slopgate-sg-", |dir| {
        let mut scan_rule_dirs: Vec<String> = rule_dirs.iter().map(|d| (*d).to_string()).collect();
        let mut sentinels = Vec::new();
        if files.is_some() && !opts.raw_targets {
            match prepare_path_participation_canary(dir, &targets) {
                Ok((canary_dirs, canary_sentinels)) => {
                    scan_rule_dirs.extend(
                        canary_dirs
                            .into_iter()
                            .map(|canary_dir| canary_dir.to_string_lossy().into_owned()),
                    );
                    sentinels = canary_sentinels;
                }
                Err(canary_errors) => {
                    errors.extend(canary_errors);
                    return AstGrepScanResult {
                        available: true,
                        violations: vec![],
                        errors,
                    };
                }
            }
        }

        let sg_config = dir.join("sgconfig.yml");
        let yml = format!(
            "ruleDirs:\n{}\n",
            scan_rule_dirs
                .iter()
                .map(|d| format!("  - {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if let Err(e) = fs::write(&sg_config, yml) {
            return AstGrepScanResult {
                available: false,
                violations: vec![],
                errors: vec![format!("ast-grep failed: {e}")],
            };
        }

        let mut args: Vec<String> = vec![
            "scan".into(),
            "--config".into(),
            sg_config.to_string_lossy().into_owned(),
            "--json".into(),
        ];
        args.extend(targets.iter().cloned());
        args.extend(
            sentinels
                .iter()
                .map(|path| path.to_string_lossy().into_owned()),
        );

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = Command::new(&bin);
        cmd.args(&arg_refs).current_dir(repo_root);
        if let Some(path) = path_env {
            cmd.env("PATH", path);
        }

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                return AstGrepScanResult {
                    available: false,
                    violations: vec![],
                    errors: vec![format!("ast-grep failed: {e}")],
                };
            }
        };

        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if detail.is_empty() {
                format!("no stderr; status {}", output.status)
            } else {
                format!("stderr: {}", detail.chars().take(500).collect::<String>())
            };
            errors.push(format!("ast-grep failed with {} ({detail})", output.status));
            return AstGrepScanResult {
                available: true,
                violations: vec![],
                errors,
            };
        }

        let stderr = match std::str::from_utf8(&output.stderr) {
            Ok(stderr) => stderr,
            Err(error) => {
                errors.push(format!("ast-grep stderr was not valid UTF-8: {error}"));
                ""
            }
        };
        if !stderr.trim().is_empty() {
            let cap = stderr.trim().chars().take(500).collect::<String>();
            errors.push(format!("ast-grep stderr: {cap}"));
        }

        let stdout = match std::str::from_utf8(&output.stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                errors.push(format!("ast-grep output was not valid UTF-8: {error}"));
                return AstGrepScanResult {
                    available: true,
                    violations: vec![],
                    errors,
                };
            }
        };
        let parsed: Value = match serde_json::from_str(stdout.trim()) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("ast-grep JSON parse error: {e}"));
                return AstGrepScanResult {
                    available: true,
                    violations: vec![],
                    errors,
                };
            }
        };

        if !sentinels.is_empty() {
            errors.extend(validate_path_participation(
                &parsed, repo_root, &targets, &sentinels,
            ));
        }

        let AstGrepParseResult {
            violations,
            errors: mut parse_errors,
        } = parse_ast_grep_json(&parsed);
        errors.append(&mut parse_errors);
        let violations = violations
            .into_iter()
            .filter(|violation| violation.id != PATH_CANARY_ID)
            .collect();

        AstGrepScanResult {
            available: true,
            violations,
            errors,
        }
    });

    match scan {
        Ok(result) => result,
        Err(e) => AstGrepScanResult {
            available: false,
            violations: vec![],
            errors: vec![format!("ast-grep failed: {e}")],
        },
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    #[cfg(unix)]
    static CAPTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// Write a runnable `ast-grep` stub that prints `stdout`: `.cmd` batch on
    /// Windows (spawnable), executable sh script elsewhere.
    fn write_stub(bin_dir: &Path, stdout: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let stub = bin_dir.join("ast-grep.cmd");
            fs::write(&stub, format!("@echo off\r\necho {stdout}\r\n")).unwrap();
            stub
        }
        #[cfg(not(windows))]
        {
            let stub = bin_dir.join("ast-grep");
            fs::write(&stub, format!("#!/bin/sh\necho '{stdout}'\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
            stub
        }
    }

    #[cfg(unix)]
    fn write_canary_stub(bin_dir: &Path, skipped_suffix: Option<&str>) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let stub = bin_dir.join("ast-grep");
        let skip = skipped_suffix
            .map(|suffix| format!("case \"$arg\" in *{suffix}) continue ;; esac"))
            .unwrap_or_default();
        let script = format!(
            "#!/bin/sh\nprintf '['\nfirst=1\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    *.ts|*.tsx)\n      {skip}\n      if [ $first -eq 0 ]; then printf ','; fi\n      printf '{{\"ruleId\":\"{PATH_CANARY_ID}\",\"file\":\"%s\",\"lines\":\"canary\"}}' \"$arg\"\n      first=0\n      ;;\n  esac\ndone\nprintf ']'\n"
        );
        fs::write(&stub, script).unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        stub
    }

    #[cfg(unix)]
    fn write_observing_stub(bin_dir: &Path, status: i32, stderr: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let stub = bin_dir.join("ast-grep");
        let escaped_stderr = stderr.replace('\\', "\\\\").replace('\'', "'\\''");
        let script = format!(
            "#!/bin/sh\nprintf 'cwd=%s\\n' \"$PWD\" > \"$SLOPGATE_CAPTURE\"\nprintf 'arg=%s\\n' \"$@\" >> \"$SLOPGATE_CAPTURE\"\nprintf '%s' '{escaped_stderr}' >&2\nprintf '[]'\nexit {status}\n"
        );
        fs::write(&stub, script).unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        stub
    }

    #[cfg(unix)]
    fn ast_config_at(root: &Path) -> ResolvedConfig {
        let rule_dir = root.join("rules/ast");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(&rule_dir).unwrap();
        fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        ResolvedConfig {
            repo_root: root.to_string_lossy().into_owned(),
            config_dir: root.to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        }
    }

    #[cfg(unix)]
    fn ast_config(dir: &TempDir) -> ResolvedConfig {
        ast_config_at(dir.path())
    }

    #[cfg(unix)]
    fn run_with_stub(
        files: &[String],
        status: i32,
        scanner_stderr: &str,
    ) -> (AstGrepScanResult, String) {
        let _capture_guard = CAPTURE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        for file in files {
            let path = dir.path().join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            if file.ends_with(".ts") || file.ends_with(".tsx") {
                fs::write(path, "export const value = 1;\n").unwrap();
            }
        }
        let stub = dir.path().join("node_modules/.bin");
        write_observing_stub(&stub, status, scanner_stderr);
        let capture = dir.path().join("capture.txt");
        let old_capture = std::env::var_os("SLOPGATE_CAPTURE");
        std::env::set_var("SLOPGATE_CAPTURE", &capture);
        let result = run_ast_grep_scan(
            &config,
            Some(files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        match old_capture {
            Some(value) => std::env::set_var("SLOPGATE_CAPTURE", value),
            None => std::env::remove_var("SLOPGATE_CAPTURE"),
        }
        let captured = fs::read_to_string(capture).unwrap_or_default();
        (result, captured)
    }

    #[test]
    fn resolve_none_when_absent() {
        let dir = TempDir::new().unwrap();
        let (bin, source) =
            resolve_ast_grep_bin_inner(dir.path(), Some(OsStr::new("/nonexistent")));
        assert!(bin.is_none());
        assert!(source.is_empty());
    }

    #[test]
    fn resolve_some_when_stub_bin_exists() {
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let stub = write_stub(&bin_dir, "[]");

        let (bin, source) = resolve_ast_grep_bin(dir.path());
        assert_eq!(bin, Some(stub));
        assert_eq!(source, "local");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_prefers_platform_exe_on_windows() {
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            _ => return,
        };
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        write_stub(&bin_dir, "[]");

        let scope = dir.path().join("node_modules/@ast-grep");
        fs::create_dir_all(scope.join("cli")).unwrap();
        let exe_dir = scope.join(format!("cli-win32-{arch}-msvc"));
        fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("ast-grep.exe");
        fs::write(&exe, "").unwrap();

        let (bin, source) = resolve_ast_grep_bin(dir.path());
        assert_eq!(bin, Some(fs::canonicalize(&exe).unwrap()));
        assert_eq!(source, "local");
    }

    #[test]
    fn run_scan_no_binary_unavailable() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let opts = AstGrepScanOpts {
            raw_targets: false,
            path_env: Some("/nonexistent".to_string()),
        };
        let got = run_ast_grep_scan(&config, None, &opts);
        assert!(!got.available);
        assert!(got.violations.is_empty());
        assert_eq!(got.errors, vec![MISSING_BIN_MSG.to_string()]);
    }

    #[test]
    fn parse_ast_grep_json_maps_canned_match() {
        let json = json!([{
            "ruleId": "no-console",
            "severity": "error",
            "file": "src/app.ts",
            "lines": "  console.log('x')\n",
            "message": "Avoid console",
            "range": { "start": { "line": 4 } },
            "note": "{\"severity\":\"critical\",\"category\":\"hygiene\",\"resolution\":\"Remove console\"}"
        }]);

        let got = parse_ast_grep_json(&json);
        assert!(got.errors.is_empty());
        assert_eq!(got.violations.len(), 1);

        let v = &got.violations[0];
        assert_eq!(v.id, "no-console");
        assert_eq!(v.severity, "critical");
        assert_eq!(v.category, "hygiene");
        assert_eq!(v.file, "src/app.ts");
        assert_eq!(v.line, 5);
        assert_eq!(v.full_line, "  console.log('x')");
        assert_eq!(v.text, "console.log('x')");
        assert_eq!(v.resolution, "Remove console");
        assert_eq!(v.engine, "ast");
    }

    #[test]
    fn parse_ast_grep_json_defaults_when_note_missing() {
        let json = json!([{
            "ruleId": "bare-rule",
            "severity": "warning",
            "file": "x.tsx",
            "lines": "foo();\n",
            "message": "fix me",
            "range": { "start": { "line": 0 } }
        }]);

        let got = parse_ast_grep_json(&json);
        assert!(got.errors.is_empty());
        assert_eq!(got.violations.len(), 1);
        assert_eq!(got.violations[0].severity, "medium");
        assert_eq!(got.violations[0].category, "convention");
        assert_eq!(got.violations[0].line, 1);
        assert_eq!(got.violations[0].resolution, "fix me");
    }

    #[test]
    fn parse_ast_grep_json_invalid_note_is_error_not_panic() {
        let json = json!([{
            "ruleId": "bad-note",
            "file": "a.ts",
            "lines": "x",
            "note": "not-json"
        }]);

        let got = parse_ast_grep_json(&json);
        assert_eq!(got.violations.len(), 1);
        assert_eq!(got.violations[0].id, "bad-note");
        assert!(got
            .errors
            .iter()
            .any(|e| e.contains("bad-note") && e.contains("note")));
    }

    #[test]
    fn parse_ast_grep_json_non_string_note_is_error() {
        let json = json!([{
            "ruleId": "bad-note-type",
            "file": "a.ts",
            "lines": "x",
            "note": {"severity": "high"}
        }]);

        let got = parse_ast_grep_json(&json);
        assert_eq!(got.violations.len(), 1);
        assert!(got
            .errors
            .iter()
            .any(|e| e.contains("bad-note-type") && e.contains("not a JSON string")));
    }

    #[test]
    fn parse_ast_grep_json_non_array_reports_error() {
        let got = parse_ast_grep_json(&json!({ "oops": true }));
        assert!(got.violations.is_empty());
        assert_eq!(
            got.errors,
            vec!["ast-grep output was not an array".to_string()]
        );
    }

    #[test]
    fn parse_ast_grep_json_rejects_malformed_match() {
        let got = parse_ast_grep_json(&json!([{ "ruleId": "broken" }]));
        assert!(got.violations.is_empty());
        assert!(got.errors.iter().any(|error| error.contains("file path")));
    }

    #[test]
    fn run_scan_empty_rule_dirs_available_noop() {
        let dir = TempDir::new().unwrap();
        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![dir
                .path()
                .join("missing-ast-rules")
                .to_string_lossy()
                .into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let got = run_ast_grep_scan(&config, None, &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty());
    }

    #[test]
    fn run_scan_temp_dir_failure_unavailable() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        write_stub(&bin_dir, "[]");

        let not_a_dir = dir.path().join("blocking-tmp");
        fs::write(&not_a_dir, "x").unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec!["src".to_string()],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let got = run_ast_grep_scan_in(&config, None, &AstGrepScanOpts::default(), &not_a_dir);

        assert!(!got.available);
        assert!(got.violations.is_empty());
        assert_eq!(got.errors.len(), 1);
        assert!(got.errors[0].starts_with("ast-grep failed:"));
    }

    #[test]
    fn run_scan_non_ts_files_filtered_to_empty() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        write_stub(&bin_dir, "[]");

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let files = vec!["README.md".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_accepts_external_raw_directory_target() {
        let _capture_guard = CAPTURE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let repo = TempDir::new().unwrap();
        let fixtures = TempDir::new().unwrap();
        let config = ast_config(&repo);
        fs::write(
            fixtures.path().join("fixture.ts"),
            "export const fixture = 1;\n",
        )
        .unwrap();
        let bin_dir = repo.path().join("node_modules/.bin");
        write_observing_stub(&bin_dir, 0, "");
        let capture = repo.path().join("capture.txt");
        let old_capture = std::env::var_os("SLOPGATE_CAPTURE");
        std::env::set_var("SLOPGATE_CAPTURE", &capture);
        let target = fixtures.path().to_string_lossy().into_owned();
        let files = vec![target.clone()];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        match old_capture {
            Some(value) => std::env::set_var("SLOPGATE_CAPTURE", value),
            None => std::env::remove_var("SLOPGATE_CAPTURE"),
        }
        let captured = fs::read_to_string(capture).unwrap();
        assert!(got.errors.is_empty(), "unexpected errors: {:?}", got.errors);
        assert!(captured.contains(&format!("arg={target}\n")));
    }
    #[cfg(unix)]
    #[test]
    fn run_scan_resolves_linked_worktree_absolute_and_relative_paths() {
        let _capture_guard = CAPTURE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let main = TempDir::new().unwrap();
        fs::create_dir_all(main.path().join("src")).unwrap();
        fs::write(main.path().join("src/base.ts"), "export const base = 1;\n").unwrap();
        git_ok(main.path(), &["init", "-b", "main"]);
        git_ok(
            main.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git_ok(main.path(), &["config", "user.name", "test"]);
        git_ok(main.path(), &["add", "src/base.ts"]);
        git_ok(main.path(), &["commit", "-m", "initial"]);
        let linked_root = main.path().join("linked-worktree");
        git_ok(
            main.path(),
            &[
                "worktree",
                "add",
                "-b",
                "linked-test",
                "linked-worktree",
                "HEAD",
            ],
        );
        let config = ast_config_at(&linked_root);
        fs::write(
            linked_root.join("src/with space.ts"),
            "export const linked = 1;\n",
        )
        .unwrap();
        let bin_dir = linked_root.join("node_modules/.bin");
        write_observing_stub(&bin_dir, 0, "");
        let capture = linked_root.join("capture.txt");
        let old_capture = std::env::var_os("SLOPGATE_CAPTURE");
        std::env::set_var("SLOPGATE_CAPTURE", &capture);
        let absolute = linked_root.join("src/with space.ts");
        let files = vec![
            "src/base.ts".to_string(),
            absolute.to_string_lossy().into_owned(),
        ];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        match old_capture {
            Some(value) => std::env::set_var("SLOPGATE_CAPTURE", value),
            None => std::env::remove_var("SLOPGATE_CAPTURE"),
        }
        let captured = fs::read_to_string(capture).unwrap();
        assert!(got.errors.is_empty(), "unexpected errors: {:?}", got.errors);
        assert!(captured.contains("arg=src/base.ts\n"));
        assert!(captured.contains("arg=src/with space.ts\n"));
        assert!(!captured.contains(&format!("arg={}\n", absolute.display())));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_resolves_absolute_and_relative_paths_from_worktree_root() {
        let _capture_guard = CAPTURE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(
            dir.path().join("src/relative.ts"),
            "export const relative = 1;\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/with space.ts"),
            "export const spaced = 1;\n",
        )
        .unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        write_observing_stub(&bin_dir, 0, "");
        let capture = dir.path().join("capture.txt");
        let old_capture = std::env::var_os("SLOPGATE_CAPTURE");
        std::env::set_var("SLOPGATE_CAPTURE", &capture);
        let absolute = dir.path().join("src/with space.ts");
        let files = vec![
            "src/relative.ts".to_string(),
            absolute.to_string_lossy().into_owned(),
        ];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        match old_capture {
            Some(value) => std::env::set_var("SLOPGATE_CAPTURE", value),
            None => std::env::remove_var("SLOPGATE_CAPTURE"),
        }
        let captured = fs::read_to_string(capture).unwrap();
        assert!(got.errors.is_empty(), "unexpected errors: {:?}", got.errors);
        assert!(captured.contains("cwd="));
        assert!(captured.contains("arg=src/relative.ts\n"));
        assert!(captured.contains("arg=src/with space.ts\n"));
        assert!(!captured.contains(&format!("arg={}\n", absolute.display())));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_reports_nonzero_scanner_status_and_stderr() {
        let files = vec!["src/changed.ts".to_string()];
        let (got, _) = run_with_stub(&files, 7, "scanner exploded\n");
        assert!(got.errors.iter().any(|error| error.contains("exit")));
        assert!(got
            .errors
            .iter()
            .any(|error| error.contains("scanner exploded")));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_rejects_stub_that_returns_empty_matches_without_scanning_targets() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(
            dir.path().join("src/changed.ts"),
            "export const changed = 1;\n",
        )
        .unwrap();
        write_stub(&dir.path().join("node_modules/.bin"), "[]");

        let files = vec!["src/changed.ts".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());

        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got
            .errors
            .iter()
            .any(|error| error.contains("path participation")));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_rejects_malformed_json_output() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(
            dir.path().join("src/changed.ts"),
            "export const changed = 1;\n",
        )
        .unwrap();
        write_stub(&dir.path().join("node_modules/.bin"), "not-json");

        let files = vec!["src/changed.ts".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());

        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got
            .errors
            .iter()
            .any(|error| error.contains("JSON parse error")));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_reports_successful_scanner_with_no_files_scanned() {
        let files = vec!["src/changed.ts".to_string()];
        let (got, _) = run_with_stub(&files, 0, "No such file or directory\n");
        assert!(got
            .errors
            .iter()
            .any(|error| error.contains("No such file")));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_rejects_mixed_valid_and_unresolvable_source_paths() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(dir.path().join("src/valid.ts"), "export const valid = 1;\n").unwrap();
        write_stub(&dir.path().join("node_modules/.bin"), "[]");
        let files = vec!["src/valid.ts".to_string(), "src/missing.ts".to_string()];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        assert!(got.errors.iter().any(|error| error.contains("missing.ts")));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_rejects_all_unresolvable_source_paths() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        write_stub(&dir.path().join("node_modules/.bin"), "[]");
        let files = vec!["src/missing.ts".to_string(), "src/other.tsx".to_string()];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        assert!(got.errors.iter().any(|error| error.contains("missing.ts")));
        assert!(got.errors.iter().any(|error| error.contains("other.tsx")));
    }

    #[cfg(unix)]
    fn git_ok(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn run_scan_for_deleted_path(args: &[&str]) -> (AstGrepScanResult, String) {
        let _capture_guard = CAPTURE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(
            dir.path().join("src/old.ts"),
            "export const oldValue = 1;\n",
        )
        .unwrap();
        git_ok(dir.path(), &["init"]);
        git_ok(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git_ok(dir.path(), &["config", "user.name", "test"]);
        git_ok(dir.path(), &["add", "src/old.ts"]);
        git_ok(dir.path(), &["commit", "-m", "initial"]);
        git_ok(dir.path(), args);
        let bin_dir = dir.path().join("node_modules/.bin");
        write_observing_stub(&bin_dir, 0, "");
        let capture = dir.path().join("capture.txt");
        let old_capture = std::env::var_os("SLOPGATE_CAPTURE");
        std::env::set_var("SLOPGATE_CAPTURE", &capture);
        let files = vec!["src/old.ts".to_string()];
        let got = run_ast_grep_scan(
            &config,
            Some(&files),
            &AstGrepScanOpts {
                raw_targets: true,
                path_env: None,
            },
        );
        match old_capture {
            Some(value) => std::env::set_var("SLOPGATE_CAPTURE", value),
            None => std::env::remove_var("SLOPGATE_CAPTURE"),
        }
        (got, fs::read_to_string(capture).unwrap_or_default())
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_excludes_intentionally_deleted_paths() {
        let (got, captured) = run_scan_for_deleted_path(&["rm", "src/old.ts"]);
        assert!(got.errors.is_empty(), "unexpected errors: {:?}", got.errors);
        assert!(!captured.contains("arg=src/old.ts"));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_excludes_old_side_of_staged_rename() {
        let (got, captured) = run_scan_for_deleted_path(&["mv", "src/old.ts", "src/new.ts"]);
        assert!(got.errors.is_empty(), "unexpected errors: {:?}", got.errors);
        assert!(!captured.contains("arg=src/old.ts"));
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_coverage_uses_actual_scan_and_accepts_comment_only_files() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        fs::write(
            dir.path().join("src/comment.ts"),
            "\u{feff} // comment\n/* block */\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/source.ts"),
            "export const source = 1;\n",
        )
        .unwrap();
        write_canary_stub(&dir.path().join("node_modules/.bin"), Some("comment.ts"));
        let files = vec!["src/comment.ts".to_string(), "src/source.ts".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.errors.is_empty(), "errors: {:?}", got.errors);
        assert!(got.violations.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_scan_rejects_partial_target_coverage() {
        let dir = TempDir::new().unwrap();
        let config = ast_config(&dir);
        for file in ["first.ts", "second.ts"] {
            fs::write(
                dir.path().join("src").join(file),
                "export const value = 1;\n",
            )
            .unwrap();
        }
        write_canary_stub(&dir.path().join("node_modules/.bin"), Some("second.ts"));
        let files = vec!["src/first.ts".to_string(), "src/second.ts".to_string()];
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.errors.iter().any(|error| error.contains("second.ts")));
    }

    #[test]
    fn run_scan_test_files_are_scanned() {
        let dir = TempDir::new().unwrap();
        let rule_dir = dir.path().join("rules/ast");
        fs::create_dir_all(&rule_dir).unwrap();
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        write_canary_stub(&bin_dir, None);

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![rule_dir.to_string_lossy().into_owned()],
            checkers: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 1,
            gate: crate::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };

        let files = vec![
            "src/example.test.ts".to_string(),
            "src/example.test.tsx".to_string(),
        ];
        fs::write(
            dir.path().join("src/example.test.ts"),
            "export const ts = 1;\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/example.test.tsx"),
            "export const tsx = 1;\n",
        )
        .unwrap();
        let got = run_ast_grep_scan(&config, Some(&files), &AstGrepScanOpts::default());
        assert!(got.available);
        assert!(got.violations.is_empty());
        assert!(got.errors.is_empty(), "errors: {:?}", got.errors);
    }
}
