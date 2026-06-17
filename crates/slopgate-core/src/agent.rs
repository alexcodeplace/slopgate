//! Optional agent hook helpers.
//!
//! These are intentionally deterministic and fail-open at the command boundary:
//! no LLM calls, no external tools, and no repo mutation.

use crate::config::{AgentGoalConfig, AgentPromptMetaConfig};
use serde_json::{json, Value};

const PROMPT_CONTEXT_LIMIT: usize = 1200;
const GOAL_REASON: &str = "Slopgate goal check: before stopping, report deterministic completion evidence: what changed, what was verified, or why verification could not run.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityLevel {
    Low,
    Medium,
    High,
}

impl ComplexityLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAnalysis {
    pub complexity: ComplexityLevel,
    pub score: u32,
    pub complexity_signals: Vec<&'static str>,
    pub ambiguity_signals: Vec<&'static str>,
    pub split_signals: Vec<&'static str>,
}

impl PromptAnalysis {
    pub fn has_advice(&self) -> bool {
        self.complexity != ComplexityLevel::Low
            || !self.ambiguity_signals.is_empty()
            || !self.split_signals.is_empty()
    }
}

fn contains_any<'a>(haystack: &str, terms: &'a [&'a str]) -> Option<&'a str> {
    terms.iter().copied().find(|term| haystack.contains(term))
}

fn word_count(prompt: &str) -> usize {
    prompt
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .count()
}

fn bullet_count(prompt: &str) -> usize {
    prompt
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            t.starts_with("- ")
                || t.starts_with("* ")
                || t.starts_with("[ ]")
                || t.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() && t.contains('.'))
        })
        .count()
}

fn file_ref_count(prompt: &str) -> usize {
    const EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".toml", ".json", ".yml",
        ".yaml", ".md", ".sh",
    ];
    prompt
        .split_whitespace()
        .filter(|tok| EXTS.iter().any(|ext| tok.contains(ext)) || tok.contains('/'))
        .count()
}

pub fn analyze_prompt(prompt: &str) -> PromptAnalysis {
    let lower = prompt.to_lowercase();
    let words = word_count(prompt);
    let lines = prompt.lines().filter(|l| !l.trim().is_empty()).count();
    let bullets = bullet_count(prompt);
    let file_refs = file_ref_count(prompt);

    let mut score = 0;
    let mut complexity_signals = Vec::new();

    if words >= 120 {
        score += 2;
        complexity_signals.push("long prompt");
    } else if words >= 60 {
        score += 1;
        complexity_signals.push("multi-paragraph prompt");
    }

    if lines >= 6 || bullets >= 3 {
        score += 1;
        complexity_signals.push("checklist shape");
    }

    if file_refs >= 3 {
        score += 1;
        complexity_signals.push("multiple file refs");
    }

    if contains_any(
        &lower,
        &[
            "implement",
            "refactor",
            "migrate",
            "scaffold",
            "add tests",
            "run tests",
            "dogfood",
            "commit",
            "acceptance",
        ],
    )
    .is_some()
    {
        score += 1;
        complexity_signals.push("implementation workflow");
    }

    if contains_any(
        &lower,
        &[
            "and then",
            "also",
            "before final",
            "final report",
            "end to end",
            "multiple",
            "phase",
        ],
    )
    .is_some()
    {
        score += 1;
        complexity_signals.push("sequenced work");
    }

    let ambiguity_signals = [
        ("fix it", "vague target"),
        ("make it better", "vague target"),
        ("clean this up", "vague target"),
        ("stuff", "vague noun"),
        ("things", "vague noun"),
        ("etc", "open-ended scope"),
        ("as needed", "open-ended scope"),
        ("whatever", "open-ended scope"),
        ("not working", "missing failure evidence"),
        ("this issue", "unclear reference"),
        ("that bug", "unclear reference"),
    ]
    .iter()
    .filter_map(|(needle, label)| lower.contains(needle).then_some(*label))
    .collect::<Vec<_>>();

    let mut domain_count = 0;
    let mut split_signals = Vec::new();
    for (terms, label) in [
        (&["research", "investigate", "look into"][..], "research"),
        (&["review", "audit", "security"][..], "review"),
        (
            &["implement", "fix", "refactor", "code"][..],
            "implementation",
        ),
        (&["test", "verify", "dogfood", "smoke"][..], "verification"),
        (
            &["frontend", "backend", "database", "api"][..],
            "multi-surface",
        ),
    ] {
        if contains_any(&lower, terms).is_some() {
            domain_count += 1;
            split_signals.push(label);
        }
    }

    let split_signals = if lower.contains("subagent")
        || lower.contains("parallel")
        || domain_count >= 3
        || (score >= 4 && domain_count >= 2)
    {
        split_signals
    } else {
        Vec::new()
    };

    let complexity = match score {
        0 | 1 => ComplexityLevel::Low,
        2 | 3 => ComplexityLevel::Medium,
        _ => ComplexityLevel::High,
    };

    PromptAnalysis {
        complexity,
        score,
        complexity_signals,
        ambiguity_signals,
        split_signals,
    }
}

