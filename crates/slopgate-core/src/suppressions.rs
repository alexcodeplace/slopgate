//! False-positive suppression registry. Mirrors `src/suppressions.mjs`.
//! Match key = (id, file, sha1-of-trimmed-line). Content hash survives line drift;
//! a file move invalidates the entry (deliberate: forces re-review).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use crate::hash::line_hash;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub file: String,
    #[serde(rename = "lineHash")]
    pub line_hash: String,
}

/// Minimal violation shape for suppression matching (`id`, `file`, `lineHash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionViolation {
    pub id: String,
    pub file: String,
    pub line_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSuppressions {
    pub entries: Vec<Entry>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneResult {
    pub pruned: Vec<Entry>,
    pub kept: Vec<Entry>,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct SuppressionsFile {
    version: u32,
    entries: Vec<Entry>,
}

/// Load suppressions from `path`. Missing file → empty entries, no error.
/// Malformed JSON or non-array `entries` → empty entries + error string (fail toward blocking).
pub fn load_suppressions(path: &Path) -> LoadedSuppressions {
    if path.as_os_str().is_empty() || !path.exists() {
        return LoadedSuppressions {
            entries: Vec::new(),
            error: None,
        };
    }

    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) => {
            return LoadedSuppressions {
                entries: Vec::new(),
                error: Some(err.to_string()),
            };
        }
    };

    let j: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(err) => {
            return LoadedSuppressions {
                entries: Vec::new(),
                error: Some(err.to_string()),
            };
        }
    };

    let entries_val = match j.get("entries") {
        Some(v) if v.is_array() => v,
        _ => {
            return LoadedSuppressions {
                entries: Vec::new(),
                error: Some(r#""entries" is not an array"#.to_string()),
            };
        }
    };

    match serde_json::from_value(entries_val.clone()) {
        Ok(entries) => LoadedSuppressions {
            entries,
            error: None,
        },
        Err(err) => LoadedSuppressions {
            entries: Vec::new(),
            error: Some(err.to_string()),
        },
    }
}

/// `violation` must carry `{ id, file, line_hash }` (typically `line_hash = line_hash(full_line)`).
pub fn is_suppressed(entries: &[Entry], violation: &SuppressionViolation) -> bool {
    entries.iter().any(|e| {
        e.id == violation.id && e.file == violation.file && e.line_hash == violation.line_hash
    })
}

