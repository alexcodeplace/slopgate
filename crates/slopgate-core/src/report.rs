//! Gate report formatting (DISPLAY surface). Returns `String` for testability; the bin prints it later.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};

/// Canonical violation shape used across the engine (regex, ast, checkers, ratchet, suppressions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub id: String,
    pub severity: String,
    pub category: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub full_line: String,
    pub text: String,
    pub resolution: String,
    #[serde(default = "default_engine")]
    pub engine: String,
}

fn default_engine() -> String {
    "regex".to_string()
}

fn engine_key(v: &Violation) -> &str {
    if v.engine.is_empty() {
        "regex"
    } else {
        &v.engine
    }
}

/// Render the human-facing gate report (mirrors `src/report.mjs` structure; ANSI omitted).
pub fn render_gate_report(violations: &[Violation], mode: &str, baselined: u32) -> String {
    if violations.is_empty() {
        return String::new();
    }

    let title = if mode == "file" {
        "SLOPGATE — VIOLATIONS IN EDITED FILE               "
    } else {
        "VIOLATIONS IN STAGED FILES — COMMIT BLOCKED         "
    };

    let mut out = String::new();
    out.push_str("\n");
    out.push_str("╔═ SLOPGATE ═════════════════════════════════════════╗\n");
    out.push_str(&format!("║ {title}║\n"));
    out.push_str("╚═════════════════════════════════════════════════════╝\n\n");

    let mut sorted: Vec<&Violation> = violations.iter().collect();
    sorted.sort_by(|a, b| {
        engine_key(a)
            .cmp(engine_key(b))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });

    let mut current_group: Option<&str> = None;
    for v in sorted {
        let group = engine_key(v);
        if current_group != Some(group) {
            current_group = Some(group);
            out.push_str(&format!("── {group} ──\n"));
        }
        let sev = v.severity.to_uppercase();
        out.push_str(&format!("[{sev}] {} {}:{}\n", v.id, v.file, v.line));
        out.push_str(&format!("  × {}\n", v.text));
        out.push_str(&format!("  ✓ {}\n\n", v.resolution));
    }

    let file_count = violations
        .iter()
        .map(|v| v.file.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let tail = if mode == "file" {
        "Fix now while context is hot."
    } else {
        "Fix → retry commit."
    };
    out.push_str(&format!(
        "{} violation(s) in {file_count} file(s). {tail}\n",
        violations.len()
    ));
    if baselined > 0 {
        out.push_str(&format!(
            "{baselined} pre-existing (baselined) violation(s) ignored.\n"
        ));
    }
    out.push_str(
        "False positive? NEVER edit suppressions.json yourself — ask the user via AskUserQuestion.\n\n",
    );
    out
}

/// Print the human-facing gate report to stderr (mirrors `src/report.mjs` `printGateReport`).
pub fn print_gate_report(violations: &[Violation], mode: &str, baselined_count: u32) {
    let _ = print_gate_report_to(violations, mode, baselined_count, &mut io::stderr());
}

/// Same as [`print_gate_report`] but writes to an arbitrary writer (unit tests).
pub fn print_gate_report_to(
    violations: &[Violation],
    mode: &str,
    baselined_count: u32,
    w: &mut dyn Write,
) -> io::Result<()> {
    let report = render_gate_report(violations, mode, baselined_count);
    w.write_all(report.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, file: &str, line: u32, sev: &str) -> Violation {
        Violation {
            id: id.into(),
            file: file.into(),
            line,
            severity: sev.into(),
            category: "test".into(),
            full_line: String::new(),
            text: "msg".into(),
            resolution: "fix".into(),
            engine: "regex".into(),
        }
    }

    #[test]
    fn renders_each_violation() {
        let s = render_gate_report(&[v("no-stubs", "src/a.ts", 12, "critical")], "staged", 0);
        assert!(
            s.contains("src/a.ts") && s.contains("12") && s.contains("CRITICAL"),
            "report:\n{s}"
        );
    }

    #[test]
    fn baselined_footer_present_when_nonzero() {
        let s = render_gate_report(&[v("x", "f", 1, "high")], "staged", 3);
        assert!(s.contains('3'));
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(render_gate_report(&[], "file", 0), "");
    }
}
