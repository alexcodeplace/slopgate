//! Checker-specific binary resolution plus neutral runtime utilities.
use serde_json::Value;
pub use slopgate_core::checkers::shared::*;
use std::path::{Path, PathBuf};

/// Resolve `node_modules/.bin/<name>` under `repo_root` when present.
pub fn local_bin(repo_root: &Path, name: &str) -> Option<PathBuf> {
    let directory = repo_root.join("node_modules").join(".bin");
    #[cfg(windows)]
    for suffix in [".exe", ".cmd"] {
        let candidate = directory.join(format!("{name}{suffix}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let candidate = directory.join(name);
    candidate.is_file().then_some(candidate)
}

/// Resolve a configured/local/PATH tool binary.
///
/// `cfg.bin` is authoritative when present. Bare names are resolved via PATH; paths
/// with separators are resolved relative to the repo root unless already absolute.
pub fn resolve_tool_bin(
    repo_root: &Path,
    cfg: &Value,
    name: &str,
    probe_args: &[&str],
) -> Option<PathBuf> {
    if let Some(bin) = cfg
        .get("bin")
        .and_then(|b| b.as_str())
        .filter(|s| !s.is_empty())
    {
        let candidate = if bin.contains('/') || bin.contains('\\') {
            let p = Path::new(bin);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                repo_root.join(p)
            }
        } else {
            PathBuf::from(bin)
        };
        return command_available(&candidate, probe_args, Some(repo_root)).then_some(candidate);
    }

    let mut candidates = Vec::new();
    if let Some(local) = local_bin(repo_root, name) {
        candidates.push(local);
    }
    candidates.push(repo_root.join(".slopgate").join("bin").join(name));
    candidates.push(repo_root.join("bin").join(name));
    candidates.push(PathBuf::from(name));

    candidates
        .into_iter()
        .find(|bin| command_available(bin, probe_args, Some(repo_root)))
}

/// Rust canonicalization returns namespace-prefixed Windows paths. Node's
/// CommonJS entrypoint loader cannot consume some of these forms (nodejs/node
/// #60435). Simplify only representations that `dunce` proves equivalent;
/// reserved device names, trailing dots/spaces and unsupported namespace forms
/// must not be redirected to a different filesystem object merely to run Node.
#[cfg(windows)]
fn node_path_argument(value: &str) -> Result<String, String> {
    use std::path::{Component, Prefix};
    let path = Path::new(value);
    let is_verbatim = |path: &Path| {
        matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)))
    };
    if !is_verbatim(path) {
        return Ok(value.to_string());
    }
    let simplified = dunce::simplified(path);
    if is_verbatim(simplified) {
        return Err("Node adapter path requires a Windows namespace that cannot be safely simplified; use a compatible project location or a native adapter".into());
    }
    simplified
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| "Node adapter path is not representable as UTF-8".into())
}

