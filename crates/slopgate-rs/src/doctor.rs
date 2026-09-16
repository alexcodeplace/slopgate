//! CLI/composition-level provenance and capability inspection.
use serde_json::{json, Value};
use slopgate_adapters::checkers::index::CHECKERS;
use slopgate_core::config::ResolvedConfig;
use std::path::{Path, PathBuf};

pub fn capabilities() -> Value {
    json!({
        "schemaVersion":1,
        "engine":{
            "version":env!("CARGO_PKG_VERSION"),
            "revision":env!("SLOPGATE_BUILD_REVISION"),
            "sourceDigest":env!("SLOPGATE_SOURCE_DIGEST"),
            "executable":std::env::current_exe().ok().map(|path|path.to_string_lossy().into_owned()),
            "specification":"universal-gate-v1"
        },
        "sharedEngines":[
            {"id":"regex","analysis":"line-scoped text policy","maxSourceBytes":slopgate_core::regex_engine::MAX_SOURCE_BYTES,"maxLineBytes":slopgate_core::regex_engine::MAX_LINE_BYTES},
            {"id":"ast","analysis":"structural matching; parser/rule coverage is reported by the selected ast-grep executable, not inferred from file discovery"}
        ],
        "builtInAdapters":CHECKERS.iter().map(|checker|json!({"id":checker.id,"scope":checker.scope,"tier":"commit"})).collect::<Vec<_>>(),
        "externalProtocol":{"version":slopgate_core::protocol::PROTOCOL_VERSION,"transport":"bounded JSON stdin/stdout","defaultRequired":true,"defaultScope":"project","sandboxed":false},
        "exitCodes":{"pass":0,"policyFindings":1,"incomplete":2},
        "cache":{"externalResults":false,"toolOwnedIncrementalState":true},
        "daemonRequired":false
    })
}

fn configured_executable_exists(repo: &Path, executable: &str) -> bool {
    let path = Path::new(executable);
    if path.is_absolute() {
        return path.is_file();
    }
    if executable.contains('/') || executable.contains('\\') {
        return repo.join(path).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            directory.join(path).is_file()
                || (cfg!(windows) && directory.join(format!("{executable}.exe")).is_file())
        })
    })
}

pub fn inspect(config: &ResolvedConfig) -> (Value, i32) {
    let mut report = capabilities();
    let mut checks = Vec::new();
    let mut errors = slopgate_core::gate::validate_registry(config, CHECKERS)
        .err()
        .unwrap_or_default();
    for checker in CHECKERS {
        let Some(settings) = config.checkers.get(checker.id) else {
            continue;
        };
        if settings == &Value::Bool(false) {
            checks.push(json!({"id":checker.id,"status":"disabled"}));
            continue;
        }
        let required = settings
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let detected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (checker.detect)(config, settings)
        }));
        match detected {
            Ok(detected) => {
                if required && !detected.available {
                    errors.push(format!(
                        "{}: {}",
                        checker.id,
                        detected.reason.as_deref().unwrap_or("unavailable")
                    ));
                }
                checks.push(json!({"id":checker.id,"required":required,"scope":checker.scope,"status":if detected.available {"available-not-executed"} else {"unavailable"},"reason":detected.reason}));
            }
            Err(_) => {
                errors.push(format!("{}: detection panicked", checker.id));
                checks.push(json!({"id":checker.id,"status":"error"}));
            }
        }
    }
    let repo = Path::new(&config.repo_root);
    if config.ast_enabled {
        let binary = config
            .ast_binary
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| slopgate_core::ast_engine::resolve_ast_grep_bin(repo).0);
        match binary {
            Some(binary) => {
                let version = slopgate_core::process::run_tool(
                    &binary,
                    &["--version"],
                    Some(repo),
                    Some(2000),
                );
                if !version.ok || version.status != Some(0) {
                    errors.push(format!(
                        "AST executable did not pass its version probe: {}",
                        binary.display()
                    ));
                }
                report["astExecutable"] = json!({"path":binary.to_string_lossy(),"version":version.stdout.trim(),"status":"available-not-executed"});
            }
            None => errors.push("required AST executable is unavailable".into()),
        }
    } else {
        report["astExecutable"] = json!({"status":"disabled"});
    }
    for (id, adapter) in &config.external_adapters {
        let available = configured_executable_exists(repo, &adapter.executable);
        if !available && adapter.required {
            errors.push(format!("required adapter {id} executable is unavailable"));
        }
        checks.push(json!({"id":id,"required":adapter.required,"scope":adapter.scope,"tier":adapter.tier,"executable":adapter.executable,"status":if available {"configured-not-executed"}else{"unavailable"}}));
    }
    report["repository"] = json!({"root":config.repo_root,"configurationDirectory":config.config_dir,"roots":config.roots_rel,"extensions":config.exts.iter().collect::<std::collections::BTreeSet<_>>(),"regexRuleCount":config.patterns.len(),"astRuleDirectories":config.ast_rule_dirs,"checkerConcurrency":config.checker_concurrency});
    report["configuredChecks"] = json!(checks);
    report["errors"] = json!(errors);
    let code = if errors.is_empty() { 0 } else { 2 };
    report["exitCode"] = json!(code);
    (report, code)
}
