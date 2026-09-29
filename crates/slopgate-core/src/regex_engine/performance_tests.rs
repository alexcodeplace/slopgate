//! Behavioral oracles for performance-only changes; no weaker scan policies.
use super::*;

#[test]
fn literal_prefilters_equal_the_original_regex_search() {
    let literals = [
        "literal",
        "[a-z]+",
        "a.b",
        "\\",
        "line\nbreak",
        "中文",
        "kelvin",
        "ß",
    ];
    let haystacks = [
        "",
        "literal",
        "a literal suffix",
        "[a-z]+",
        "a.b",
        "aXb",
        "\\",
        "line\nbreak",
        "中文",
        "KELVIN",
        "KELVIN",
        "kelvin",
        "SS",
        "ẞ",
        "ß",
    ];
    for value in literals {
        for casei in [false, true] {
            let original = regex::RegexBuilder::new(&regex::escape(value))
                .case_insensitive(casei)
                .build()
                .unwrap();
            let optimized = NecessaryLiteral::new(value.to_owned(), casei).unwrap();
            for text in haystacks {
                assert_eq!(
                    optimized.is_match(text),
                    original.is_match(text),
                    "literal={value:?}, casei={casei}, text={text:?}"
                );
            }
        }
    }
}

#[test]
fn optimized_matchers_preserve_all_shipped_canaries() {
    let mut patterns: Vec<Pattern> = crate::rules::packs::baseline_packs()
        .into_values()
        .flatten()
        .collect();
    patterns.extend(crate::rules::packs::stack_packs().into_values().flatten());
    patterns.extend(
        crate::rules::packs::ux_packs()
            .into_values()
            .flat_map(|pack| pack.regex),
    );
    for pattern in patterns {
        let flags = pattern.flags.as_deref().unwrap_or("");
        let original = compile_line_regex(&pattern.pattern, flags).unwrap();
        let optimized = compile_matcher(&pattern.pattern, flags).unwrap();
        for text in pattern
            .canary
            .iter()
            .chain(pattern.negative_canary.iter().flatten())
        {
            for line in text.split('\n') {
                assert_eq!(
                    optimized.is_match(line).unwrap(),
                    original.is_match(line).unwrap(),
                    "rule={}, line={line:?}",
                    pattern.id
                );
            }
        }
    }
}

#[test]
fn optimized_matcher_retains_backtracking_failure() {
    let matcher = compile_matcher(r"^(a+)+(?<=a)b", "").unwrap();
    let text = format!("{}!", "a".repeat(512));
    assert!(matcher.is_match(&text).unwrap_err().contains("work limit"));
}

#[test]
fn literal_proofs_preserve_scoped_flags_and_optional_branches() {
    for (pattern, flags, positive, negative) in [
        (r"kelvin(?=!)", "i", "KELVIN!", "kelvin?"),
        (r"(?i:foo)(?=BAR)", "", "FoOBAR", "FoObar"),
        (r"(?:optional)?value(?=!)", "", "value!", "value?"),
        (r"(?:foo|bar)(?=!)", "", "bar!", "baz!"),
        (r"(?<=prefix_)value", "", "prefix_value", "value"),
        (r"(foo|bar)\1", "", "barbar", "foobar"),
    ] {
        let matcher = compile_matcher(pattern, flags).unwrap();
        assert!(matcher.is_match(positive).unwrap(), "{pattern}");
        assert!(!matcher.is_match(negative).unwrap(), "{pattern}");
    }
}

#[test]
fn file_prefilter_has_no_false_negatives_for_shipped_canaries() {
    let mut patterns: Vec<Pattern> = crate::rules::packs::baseline_packs()
        .into_values()
        .flatten()
        .collect();
    patterns.extend(crate::rules::packs::stack_packs().into_values().flatten());
    patterns.extend(
        crate::rules::packs::ux_packs()
            .into_values()
            .flat_map(|pack| pack.regex),
    );
    for pattern in patterns {
        let matcher =
            compile_matcher(&pattern.pattern, pattern.flags.as_deref().unwrap_or("")).unwrap();
        for text in pattern
            .canary
            .iter()
            .chain(pattern.negative_canary.iter().flatten())
        {
            let contents = format!(
                "{}\n{text}\n{}",
                "padding\n".repeat(12),
                "padding\n".repeat(12)
            );
            if contents
                .split('\n')
                .any(|line| matcher.is_match(line).unwrap())
            {
                assert!(
                    matcher.may_match_file(&contents),
                    "rule={}, text={text:?}",
                    pattern.id
                );
            }
        }
    }
}

fn scan_bytes(pattern: &str, contents: &[u8]) -> Result<Vec<Violation>, Vec<String>> {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("input.ts"), contents).unwrap();
    let mut config = crate::config::resolve_config_str("astEnabled=false").unwrap();
    config.repo_root = dir.path().to_string_lossy().into_owned();
    config.patterns.push(
        serde_json::from_value(serde_json::json!({
            "id": "probe", "severity": "high", "resolution": "fixture", "pattern": pattern
        }))
        .unwrap(),
    );
    scan_regex_checked(&config, &["input.ts".into()], false)
}

