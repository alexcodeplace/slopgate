//! Checker registry — mirrors `src/checkers/index.mjs`.
//! Empty until Wave 5 fills adapters; commit-tier loop in `gate.rs` compiles against this slice.

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

/// Commit-tier checkers, in execution order. Wave 5 populates this slice.
pub static CHECKERS: &[Checker] = &[];
