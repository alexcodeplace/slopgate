//! Regex violation scanner — port of `src/regex-engine.mjs` (`scanRegex`).
//! Uses `fancy-regex` for lookaround/backref parity with JS RegExp.

use crate::config::ResolvedConfig;
use crate::glob::path_matches_globs;
use crate::report::Violation;
use crate::rules::packs::Pattern;
use fancy_regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::path::Path;

struct CompiledPattern<'a> {
    pattern: &'a Pattern,
    re: Regex,
}

struct LineHit {
    line: u32,
    text: String,
}

/// Replace JS non-`u` ASCII shorthands so `\d`/`\w`/`\s` match ASCII under fancy-regex.
fn asciiize_shorthands(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let bytes = pattern.as_bytes();
    let mut i = 0;
    let mut in_class = false;
    // Byte index where an immediate `]` is literal (right after `[` or `[^`).
    let mut class_literal_close: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && !is_escaped(pattern, i) {
            let esc = bytes[i + 1];
            let rep = if in_class {
                match esc {
                    b'd' => Some("0-9"),
                    b'D' => Some("\x00-\x2F\x3A-\u{10FFFF}"),
                    b'w' => Some("0-9A-Za-z_"),
                    b'W' => Some("\x00-\x2F\x3A-\x40\x5B-\x5E\x60-\u{10FFFF}"),
                    b's' => Some(" \t\n\r\x0c\x0b"),
                    b'S' => Some("\x00-\x08\x0B\x0C\x0E-\x1F\x21-\u{10FFFF}"),
                    _ => None,
                }
            } else {
                match esc {
                    b'd' => Some("[0-9]"),
                    b'D' => Some("[^0-9]"),
                    b'w' => Some("[0-9A-Za-z_]"),
                    b'W' => Some("[^0-9A-Za-z_]"),
                    b's' => Some("[\\t\\n\\r\\f\\v ]"),
                    b'S' => Some("[^\\t\\n\\r\\f\\v ]"),
                    _ => None,
                }
            };
            if let Some(r) = rep {
                out.push_str(r);
                i += 2;
                continue;
            }
        }
        let ch = pattern[i..].chars().next().unwrap();
        if !is_escaped(pattern, i) {
            if ch == '[' && !in_class {
                in_class = true;
                let mut start = i + ch.len_utf8();
                if bytes.get(start) == Some(&b'^') {
                    start += 1;
                }
                class_literal_close = Some(start);
            } else if ch == ']' && in_class {
                if class_literal_close != Some(i) {
                    in_class = false;
                    class_literal_close = None;
                }
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_escaped(s: &str, pos: usize) -> bool {
    let mut slashes = 0usize;
    let mut i = pos;
    while i > 0 {
        i -= 1;
        if s.as_bytes()[i] == b'\\' {
            slashes += 1;
        } else {
            break;
        }
    }
    slashes % 2 == 1
}

/// Translate JS `\uXXXX` (incl. surrogate pairs) to Rust `\u{...}` for fancy-regex.
fn translate_js_unicode_escapes(pattern: &str) -> String {
    let bytes = pattern.as_bytes();
    let mut out = String::with_capacity(pattern.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'u' {
            if i + 2 < bytes.len() && bytes[i + 2] == b'{' {
                let start = i;
                i += 3;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                out.push_str(&pattern[start..i]);
                continue;
            }
            if i + 6 <= bytes.len() {
                let hex = &pattern[i + 2..i + 6];
                if hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    let code = u32::from_str_radix(hex, 16).unwrap_or(0);
                    if (0xD800..=0xDBFF).contains(&code) && i + 12 <= bytes.len() {
                        let rest = &pattern[i + 6..];
                        if rest.starts_with("\\u") {
                            let low_hex = &rest[2..6];
                            if low_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                                let low = u32::from_str_radix(low_hex, 16).unwrap_or(0);
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined =
                                        0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    out.push_str(&format!("\\u{{{combined:X}}}"));
                                    i += 12;
                                    continue;
                                }
                            }
                        }
                    }
                    out.push_str(&format!("\\u{{{code:X}}}"));
                    i += 6;
                    continue;
                }
            }
        }
        out.push(pattern[i..].chars().next().unwrap());
        i += pattern[i..].chars().next().unwrap().len_utf8();
    }
    out
}

/// Compile a rule regex for line-by-line scanning (never stateful). Never panics.
pub fn compile_line_regex(pattern: &str, flags: &str) -> Result<Regex, String> {
    let safe: String = flags
        .chars()
        .filter(|c| *c != 'g' && *c != 'y')
        .collect();
    let mut body = translate_js_unicode_escapes(pattern);
    let unicode = safe.contains('u');
    if !unicode {
        body = asciiize_shorthands(&body);
    }
    let mut builder = RegexBuilder::new(&body);
    builder.case_insensitive(safe.contains('i'));
    // fancy-regex cannot disable Unicode globally (`(?-u)` unsupported); keep Unicode on
    // for `[^…]` negated classes and ASCIIize shorthands when JS `u` is absent.
    builder.unicode_mode(true);
    builder.dot_matches_new_line(safe.contains('s'));
    builder.multi_line(safe.contains('m'));
    builder.build().map_err(|e| e.to_string())
}