fn truncate_context(s: &str) -> String {
    if s.len() <= PROMPT_CONTEXT_LIMIT {
        return s.to_string();
    }

    let mut end = PROMPT_CONTEXT_LIMIT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

pub fn format_prompt_advisory(analysis: &PromptAnalysis) -> Option<String> {
    if !analysis.has_advice() {
        return None;
    }

    let mut parts = Vec::new();
    if analysis.complexity != ComplexityLevel::Low {
        let signals = analysis.complexity_signals.join(", ");
        parts.push(format!(
            "complexity: {}{}",
            analysis.complexity.as_str(),
            if signals.is_empty() {
                String::new()
            } else {
                format!(" ({signals})")
            }
        ));
    }
    if !analysis.ambiguity_signals.is_empty() {
        parts.push(format!(
            "ambiguity: {}",
            analysis.ambiguity_signals.join(", ")
        ));
    }
    if !analysis.split_signals.is_empty() {
        parts.push(format!(
            "split: consider separating {}",
            analysis.split_signals.join(" + ")
        ));
    }

    let mut msg = format!("Slopgate prompt meta (advisory): {}.", parts.join("; "));
    if analysis.complexity == ComplexityLevel::High || !analysis.ambiguity_signals.is_empty() {
        msg.push_str(" Suggested agent behavior: restate assumptions, make a short checklist, and verify before the final answer.");
    }
    Some(truncate_context(&msg))
}

fn prompt_from_hook_json(input: &Value) -> Option<&str> {
    input
        .get("prompt")
        .and_then(Value::as_str)
        .or_else(|| input.pointer("/tool_input/prompt").and_then(Value::as_str))
        .or_else(|| input.get("message").and_then(Value::as_str))
}

pub fn prompt_meta_hook_output(config: &AgentPromptMetaConfig, hook_json: &str) -> Option<Value> {
    if !config.enabled {
        return None;
    }
    let input: Value = serde_json::from_str(hook_json).ok()?;
    let prompt = prompt_from_hook_json(&input)?.trim();
    if prompt.is_empty() {
        return None;
    }
    let advisory = format_prompt_advisory(&analyze_prompt(prompt))?;
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": advisory
        }
    }))
}

fn last_assistant_message(input: &Value) -> Option<&str> {
    input.get("last_assistant_message").and_then(Value::as_str)
}

