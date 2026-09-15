//! Bounded analysis-process execution (SG-PROC-001).
//!
//! Anonymous temporary files, rather than pipes, eliminate output/input pipe
//! deadlocks and blocked reader-thread joins. Capture is bounded before decoding.
//! A process group (Unix) or suspended-then-assigned Job Object (Windows) owns
//! descendants. This is lifecycle management, not a sandbox against an adapter
//! that deliberately escapes its process group or an uninterruptible kernel call.

use process_wrap::std::CommandWrap;
#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const CLEANUP_LIMIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOut {
    /// True only when execution and capture completed. Interpret `status`
    /// separately: an ordinary nonzero exit is not necessarily infrastructure.
    pub ok: bool,
    pub error: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub status: Option<i32>,
}

fn failed(error: impl ToString) -> ToolOut {
    ToolOut {
        ok: false,
        error: Some(error.to_string()),
        stdout: String::new(),
        stderr: String::new(),
        status: None,
    }
}

pub fn run_tool(bin: &Path, args: &[&str], cwd: Option<&Path>, timeout_ms: Option<u64>) -> ToolOut {
    run_tool_with_input(bin, args, cwd, timeout_ms, None, DEFAULT_OUTPUT_LIMIT)
}

pub fn run_tool_with_input(
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: Option<u64>,
    input: Option<&[u8]>,
    max_output_bytes: usize,
) -> ToolOut {
    #[cfg(windows)]
    if bin
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
    {
        return failed("command scripts require an explicit interpreter adapter; the core does not evaluate command shims");
    }
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    if timeout_ms == 0
        || timeout_ms > 3_600_000
        || max_output_bytes == 0
        || max_output_bytes > 256 * 1024 * 1024
    {
        return failed(
            "invalid process bounds: timeout must be 1..3600000 ms and output 1..268435456 bytes",
        );
    }
    if input.is_some_and(|bytes| bytes.len() > MAX_INPUT_BYTES) {
        return failed("process input exceeds 8 MiB limit");
    }
    match execute(
        bin,
        args,
        cwd,
        Duration::from_millis(timeout_ms),
        input,
        max_output_bytes,
        &Default::default(),
    ) {
        Ok(result) => result,
        Err(error) => failed(format!("process {}: {error}", bin.display())),
    }
}

fn capture(file: &mut File, cap: usize) -> std::io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(cap as u64).read_to_end(&mut bytes)?;
    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("non-UTF-8 tool output: {error}"),
        )
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
fn leader_exited_without_reaping(id: u32) -> std::io::Result<bool> {
    // SAFETY: siginfo_t is a C POD output object; zero initialization is required
    // to distinguish WNOHANG's no-event result portably. waitid only writes this
    // valid object and WNOWAIT deliberately retains ownership of our child PID.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let code = unsafe {
        libc::waitid(
            libc::P_PID,
            id as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if code != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.si_signo != 0)
}

