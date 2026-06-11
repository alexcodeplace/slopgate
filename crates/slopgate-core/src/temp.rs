//! Temp directory helper. Mirrors `src/temp.mjs` `withTempDir`.

use std::path::Path;

/// Make a temp dir, pass its path to `f`, then remove it (even if `f` panics).
/// Returns `f`'s result.
pub fn with_temp_dir<T, F>(prefix: &str, f: F) -> T
where
    F: FnOnce(&Path) -> T,
{
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("failed to create temp dir");
    f(dir.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn with_temp_dir_writes_file_and_removes_dir() {
        let captured: PathBuf = with_temp_dir("slopgate-test-", |dir| {
            let file = dir.join("probe.txt");
            fs::write(&file, "hello").unwrap();
            assert!(dir.is_dir());
            assert!(file.is_file());
            dir.to_path_buf()
        });
        assert!(!captured.exists(), "temp dir should be removed after return");
    }

    #[test]
    fn with_temp_dir_removes_dir_when_closure_returns_error() {
        let captured: (PathBuf, Result<(), &str>) = with_temp_dir("slopgate-err-", |dir| {
            let path = dir.to_path_buf();
            (path, Err("simulated failure"))
        });
        let (dir_path, result) = captured;
        assert!(result.is_err());
        assert!(!dir_path.exists(), "temp dir should be removed even on error return");
    }
}