#[test]
fn file_prefilter_cannot_skip_input_validation() {
    let pattern = r"needle(?=!)";
    assert!(scan_bytes(pattern, &[0xff])
        .unwrap_err()
        .iter()
        .any(|error| error.contains("utf-8")));
    let long_line = "x".repeat(MAX_LINE_BYTES + 1);
    assert!(scan_bytes(pattern, long_line.as_bytes())
        .unwrap_err()
        .iter()
        .any(|error| error.contains("line exceeds")));
    let long_file = "x\n".repeat(MAX_SOURCE_BYTES as usize / 2 + 1);
    assert!(scan_bytes(pattern, long_file.as_bytes())
        .unwrap_err()
        .iter()
        .any(|error| error.contains("source exceeds")));
}

#[test]
fn file_prefilter_preserves_line_scoping_and_error_propagation() {
    let padding = "padding\n".repeat(12);
    let split = format!("{padding}needle\n!\n");
    assert!(scan_bytes(r"needle(?=!)", split.as_bytes())
        .unwrap()
        .is_empty());
    let joined = format!("{padding}needle!\n");
    let findings = scan_bytes(r"needle(?=!)", joined.as_bytes()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 13);
    assert_eq!(findings[0].full_line, "needle!");
    let failing = format!("{padding}BEGIN{}!\n", "a".repeat(512));
    assert!(scan_bytes(r"BEGIN(a+)+(?<=a)b", failing.as_bytes())
        .unwrap_err()
        .iter()
        .any(|error| error.contains("work limit")));
}

#[test]
fn linear_file_prefilter_preserves_anchors_flags_and_optional_literals() {
    let expressions = [
        r"^needle$",
        r"\Aneedle\z",
        r"(?i:needle)",
        r"(?:foo|needle)",
        r"(?:needle)?",
        r"(?:needle){0,2}",
        r"(?:needle){1,2}",
        r"^$",
        r"(?i:foo)(?-i:BAR)",
        r"(?x)needle # ignored comment",
        r"\bneedle\b",
        r"n[eE]edle",
        r"\x6e\u{65}edle",
        r"\p{L}+",
        r"line\nbreak",
    ];
    let contents = [
        "padding\nneedle\npadding\n",
        "padding\nnEEdLe\n",
        "padding\nFOOBAR\n",
        "padding\nFoobar\n",
        "padding\nKELVIN\n",
        "padding\n中文\n",
        "padding\n",
        "line\nbreak\n",
        "padding\nneedleneedle\n",
        "",
        "nothing",
    ];
    for expression in expressions {
        for flags in ["", "i", "m", "s", "u", "im"] {
            let matcher = compile_matcher(expression, flags).unwrap();
            // V1 uses the linear engine for these patterns. fancy-regex differs
            // for global case folding combined with scoped flag removal, so it
            // is not an oracle for the original scanner's linear branch.
            let Matcher::Linear {
                regex: original, ..
            } = &matcher
            else {
                panic!("fixture must use the original linear engine: {expression}");
            };
            for text in contents {
                let matched = text.split('\n').any(|line| original.is_match(line));
                if matched {
                    assert!(
                        matcher.may_match_file(text),
                        "expression={expression}, flags={flags}, text={text:?}"
                    );
                }
            }
        }
    }
    let text = format!("{}needle\n", "padding\n".repeat(4096));
    for expression in [r"^needle$", r"\Aneedle\z"] {
        let findings = scan_bytes(expression, text.as_bytes()).unwrap();
        assert_eq!(findings.len(), 1, "{expression}");
        assert_eq!(findings[0].line, 4097);
    }
}

#[test]
fn linear_prefilters_are_lazy_and_amortized() {
    let matcher = compile_matcher("needle", "i").unwrap();
    let Matcher::Linear { necessary, .. } = &matcher else {
        panic!("fixture must use the linear engine");
    };
    assert!(necessary.get().is_none());
    assert!(!matcher.should_prefilter_file(32, 1));
    assert!(!matcher.should_prefilter_file(32, 2));
    assert!(!matcher.should_prefilter_file(7, 1000));
    assert!(matcher.should_prefilter_file(32, 128));
    assert!(matcher.should_prefilter_file(4096, 1));
    assert!(matcher.is_match("NEEDLE").unwrap());
    assert!(necessary.get().is_none(), "ordinary line matches need no auxiliary proof");
    assert!(!matcher.may_match_file("ordinary text"));
    assert!(necessary.get().is_some());
    assert!(matcher.may_match_file("NEEDLE"));
    let compatibility = compile_matcher(r"needle(?=!)", "i").unwrap();
    assert!(compatibility.should_prefilter_file(8, 1));
    assert!(!compatibility.should_prefilter_file(7, 1000));
}
