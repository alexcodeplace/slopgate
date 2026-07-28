//! tsc --noEmit adapter — mirrors `src/checkers/tsc.mjs`.

use crate::checkers::index::{CheckerRunResult, DetectResult};
use crate::checkers::shared::{
    emit_stage_progress, ensure_cache_dir, local_bin, map_limit, run_tool, source_line,
    truncate_chars,
};
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let probe = Command::new("tsc").arg("--version").output().ok()?;
    if probe.status.success() {
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
    for rel in tsconfig_list(cfg) {
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

pub fn run(
    config: &ResolvedConfig,
    cfg: &Value,
    _opts: crate::checkers::index::CheckerRunOpts<'_>,
) -> CheckerRunResult {
    let repo = Path::new(&config.repo_root);
    let Some(resolved) = resolve_tsc_bin(repo) else {
        return CheckerRunResult {
            violations: vec![],
            errors: vec![],
        };
    };

    let mut violations = Vec::new();
    let mut errors = Vec::new();
    if resolved.source == "path" {
        errors.push(
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
    let slug_re = regex::Regex::new(r"[^\w.-]+").unwrap();

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
            if incremental {
                if let Some(ref cache) = cache_dir {
                    let slug = slug_re.replace_all(&rel, "_");
                    let tsbuildinfo = cache.join(format!("tsc-{slug}.tsbuildinfo"));
                    all_args.push("--incremental".into());
                    all_args.push("--tsBuildInfoFile".into());
                    all_args.push(tsbuildinfo.to_string_lossy().into_owned());
                }
            }
            (rel, all_args)
        })
        .collect();
    let results = map_limit(
        &invocations,
        config.checker_concurrency as usize,
        |(rel, all_args)| {
            let started = std::time::Instant::now();
            let stage = format!("tsc:{rel}");
            emit_stage_progress(&stage, "start", None);
            let arg_refs: Vec<&str> = all_args.iter().map(String::as_str).collect();
            let result = run_tool(&resolved.bin, &arg_refs, Some(repo), Some(timeout_ms));
            emit_stage_progress(&stage, "end", Some(started.elapsed().as_millis()));
            result
        },
    );

    for ((rel, _), res) in invocations.into_iter().zip(results) {
        if !res.ok && res.status.is_none() {
            errors.push(format!(
                "tsc({rel}) failed: {}",
                res.error.unwrap_or_else(|| "spawn failed".to_string())
            ));
            continue;
        }
        for ce in parse_tsc_config_errors(&res.stdout) {
            errors.push(format!("tsc({rel}) failed: {ce}"));
        }
        for e in parse_tsc_output(&res.stdout) {
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

    CheckerRunResult { violations, errors }
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
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 3,
            gate: crate::config::GateAllow {
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
            ast_disable: Default::default(),
            baseline_path: String::new(),
            suppressions_path: String::new(),
            fixtures_dirs: vec![],
            checker_concurrency: 2,
            gate: crate::config::GateAllow {
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