fn regex_matches(re: &Regex, line: &str) -> bool {
    re.is_match(line).unwrap_or(false)
}

fn violation_text(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.encode_utf16().count() <= 90 {
        return trimmed.to_string();
    }
    let mut units = 0usize;
    trimmed
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 90
        })
        .collect()
}

/// Two-pass regex scan mirroring `scanRegex` in `regex-engine.mjs`.
pub fn scan_regex(config: &ResolvedConfig, files: &[String], file_mode: bool) -> Vec<Violation> {
    let mut compiled = Vec::new();
    for p in &config.patterns {
        let min_files = p.min_files.unwrap_or(1);
        if file_mode && min_files > 1 {
            continue;
        }
        if let Ok(re) = compile_line_regex(&p.pattern, p.flags.as_deref().unwrap_or("")) {
            compiled.push(CompiledPattern { pattern: p, re });
        }
    }

    // pass 1: one read per file; hits per pattern id → file → line hits
    let mut hits: HashMap<&str, HashMap<&str, Vec<LineHit>>> = HashMap::new();
    for file in files {
        let path = Path::new(&config.repo_root).join(file);
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = contents.split('\n').collect();

        for cp in &compiled {
            let p = cp.pattern;
            let include = p.include_globs.as_deref().unwrap_or(&[]);
            if !include.is_empty() && !path_matches_globs(file, include) {
                continue;
            }
            let exclude = p.exclude_globs.as_deref().unwrap_or(&[]);
            if path_matches_globs(file, exclude) {
                continue;
            }

            let mut per_file: Vec<LineHit> = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                if regex_matches(&cp.re, line) {
                    per_file.push(LineHit {
                        line: (i + 1) as u32,
                        text: (*line).to_string(),
                    });
                }
            }
            if !per_file.is_empty() {
                hits.entry(&p.id)
                    .or_default()
                    .insert(file.as_str(), per_file);
            }
        }
    }

    // pass 2: min_files threshold + expand to violations
    let mut violations = Vec::new();
    for cp in &compiled {
        let p = cp.pattern;
        let min_files = p.min_files.unwrap_or(1);
        let Some(by_file) = hits.get(p.id.as_str()) else {
            continue;
        };
        if by_file.len() < min_files as usize {
            continue;
        }

        let mut files_sorted: Vec<&str> = by_file.keys().copied().collect();
        files_sorted.sort();

        for file in files_sorted {
            let Some(hits_for_file) = by_file.get(file) else {
                continue;
            };
            for hit in hits_for_file {
                violations.push(Violation {
                    id: p.id.clone(),
                    severity: p.severity.clone(),
                    category: p.category.clone().unwrap_or_default(),
                    file: file.to_string(),
                    line: hit.line,
                    full_line: hit.text.clone(),
                    text: violation_text(&hit.text),
                    resolution: p.resolution.clone(),
                    engine: "regex".to_string(),
                });
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::line_hash;
    use crate::rules::packs::{self, Pattern};
    use serde_json::Value;
    use std::fs;
    use tempfile::TempDir;

    fn vectors(name: &str) -> Value {
        let p = format!("{}/tests/parity_vectors/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    fn cfg_path() -> String {
        format!(
            "{}/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn compile_lookahead_pattern() {
        let packs = packs::baseline_packs();
        let p = packs["raw-hex"]
            .iter()
            .find(|r| r.id == "raw-rgb-color")
            .unwrap();
        assert!(compile_line_regex(&p.pattern, p.flags.as_deref().unwrap_or("")).is_ok());
    }

    #[test]
    fn compile_lookbehind_pattern() {
        let packs = packs::stack_packs();
        let p = packs["cloudflare"]
            .iter()
            .find(|r| r.id == "waituntil-bare-method-ref")
            .unwrap();
        assert!(compile_line_regex(&p.pattern, p.flags.as_deref().unwrap_or("")).is_ok());
    }

    #[test]
    fn non_u_d_matches_ascii_digit_not_arabic_indic() {
        let re = compile_line_regex(r"\d", "").unwrap();
        assert!(regex_matches(&re, "0"));
        assert!(!regex_matches(&re, "\u{0660}"));
    }

    #[test]
    fn non_u_class_d_matches_ascii_not_arabic_indic() {
        let re = compile_line_regex(r"[\d]", "").unwrap();
        assert!(regex_matches(&re, "0"));
        assert!(!regex_matches(&re, "\u{0660}"));
    }

    #[test]
    fn i_flag_case_insensitive() {
        let re = compile_line_regex("foo", "i").unwrap();
        assert!(regex_matches(&re, "FOO"));
        assert!(regex_matches(&re, "foo"));
    }

    #[test]
    fn u_flag_pattern_compiles() {
        let packs = packs::ux_packs();
        let p = packs["taste"]
            .regex
            .iter()
            .find(|r| r.id == "ux-emoji-in-ui")
            .unwrap();
        let re = compile_line_regex(&p.pattern, p.flags.as_deref().unwrap_or("")).unwrap();
        assert!(regex_matches(&re, "const label = \"🚀 Launch\";"));
    }

    #[test]
    fn regex_compat_matches_js_oracle() {
        for entry in vectors("regex_compat.json").as_array().unwrap() {
            let pattern = entry["pattern"].as_str().unwrap();
            let flags = entry["flags"].as_str().unwrap_or("");
            let id = entry["id"].as_str().unwrap();
            let re = compile_line_regex(pattern, flags)
                .unwrap_or_else(|e| panic!("compile {id}: {e}"));
            for case in entry["cases"].as_array().unwrap() {
                let line = case["line"].as_str().unwrap();
                let expect = case["match"].as_bool().unwrap();
                assert_eq!(
                    regex_matches(&re, line),
                    expect,
                    "id={id} line={line:?}"
                );
            }
        }
    }

    #[test]
    fn scan_regex_finds_known_violation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/bad.ts"), "const x = foo as any;\n").unwrap();

        let config = resolve_config_str_with_root(
            root,
            &fs::read_to_string(cfg_path()).unwrap(),
        );

        let files = vec!["src/bad.ts".to_string()];
        let violations = scan_regex(&config, &files, false);
        let v = violations
            .iter()
            .find(|v| v.id == "as-any-cast")
            .expect("as-any-cast violation");
        assert_eq!(v.line, 1);
        assert_eq!(v.severity, "high");
        assert_eq!(v.engine, "regex");
        assert_eq!(line_hash(&v.full_line), line_hash("const x = foo as any;"));
        assert_eq!(v.text, "const x = foo as any;");
    }

    #[test]
    fn scan_regex_file_mode_skips_min_files_gt_one() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "eval(x);\n").unwrap();
        fs::write(root.join("src/b.ts"), "eval(y);\n").unwrap();

        let mut config = resolve_config_str_with_root(
            root,
            r#"
baseline = []
rules = []
[gate]
file = ["critical"]
staged = ["critical"]
"#,
        );
        config.patterns.push(Pattern {
            id: "cross-file-eval".into(),
            severity: "critical".into(),
            pattern: r"eval\(".into(),
            resolution: "no eval".into(),
            title: None,
            description: None,
            category: Some("test".into()),
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: None,
            min_files: Some(2),
        });

        let files = vec!["src/a.ts".to_string()];
        let violations = scan_regex(&config, &files, true);
        assert!(violations.is_empty(), "min_files>1 must be skipped in file_mode");
    }

    #[test]
    fn scan_regex_include_exclude_globs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("tokens")).unwrap();
        fs::write(root.join("src/a.ts"), "color: #ff0044;\n").unwrap();
        fs::write(root.join("tokens/a.ts"), "color: #ff0044;\n").unwrap();

        let mut config = resolve_config_str_with_root(
            root,
            r#"
baseline = []
rules = []
[gate]
file = ["high"]
staged = ["high"]
"#,
        );
        config.patterns.push(Pattern {
            id: "raw-hex-color".into(),
            severity: "high".into(),
            pattern: r"#[0-9a-fA-F]{3,8}\b".into(),
            resolution: "token".into(),
            title: None,
            description: None,
            category: Some("convention".into()),
            flags: None,
            canary: None,
            negative_canary: None,
            include_globs: None,
            exclude_globs: Some(vec!["**/tokens/**".into()]),
            min_files: None,
        });

        let files = vec!["src/a.ts".to_string(), "tokens/a.ts".to_string()];
        let violations = scan_regex(&config, &files, false);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].file, "src/a.ts");
    }

    #[test]
    fn scan_regex_skips_unreadable_file_without_panic() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/ok.ts"), "const x = foo as any;\n").unwrap();

        let config = resolve_config_str_with_root(
            root,
            &fs::read_to_string(cfg_path()).unwrap(),
        );

        let files = vec![
            "src/missing.ts".to_string(),
            "src/ok.ts".to_string(),
        ];
        let violations = scan_regex(&config, &files, false);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].file, "src/ok.ts");
    }

    fn resolve_config_str_with_root(root: &std::path::Path, toml: &str) -> ResolvedConfig {
        use crate::config::resolve_config_str;
        let mut config = resolve_config_str(toml).unwrap();
        config.repo_root = root.to_string_lossy().into_owned();
        config
    }
}
