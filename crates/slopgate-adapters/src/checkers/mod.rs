//! Concrete adapters: language-specific logic does not belong in the coordinator.
pub mod actionlint;
pub mod cargo_check;
pub mod depcruise;
pub mod diff_shape;
pub mod index;
pub mod jscpd;
pub mod knip;
pub mod leakscan;
pub mod shared;
pub mod shellcheck;
pub mod tsc;
pub mod type_coverage;
pub mod typos;
