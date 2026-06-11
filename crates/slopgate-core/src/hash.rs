//! Hashing primitives. line_hash = sha1 (suppression key); fingerprint = sha256→16hex (ratchet key).
use regex::Regex;
use sha1::Sha1;
use sha2::{Digest as Sha2Digest, Sha256};

/// sha1-hex of the trimmed line. Mirrors suppressions.mjs lineHash.
pub fn line_hash(line: &str) -> String {
    let mut h = Sha1::new();
    h.update(line.trim().as_bytes());
    hex(&h.finalize())
}

/// Replace each maximal run of ASCII digits with a single '#'. Mirrors /\d+/g → '#'.
fn digit_norm(s: &str) -> String {
    Regex::new(r"(?-u:\d)+")
        .expect("digit regex")
        .replace_all(s, "#")
        .into_owned()
}

/// sha256-hex(engine|id|file|digitNorm(text)|trim(full_line)).slice(0,16). Mirrors ratchet.mjs.
pub fn fingerprint(
    engine: &str,
    id: &str,
    file: &str,
    text: &str,
    full_line: &str,
    file_override: Option<&str>,
) -> String {
    let key = [
        engine,
        id,
        file_override.unwrap_or(file),
        &digit_norm(text),
        full_line.trim(),
    ]
    .join("|");
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    hex(&h.finalize())[..16].to_string()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors(name: &str) -> Value {
        let p = format!("{}/tests/parity_vectors/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn line_hash_matches_js_oracle() {
        for case in vectors("line_hash.json").as_array().unwrap() {
            let line = case["line"].as_str().unwrap();
            assert_eq!(line_hash(line), case["hash"].as_str().unwrap(), "line={line:?}");
        }
    }

    #[test]
    fn line_hash_trims_before_hashing() {
        assert_eq!(line_hash("  x  "), line_hash("x")); // happy: trim parity
        assert_ne!(line_hash("x"), line_hash("y")); // distinct content
    }

    #[test]
    fn fingerprint_matches_js_oracle() {
        for case in vectors("fingerprint.json").as_array().unwrap() {
            let v = &case["v"];
            let got = fingerprint(
                v["engine"].as_str().unwrap_or(""),
                v["id"].as_str().unwrap(),
                v["file"].as_str().unwrap(),
                v["text"].as_str().unwrap_or(""),
                v["fullLine"].as_str().unwrap_or(""),
                None,
            );
            assert_eq!(got, case["fp"].as_str().unwrap());
            assert_eq!(got.len(), 16);
        }
    }

    #[test]
    fn fingerprint_digit_normalized() {
        // unhappy/edge: line numbers in text differ but fingerprint identical (fullLine unchanged)
        let a = fingerprint("regex", "r", "f", "err 12", "x", None);
        let b = fingerprint("regex", "r", "f", "err 999", "x", None);
        assert_eq!(a, b);
    }
}