fn evidence_present(message: &str) -> bool {
    let lower = message.to_lowercase();
    let blocked = contains_any(
        &lower,
        &[
            "blocked",
            "unable to",
            "could not",
            "couldn't",
            "wasn't able",
            "not able",
            "failed because",
        ],
    )
    .is_some();
    if blocked {
        return true;
    }

    let completion = contains_any(
        &lower,
        &[
            "implemented",
            "added",
            "updated",
            "fixed",
            "created",
            "changed",
            "completed",
            "done",
            "committed",
            "staged",
            "verified",
            "ran",
            "passed",
            "failed",
        ],
    )
    .is_some();

    let verification = contains_any(
        &lower,
        &[
            "verification",
            "verified",
            "tested",
            "tests",
            "cargo test",
            "npm test",
            "pytest",
            "go test",
            "self-test",
            "slopgate --staged",
            "dogfood",
            "smoke",
            "lint",
            "build",
            "checked",
            "not run",
            "could not run",
            "unable to run",
            "commit hash",
            "commit:",
        ],
    )
    .is_some();

    completion && verification
}

pub fn goal_check_hook_output(config: &AgentGoalConfig, hook_json: &str) -> Option<Value> {
    if !config.enabled || config.max_continuations == 0 {
        return None;
    }

    let input: Value = serde_json::from_str(hook_json).ok()?;
    if input
        .get("stop_hook_active")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }

    let message = last_assistant_message(&input).unwrap_or("").trim();
    if evidence_present(message) {
        return None;
    }

    Some(json!({
        "decision": "block",
        "reason": GOAL_REASON
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentGoalConfig, AgentPromptMetaConfig};

    fn enabled_prompt_meta() -> AgentPromptMetaConfig {
        AgentPromptMetaConfig { enabled: true }
    }

    fn enabled_goal() -> AgentGoalConfig {
        AgentGoalConfig {
            enabled: true,
            max_continuations: 1,
        }
    }

    #[test]
    fn prompt_meta_disabled_is_silent() {
        let out = prompt_meta_hook_output(
            &AgentPromptMetaConfig { enabled: false },
            r#"{"prompt":"Implement this and run tests"}"#,
        );
        assert!(out.is_none());
    }

    #[test]
    fn simple_prompt_has_no_advisory() {
        let out =
            prompt_meta_hook_output(&enabled_prompt_meta(), r#"{"prompt":"What time is it?"}"#);
        assert!(out.is_none());
    }

    #[test]
    fn complex_ambiguous_prompt_gets_advisory() {
        let prompt = r#"{
          "prompt": "Research the current code, fix it as needed, implement the parser, add tests, dogfood the hook, and then commit. Also update crates/slopgate-core/src/config.rs, crates/slopgate-rs/src/main.rs, hooks/goal-stop-hook.sh, and final report."
        }"#;
        let out = prompt_meta_hook_output(&enabled_prompt_meta(), prompt).unwrap();
        let text = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("complexity:"));
        assert!(text.contains("ambiguity:"));
        assert!(text.contains("split:"));
    }

    #[test]
    fn goal_disabled_allows_missing_evidence() {
        let out = goal_check_hook_output(
            &AgentGoalConfig {
                enabled: false,
                max_continuations: 1,
            },
            r#"{"hook_event_name":"Stop","last_assistant_message":"Done"}"#,
        );
        assert!(out.is_none());
    }

    #[test]
    fn goal_blocks_when_evidence_missing() {
        let out = goal_check_hook_output(
            &enabled_goal(),
            r#"{"hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"Done"}"#,
        )
        .unwrap();
        assert_eq!(out["decision"], "block");
        assert!(out["reason"]
            .as_str()
            .unwrap()
            .contains("completion evidence"));
    }

    #[test]
    fn goal_allows_when_evidence_present() {
        let out = goal_check_hook_output(
            &enabled_goal(),
            r#"{"hook_event_name":"Stop","last_assistant_message":"Implemented parser. Verification: cargo test -p slopgate-core passed. Commit hash: abc123."}"#,
        );
        assert!(out.is_none());
    }

    #[test]
    fn goal_allows_when_stop_hook_already_active() {
        let out = goal_check_hook_output(
            &enabled_goal(),
            r#"{"hook_event_name":"Stop","stop_hook_active":true,"last_assistant_message":"Done"}"#,
        );
        assert!(out.is_none());
    }
}
