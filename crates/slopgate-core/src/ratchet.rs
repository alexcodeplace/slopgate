//! Ratchet baseline: snapshot existing violations; gate fails only on NEW ones.
//! Mirrors `src/ratchet.mjs`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::process::Command;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::hash;
use crate::report::Violation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineEntry {
    #[serde(rename = "ruleId")]
    pub rule_id: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBaseline {
    pub entries: HashMap<String, BaselineEntry>,
    pub missing: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterNewResult {
    pub fresh: Vec<Violation>,
    pub baselined_count: u32,
}

/// Fingerprint = sha256(engine|id|file|digit-normalized message|trimmed line text), 16 hex.
pub fn fingerprint_violation(v: &Violation, file_override: Option<&str>) -> String {
    hash::fingerprint(
        &v.engine,
        &v.id,
        &v.file,
        &v.text,
        &v.full_line,
        file_override,
    )
}

/// Staged renames as `{ newPath: oldPath }`. Fail-open: any git error → empty map.
pub fn staged_renames(repo_root: &Path) -> HashMap<String, String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "-M",
            "--name-status",
            "--diff-filter=R",
        ])
        .current_dir(repo_root)
        .output();

    let Ok(output) = output else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let re = rename_line_re();
    let mut map = HashMap::new();
    for line in raw.lines() {
        if let Some(caps) = re.captures(line) {
            let old_path = caps.get(1).unwrap().as_str().to_string();
            let new_path = caps.get(2).unwrap().as_str().to_string();
            map.insert(new_path, old_path);
        }
    }
    map
}

fn rename_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^R\d*\t([^\t]+)\t([^\t]+)$").expect("rename regex"))
}

/// Load baseline from `path`. Missing file → `missing: true`. Malformed → `error` set.
pub fn load_baseline(path: &Path) -> LoadedBaseline {
    if path.as_os_str().is_empty() || !path.exists() {
        return LoadedBaseline {
            entries: HashMap::new(),
            missing: true,
            error: None,
        };
    }

    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) => {
            return LoadedBaseline {
                entries: HashMap::new(),
                missing: false,
                error: Some(err.to_string()),
            };
        }
    };

    let j: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(err) => {
            return LoadedBaseline {
                entries: HashMap::new(),
                missing: false,
                error: Some(err.to_string()),
            };
        }
    };

    let entries_val = match j.get("entries") {
        Some(v) if v.is_object() => v,
        _ => {
            return LoadedBaseline {
                entries: HashMap::new(),
                missing: false,
                error: Some(r#""entries" is not an object"#.to_string()),
            };
        }
    };

    match serde_json::from_value(entries_val.clone()) {
        Ok(entries) => LoadedBaseline {
            entries,
            missing: false,
            error: None,
        },
        Err(err) => LoadedBaseline {
            entries: HashMap::new(),
            missing: false,
            error: Some(err.to_string()),
        },
    }
}

/// Split violations into fresh (not baselined) vs baselined. `renames` maps new path → old path.
pub fn filter_new(
    violations: &[Violation],
    entries: &HashMap<String, BaselineEntry>,
    renames: &HashMap<String, String>,
) -> FilterNewResult {
    let mut fresh = Vec::new();
    let mut baselined_count = 0u32;

    for v in violations {
        let fp = fingerprint_violation(v, None);
        let hit = entries.contains_key(&fp)
            || renames
                .get(&v.file)
                .is_some_and(|old| entries.contains_key(&fingerprint_violation(v, Some(old))));

        if hit {
            baselined_count += 1;
        } else {
            fresh.push(v.clone());
        }
    }

    FilterNewResult {
        fresh,
        baselined_count,
    }
}

/// Write baseline JSON with sorted entry keys + trailing newline. Returns entry count.
pub fn write_baseline_raw(
    path: &Path,
    entries: &HashMap<String, BaselineEntry>,
    generated: &str,
) -> Result<usize, String> {
    let sorted: BTreeMap<_, _> = entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let out = BaselineFile {
        version: 1,
        generated: generated.to_string(),
        entries: sorted,
    };
    let mut s = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    s.push('\n');
    fs::write(path, &s).map_err(|e| e.to_string())?;
    Ok(entries.len())
}

/// Build entries from violations and write baseline file.
pub fn write_baseline(
    path: &Path,
    violations: &[Violation],
    generated: &str,
) -> Result<usize, String> {
    let mut entries = HashMap::new();
    for v in violations {
        entries.insert(
            fingerprint_violation(v, None),
            BaselineEntry {
                rule_id: v.id.clone(),
                file: v.file.clone(),
            },
        );
    }
    write_baseline_raw(path, &entries, generated)
}

#[derive(Serialize)]
struct BaselineFile {
    version: u32,
    generated: String,
    entries: BTreeMap<String, BaselineEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn violation_from_json(v: &serde_json::Value) -> Violation {
        Violation {
            id: v["id"].as_str().unwrap().to_string(),
            severity: v.get("severity").and_then(|s| s.as_str()).unwrap_or("warn").to_string(),
            category: v.get("category").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            file: v["file"].as_str().unwrap().to_string(),
            line: v.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32,
            full_line: v.get("fullLine").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            text: v.get("text").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            resolution: v.get("resolution").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            engine: v.get("engine").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        }
    }