/// Keep Windows npm command shims out of the process boundary. Their reviewed
/// package metadata identifies a JavaScript entrypoint, executed with Node and
/// an argument array, never by interpreting `.cmd` text or concatenating a shell.
#[cfg(windows)]
fn node_invocation(bin: &Path, args: &[&str]) -> Result<(PathBuf, Vec<String>), String> {
    let mut arguments: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    if !bin
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        return Ok((bin.to_path_buf(), arguments));
    }
    let name = bin
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("invalid npm shim name")?;
    let package = match name {
        "tsc" => "typescript", "knip" => "knip", "jscpd" => "jscpd",
        "depcruise" | "dependency-cruise" => "dependency-cruiser", "type-coverage" => "type-coverage",
        _ => return Err(format!("unsupported command shim {name}; configure a native executable or explicit interpreter")),
    };
    let node_modules = bin
        .parent()
        .and_then(Path::parent)
        .ok_or("npm shim is not under node_modules/.bin")?;
    let directory = node_modules
        .join(package)
        .canonicalize()
        .map_err(|error| format!("npm package {package}: {error}"))?;
    let metadata: Value = serde_json::from_str(
        &std::fs::read_to_string(directory.join("package.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let entry = metadata
        .get("bin")
        .and_then(|bin| {
            bin.as_str()
                .or_else(|| bin.get(name).and_then(Value::as_str))
        })
        .ok_or("npm package does not declare the requested executable")?;
    let target = directory
        .join(entry)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !target.is_file() || !target.starts_with(&directory) {
        return Err("npm executable escapes its declared package".into());
    }
    // Canonical containment is checked above before changing representation.
    // This adapter owns these arguments: they are compiler options and paths,
    // not user-provided shell fragments or arbitrary command text.
    arguments = arguments
        .iter()
        .map(|argument| node_path_argument(argument))
        .collect::<Result<Vec<_>, _>>()?;
    let target = target
        .to_str()
        .ok_or("npm entrypoint is not representable as UTF-8")?;
    arguments.insert(0, node_path_argument(target)?);
    Ok((PathBuf::from("node"), arguments))
}

pub fn run_tool(bin: &Path, args: &[&str], cwd: Option<&Path>, timeout_ms: Option<u64>) -> ToolOut {
    #[cfg(windows)]
    {
        let (binary, arguments) = match node_invocation(bin, args) {
            Ok(invocation) => invocation,
            Err(error) => {
                return ToolOut {
                    ok: false,
                    error: Some(error),
                    stdout: String::new(),
                    stderr: String::new(),
                    status: None,
                }
            }
        };
        let refs: Vec<&str> = arguments.iter().map(String::as_str).collect();
        slopgate_core::process::run_tool(&binary, &refs, cwd, timeout_ms)
    }
    #[cfg(not(windows))]
    slopgate_core::process::run_tool(bin, args, cwd, timeout_ms)
}

pub fn run_json_tool(
    label: &str,
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: Option<u64>,
) -> JsonToolResult {
    #[cfg(windows)]
    {
        let (binary, arguments) = match node_invocation(bin, args) {
            Ok(invocation) => invocation,
            Err(error) => {
                return JsonToolResult {
                    status: None,
                    data: None,
                    errors: vec![error],
                    warnings: vec![],
                }
            }
        };
        let refs: Vec<&str> = arguments.iter().map(String::as_str).collect();
        slopgate_core::checkers::shared::run_json_tool(label, &binary, &refs, cwd, timeout_ms)
    }
    #[cfg(not(windows))]
    slopgate_core::checkers::shared::run_json_tool(label, bin, args, cwd, timeout_ms)
}

pub fn command_available(bin: &Path, args: &[&str], cwd: Option<&Path>) -> bool {
    let result = run_tool(bin, args, cwd, Some(2000));
    result.ok && result.status == Some(0)
}

#[cfg(test)]
mod local_resolution_tests {
    use super::*;
    #[test]
    fn absent_local_binary_is_not_available() {
        let root = tempfile::tempdir().unwrap();
        assert!(local_bin(root.path(), "missing-tool").is_none());
    }
    #[cfg(unix)]
    #[test]
    fn local_binary_wins_without_global_installation() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("node_modules/.bin/checker");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "fixture").unwrap();
        assert_eq!(local_bin(root.path(), "checker"), Some(binary));
    }
    #[cfg(windows)]
    #[test]
    fn node_shim_uses_package_entrypoint_not_shell_text() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("node_modules/typescript");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), r#"{"bin":{"tsc":"bin/tsc"}}"#).unwrap();
        std::fs::write(package.join("bin/tsc"), "entry").unwrap();
        let shim = root.path().join("node_modules/.bin/tsc.cmd");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&shim, "this shell text must never execute").unwrap();
        let (executable, args) = node_invocation(&shim, &["--project", "a & b.json"]).unwrap();
        assert_eq!(executable, PathBuf::from("node"));
        assert_eq!(&args[1..], &["--project", "a & b.json"]);
        assert!(args[0].ends_with("tsc"));
        assert!(!args[0].starts_with(r"\\?\"));
        assert_eq!(
            Path::new(&args[0]).canonicalize().unwrap(),
            package.join("bin/tsc").canonicalize().unwrap()
        );
    }
    #[cfg(windows)]
    #[test]
    fn node_paths_preserve_argument_boundaries_and_refuse_ambiguous_rewrites() {
        assert_eq!(
            node_path_argument(r"\\?\C:\project\file.ts").unwrap(),
            r"C:\project\file.ts"
        );
        assert_eq!(
            node_path_argument(r"\\?\C:\project\שלום & space.ts").unwrap(),
            r"C:\project\שלום & space.ts"
        );
        assert_eq!(node_path_argument("--showConfig").unwrap(), "--showConfig");
        assert_eq!(node_path_argument("a & b.json").unwrap(), "a & b.json");
        for path in [
            r"\\?\C:\project\NUL",
            r"\\?\C:\project\trailing. ",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
        ] {
            assert!(
                node_path_argument(path).is_err(),
                "unsafe namespace rewrite: {path}"
            );
        }
    }
}
