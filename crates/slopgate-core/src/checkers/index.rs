//! Checker registry — mirrors `src/checkers/index.mjs`.

use crate::checkers::actionlint;
use crate::checkers::depcruise;
use crate::checkers::diff_shape;
use crate::checkers::jscpd;
use crate::checkers::knip;
use crate::checkers::leakscan;
use crate::checkers::shellcheck;
use crate::checkers::tsc;
use crate::checkers::type_coverage;
use crate::checkers::typos;
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde_json::Value;

/// Outcome of checker `detect` (tool availability).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectResult {
    pub available: bool,
    pub reason: Option<String>,
}

/// Outcome of checker `run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerRunResult {
    pub violations: Vec<Violation>,
    pub errors: Vec<String>,
}

/// Options passed to checker `run` — mirrors JS `{ files, mode }`.
#[derive(Debug, Clone, Copy)]
pub struct CheckerRunOpts<'a> {
    pub files: Option<&'a [String]>,
    /// `"file"` | `"staged"` | `"full"`
    pub mode: &'a str,
}

/// Commit-tier checker adapter shape (`src/checkers/*.mjs` default export).
pub struct Checker {
    pub id: &'static str,
    pub detect: fn(&ResolvedConfig, &Value) -> DetectResult,
    pub run: fn(&ResolvedConfig, &Value, CheckerRunOpts<'_>) -> CheckerRunResult,
}

/// Commit-tier checkers, in execution order (mirrors `src/checkers/index.mjs`).
pub static CHECKERS: &[Checker] = &[
    Checker {
        id: "tsc",
        detect: tsc::detect,
        run: tsc::run,
    },
    Checker {
        id: "knip",
        detect: knip::detect,
        run: knip::run,
    },
    Checker {
        id: "jscpd",
        detect: jscpd::detect,
        run: jscpd::run,
    },
    Checker {
        id: "depcruise",
        detect: depcruise::detect,
        run: depcruise::run,
    },
    Checker {
        id: "leakscan",
        detect: leakscan::detect,
        run: leakscan::run,
    },
    Checker {
        id: "type-coverage",
        detect: type_coverage::detect,
        run: type_coverage::run,
    },
    Checker {
        id: "shellcheck",
        detect: shellcheck::detect,
        run: shellcheck::run,
    },
    Checker {
        id: "actionlint",
        detect: actionlint::detect,
        run: actionlint::run,
    },
    Checker {
        id: "typos",
        detect: typos::detect,
        run: typos::run,
    },
    Checker {
        id: "diff-shape",
        detect: diff_shape::detect,
        run: diff_shape::run,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkers_registry_order_matches_js() {
        let ids: Vec<&str> = CHECKERS.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            vec![
                "tsc",
                "knip",
                "jscpd",
                "depcruise",
                "leakscan",
                "type-coverage",
                "shellcheck",
                "actionlint",
                "typos",
                "diff-shape"
            ]
        );
    }
}
