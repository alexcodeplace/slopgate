//! Behavioral oracles for performance-only changes; no weaker scan policies.
use super::*;

#[test]
fn literal_prefilters_equal_the_original_regex_search() {
    let literals = ["literal", "[a-z]+", "a.b", "\\", "line\nbreak", "中文", "kelvin", "ß"];
    let haystacks = ["", "literal", "a literal suffix", "[a-z]+", "a.b", "aXb", "\\",
        "line\nbreak", "中文", "KELVIN", "KELVIN", "kelvin", "SS", "ẞ", "ß"];
    for value in literals {
        for casei in [false, true] {
            let original = regex::RegexBuilder::new(&regex::escape(value))
                .case_insensitive(casei).build().unwrap();
            let optimized = NecessaryLiteral::new(value.to_owned(), casei).unwrap();
            for text in haystacks {
                assert_eq!(optimized.is_match(text), original.is_match(text),
                    "literal={value:?}, casei={casei}, text={text:?}");
            }
        }
    }
}

#[test]
fn optimized_matchers_preserve_all_shipped_canaries() {
    let mut patterns: Vec<Pattern> = crate::rules::packs::baseline_packs()
        .into_values().flatten().collect();
    patterns.extend(crate::rules::packs::stack_packs().into_values().flatten());
    patterns.extend(crate::rules::packs::ux_packs().into_values().flat_map(|pack| pack.regex));
    for pattern in patterns {
        let flags = pattern.flags.as_deref().unwrap_or("");
        let original = compile_line_regex(&pattern.pattern, flags).unwrap();
        let optimized = compile_matcher(&pattern.pattern, flags).unwrap();
        for text in pattern.canary.iter().chain(pattern.negative_canary.iter().flatten()) {
            for line in text.split('\n') {
                assert_eq!(optimized.is_match(line).unwrap(), original.is_match(line).unwrap(),
                    "rule={}, line={line:?}", pattern.id);
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
