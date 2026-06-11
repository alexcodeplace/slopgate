//! Glob → regex translation matching regex-engine.mjs pathMatchesGlobs EXACTLY.
use regex::Regex;

const META: &[char] = &['.', '+', '^', '$', '{', '}', '(', ')', '|', '[', ']', '\\'];

pub fn path_matches_globs(path: &str, globs: &[String]) -> bool {
    if globs.is_empty() {
        return false;
    }
    globs.iter().any(|g| {
        let re = glob_to_regex(g);
        Regex::new(&re).map(|r| r.is_match(path)).unwrap_or(false)
    })
}

fn glob_to_regex(glob: &str) -> String {
    // 1. escape regex metachars (note: '*' is NOT escaped — handled next)
    let mut s = String::with_capacity(glob.len() * 2);
    for c in glob.chars() {
        if META.contains(&c) {
            s.push('\\');
        }
        s.push(c);
    }
    // 2. sentinel substitutions in the SAME order as the JS chain
    let s = s.replace("**/", "\u{0}").replace("**", "\u{1}");
    let s = s.replace('*', "[^/]*");
    let s = s.replace('\u{0}', "(?:.*/)?").replace('\u{1}', ".*");
    format!("^{s}$")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn glob_matches_js_oracle() {
        let p = format!("{}/tests/parity_vectors/glob.json", env!("CARGO_MANIFEST_DIR"));
        let cases: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        for c in cases.as_array().unwrap() {
            let g = c["glob"].as_str().unwrap();
            let path = c["path"].as_str().unwrap();
            assert_eq!(
                path_matches_globs(path, &[g.to_string()]),
                c["match"].as_bool().unwrap(),
                "glob={g} path={path}"
            );
        }
    }

    #[test]
    fn empty_globs_never_match() {
        assert!(!path_matches_globs("a.ts", &[]));
    }

    #[test]
    fn single_star_does_not_cross_slash() {
        assert!(path_matches_globs("src/x.ts", &["src/*.ts".into()]));
        assert!(!path_matches_globs("src/x/y.ts", &["src/*.ts".into()])); // unhappy
    }
}
