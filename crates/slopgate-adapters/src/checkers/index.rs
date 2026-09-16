//! Composition of concrete checker adapters.
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
pub use slopgate_core::checkers::index::*;

/// Commit-tier checkers, in execution order (mirrors `src/checkers/index.mjs`).
pub static CHECKERS: &[Checker] = &[
    Checker {
        id: "cargo-check",
        scope: CheckerScope::Project,
        detect: crate::checkers::cargo_check::detect,
        run: crate::checkers::cargo_check::run,
    },
    Checker {
        id: "tsc",
        scope: CheckerScope::Project,
        detect: tsc::detect,
        run: tsc::run,
    },
    Checker {
        id: "knip",
        scope: CheckerScope::Repository,
        detect: knip::detect,
        run: knip::run,
    },
    Checker {
        id: "jscpd",
        scope: CheckerScope::Repository,
        detect: jscpd::detect,
        run: jscpd::run,
    },
    Checker {
        id: "depcruise",
        scope: CheckerScope::Repository,
        detect: depcruise::detect,
        run: depcruise::run,
    },
    Checker {
        id: "leakscan",
        scope: CheckerScope::Project,
        detect: leakscan::detect,
        run: leakscan::run,
    },
    Checker {
        id: "type-coverage",
        scope: CheckerScope::Project,
        detect: type_coverage::detect,
        run: type_coverage::run,
    },
    Checker {
        id: "shellcheck",
        scope: CheckerScope::Project,
        detect: shellcheck::detect,
        run: shellcheck::run,
    },
    Checker {
        id: "actionlint",
        scope: CheckerScope::Project,
        detect: actionlint::detect,
        run: actionlint::run,
    },
    Checker {
        id: "typos",
        scope: CheckerScope::Project,
        detect: typos::detect,
        run: typos::run,
    },
    Checker {
        id: "diff-shape",
        scope: CheckerScope::Files,
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
                "cargo-check",
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
