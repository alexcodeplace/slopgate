//! Temp directory helper. Mirrors `src/temp.mjs` `withTempDir`.

use std::io;
use std::path::Path;

/// Make a temp dir under `base`, pass its path to `f`, then remove it (even if `f` panics).
/// Returns `f`'s result, or an I/O error when tempdir creation fails.
pub fn with_temp_dir_in<T, F>(base: impl AsRef<Path>, prefix: &str, f: F) -> Result<T, io::Error>
where
    F: FnOnce(&Path) -> T,
{
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(base.as_ref())?;
    Ok(f(dir.path()))
}

/// Make a temp dir, pass its path to `f`, then remove it (even if `f` panics).
/// Returns `f`'s result, or an I/O error when tempdir creation fails.
pub fn with_temp_dir<T, F>(prefix: &str, f: F) -> Result<T, io::Error>
where
    F: FnOnce(&Path) -> T,
{
    with_temp_dir_in(std::env::temp_dir(), prefix, f)
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
        })
        .unwrap();
        assert!(!captured.exists(), "temp dir should be removed after return");
    }

    #[test]
    fn with_temp_dir_returns_err_when_creation_fails() {
        let parent = tempfile::TempDir::new().unwrap();
        let not_a_dir = parent.path().join("not-a-dir");
        fs::write(&not_a_dir, "blocking").unwrap();

        let result = with_temp_dir_in(&not_a_dir, "slopgate-fail-", |_| ());
        assert!(result.is_err());
    }

    #[test]
    fn with_temp_dir_removes_dir_when_closure_returns_error() {
        let captured: (PathBuf, Result<(), &str>) = with_temp_dir("slopgate-err-", |dir| {
            let path = dir.to_path_buf();
            (path, Err("simulated failure"))
        })
        .unwrap();
        let (dir_path, result) = captured;
        assert!(result.is_err());
        assert!(!dir_path.exists(), "temp dir should be removed even on error return");
    }
}