fn execute(
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
    input: Option<&[u8]>,
    cap: usize,
    environment: &std::collections::BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
) -> std::io::Result<ToolOut> {
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    )))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process-tree isolation is unsupported on this platform",
    ));

    let started = Instant::now();
    let mut stdout_file = tempfile::tempfile()?;
    let mut stderr_file = tempfile::tempfile()?;
    let mut command = Command::new(bin);
    command
        .args(args)
        .stdout(stdout_file.try_clone()?)
        .stderr(stderr_file.try_clone()?);
    // Git hooks can export worktree bookkeeping that would redirect an adapter
    // away from its configured cwd. Preserve GIT_INDEX_FILE: an alternate index
    // can be the exact proposed commit and must be validated as such.
    command.env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for (key, value) in environment {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    if let Some(bytes) = input {
        let mut file = tempfile::tempfile()?;
        file.write_all(bytes)?;
        file.seek(SeekFrom::Start(0))?;
        command.stdin(file);
    } else {
        command.stdin(Stdio::null());
    }
    let mut command = CommandWrap::from(command);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    let mut child = command.spawn()?;
    // Do not retain duplicate descriptors owned by Command after spawning.
    drop(command);
    let mut error = None;
    let mut exit = None;
    #[allow(unused_mut)]
    let mut owns_group = true;
    loop {
        let size = match stdout_file.metadata().and_then(|out| {
            stderr_file
                .metadata()
                .map(|err| out.len().saturating_add(err.len()))
        }) {
            Ok(size) => size,
            Err(reason) => {
                error = Some(format!("capture metadata failed: {reason}"));
                break;
            }
        };
        if size > cap as u64 {
            error = Some(format!("combined process output exceeds {cap} bytes"));
            break;
        }
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
        match leader_exited_without_reaping(child.id()) {
            Ok(true) => break,
            Ok(false) => {}
            Err(reason) if reason.kind() == std::io::ErrorKind::Interrupted => {}
            Err(reason) => {
                // Another process-wide reaper or inherited SIGCHLD policy can
                // invalidate PID ownership. Never signal a possibly reused PID.
                owns_group = reason.raw_os_error() != Some(libc::ECHILD);
                error = Some(format!("child ownership/wait failed: {reason}"));
                break;
            }
        }
        #[cfg(windows)]
        match child.try_wait() {
            Ok(Some(status)) => {
                exit = Some(status);
                break;
            }
            Ok(None) => {}
            Err(reason) => {
                error = Some(format!("wait failed: {reason}"));
                break;
            }
        }
        if started.elapsed() >= timeout {
            error = Some(format!(
                "process timed out after {} ms",
                timeout.as_millis()
            ));
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    // Kill descendants even after their leader exits normally. No adapter may
    // leave a background process holding capture files or consuming CI resources.
    if let Err(reason) = if owns_group {
        child.start_kill()
    } else {
        Ok(())
    } {
        // ESRCH / InvalidInput after an already-reaped group is harmless.
        #[cfg(unix)]
        let gone = reason.raw_os_error() == Some(3);
        #[cfg(not(unix))]
        let gone = reason.kind() == std::io::ErrorKind::InvalidInput;
        if !gone && error.is_none() {
            error = Some(format!("process-tree cleanup failed: {reason}"));
        }
    }
    let cleanup_started = Instant::now();
    while exit.is_none() && cleanup_started.elapsed() < CLEANUP_LIMIT {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit = Some(status);
                break;
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(reason) => {
                error.get_or_insert_with(|| format!("cleanup wait failed: {reason}"));
                break;
            }
        }
    }
    if exit.is_none() {
        error.get_or_insert_with(|| "process did not exit within cleanup deadline".to_string());
    }
    let status = exit.and_then(|value| value.code());
    if status.is_none() {
        error.get_or_insert_with(|| "process terminated without a normal exit code".to_string());
    }
    let size = stdout_file
        .metadata()?
        .len()
        .saturating_add(stderr_file.metadata()?.len());
    if size > cap as u64 {
        error.get_or_insert_with(|| format!("combined process output exceeds {cap} bytes"));
    }
    let stdout = capture(&mut stdout_file, cap)?;
    let remaining = cap.saturating_sub(stdout.len());
    let stderr = capture(&mut stderr_file, remaining)?;
    Ok(ToolOut {
        ok: error.is_none(),
        error,
        stdout,
        stderr,
        status,
    })
}

/// Compatibility builder for short-lived infrastructure operations such as
/// Git metadata and tool probes. It shares the same process-tree/output safety
/// implementation as checker execution; ordinary nonzero exits remain visible.
/// No shell is inserted. Analysis tools with a declared budget use `run_tool`.
#[derive(Debug)]
pub struct BoundedCommand {
    program: std::ffi::OsString,
    args: Vec<std::ffi::OsString>,
    cwd: Option<std::path::PathBuf>,
    env: std::collections::BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
    timeout_ms: u64,
}

impl BoundedCommand {
    pub fn new(program: impl AsRef<std::ffi::OsStr>) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            args: vec![],
            cwd: None,
            env: Default::default(),
            timeout_ms: 10_000,
        }
    }
    pub fn arg(&mut self, arg: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }
    pub fn current_dir(&mut self, cwd: impl AsRef<Path>) -> &mut Self {
        self.cwd = Some(cwd.as_ref().to_path_buf());
        self
    }
    pub fn env(
        &mut self,
        key: impl AsRef<std::ffi::OsStr>,
        value: impl AsRef<std::ffi::OsStr>,
    ) -> &mut Self {
        self.env.insert(
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        );
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.env.insert(key.as_ref().to_os_string(), None);
        self
    }
    pub fn timeout_ms(&mut self, milliseconds: u64) -> &mut Self {
        self.timeout_ms = milliseconds;
        self
    }
    pub fn output(&mut self) -> std::io::Result<std::process::Output> {
        if self.timeout_ms == 0 || self.timeout_ms > 3_600_000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid process timeout",
            ));
        }
        let args: Result<Vec<&str>, _> = self
            .args
            .iter()
            .map(|arg| {
                arg.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "non-UTF-8 infrastructure argument",
                    )
                })
            })
            .collect();
        let output = execute(
            Path::new(&self.program),
            &args?,
            self.cwd.as_deref(),
            Duration::from_millis(self.timeout_ms),
            None,
            DEFAULT_OUTPUT_LIMIT,
            &self.env,
        )?;
        if !output.ok {
            return Err(std::io::Error::other(
                output
                    .error
                    .unwrap_or_else(|| "incomplete process execution".into()),
            ));
        }
        let status = output
            .status
            .ok_or_else(|| std::io::Error::other("process did not provide an exit status"))?;
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(status << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(status as u32)
        };
        Ok(std::process::Output {
            status,
            stdout: output.stdout.into_bytes(),
            stderr: output.stderr.into_bytes(),
        })
    }
    pub fn status(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.output().map(|output| output.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn shell(script: &str, timeout: u64, cap: usize) -> ToolOut {
        run_tool_with_input(
            Path::new("sh"),
            &["-c", script],
            None,
            Some(timeout),
            None,
            cap,
        )
    }

    #[test]
    fn bounds_and_missing_executable_fail() {
        assert!(
            !run_tool(
                Path::new("slopgate-no-such-executable-001"),
                &[],
                None,
                Some(100)
            )
            .ok
        );
        assert!(!run_tool(Path::new("anything"), &[], None, Some(0)).ok);
        assert!(!run_tool_with_input(Path::new("anything"), &[], None, Some(1), None, 0).ok);
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_exit_and_both_streams_are_preserved() {
        let result = shell("printf out; printf err >&2; exit 7", 1000, 1024);
        assert!(result.ok, "{result:?}");
        assert_eq!(result.status, Some(7));
        assert_eq!(result.stdout, "out");
        assert_eq!(result.stderr, "err");
    }

    #[cfg(unix)]
    #[test]
    fn protocol_input_and_null_stdin_never_deadlock() {
        let result = run_tool_with_input(
            Path::new("cat"),
            &[],
            None,
            Some(1000),
            Some(b"protocol input\n"),
            1024,
        );
        assert!(result.ok, "{result:?}");
        assert_eq!(result.stdout, "protocol input\n");
        assert_eq!(run_tool(Path::new("cat"), &[], None, Some(1000)).stdout, "");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_and_combined_output_are_bounded() {
        let start = Instant::now();
        let result = shell("sleep 10", 40, 1024);
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
        let result = shell(
            "while :; do printf '12345678901234567890'; printf '12345678901234567890' >&2; done",
            2000,
            2048,
        );
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("output exceeds"));
        assert!(result.stdout.len() + result.stderr.len() <= 2048);
    }

    #[cfg(unix)]
    #[test]
    fn exited_parent_with_background_descendant_cannot_hang_capture() {
        let start = Instant::now();
        let result = shell("sleep 15 & printf finished", 1000, 1024);
        assert!(result.ok, "{result:?}");
        assert_eq!(result.stdout, "finished");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_captures_and_times_out() {
        let result = run_tool(
            Path::new("cmd.exe"),
            &["/d", "/c", "echo hello"],
            None,
            Some(5000),
        );
        assert!(result.ok, "{result:?}");
        assert!(result.stdout.contains("hello"));
        let start = Instant::now();
        let result = run_tool(
            Path::new("powershell.exe"),
            &["-NoProfile", "-Command", "Start-Sleep -Seconds 30"],
            None,
            Some(100),
        );
        assert!(!result.ok, "{result:?}");
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
