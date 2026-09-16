//! Versioned, language-independent executable adapter protocol (SG-PROTO-001).
//! Executable adapters are trusted project tooling, not sandboxed by this API.
use crate::checkers::index::{CheckerRunResult, CheckerScope};
use crate::config::ResolvedConfig;
use crate::process::{run_tool_with_input, DEFAULT_OUTPUT_LIMIT};
use crate::report::Violation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_DIAGNOSTICS: usize = 10_000;
const MAX_NORMALIZED_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SOURCE_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AdapterTier {
    Fast,
    #[default]
    Commit,
}

fn default_required() -> bool {
    true
}
fn default_timeout() -> u64 {
    120_000
}
fn default_cap() -> usize {
    DEFAULT_OUTPUT_LIMIT
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalAdapterConfig {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub scope: CheckerScope,
    #[serde(default)]
    pub tier: AdapterTier,
    #[serde(default = "default_required")]
    pub required: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_cap")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub settings: Value,
}

impl ExternalAdapterConfig {
    pub fn validate(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!("invalid adapter ID: {id:?}"));
        }
        if self.executable.trim().is_empty()
            || self.executable.contains('\0')
            || self.args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(format!(
                "adapter {id}: executable/arguments must be nonempty and NUL-free"
            ));
        }
        if self.timeout_ms == 0
            || self.timeout_ms > 3_600_000
            || self.max_output_bytes == 0
            || self.max_output_bytes > 256 * 1024 * 1024
        {
            return Err(format!("adapter {id}: invalid timeout/output bound"));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterRequest<'a> {
    pub protocol_version: u32,
    pub adapter_id: &'a str,
    pub repo_root: &'a str,
    pub scope: CheckerScope,
    /// null explicitly requests the whole declared project/repository.
    pub files: Option<&'a [String]>,
    pub mode: &'a str,
    pub tier: AdapterTier,
    pub settings: &'a Value,
    pub max_concurrency: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdapterResponse {
    protocol_version: u32,
    status: String,
    violations: Vec<Violation>,
    errors: Vec<String>,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:./@".contains(&c))
}

pub fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\0', '\\', ':'])
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn execution_error(message: impl ToString) -> CheckerRunResult {
    CheckerRunResult {
        violations: vec![],
        errors: vec![message.to_string()],
        warnings: vec![],
    }
}

pub fn run_adapter(
    id: &str,
    adapter: &ExternalAdapterConfig,
    config: &ResolvedConfig,
    files: &[String],
    mode: &str,
    tier: AdapterTier,
) -> CheckerRunResult {
    let request = AdapterRequest {
        protocol_version: PROTOCOL_VERSION,
        adapter_id: id,
        repo_root: &config.repo_root,
        scope: adapter.scope,
        files: if adapter.scope == CheckerScope::Files {
            Some(files)
        } else {
            None
        },
        mode,
        tier,
        settings: &adapter.settings,
        max_concurrency: config.checker_concurrency,
    };
    let input = match serde_json::to_vec(&request) {
        Ok(input) => input,
        Err(error) => return execution_error(error),
    };
    let executable = if Path::new(&adapter.executable).is_absolute() {
        PathBuf::from(&adapter.executable)
    } else if adapter.executable.contains('/') || adapter.executable.contains('\\') {
        Path::new(&config.repo_root).join(&adapter.executable)
    } else {
        PathBuf::from(&adapter.executable)
    };
    let args: Vec<&str> = adapter.args.iter().map(String::as_str).collect();
    let output = run_tool_with_input(
        &executable,
        &args,
        Some(Path::new(&config.repo_root)),
        Some(if tier == AdapterTier::Fast {
            adapter.timeout_ms.min(5_000)
        } else {
            adapter.timeout_ms
        }),
        Some(&input),
        adapter.max_output_bytes,
    );
    if !output.ok {
        return execution_error(format!(
            "adapter {id}: {}",
            output.error.as_deref().unwrap_or("incomplete execution")
        ));
    }
    match parse_response(
        id,
        &output.stdout,
        output.status,
        Path::new(&config.repo_root),
    ) {
        Ok(violations) => CheckerRunResult {
            violations,
            errors: vec![],
            warnings: vec![],
        },
        Err(error) => execution_error(error),
    }
}

pub fn parse_response(
    id: &str,
    stdout: &str,
    exit: Option<i32>,
    repo: &Path,
) -> Result<Vec<Violation>, String> {
    let mut response: AdapterResponse = serde_json::from_str(stdout)
        .map_err(|error| format!("adapter {id}: invalid protocol JSON: {error}"))?;
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "adapter {id}: unsupported protocol version {}",
            response.protocol_version
        ));
    }
    if response.status != "complete" {
        return Err(format!(
            "adapter {id}: incomplete status {:?}: {}",
            response.status,
            response.errors.join("; ")
        ));
    }
    if exit != Some(0) || !response.errors.is_empty() {
        return Err(format!(
            "adapter {id}: complete response contradicts exit/status or errors"
        ));
    }
    if response.violations.len() > MAX_DIAGNOSTICS {
        return Err(format!(
            "adapter {id}: diagnostic count exceeds {MAX_DIAGNOSTICS}"
        ));
    }
    let mut sources: HashMap<String, String> = HashMap::new();
    let mut cached_bytes = 0usize;
    let mut normalized_bytes = 0usize;
    let root = repo
        .canonicalize()
        .map_err(|error| format!("adapter {id}: repository cannot be resolved: {error}"))?;
    for violation in &mut response.violations {
        if !valid_id(&violation.id)
            || !matches!(
                violation.severity.as_str(),
                "critical" | "high" | "medium" | "low" | "info"
            )
            || violation.line == 0
            || !valid_relative_path(&violation.file)
        {
            return Err(format!(
                "adapter {id}: invalid finding ID, severity, path or location"
            ));
        }
        if !sources.contains_key(&violation.file) {
            let file = root.join(&violation.file);
            let canonical = file.canonicalize().map_err(|error| {
                format!("adapter {id}: finding file {}: {error}", violation.file)
            })?;
            if !canonical.starts_with(&root) {
                return Err(format!(
                    "adapter {id}: finding escapes repository through a symlink"
                ));
            }
            // A checker may name a path that was not selected for source scanning.
            // Refuse FIFOs, devices and directories before opening: an otherwise
            // valid response must not block indefinitely on a named pipe.
            if !canonical.is_file() {
                return Err(format!(
                    "adapter {id}: finding source must be a regular file"
                ));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(file)
                .map_err(|error| error.to_string())?
                .take(crate::regex_engine::MAX_SOURCE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > crate::regex_engine::MAX_SOURCE_BYTES {
                return Err(format!(
                    "adapter {id}: finding source exceeds the source size limit"
                ));
            }
            cached_bytes = cached_bytes.saturating_add(bytes.len());
            if cached_bytes > MAX_SOURCE_CACHE_BYTES {
                return Err(format!("adapter {id}: finding source cache exceeds 64 MiB"));
            }
            sources.insert(
                violation.file.clone(),
                String::from_utf8(bytes)
                    .map_err(|error| format!("adapter {id}: non-UTF-8 finding source: {error}"))?,
            );
        }
        let content = &sources[&violation.file];
        let line = if content.is_empty() && violation.line == 1 {
            Some("")
        } else {
            content.lines().nth(violation.line as usize - 1)
        };
        let line =
            line.ok_or_else(|| format!("adapter {id}: finding location is outside source"))?;
        normalized_bytes = normalized_bytes.saturating_add(line.len());
        if normalized_bytes > MAX_NORMALIZED_SOURCE_BYTES {
            return Err(format!(
                "adapter {id}: normalized source excerpts exceed 16 MiB"
            ));
        }
        violation.full_line = line.to_string();
        violation.engine = format!("adapter:{id}");
    }
    response
        .violations
        .sort_by(|a, b| (&a.file, a.line, &a.id, &a.text).cmp(&(&b.file, b.line, &b.id, &b.text)));
    Ok(response.violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_ids_are_unambiguous() {
        for bad in ["../a", "/a", "C:/a", "a/../b", "a\\b", "a//b", "./a", ""] {
            assert!(!valid_relative_path(bad), "{bad}");
        }
        assert!(valid_relative_path("src/שלום.rs"));
        assert!(valid_id("team/no-slop"));
        assert!(!valid_id("rule\nforged"));
    }
    #[test]
    fn response_validates_version_exit_schema_and_findings() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("sample.py"), "print('bad')\n").unwrap();
        let value = serde_json::json!({"protocolVersion":1,"status":"complete","violations":[{"id":"policy/no-print","severity":"high","category":"policy","file":"sample.py","line":1,"text":"bad","resolution":"remove print","engine":"forged"}],"errors":[]});
        let out = parse_response("python", &value.to_string(), Some(0), root.path()).unwrap();
        assert_eq!(out[0].engine, "adapter:python");
        assert_eq!(out[0].full_line, "print('bad')");
        assert!(parse_response("python", &value.to_string(), Some(1), root.path()).is_err());
        for field in ["protocolVersion", "status", "violations", "errors"] {
            let mut invalid = value.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(parse_response("python", &invalid.to_string(), Some(0), root.path()).is_err());
        }
        for bad in [
            "{}",
            "[]",
            "",
            "not json",
            r#"{"protocolVersion":2,"status":"complete","violations":[],"errors":[]}"#,
        ] {
            assert!(parse_response("python", bad, Some(0), root.path()).is_err());
        }
    }
}
