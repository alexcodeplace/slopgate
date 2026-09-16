//! tsc --noEmit adapter — mirrors `src/checkers/tsc.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    emit_stage_progress, ensure_cache_dir, local_bin, map_limit, run_tool, source_line,
    truncate_chars,
};
use serde_json::Value;
use slopgate_core::config::ResolvedConfig;
use slopgate_core::report::Violation;
use std::path::{Path, PathBuf};

pub struct TscBin {
    pub bin: PathBuf,
    pub source: &'static str,
}

pub fn resolve_tsc_bin(repo_root: &Path) -> Option<TscBin> {
    if let Some(local) = local_bin(repo_root, "tsc") {
        return Some(TscBin {
            bin: local,
            source: "local",
        });
    }
    let probe = run_tool(
        Path::new("tsc"),
        &["--version"],
        Some(repo_root),
        Some(2_000),
    );
    if probe.ok && probe.status == Some(0) {
        return Some(TscBin {
            bin: PathBuf::from("tsc"),
            source: "path",
        });
    }
    None
}

fn tsconfig_list(cfg: &Value) -> Vec<String> {
    match cfg.get("tsconfig") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(_) | None => vec!["tsconfig.json".to_string()],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TscError {
    pub file: String,
    pub line: u32,
    pub code: String,
    pub message: String,
}

pub fn parse_tsc_output(stdout: &str) -> Vec<TscError> {
    let re = regex::Regex::new(r"^(.+?)\((\d+),(\d+)\): error (TS\d+): (.*)$").unwrap();
    let cont_re = regex::Regex::new(r"^\s+\S").unwrap();
    let mut errors = Vec::new();
    for raw in stdout.lines() {
        if let Some(caps) = re.captures(raw) {
            errors.push(TscError {
                file: caps[1].replace('\\', "/"),
                line: caps[2].parse().unwrap_or(1),
                code: caps[4].to_string(),
                message: caps[5].to_string(),
            });
        } else if !errors.is_empty() && cont_re.is_match(raw) {
            let last = errors.len() - 1;
            if !errors[last].message.is_empty() {
                errors[last].message.push(' ');
            }
            errors[last].message.push_str(raw.trim());
        }
    }
    errors
}

pub fn parse_tsc_config_errors(stdout: &str) -> Vec<String> {
    let re = regex::Regex::new(r"^error (TS\d+): (.*)$").unwrap();
    stdout
        .lines()
        .filter_map(|raw| re.captures(raw).map(|c| format!("{}: {}", &c[1], &c[2])))
        .collect()
}

pub fn detect(config: &ResolvedConfig, cfg: &Value) -> DetectResult {
    let repo = Path::new(&config.repo_root);
    for key in ["incremental", "build"] {
        if cfg.get(key).is_some_and(|value| !value.is_boolean()) {
            return DetectResult {
                available: false,
                reason: Some(format!("tsc {key} must be boolean")),
            };
        }
    }
    let configurations = tsconfig_list(cfg);
    if configurations.is_empty()
        || cfg
            .get("tsconfig")
            .is_some_and(|value| !value.is_string() && !value.is_array())
        || cfg
            .get("tsconfig")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str().is_none_or(str::is_empty))
            })
    {
        return DetectResult {
            available: false,
            reason: Some("tsconfig must be a nonempty path or array of nonempty paths".into()),
        };
    }
    for rel in configurations {
        if !repo.join(&rel).exists() {
            return DetectResult {
                available: false,
                reason: Some(format!("no {rel}")),
            };
        }
    }
    if resolve_tsc_bin(repo).is_none() {
        return DetectResult {
            available: false,
            reason: Some("no tsc binary (local or PATH)".to_string()),
        };
    }
    DetectResult {
        available: true,
        reason: None,
    }
}

/// Ask the selected compiler to resolve JSONC, extends and file inclusion.
/// A solution configuration cannot silently pass `tsc -p` without its references
/// being checked; require the explicit reference-aware build mode instead.
fn validate_project_scope(
    binary: &Path,
    repo: &Path,
    relative: &str,
    build_mode: bool,
    timeout_ms: u64,
) -> Result<(), String> {
    let project = repo.join(relative);
    let project = project.to_string_lossy();
    let output = run_tool(
        binary,
        &["--showConfig", "--project", &project],
        Some(repo),
        Some(timeout_ms),
    );
    if !output.ok || output.status != Some(0) || !output.stderr.trim().is_empty() {
        return Err(format!(
            "cannot resolve TypeScript project scope: {} {} {}",
            output.error.unwrap_or_default(),
            output.stdout.chars().take(800).collect::<String>(),
            output.stderr.chars().take(400).collect::<String>()
        ));
    }
    let resolved: Value = serde_json::from_str(&output.stdout)
        .map_err(|error| format!("tsc --showConfig did not return valid JSON: {error}"))?;
    let references = resolved
        .get("references")
        .and_then(Value::as_array)
        .is_some_and(|references| !references.is_empty());
    if references && !build_mode {
        return Err("project references require [checkers.tsc] build=true; a plain -p check does not validate referenced projects".into());
    }
    let files = resolved
        .get("files")
        .and_then(Value::as_array)
        .is_some_and(|files| !files.is_empty() && files.iter().all(Value::is_string));
    if !(files || build_mode && references) {
        return Err(
            "required TypeScript project has no resolvable source files or referenced projects"
                .into(),
        );
    }
    Ok(())
}

