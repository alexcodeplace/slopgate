//! Stats query — aggregate JSONL rows by dimension; render table or JSON.
//! Mirrors `src/stats/query.mjs`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Supported group-by dimensions → row field (`rule` == gate slug == `ruleId`).
pub fn dimension_field(by: &str) -> Result<&'static str, String> {
    match by {
        "rule" => Ok("ruleId"),
        "model" => Ok("model"),
        "project" => Ok("project"),
        "severity" => Ok("severity"),
        "engine" => Ok("engine"),
        "category" => Ok("category"),
        _ => Err(format!("unknown dimension: {by}")),
    }
}

/// One JSONL stat event (`src/stats/record.mjs` row shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    #[serde(default)]
    pub ts: Option<String>,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub key: String,
    pub count: u32,
    pub last_seen: Option<String>,
    pub first_seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateResult {
    pub total: u32,
    pub by: String,
    pub last_seen: Option<String>,
    pub first_seen: Option<String>,
    pub groups: Vec<Group>,
}

fn row_field(row: &Row, field: &str) -> String {
    let v = match field {
        "ruleId" => row.rule_id.as_deref(),
        "model" => row.model.as_deref(),
        "project" => row.project.as_deref(),
        "severity" => row.severity.as_deref(),
        "engine" => row.engine.as_deref(),
        "category" => row.category.as_deref(),
        _ => None,
    };
    v.unwrap_or("unknown").to_string()
}

fn row_ts(row: &Row) -> Option<&str> {
    row.ts.as_deref().filter(|ts| !ts.is_empty())
}

fn update_bounds(
    ts: &str,
    last: &mut Option<String>,
    first: &mut Option<String>,
) {
    if last.as_deref().is_none_or(|l| ts > l) {
        *last = Some(ts.to_string());
    }
    if first.as_deref().is_none_or(|f| ts < f) {
        *first = Some(ts.to_string());
    }
}

/// Aggregate rows by dimension. `by` defaults to `"rule"` when `None`.
pub fn aggregate(
    rows: &[Row],
    by: Option<&str>,
    since: Option<&str>,
) -> Result<AggregateResult, String> {
    let by = by.unwrap_or("rule");
    let field = dimension_field(by)?;

    let filtered: Vec<&Row> = if let Some(since) = since {
        rows.iter()
            .filter(|r| row_ts(r).is_some_and(|ts| ts >= since))
            .collect()
    } else {
        rows.iter().collect()
    };

    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut last_seen: Option<String> = None;
    let mut first_seen: Option<String> = None;

    for r in &filtered {
        let key = row_field(r, field);
        let entry = groups.entry(key.clone()).or_insert_with(|| Group {
            key,
            count: 0,
            last_seen: None,
            first_seen: None,
        });
        entry.count += 1;
        if let Some(ts) = row_ts(r) {
            update_bounds(ts, &mut entry.last_seen, &mut entry.first_seen);
            update_bounds(ts, &mut last_seen, &mut first_seen);
        }
    }

    let mut sorted: Vec<Group> = groups.into_values().collect();
    sorted.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.key.cmp(&b.key))
    });

    Ok(AggregateResult {
        total: filtered.len() as u32,
        by: by.to_string(),
        last_seen,
        first_seen,
        groups: sorted,
    })
}

fn pad_end(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn pad_start(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - len))
    }
}

