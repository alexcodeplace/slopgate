//! Gate report formatting (DISPLAY surface). Returns `String` for testability; the bin prints it later.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};

const R: &str = "\x1b[31m";
const Y: &str = "\x1b[33m";
const B: &str = "\x1b[1m";
const D: &str = "\x1b[2m";
const Z: &str = "\x1b[0m";

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

/// Render the human-facing gate report (byte-for-byte mirror of `src/report.mjs` including ANSI).
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
    out.push_str(B);
    out.push_str(R);
    out.push_str("╔═ SLOPGATE ═════════════════════════════════════════╗");
    out.push_str(Z);
    out.push('\n');
    out.push_str(B);
    out.push_str(R);
    out.push_str("║ ");
    out.push_str(title);
    out.push_str("║");
    out.push_str(Z);
    out.push('\n');
    out.push_str(B);
    out.push_str(R);
    out.push_str("╚═════════════════════════════════════════════════════╝");
    out.push_str(Z);
    out.push_str("\n\n");

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
            out.push_str(B);
            out.push_str("── ");
            out.push_str(group);
            out.push_str(" ──");
            out.push_str(Z);
            out.push('\n');
        }
        let c = if v.severity == "critical" { R } else { Y };
        let sev = v.severity.to_uppercase();
        out.push_str(B);
        out.push_str(c);
        out.push('[');
        out.push_str(&sev);
        out.push(']');
        out.push_str(Z);
        out.push(' ');
        out.push_str(B);
        out.push_str(&v.id);
        out.push_str(Z);
        out.push(' ');
        out.push_str(D);
        out.push_str(&v.file);
        out.push(':');
        out.push_str(&v.line.to_string());
        out.push_str(Z);
        out.push('\n');
        out.push_str("  ");
        out.push_str(D);
        out.push('×');
        out.push_str(Z);
        out.push(' ');
        out.push_str(&v.text);
        out.push('\n');
        out.push_str("  ");
        out.push_str(B);
        out.push('✓');
        out.push_str(Z);
        out.push(' ');
        out.push_str(&v.resolution);
        out.push_str("\n\n");
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
    out.push_str(B);
    out.push_str(&violations.len().to_string());
    out.push_str(" violation(s) in ");
    out.push_str(&file_count.to_string());
    out.push_str(" file(s). ");
    out.push_str(tail);
    out.push_str(Z);
    out.push('\n');
    if baselined > 0 {
        out.push_str(D);
        out.push_str(&baselined.to_string());
        out.push_str(" pre-existing (baselined) violation(s) ignored.");
        out.push_str(Z);
        out.push('\n');
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
    fn renders_staged_report_with_ansi() {
        let s = render_gate_report(&[v("no-stubs", "src/a.ts", 12, "critical")], "staged", 0);
        let expected = concat!(
            "\n",
            "\x1b[1m\x1b[31m╔═ SLOPGATE ═════════════════════════════════════════╗\x1b[0m\n",
            "\x1b[1m\x1b[31m║ VIOLATIONS IN STAGED FILES — COMMIT BLOCKED         ║\x1b[0m\n",
            "\x1b[1m\x1b[31m╚═════════════════════════════════════════════════════╝\x1b[0m\n\n",
            "\x1b[1m── regex ──\x1b[0m\n",
            "\x1b[1m\x1b[31m[CRITICAL]\x1b[0m \x1b[1mno-stubs\x1b[0m \x1b[2msrc/a.ts:12\x1b[0m\n",
            "  \x1b[2m×\x1b[0m msg\n",
            "  \x1b[1m✓\x1b[0m fix\n\n",
            "\x1b[1m1 violation(s) in 1 file(s). Fix → retry commit.\x1b[0m\n",
            "False positive? NEVER edit suppressions.json yourself — ask the user via AskUserQuestion.\n\n",
        );
        assert_eq!(s, expected);
    }

    #[test]
    fn renders_file_mode_header_with_ansi() {
        let s = render_gate_report(&[v("x", "f.ts", 1, "high")], "file", 0);
        assert!(s.starts_with(concat!(
            "\n",
            "\x1b[1m\x1b[31m╔═ SLOPGATE ═════════════════════════════════════════╗\x1b[0m\n",
            "\x1b[1m\x1b[31m║ SLOPGATE — VIOLATIONS IN EDITED FILE               ║\x1b[0m\n",
        )));
        assert!(s.contains(concat!("\x1b[1m\x1b[33m[HIGH]\x1b[0m \x1b[1mx\x1b[0m")));
        assert!(s.contains("Fix now while context is hot."));
    }

    #[test]
    fn baselined_footer_present_when_nonzero() {
        let s = render_gate_report(&[v("x", "f", 1, "high")], "staged", 3);
        assert!(s.contains(concat!(
            "\x1b[2m3 pre-existing (baselined) violation(s) ignored.\x1b[0m\n"
        )));
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(render_gate_report(&[], "file", 0), "");
    }
}
