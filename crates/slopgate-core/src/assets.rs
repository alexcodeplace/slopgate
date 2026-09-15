//! Runtime location of shipped engine assets.
use std::path::{Path, PathBuf};

/// Engine root: the directory that ships `hooks/`, `rules/`, and `skills/`.
///
/// Must be resolved at *runtime*, never from `env!("CARGO_MANIFEST_DIR")` — that
/// is a compile-time constant baked into the binary, so a CI-built artifact would
/// carry the CI build path (`/home/runner/work/...`) and write it into user
/// settings on `init`. Resolution order:
///   1. `$SLOPGATE_ENGINE_ROOT`              — explicit override escape hatch
///   2. walk up from `current_exe()`         — find the ancestor containing the
///      shipped `hooks/session-start.sh` marker (robust to `vendor/<host>/` vs
///      `target/release/` layout depth)
///   3. `CARGO_MANIFEST_DIR` + `../..`       — DEBUG-ONLY source-tree fallback for
///      `cargo test`/`cargo run` from a checkout, where no shipped layout exists.
///      NEVER compiled into release builds: it is the compile-time path (a CI
///      runner's `/home/runner/work/...`), and emitting it is the exact poison
///      this resolver exists to prevent. In release a failed walk-up means a
///      corrupt install (shipped `hooks/` missing); we fall back to the exe's own
///      directory — a real local path that fails *visibly* rather than a phantom
///      CI path that looks valid.
pub fn engine_root() -> PathBuf {
    if let Some(root) = std::env::var_os("SLOPGATE_ENGINE_ROOT") {
        return PathBuf::from(root);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.canonicalize().unwrap_or(exe);
        while let Some(parent) = dir.parent() {
            if parent.join("hooks/session-start.sh").is_file() {
                return parent.to_path_buf();
            }
            dir = parent.to_path_buf();
        }
    }
    #[cfg(debug_assertions)]
    {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
    }
    #[cfg(not(debug_assertions))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