pub fn run(
    config: &ResolvedConfig,
    cfg: &Value,
    _opts: crate::checkers::index::CheckerRunOpts<'_>,
) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(resolved) = resolve_tsc_bin(repo) else {
        return CheckerRunResult {
            warnings: vec![],
            violations: vec![],
            errors: vec!["configured checker executable disappeared before execution".into()],
        };
    };

    let mut violations = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if resolved.source == "path" {
        warnings.push(
            "tsc: using PATH binary (version not pinned — results may differ from CI)".to_string(),
        );
    }

    let timeout_ms = cfg
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .saturating_mul(1000);
    let incremental = cfg
        .get("incremental")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let cache_dir = ensure_cache_dir(Path::new(&config.config_dir)).ok();
    use sha2::{Digest, Sha256};

    let build_mode = cfg.get("build").and_then(Value::as_bool).unwrap_or(false);
    let invocations: Vec<(String, Vec<String>)> = tsconfig_list(cfg)
        .into_iter()
        .map(|rel| {
            let mut all_args: Vec<String> = vec![
                "--noEmit".into(),
                "--pretty".into(),
                "false".into(),
                "-p".into(),
                repo.join(&rel).to_string_lossy().into_owned(),
            ];
            if build_mode {
                all_args = vec![
                    "--build".into(),
                    repo.join(&rel).to_string_lossy().into_owned(),
                    "--pretty".into(),
                    "false".into(),
                ];
            }
            if incremental && !build_mode {
                if let Some(ref cache) = cache_dir {
                    let slug = format!("{:x}", Sha256::digest(rel.as_bytes()));
                    let tsbuildinfo = cache.join(format!("tsc-{slug}.tsbuildinfo"));
                    all_args.push("--incremental".into());
                    all_args.push("--tsBuildInfoFile".into());
                    all_args.push(tsbuildinfo.to_string_lossy().into_owned());
                }
            }
            (rel, all_args)
        })
        .collect();
    let adapter_started = std::time::Instant::now();
    let results = map_limit(
        &invocations,
        if build_mode {
            1
        } else {
            config.checker_concurrency as usize
        },
        |(rel, all_args)| {
            let started = std::time::Instant::now();
            let stage = format!("tsc:{rel}");
            emit_stage_progress(&stage, "start", None);
            let remaining = || {
                timeout_ms.saturating_sub(
                    adapter_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                )
            };
            let checked = validate_project_scope(&resolved.bin, repo, rel, build_mode, remaining());
            if let Err(error) = checked {
                return crate::checkers::shared::ToolOut {
                    ok: false,
                    error: Some(error),
                    stdout: String::new(),
                    stderr: String::new(),
                    status: None,
                };
            }
            let arg_refs: Vec<&str> = all_args.iter().map(String::as_str).collect();
            let result = run_tool(&resolved.bin, &arg_refs, Some(repo), Some(remaining()));
            emit_stage_progress(&stage, "end", Some(started.elapsed().as_millis()));
            result
        },
    );

    for ((rel, _), res) in invocations.into_iter().zip(results) {
        if !res.ok {
            errors.push(format!(
                "tsc({rel}) failed: {}",
                res.error.unwrap_or_else(|| "spawn failed".to_string())
            ));
            continue;
        }
        let parsed = parse_tsc_output(&res.stdout);
        let config_errors = parse_tsc_config_errors(&res.stdout);
        if !res.stderr.trim().is_empty()
            || !matches!(res.status, Some(0..=2))
            || (res.status == Some(0) && (!parsed.is_empty() || !config_errors.is_empty()))
            || (res.status != Some(0) && parsed.is_empty() && config_errors.is_empty())
        {
            errors.push(format!(
                "tsc({rel}) returned an unverifiable exit/output (exit {:?}): {} {}",
                res.status,
                res.stdout.chars().take(400).collect::<String>(),
                res.stderr.chars().take(400).collect::<String>()
            ));
        }
        if res.status == Some(0) && !res.stdout.trim().is_empty() {
            errors.push(format!("tsc({rel}) emitted unexpected output on success"));
        }
        for ce in config_errors {
            errors.push(format!("tsc({rel}) failed: {ce}"));
        }
        for e in parsed {
            violations.push(Violation {
                id: format!("tsc-{}", e.code),
                severity: "high".into(),
                category: "types".into(),
                file: e.file.clone(),
                line: e.line,
                full_line: source_line(repo, &e.file, e.line),
                text: truncate_chars(e.message.trim(), 90),
                resolution: "Fix the type error — do not suppress.".into(),
                engine: "checker:tsc".into(),
            });
        }
    }

    CheckerRunResult {
        warnings,
        violations,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn parse_tsc_output_matches_fixture() {
        let stdout = concat!(
            "src/a.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.\n",
            "src/b.tsx(3,1): error TS2304: Cannot find name 'foo'.\n",
            "src/long.ts(7,9): error TS2345: Argument of type '{ a: string; }' is not assignable to parameter of type 'Opts'.\n",
            "  Property 'b' is missing in type '{ a: string; }' but required in type 'Opts'.",
        );
        let got = parse_tsc_output(stdout);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].file, "src/a.ts");
        assert_eq!(got[0].line, 12);
        assert_eq!(got[0].code, "TS2322");
        assert_eq!(
            got[2].message,
            "Argument of type '{ a: string; }' is not assignable to parameter of type 'Opts'. Property 'b' is missing in type '{ a: string; }' but required in type 'Opts'."
        );
    }

    #[test]
    fn parse_tsc_config_errors_finds_fileless_errors() {
        let out = parse_tsc_config_errors("error TS5023: Unknown compiler option 'foo'.\n");
        assert_eq!(
            out,
            vec!["TS5023: Unknown compiler option 'foo'.".to_string()]
        );
    }

    #[test]
    fn detect_false_when_no_tsconfig() {
        let dir = TempDir::new().unwrap();
        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().join(".slopgate").to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![],
            checkers: Default::default(),
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 3,
            gate: slopgate_core::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };
        let det = detect(&config, &serde_json::json!({}));
        assert!(!det.available);
        assert_eq!(det.reason.as_deref(), Some("no tsconfig.json"));
    }

    #[cfg(unix)]
    #[test]
    fn runs_configs_with_bounded_parallelism_and_deterministic_merge() {
        let dir = TempDir::new().unwrap();
        let state = dir.path().join("state");
        let bin_dir = dir.path().join("node_modules/.bin");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(state.join("max"), "0\n").unwrap();

        let configs = ["a.json", "b.json", "c.json", "d.json"];
        for rel in configs {
            fs::write(dir.path().join(rel), "{}\n").unwrap();
        }

        let script = format!(
            r#"#!/bin/sh
case "$*" in *--showConfig*) printf '{{"files":["fixture.ts"]}}\n'; exit 0 ;; esac
config=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-p" ]; then
    shift
    config="$1"
    break
  fi
  shift
done
name=$(basename "$config" .json)
state='{}'
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
touch "$state/active-$name"
active=$(find "$state" -name 'active-*' | wc -l)
max=$(cat "$state/max")
if [ "$active" -gt "$max" ]; then
  printf '%s\n' "$active" > "$state/max"
fi
rmdir "$state/lock"
case "$name" in
  a) sleep 0.30; code=2301 ;;
  b) sleep 0.20; code=2302 ;;
  c) sleep 0.10; code=2303 ;;
  d) sleep 0.05; code=2304 ;;