    fn parity_vectors(name: &str) -> serde_json::Value {
        let p = format!("{}/tests/parity_vectors/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn fingerprint_violation_matches_js_oracle() {
        for case in parity_vectors("fingerprint.json").as_array().unwrap() {
            let v = violation_from_json(&case["v"]);
            let got = fingerprint_violation(&v, None);
            assert_eq!(got, case["fp"].as_str().unwrap());
            assert_eq!(got.len(), 16);
        }
    }

    #[test]
    fn filter_new_baselined_vs_fresh() {
        let v_known = violation_from_json(&serde_json::json!({
            "engine": "regex",
            "id": "no-stubs",
            "file": "src/a.ts",
            "text": "TODO line 12",
            "fullLine": "  // TODO line 12  "
        }));
        let fp = fingerprint_violation(&v_known, None);

        let v_novel = violation_from_json(&serde_json::json!({
            "engine": "regex",
            "id": "other-rule",
            "file": "src/new.ts",
            "text": "brand new",
            "fullLine": "brand new"
        }));

        let mut entries = HashMap::new();
        entries.insert(
            fp,
            BaselineEntry {
                rule_id: "no-stubs".into(),
                file: "src/a.ts".into(),
            },
        );

        let result = filter_new(&[v_known.clone(), v_novel.clone()], &entries, &HashMap::new());
        assert_eq!(result.baselined_count, 1);
        assert_eq!(result.fresh.len(), 1);
        assert_eq!(result.fresh[0].id, "other-rule");
    }

    #[test]
    fn filter_new_rename_redirect() {
        let v_at_new_path = violation_from_json(&serde_json::json!({
            "engine": "regex",
            "id": "no-stubs",
            "file": "src/renamed.ts",
            "text": "TODO line 12",
            "fullLine": "  // TODO line 12  "
        }));
        let v_at_old_path = violation_from_json(&serde_json::json!({
            "engine": "regex",
            "id": "no-stubs",
            "file": "src/original.ts",
            "text": "TODO line 12",
            "fullLine": "  // TODO line 12  "
        }));

        let fp_old = fingerprint_violation(&v_at_old_path, None);
        let mut entries = HashMap::new();
        entries.insert(
            fp_old,
            BaselineEntry {
                rule_id: "no-stubs".into(),
                file: "src/original.ts".into(),
            },
        );

        let mut renames = HashMap::new();
        renames.insert("src/renamed.ts".into(), "src/original.ts".into());

        let result = filter_new(&[v_at_new_path], &entries, &renames);
        assert_eq!(result.baselined_count, 1);
        assert!(result.fresh.is_empty());
    }

    #[test]
    fn load_baseline_missing_file() {
        let r = load_baseline(Path::new("/nonexistent/sloppath/baseline.json"));
        assert!(r.entries.is_empty());
        assert!(r.missing);
        assert!(r.error.is_none());
    }

    #[test]
    fn load_baseline_entries_not_object_sets_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("baseline.json");
        fs::write(&p, r#"{"entries":[]}"#).unwrap();
        let r = load_baseline(&p);
        assert!(r.entries.is_empty());
        assert!(!r.missing);
        assert_eq!(r.error.as_deref(), Some(r#""entries" is not an object"#));
    }

    #[test]
    fn write_baseline_raw_exact_string() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("baseline.json");

        let mut entries = HashMap::new();
        entries.insert(
            "bbb222".into(),
            BaselineEntry {
                rule_id: "rule-b".into(),
                file: "src/b.ts".into(),
            },
        );
        entries.insert(
            "aaa111".into(),
            BaselineEntry {
                rule_id: "rule-a".into(),
                file: "src/a.ts".into(),
            },
        );

        let generated = "2026-06-11T12:00:00.000Z";
        let count = write_baseline_raw(&p, &entries, generated).unwrap();
        assert_eq!(count, 2);

        let expected = concat!(
            "{\n",
            "  \"version\": 1,\n",
            "  \"generated\": \"2026-06-11T12:00:00.000Z\",\n",
            "  \"entries\": {\n",
            "    \"aaa111\": {\n",
            "      \"ruleId\": \"rule-a\",\n",
            "      \"file\": \"src/a.ts\"\n",
            "    },\n",
            "    \"bbb222\": {\n",
            "      \"ruleId\": \"rule-b\",\n",
            "      \"file\": \"src/b.ts\"\n",
            "    }\n",
            "  }\n",
            "}\n"
        );
        let got = fs::read_to_string(&p).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn write_baseline_raw_unwritable_path_returns_err() {
        let dir = tempfile::TempDir::new().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        let p = blocker.join("baseline.json");

        let entries = HashMap::new();
        let result = write_baseline_raw(&p, &entries, "2026-01-01T00:00:00.000Z");
        assert!(result.is_err());
    }

    #[test]
    fn staged_renames_fail_open_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let map = staged_renames(dir.path());
        assert!(map.is_empty());
    }
}
