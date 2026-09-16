//! Tool-specific diagnostic mappings.
/// dependency-cruiser raw severity → slopgate severity (`depcruise.mjs` SEVERITY_MAP).
pub fn map_depcruise(raw: &str) -> Option<&'static str> {
    match raw {
        "error" => Some("critical"),
        "warn" => Some("high"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn depcruise_map() {
        assert_eq!(map_depcruise("error"), Some("critical"));
        assert_eq!(map_depcruise("warn"), Some("high"));
        assert_eq!(map_depcruise("info"), None); // dropped
    }
}
