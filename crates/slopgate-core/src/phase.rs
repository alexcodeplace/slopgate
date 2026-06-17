//! Gate phase and tier metadata.

use std::fmt;

/// Checker tier — mirrors JS `'fast'|'commit'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Fast,
    Commit,
}

impl Tier {
    pub const ALL: [Tier; 2] = [Tier::Fast, Tier::Commit];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Fast => "fast",
            Tier::Commit => "commit",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "fast" => Some(Tier::Fast),
            "commit" => Some(Tier::Commit),
            _ => None,
        }
    }

    pub fn values() -> &'static str {
        "fast|commit"
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle phase for a gate run. A phase is distinct from tier: multiple
/// phases may use commit-tier checkers while carrying different budgets and
/// ratchet policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Edit,
    Commit,
    Push,
    Ci,
    Pr,
    Nightly,
}

impl Phase {
    pub const ALL: [Phase; 6] = [
        Phase::Edit,
        Phase::Commit,
        Phase::Push,
        Phase::Ci,
        Phase::Pr,
        Phase::Nightly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Edit => "edit",
            Phase::Commit => "commit",
            Phase::Push => "push",
            Phase::Ci => "ci",
            Phase::Pr => "pr",
            Phase::Nightly => "nightly",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "edit" => Some(Phase::Edit),
            "commit" => Some(Phase::Commit),
            "push" => Some(Phase::Push),
            "ci" => Some(Phase::Ci),
            "pr" => Some(Phase::Pr),
            "nightly" => Some(Phase::Nightly),
            _ => None,
        }
    }

    pub fn values() -> &'static str {
        "edit|commit|push|ci|pr|nightly"
    }

    pub fn default_tier(self) -> Tier {
        match self {
            Phase::Edit => Tier::Fast,
            Phase::Commit | Phase::Push | Phase::Ci | Phase::Pr | Phase::Nightly => Tier::Commit,
        }
    }

    pub fn default_baseline_filter(self) -> bool {
        match self {
            Phase::Edit | Phase::Nightly => false,
            Phase::Commit | Phase::Push | Phase::Ci | Phase::Pr => true,
        }
    }

    pub fn default_budget_seconds(self) -> u32 {
        match self {
            Phase::Edit => 5,
            Phase::Commit => 30,
            Phase::Push => 60,
            Phase::Ci | Phase::Pr => 120,
            Phase::Nightly => 300,
        }
    }

    pub fn runs_checkers_by_default(self) -> bool {
        self != Phase::Edit
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
