//! Language-neutral checker contracts (SG-ARCH-001).
//! The composition root supplies concrete implementations to the coordinator.
use crate::config::ResolvedConfig;
use crate::report::Violation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectResult {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckerRunResult {
    pub violations: Vec<Violation>,
    /// Infrastructure failures, never policy findings or provenance warnings.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CheckerScope {
    Files,
    #[default]
    Project,
    Repository,
}

#[derive(Debug, Clone, Copy)]
pub struct CheckerRunOpts<'a> {
    pub files: Option<&'a [String]>,
    pub mode: &'a str,
}

pub struct Checker {
    pub id: &'static str,
    pub scope: CheckerScope,
    pub detect: fn(&ResolvedConfig, &Value) -> DetectResult,
    pub run: fn(&ResolvedConfig, &Value, CheckerRunOpts<'_>) -> CheckerRunResult,
}
