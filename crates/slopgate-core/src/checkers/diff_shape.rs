//! diff_shape — staged set spanning too many concern areas (mirrors `src/checkers/diff-shape.mjs`).
//!
//! Concern area = configured root + first path segment under it (or `(root)` for files directly
//! under the root). Emits one violation when the number of distinct areas exceeds `max`.

use crate::report::Violation;
use std::collections::HashSet;

/// Collect concern groups: `{root}/{first-segment}` or `{root}/(root)` for root-level files.
pub fn concern_groups(files: &[String], roots_rel: &[String]) -> HashSet<String> {
    let mut groups = HashSet::new();
    for f in files {
        let Some(root) = roots_rel
            .iter()
            .find(|r| *f == **r || f.starts_with(&format!("{}/", r)))
        else {
            continue;
        };
        let rest = if f.as_str() == root.as_str() {
            ""
        } else {
            &f[root.len() + 1..]
        };
        let seg = if rest.contains('/') {
            rest.split('/').next().unwrap_or("(root)")
        } else {
            "(root)"
        };
        groups.insert(format!("{root}/{seg}"));
    }
    groups
}

/// True when staged files span more than `max` distinct concern areas.
pub fn exceeds(files: &[String], roots_rel: &[String], max: usize) -> bool {
    concern_groups(files, roots_rel).len() > max
}

/// Staged-mode checker: empty when within limit, one violation when over `max`.
pub fn check(files: &[String], roots_rel: &[String], max: usize) -> Vec<Violation> {
    let groups = concern_groups(files, roots_rel);
    if groups.len() <= max {
        return vec![];
    }
    let count = groups.len();
    let mut areas: Vec<String> = groups.into_iter().collect();
    areas.sort();
    let areas_preview = areas.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
    let text = format!("staged files span {count} areas (max {max})")
        .chars()
        .take(90)
        .collect();
    vec![Violation {
        id: "diff-shape-mixed-concerns".into(),
        severity: "high".into(),
        category: "hygiene".into(),
        file: files.first().cloned().unwrap_or_default(),
        line: 1,
        full_line: String::new(),
        text,
        resolution: format!("Split into focused commits. Areas: {areas_preview}"),
        engine: "checker:diff-shape".into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_root_and_first_segment() {
        let g = concern_groups(
            &[
                "src/a/x.ts".into(),
                "src/a/y.ts".into(),
                "src/b/z.ts".into(),
            ],
            &["src".into()],
        );
        assert_eq!(g.len(), 2); // src/a, src/b
    }

    #[test]
    fn root_level_file_is_root_bucket() {
        let g = concern_groups(&["src/x.ts".into()], &["src".into()]);
        assert!(g.iter().any(|s| s.ends_with("(root)")));
    }

    #[test]
    fn over_max_triggers_violation() {
        let files: Vec<String> = (0..7).map(|i| format!("src/d{i}/f.ts")).collect();
        assert!(exceeds(&files, &["src".into()], 5)); // 7 groups > 5
        assert!(!exceeds(&["src/a/f.ts".into()], &["src".into()], 5));
    }

    #[test]
    fn check_under_limit_returns_empty() {
        assert!(check(&["src/a/f.ts".into()], &["src".into()], 5).is_empty());
    }

    #[test]
    fn check_over_max_returns_violation() {
        let files: Vec<String> = (0..7).map(|i| format!("src/d{i}/f.ts")).collect();
        let v = check(&files, &["src".into()], 5);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "diff-shape-mixed-concerns");
        assert_eq!(v[0].severity, "high");
        assert!(v[0].text.contains("7 areas"));
    }
}