/// Stale detection: entry whose file is missing, or no line in file hashes to `line_hash`.
/// When not `dry_run`, writes `{ version: 1, entries: kept }` + trailing newline if anything pruned.
pub fn prune_stale(repo_root: &Path, path: &Path, dry_run: bool) -> PruneResult {
    let loaded = load_suppressions(path);
    if loaded.error.is_some() {
        return PruneResult {
            pruned: Vec::new(),
            kept: loaded.entries,
            error: loaded.error,
        };
    }

    let mut kept = Vec::new();
    let mut pruned = Vec::new();

    for entry in loaded.entries {
        let abs = repo_root.join(&entry.file);
        if !abs.exists() {
            pruned.push(entry);
            continue;
        }

        let contents = match fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => {
                pruned.push(entry);
                continue;
            }
        };

        if contents
            .split('\n')
            .any(|line| line_hash(line) == entry.line_hash)
        {
            kept.push(entry);
        } else {
            pruned.push(entry);
        }
    }

    let mut error = None;
    if !pruned.is_empty() && !dry_run {
        let out = SuppressionsFile {
            version: 1,
            entries: kept.clone(),
        };
        match serde_json::to_string_pretty(&out) {
            Ok(mut s) => {
                s.push('\n');
                if let Err(err) = fs::write(path, s) {
                    error = Some(err.to_string());
                }
            }
            Err(err) => error = Some(err.to_string()),
        }
    }

    PruneResult {
        pruned,
        kept,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn is_suppressed_matches_exact_triple() {
        let lh = line_hash("  // TODO  ");
        let entries = vec![Entry {
            id: "no-stubs".into(),
            file: "src/a.ts".into(),
            line_hash: lh.clone(),
        }];
        let v = SuppressionViolation {
            id: "no-stubs".into(),
            file: "src/a.ts".into(),
            line_hash: lh,
        };
        assert!(is_suppressed(&entries, &v));
    }

    #[test]
    fn is_suppressed_false_on_field_mismatch() {
        let lh = line_hash("x");
        let entries = vec![Entry {
            id: "a".into(),
            file: "f.ts".into(),
            line_hash: lh.clone(),
        }];
        assert!(!is_suppressed(
            &entries,
            &SuppressionViolation {
                id: "b".into(),
                file: "f.ts".into(),
                line_hash: lh.clone(),
            }
        ));
        assert!(!is_suppressed(
            &entries,
            &SuppressionViolation {
                id: "a".into(),
                file: "g.ts".into(),
                line_hash: lh.clone(),
            }
        ));
        assert!(!is_suppressed(
            &entries,
            &SuppressionViolation {
                id: "a".into(),
                file: "f.ts".into(),
                line_hash: line_hash("y"),
            }
        ));
    }

    #[test]
    fn load_suppressions_missing_path_empty_no_error() {
        let r = load_suppressions(Path::new("/nonexistent/sloppath/suppressions.json"));
        assert!(r.entries.is_empty());
        assert!(r.error.is_none());
    }

    #[test]
    fn load_suppressions_entries_not_array_sets_error() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("suppressions.json");
        fs::write(&p, r#"{"entries": 5}"#).unwrap();
        let r = load_suppressions(&p);
        assert!(r.entries.is_empty());
        assert_eq!(r.error.as_deref(), Some(r#""entries" is not an array"#));
    }

    #[test]
    fn load_suppressions_invalid_json_sets_error() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("suppressions.json");
        fs::write(&p, "{not json").unwrap();
        let r = load_suppressions(&p);
        assert!(r.entries.is_empty());
        assert!(r.error.is_some());
    }

    #[test]
    fn prune_stale_drops_missing_file_keeps_matching_line() {
        let repo = TempDir::new().unwrap();
        let repo_root = repo.path();

        let rel = "src/keep.ts";
        let keep_path = repo_root.join(rel);
        fs::create_dir_all(keep_path.parent().unwrap()).unwrap();
        let line = "  const x = 1  ";
        fs::write(&keep_path, format!("{line}\n")).unwrap();
        let keep_hash = line_hash(line);

        let deleted_rel = "src/gone.ts";
        let sup_path = repo_root.join("suppressions.json");
        let json = serde_json::json!({
            "version": 1,
            "entries": [
                {"id": "r1", "file": rel, "lineHash": keep_hash},
                {"id": "r2", "file": deleted_rel, "lineHash": line_hash("anything")},
            ]
        });
        fs::write(&sup_path, serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();

        let result = prune_stale(repo_root, &sup_path, false);
        assert_eq!(result.error, None);
        assert_eq!(result.pruned.len(), 1);
        assert_eq!(result.pruned[0].id, "r2");
        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0].id, "r1");

        let reloaded = load_suppressions(&sup_path);
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].id, "r1");
    }

    #[test]
    fn prune_stale_drops_entry_when_no_line_hashes() {
        let repo = TempDir::new().unwrap();
        let repo_root = repo.path();

        let rel = "src/changed.ts";
        let file_path = repo_root.join(rel);
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "totally different content\n").unwrap();

        let sup_path = repo_root.join("suppressions.json");
        let json = serde_json::json!({
            "version": 1,
            "entries": [
                {"id": "stale", "file": rel, "lineHash": line_hash("old line that is gone")},
            ]
        });
        fs::write(&sup_path, serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();

        let result = prune_stale(repo_root, &sup_path, true);
        assert_eq!(result.error, None);
        assert_eq!(result.pruned.len(), 1);
        assert!(result.kept.is_empty());
    }

    #[test]
    fn prune_stale_write_failure_does_not_panic() {
        let repo = TempDir::new().unwrap();
        let repo_root = repo.path();

        let deleted_rel = "src/gone.ts";
        let sup_path = repo_root.join("suppressions.json");
        let json = serde_json::json!({
            "version": 1,
            "entries": [
                {"id": "r2", "file": deleted_rel, "lineHash": line_hash("anything")},
            ]
        });
        fs::write(&sup_path, serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&sup_path).unwrap().permissions();
            perms.set_mode(0o444);
            fs::set_permissions(&sup_path, perms).unwrap();
        }
        #[cfg(not(unix))]
        {
            let mut perms = fs::metadata(&sup_path).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(&sup_path, perms).unwrap();
        }

        let result = prune_stale(repo_root, &sup_path, false);
        assert_eq!(result.pruned.len(), 1);
        assert!(result.kept.is_empty());
        assert!(result.error.is_some());
    }

    #[test]
    fn line_hash_keyed_match_parity() {
        let p = format!(
            "{}/tests/parity_vectors/line_hash.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let cases: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
        for case in cases.as_array().unwrap() {
            let line = case["line"].as_str().unwrap();
            let expected = case["hash"].as_str().unwrap();
            assert_eq!(line_hash(line), expected, "line={line:?}");

            let entries = vec![Entry {
                id: "parity".into(),
                file: "src/f.ts".into(),
                line_hash: expected.to_string(),
            }];
            let v = SuppressionViolation {
                id: "parity".into(),
                file: "src/f.ts".into(),
                line_hash: line_hash(line),
            };
            assert!(is_suppressed(&entries, &v), "line={line:?}");
        }
    }
}
