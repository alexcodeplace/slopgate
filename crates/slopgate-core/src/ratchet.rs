//! Ratchet baseline: snapshot existing violations; gate fails only on NEW ones.
//! Mirrors `src/ratchet.mjs`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::hash;
use crate::report::Violation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineEntry {
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: u32,
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

fn one() -> u32 {
    1
}
fn is_one(value: &u32) -> bool {
    *value == 1
}

/// Exact NUL-delimited staged rename paths. Discovery errors are not evidence
/// that no rename occurred, and filenames may contain tabs/newlines.
pub fn staged_renames(repo_root: &Path) -> Result<HashMap<String, String>, String> {
    let output = crate::process::run_tool(
        Path::new("git"),
        &[
            "diff",
            "--cached",
            "-M",
            "--name-status",
            "--diff-filter=R",
            "--no-ext-diff",
            "-z",
        ],
        Some(repo_root),
        Some(10_000),
    );
    if !output.ok || output.status != Some(0) {
        return Err(format!(
            "rename discovery failed: {} {}",
            output.error.unwrap_or_default(),
            output.stderr
        ));
    }
    if !output.stdout.is_empty() && !output.stdout.ends_with('\0') {
        return Err("rename paths are not NUL-terminated".into());
    }
    let parts: Vec<_> = output.stdout.split_terminator('\0').collect();
    if parts.len() % 3 != 0 {
        return Err("malformed rename record".into());
    }
    let mut renames = HashMap::new();
    for record in parts.chunks(3) {
        if !record[0].starts_with('R') || record[1].is_empty() || record[2].is_empty() {
            return Err("invalid rename record".into());
        }
        renames.insert(record[2].to_string(), record[1].to_string());
    }
    Ok(renames)
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

    if j.get("version")
        .is_some_and(|value| value.as_u64() != Some(1))
    {
        return LoadedBaseline {
            entries: HashMap::new(),
            missing: false,
            error: Some("unsupported baseline version".into()),
        };
    }
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

    match serde_json::from_value::<HashMap<String, BaselineEntry>>(entries_val.clone()) {
        Ok(entries) if entries.values().any(|entry| entry.count == 0) => LoadedBaseline {
            entries: HashMap::new(),
            missing: false,
            error: Some("baseline occurrence counts must be positive".into()),
        },
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
    let mut consumed: HashMap<String, u32> = HashMap::new();

    for v in violations {
        let fp = fingerprint_violation(v, None);
        let candidates = [
            Some(fp),
            renames
                .get(&v.file)
                .map(|old| fingerprint_violation(v, Some(old))),
        ];
        let mut hit = false;
        for fingerprint in candidates.into_iter().flatten() {
            if let Some(entry) = entries.get(&fingerprint) {
                let used = consumed.entry(fingerprint).or_default();
                if *used < entry.count
                    && entry.rule_id == v.id
                    && (entry.file == v.file || renames.get(&v.file) == Some(&entry.file))
                {
                    *used += 1;
                    hit = true;
                    break;
                }
            }
        }

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
    let sorted: BTreeMap<_, _> = entries
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let out = BaselineFile {
        version: 1,
        generated: generated.to_string(),
        entries: sorted,
    };
    let mut s = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    s.push('\n');
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temporary
        .write_all(s.as_bytes())
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary.persist(path).map_err(|error| error.to_string())?;
    Ok(entries.len())
}

/// Build entries from violations and write baseline file.
pub fn write_baseline(
    path: &Path,
    violations: &[Violation],
    generated: &str,
) -> Result<usize, String> {
    let mut entries: HashMap<String, BaselineEntry> = HashMap::new();
    for finding in violations {
        let entry = entries
            .entry(fingerprint_violation(finding, None))
            .or_insert_with(|| BaselineEntry {
                count: 0,
                rule_id: finding.id.clone(),
                file: finding.file.clone(),
            });
        entry.count = entry
            .count
            .checked_add(1)
            .ok_or("baseline occurrence count overflow")?;
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
            severity: v
                .get("severity")
                .and_then(|s| s.as_str())
                .unwrap_or("warn")
                .to_string(),
            category: v
                .get("category")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            file: v["file"].as_str().unwrap().to_string(),
            line: v.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as u32,
            full_line: v
                .get("fullLine")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            text: v
                .get("text")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            resolution: v
                .get("resolution")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            engine: v
                .get("engine")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
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
                count: 1,
                rule_id: "no-stubs".into(),
                file: "src/a.ts".into(),
            },
        );

        let result = filter_new(
            &[v_known.clone(), v_novel.clone()],
            &entries,
            &HashMap::new(),
        );
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
                count: 1,
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
                count: 1,
                rule_id: "rule-b".into(),
                file: "src/b.ts".into(),
            },
        );
        entries.insert(
            "aaa111".into(),
            BaselineEntry {
                count: 1,
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
    fn staged_renames_failure_is_incomplete() {
        let dir = tempfile::TempDir::new().unwrap();
        let map = staged_renames(dir.path());
        assert!(map.is_err());
    }
    #[test]
    fn baseline_quota_does_not_hide_new_identical_violations() {
        let finding = violation_from_json(
            &serde_json::json!({"engine":"regex","id":"rule","file":"src/a.data","line":1,"text":"bad","fullLine":"bad"}),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("baseline.json");
        write_baseline(&path, std::slice::from_ref(&finding), "test").unwrap();
        let loaded = load_baseline(&path);
        let mut copy = finding.clone();
        copy.line = 99;
        let result = filter_new(&[finding.clone(), copy], &loaded.entries, &HashMap::new());
        assert_eq!(result.baselined_count, 1);
        assert_eq!(result.fresh.len(), 1);
        write_baseline(&path, &[finding.clone(), finding.clone()], "test").unwrap();
        let loaded = load_baseline(&path);
        assert_eq!(loaded.entries.values().next().unwrap().count, 2);
        assert!(filter_new(
            &[finding.clone(), finding],
            &loaded.entries,
            &HashMap::new()
        )
        .fresh
        .is_empty());
    }
}
