//! Stage-0b durable parity test.
//!
//! The Rust engine's gate output must match the golden files that were frozen
//! from the (now-deleted) JS oracle at the moment of the clean switch. This is
//! the regression guard that survives deletion of the JS engine — there is no
//! live oracle to diff against anymore, so the committed `*.norm` goldens ARE
//! the contract.
//!
//! Regenerating goldens: the JS oracle is gone, so there is no `--write` path.
//! A *deliberate*, reviewed change in Rust gate output is re-blessed by copying
//! this test's normalized actual output over the `*.norm` files — never to make
//! a red test green without understanding why the output moved.
//!
//! Normalization is self-contained below: strip ANSI SGR sequences (`ESC [ … m`)
//! and `<n>ms` / `<n>.<n>ms` timings.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/crates/slopgate-rs
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn ast_grep_available() -> bool {
    Command::new("ast-grep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Remove `[0-9]+(\.[0-9]+)?ms` occurrences (whole match, "ms" included).
fn strip_ms(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '.' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    j = k;
                }
            }
            if j + 1 < chars.len() && chars[j] == 'm' && chars[j + 1] == 's' {
                i = j + 2; // drop the whole "<num>ms"
                continue;
            }
            out.extend(&chars[start..j]);
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn normalize(s: &str) -> String {
    strip_ms(&strip_ansi(s))
}

fn gate_output(repo: &Path, file: &str) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_slopgate-rs"))
        .current_dir(repo)
        .args([
            "--file",
            file,
            "--config",
            "rules/baseline/selftest.config.toml",
        ])
        .output()
        .expect("run slopgate-rs");
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), combined)
}

#[test]
fn gate_output_matches_js_oracle_golden() {
    if !ast_grep_available() {
        eprintln!(
            "SKIP parity_golden: ast-grep not available (AST canary parity cannot be checked)"
        );
        return;
    }
    let repo = repo_root();
    let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for name in ["canary", "ux-ast"] {
        let file = format!("rules/baseline/fixtures/src/{name}.tsx");
        let (code, raw) = gate_output(&repo, &file);
        assert_eq!(code, 1, "{name}: expected exit 1 (violations present)");
        let golden = std::fs::read_to_string(golden_dir.join(format!("{name}.norm")))
            .unwrap_or_else(|e| panic!("read golden {name}.norm: {e}"));
        assert_eq!(
            normalize(&raw),
            golden,
            "{name}: gate output diverges from the frozen golden — \
             investigate WHY before re-blessing the `{name}.norm` file"
        );
    }
}
