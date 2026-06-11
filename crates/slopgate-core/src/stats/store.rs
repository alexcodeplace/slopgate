//! JSONL stats store: location resolution + line-atomic append + tolerant read.
//! Mirrors `src/stats/store.mjs`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::install::agent_hooks::home_dir;

/// Cross-project global store under `home` (`~/.slopgate/stats.jsonl` when `home` is `$HOME`).
pub fn global_stats_path_in(home: &Path) -> PathBuf {
    home.join(".slopgate").join("stats.jsonl")
}

/// Cross-project global store (`~/.slopgate/stats.jsonl`).
pub fn global_stats_path() -> PathBuf {
    global_stats_path_in(&home_dir())
}

/// Config object slice used for project store path (mirrors `config.configDir` in JS).
pub trait ProjectStatsConfig {
    fn config_dir(&self) -> &Path;
}

/// Per-project mirror, next to the project's config.
pub fn project_stats_path(config: &impl ProjectStatsConfig) -> PathBuf {
    config.config_dir().join("stats.jsonl")
}

/// Append one row as a single JSON line. One `write_all` per row — under `O_APPEND`
/// the write offset is atomic, so concurrent sessions' rows never interleave.
pub fn append_row(path: &Path, obj: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(obj).map_err(std::io::Error::other)? + "\n";
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Read all rows. Missing file -> `[]`. Malformed lines skipped silently.
pub fn read_rows(path: &Path) -> Vec<Value> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for line in contents.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(row) = serde_json::from_str(line) {
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    struct TestConfig {
        config_dir: PathBuf,
    }

    impl ProjectStatsConfig for TestConfig {
        fn config_dir(&self) -> &Path {
            &self.config_dir
        }
    }

    #[test]
    fn global_stats_path_under_home_slopgate() {
        let home = TempDir::new().unwrap();
        let path = global_stats_path_in(home.path());
        assert_eq!(path, home.path().join(".slopgate/stats.jsonl"));
    }

    #[test]
    fn project_stats_path_next_to_config_dir() {
        let dir = TempDir::new().unwrap();
        let cfg = TestConfig {
            config_dir: dir.path().join(".slopgate"),
        };
        assert_eq!(
            project_stats_path(&cfg),
            cfg.config_dir.join("stats.jsonl")
        );
    }

    #[test]
    fn append_row_twice_read_rows_returns_two_in_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stats.jsonl");
        let first = json!({ "ts": "2026-01-01T00:00:00.000Z", "ruleId": "a" });
        let second = json!({ "ts": "2026-01-02T00:00:00.000Z", "ruleId": "b" });

        append_row(&path, &first).unwrap();
        append_row(&path, &second).unwrap();

        let rows = read_rows(&path);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], first);
        assert_eq!(rows[1], second);
    }

    #[test]
    fn read_rows_missing_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.jsonl");
        assert_eq!(read_rows(&path), Vec::<Value>::new());
    }

    #[test]
    fn read_rows_skips_malformed_lines_without_panic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stats.jsonl");
        let good = json!({ "ruleId": "ok" });
        fs::write(
            &path,
            format!(
                "{}\n{{not valid json}}\n{}\n",
                serde_json::to_string(&good).unwrap(),
                serde_json::to_string(&json!({ "ruleId": "also-ok" })).unwrap()
            ),
        )
        .unwrap();

        let rows = read_rows(&path);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["ruleId"], "ok");
        assert_eq!(rows[1]["ruleId"], "also-ok");
    }

    #[test]
    fn read_rows_corrupt_unreadable_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stats.jsonl");
        fs::write(&path, &[0xff, 0xfe, 0xfd]).unwrap();

        assert_eq!(read_rows(&path), Vec::<Value>::new());
    }

    #[test]
    fn append_row_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dir").join("stats.jsonl");
        append_row(&path, &json!({ "probe": true })).unwrap();
        assert!(path.is_file());
        assert_eq!(read_rows(&path).len(), 1);
    }
}
