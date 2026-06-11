//! Pure audit computations — mirrors `src/audit/measures.mjs`.
//! No fs, no git, no subprocess. Function counts / export ratios are regex proxies.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static FN_COUNT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfunction\b|=>").unwrap());
static DECL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(export\s+)?(const|let|var|function|async function|class|type|interface|enum)\b")
        .unwrap()
});
static BARREL_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+(\*|\{[^}]*\})\s+from\s").unwrap());

/// LOC (non-blank) + function-count proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Complexity {
    pub loc: usize,
    pub fn_count: usize,
}

/// Non-blank line count + regex proxy for top-level functions / arrow fns.
pub fn complexity(source: &str) -> Complexity {
    let loc = source.lines().filter(|l| !l.trim().is_empty()).count();
    let fn_count = FN_COUNT_RE.find_iter(source).count();
    Complexity { loc, fn_count }
}

/// Hotspot rank: churn × LOC × functions (clamped ≥ 1 so plain data files still rank).
pub fn hotspot_score(churn: f64, complexity: &Complexity) -> f64 {
    churn * complexity.loc as f64 * complexity.fn_count.max(1) as f64
}

/// Short-window churn rate ÷ long-window rate. >1 = heating up, <1 = cooling.
pub fn acceleration(churn_short: f64, short_days: f64, churn_long: f64, long_days: f64) -> f64 {
    let rs = churn_short / short_days;
    let rl = churn_long / long_days;
    if rl == 0.0 {
        if rs > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    } else {
        rs / rl
    }
}

/// Exported ÷ total TOP-LEVEL decls (column-0 only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportRatio {
    pub total: usize,
    pub exported: usize,
    pub ratio: f64,
}

pub fn export_ratio(source: &str) -> ExportRatio {
    let mut total = 0usize;
    let mut exported = 0usize;
    for line in source.lines() {
        if let Some(caps) = DECL_RE.captures(line) {
            total += 1;
            if caps.get(1).is_some() {
                exported += 1;
            }
        }
    }
    let ratio = if total == 0 {
        0.0
    } else {
        exported as f64 / total as f64
    };
    ExportRatio {
        total,
        exported,
        ratio,
    }
}

/// Barrel = every significant line is a re-export.
pub fn is_barrel(source: &str) -> bool {
    let sig: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("//")
                && !l.starts_with("/*")
                && !l.starts_with('*')
        })
        .collect();
    !sig.is_empty() && sig.iter().all(|l| BARREL_LINE_RE.is_match(l))
}

#[derive(Debug, Clone)]
pub struct DepCruiseModule {
    pub source: String,
    pub dependencies: Vec<DepCruiseDependency>,
}

