//! Severity domain. Membership-based (mirrors gate.mjs allow-set), not ranked.
use std::collections::HashSet;

/// Known severity labels in the JS engine (`critical|high|medium|low|info`).
pub const CRITICAL: &str = "critical";
pub const HIGH: &str = "high";
pub const MEDIUM: &str = "medium";
pub const LOW: &str = "low";
pub const INFO: &str = "info";

/// Default gate allow-set: `{critical, high}` (mirrors `gate.mjs:98`).
pub fn default_allow() -> HashSet<String> {
    ["critical", "high"].iter().map(|s| s.to_string()).collect()
}

/// A violation gates iff its severity is in the active allow-set for the mode.
pub fn is_allowed(sev: &str, allow: &HashSet<String>) -> bool {
    allow.contains(sev)
}

/// leakscan pass-through map (`leakscan.mjs` SEVERITY_MAP); unknown → dropped.
pub fn map_passthrough(raw: &str) -> Option<&'static str> {
    match raw {
        "critical" => Some("critical"),
        "high" => Some("high"),
        "medium" => Some("medium"),
        "low" => Some("low"),
        "info" => Some("info"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allow_is_critical_high() {
        let a = default_allow();
        assert!(a.contains("critical") && a.contains("high") && !a.contains("medium"));
    }

    #[test]
    fn is_allowed_membership() {
        let a = default_allow();
        assert!(is_allowed("critical", &a));
        assert!(!is_allowed("medium", &a)); // unhappy
    }

    #[test]
    fn passthrough_map() {
        assert_eq!(map_passthrough("medium"), Some("medium"));
        assert_eq!(map_passthrough("bogus"), None);
    }
}