esac
printf '%s.ts(1,1): error TS%s: finding %s\n' "$name" "$code" "$name"
printf 'error TS5000: config %s\n' "$name"
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
rm "$state/active-$name"
rmdir "$state/lock"
exit 2
"#,
            state.to_string_lossy()
        );
        let bin = bin_dir.join("tsc");
        fs::write(&bin, script).unwrap();
        let mut permissions = fs::metadata(&bin).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&bin, permissions).unwrap();

        let config = ResolvedConfig {
            repo_root: dir.path().to_string_lossy().into_owned(),
            config_dir: dir.path().join(".slopgate").to_string_lossy().into_owned(),
            roots: vec![],
            roots_rel: vec![],
            exts: Default::default(),
            skip_dirs: Default::default(),
            patterns: vec![],
            ast_rule_dirs: vec![],
            checkers: Default::default(),
            external_adapters: Default::default(),
            ast_enabled: true,
            ast_binary: None,
            project_rule_ids: Default::default(),
            project_ast_dirs: Default::default(),
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 2,
            gate: slopgate_core::config::GateAllow {
                file: Default::default(),
                staged: Default::default(),
            },
            ux_ast_severity: Default::default(),
            ux_ast_all: Default::default(),
        };
        let cfg = serde_json::json!({
            "tsconfig": configs,
            "incremental": false,
            "timeout": 5
        });

        let result = run(
            &config,
            &cfg,
            crate::checkers::index::CheckerRunOpts {
                files: None,
                mode: "full",
            },
        );

        assert_eq!(fs::read_to_string(state.join("max")).unwrap().trim(), "2");
        assert_eq!(
            result
                .violations
                .iter()
                .map(|violation| violation.file.as_str())
                .collect::<Vec<_>>(),
            vec!["a.ts", "b.ts", "c.ts", "d.ts"]
        );
        assert_eq!(
            result.errors,
            vec![
                "tsc(a.json) failed: TS5000: config a",
                "tsc(b.json) failed: TS5000: config b",
                "tsc(c.json) failed: TS5000: config c",
                "tsc(d.json) failed: TS5000: config d",
            ]
        );
    }
}