#[derive(Debug, Clone)]
pub struct DepCruiseDependency {
    pub resolved: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanRow {
    pub module: String,
    pub fan_out: usize,
    pub fan_in: usize,
}

/// Fan-in/fan-out per internal module from a depcruise `modules` array.
pub fn fan_metrics(modules: &[DepCruiseModule]) -> Vec<FanRow> {
    let internal: Vec<&DepCruiseModule> = modules
        .iter()
        .filter(|m| !m.source.contains("node_modules"))
        .collect();
    let names: HashSet<&str> = internal.iter().map(|m| m.source.as_str()).collect();
    let mut fan_in: HashMap<&str, usize> = HashMap::new();
    let rows: Vec<(String, usize)> = internal
        .iter()
        .map(|m| {
            let deps: HashSet<&str> = m
                .dependencies
                .iter()
                .map(|d| d.resolved.as_str())
                .filter(|r| names.contains(r) && *r != m.source.as_str())
                .collect();
            for d in &deps {
                *fan_in.entry(d).or_insert(0) += 1;
            }
            (m.source.clone(), deps.len())
        })
        .collect();
    rows.into_iter()
        .map(|(module, fan_out)| FanRow {
            fan_in: fan_in.get(module.as_str()).copied().unwrap_or(0),
            module,
            fan_out,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoChangePair {
    pub a: String,
    pub b: String,
    pub shared: usize,
    pub ratio: f64,
}

/// Co-change pairs across per-commit file sets.
pub fn co_change_pairs<F>(
    file_sets: &[Vec<String>],
    group_of: F,
    min_shared: usize,
    min_ratio: f64,
) -> Vec<CoChangePair>
where
    F: Fn(&str) -> Option<String>,
{
    let mut file_count: HashMap<String, usize> = HashMap::new();
    let mut pair_count: HashMap<String, usize> = HashMap::new();
    for set in file_sets {
        let mut uniq: Vec<String> = set.iter().cloned().collect::<HashSet<_>>().into_iter().collect();
        uniq.sort();
        for f in &uniq {
            *file_count.entry(f.clone()).or_insert(0) += 1;
        }
        for i in 0..uniq.len() {
            for j in (i + 1)..uniq.len() {
                let key = format!("{} {}", uniq[i], uniq[j]);
                *pair_count.entry(key).or_insert(0) += 1;
            }
        }
    }
    let mut out = Vec::new();
    for (key, shared) in pair_count {
        if shared < min_shared {
            continue;
        }
        let parts: Vec<&str> = key.split(' ').collect();
        if parts.len() != 2 {
            continue;
        }
        let a = parts[0].to_string();
        let b = parts[1].to_string();
        let min_count = file_count.get(&a).copied().unwrap_or(0).min(file_count.get(&b).copied().unwrap_or(0));
        if min_count == 0 {
            continue;
        }
        let ratio = shared as f64 / min_count as f64;
        if ratio < min_ratio {
            continue;
        }
        let ga = group_of(&a);
        let gb = group_of(&b);
        if ga.is_none() || gb.is_none() || ga == gb {
            continue;
        }
        out.push(CoChangePair {
            a,
            b,
            shared,
            ratio,
        });
    }
    out.sort_by(|x, y| {
        y.ratio
            .partial_cmp(&x.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| y.shared.cmp(&x.shared))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_counts_branch_keywords() {
        let c = complexity("if (a) {} else { while(b){} }");
        assert_eq!(c.loc, 1);
        assert_eq!(c.fn_count, 0);
    }

    #[test]
    fn complexity_counts_functions_and_nonempty_lines() {
        let c = complexity("function a() {}\nfunction b() {}\n\n");
        assert_eq!(c.loc, 2);
        assert_eq!(c.fn_count, 2);
    }

    #[test]
    fn hotspot_score_clamps_zero_fn_count_to_one() {
        let c = complexity("const x = 1;");
        assert_eq!(c.fn_count, 0);
        assert_eq!(hotspot_score(2.0, &c), 2.0 * c.loc as f64);
    }

    #[test]
    fn hotspot_score_multiplies_churn_loc_and_fn_count() {
        let c = complexity("function f() {}\nfunction g() {}\n");
        assert_eq!(hotspot_score(3.0, &c), 3.0 * 2.0 * 2.0);
    }

    #[test]
    fn acceleration_equal_rates_return_one() {
        assert_eq!(acceleration(10.0, 5.0, 20.0, 10.0), 1.0);
    }

    #[test]
    fn acceleration_zero_long_rate_with_short_churn_is_infinity() {
        assert!(acceleration(5.0, 5.0, 0.0, 10.0).is_infinite());
    }

    #[test]
    fn acceleration_zero_long_and_short_churn_is_zero() {
        assert_eq!(acceleration(0.0, 5.0, 0.0, 10.0), 0.0);
    }

    #[test]
    fn export_ratio_zero_on_empty() {
        let r = export_ratio("");
        assert_eq!(r.total, 0);
        assert_eq!(r.exported, 0);
        assert_eq!(r.ratio, 0.0);
    }

    #[test]
    fn export_ratio_counts_top_level_decls() {
        let r = export_ratio("export const x = 1\nconst y = 2\nexport function f() {}\n");
        assert_eq!(r.total, 3);
        assert_eq!(r.exported, 2);
        assert!((r.ratio - 2.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn is_barrel_detects_reexport_only() {
        assert!(is_barrel("export * from './a';\nexport { b } from './b';\n"));
        assert!(!is_barrel("export function f(){}\n"));
    }

    #[test]
    fn is_barrel_false_on_empty_or_comments_only() {
        assert!(!is_barrel(""));
        assert!(!is_barrel("// just a comment\n"));
    }

    #[test]
    fn fan_metrics_counts_internal_edges() {
        let modules = vec![
            DepCruiseModule {
                source: "src/a.ts".into(),
                dependencies: vec![DepCruiseDependency {
                    resolved: "src/b.ts".into(),
                }],
            },
            DepCruiseModule {
                source: "src/b.ts".into(),
                dependencies: vec![DepCruiseDependency {
                    resolved: "src/a.ts".into(),
                }],
            },
            DepCruiseModule {
                source: "node_modules/x/index.js".into(),
                dependencies: vec![],
            },
        ];
        let rows = fan_metrics(&modules);
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r.module == "src/a.ts").unwrap();
        let b = rows.iter().find(|r| r.module == "src/b.ts").unwrap();
        assert_eq!(a.fan_out, 1);
        assert_eq!(a.fan_in, 1);
        assert_eq!(b.fan_out, 1);
        assert_eq!(b.fan_in, 1);
    }

    #[test]
    fn co_change_pairs_keeps_cross_group_high_ratio_pairs() {
        let file_sets: Vec<Vec<String>> =
            vec![vec!["a.ts".into(), "b.ts".into()]; 10];
        let pairs = co_change_pairs(&file_sets, |f| match f {
            "a.ts" => Some("group1".into()),
            "b.ts" => Some("group2".into()),
            _ => None,
        }, 5, 0.7);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].shared, 10);
        assert!((pairs[0].ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn co_change_pairs_drops_same_group_and_below_threshold() {
        let file_sets = vec![vec!["a.ts".into(), "b.ts".into()]];
        let same_group = co_change_pairs(&file_sets, |_| Some("g".into()), 1, 0.5);
        assert!(same_group.is_empty());
        let low_shared = co_change_pairs(&file_sets, |f| match f {
            "a.ts" => Some("g1".into()),
            "b.ts" => Some("g2".into()),
            _ => None,
        }, 5, 0.7);
        assert!(low_shared.is_empty());
    }
}