/// Render aggregate result as a table or pretty JSON (`formatStats` in JS).
pub fn format_stats(result: &AggregateResult, json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".into());
    }

    let range = if result.total > 0 {
        format!(" (last {})", result.last_seen.as_deref().unwrap_or("—"))
    } else {
        String::new()
    };
    let mut lines = vec![format!("{} incident(s) stopped{range}", result.total)];
    if result.total == 0 {
        return lines.join("\n");
    }

    let key_header = result.by.to_uppercase();
    let key_w = key_header
        .chars()
        .count()
        .max(
            result
                .groups
                .iter()
                .map(|g| g.key.chars().count())
                .max()
                .unwrap_or(0),
        );
    let count_w = 5_usize.max(
        result
            .groups
            .iter()
            .map(|g| g.count.to_string().chars().count())
            .max()
            .unwrap_or(0),
    );

    lines.push(format!(
        "{}  {}  LAST SEEN",
        pad_end(&key_header, key_w),
        pad_start("COUNT", count_w),
    ));
    for g in &result.groups {
        lines.push(format!(
            "{}  {}  {}",
            pad_end(&g.key, key_w),
            pad_start(&g.count.to_string(), count_w),
            g.last_seen.as_deref().unwrap_or("—"),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rows() -> Vec<Row> {
        vec![
            Row {
                ts: Some("2026-01-01T10:00:00.000Z".into()),
                rule_id: Some("no-stubs".into()),
                project: Some("slopgate".into()),
                model: Some("claude".into()),
                severity: Some("critical".into()),
                engine: Some("regex".into()),
                category: Some("quality".into()),
                file: None,
                line: None,
            },
            Row {
                ts: Some("2026-01-02T10:00:00.000Z".into()),
                rule_id: Some("no-stubs".into()),
                project: Some("slopgate".into()),
                model: Some("claude".into()),
                severity: Some("critical".into()),
                engine: Some("regex".into()),
                category: Some("quality".into()),
                file: None,
                line: None,
            },
            Row {
                ts: Some("2026-01-03T10:00:00.000Z".into()),
                rule_id: Some("as-any".into()),
                project: Some("slopgate".into()),
                model: Some("gpt".into()),
                severity: Some("high".into()),
                engine: Some("regex".into()),
                category: Some("types".into()),
                file: None,
                line: None,
            },
            Row {
                ts: Some("2026-01-01T12:00:00.000Z".into()),
                rule_id: Some("as-any".into()),
                project: Some("other".into()),
                model: Some("gpt".into()),
                severity: Some("high".into()),
                engine: Some("ast".into()),
                category: Some("types".into()),
                file: None,
                line: None,
            },
        ]
    }

    #[test]
    fn aggregate_by_rule_counts_and_ts_bounds() {
        let result = aggregate(&sample_rows(), Some("rule"), None).unwrap();
        assert_eq!(result.total, 4);
        assert_eq!(result.by, "rule");
        assert_eq!(result.first_seen.as_deref(), Some("2026-01-01T10:00:00.000Z"));
        assert_eq!(result.last_seen.as_deref(), Some("2026-01-03T10:00:00.000Z"));
        assert_eq!(result.groups.len(), 2);
        // Equal counts → localeCompare on key ("as-any" before "no-stubs").
        assert_eq!(result.groups[0].key, "as-any");
        assert_eq!(result.groups[0].count, 2);
        assert_eq!(
            result.groups[0].first_seen.as_deref(),
            Some("2026-01-01T12:00:00.000Z")
        );
        assert_eq!(
            result.groups[0].last_seen.as_deref(),
            Some("2026-01-03T10:00:00.000Z")
        );
        assert_eq!(result.groups[1].key, "no-stubs");
        assert_eq!(result.groups[1].count, 2);
    }

    #[test]
    fn since_filter_drops_older_rows() {
        let result =
            aggregate(&sample_rows(), Some("rule"), Some("2026-01-02T00:00:00.000Z")).unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.groups.len(), 2);
        for g in &result.groups {
            assert_eq!(g.count, 1);
        }
    }

    #[test]
    fn default_by_is_rule() {
        let result = aggregate(&sample_rows(), None, None).unwrap();
        assert_eq!(result.by, "rule");
        assert_eq!(result.total, 4);
    }

    #[test]
    fn unknown_dimension_errors() {
        let err = aggregate(&sample_rows(), Some("bogus"), None).unwrap_err();
        assert!(err.contains("unknown dimension"));
    }

    #[test]
    fn empty_rows_yields_empty_result() {
        let result = aggregate(&[], Some("rule"), None).unwrap();
        assert_eq!(result.total, 0);
        assert!(result.groups.is_empty());
        assert!(result.last_seen.is_none());
        assert!(result.first_seen.is_none());
        assert_eq!(format_stats(&result, false), "0 incident(s) stopped");
    }

    #[test]
    fn missing_field_key_becomes_unknown() {
        let rows = vec![Row {
            ts: Some("2026-01-01T00:00:00.000Z".into()),
            rule_id: None,
            project: None,
            model: None,
            severity: None,
            engine: None,
            category: None,
            file: None,
            line: None,
        }];
        let result = aggregate(&rows, Some("rule"), None).unwrap();
        assert_eq!(result.groups[0].key, "unknown");
        assert_eq!(result.groups[0].count, 1);
    }

    #[test]
    fn non_string_ts_ignored_for_bounds() {
        let rows = vec![Row {
            ts: None,
            rule_id: Some("x".into()),
            project: None,
            model: None,
            severity: None,
            engine: None,
            category: None,
            file: None,
            line: None,
        }];
        let result = aggregate(&rows, Some("rule"), None).unwrap();
        assert_eq!(result.total, 1);
        assert!(result.last_seen.is_none());
        assert!(result.first_seen.is_none());
        assert!(result.groups[0].last_seen.is_none());
    }

    #[test]
    fn format_stats_text_table() {
        let result = aggregate(&sample_rows(), Some("rule"), None).unwrap();
        let text = format_stats(&result, false);
        assert!(text.contains("4 incident(s) stopped"));
        assert!(text.contains("2026-01-03T10:00:00.000Z"));
        assert!(text.contains("RULE"));
        assert!(text.contains("no-stubs"));
        assert!(text.contains("as-any"));
    }

    #[test]
    fn format_stats_json() {
        let result = aggregate(&sample_rows(), Some("model"), None).unwrap();
        let json = format_stats(&result, true);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["total"], 4);
        assert_eq!(parsed["by"], "model");
        assert!(parsed["groups"].is_array());
    }
}
