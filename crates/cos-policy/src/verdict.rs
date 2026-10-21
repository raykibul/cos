//! The result of classifying a request: which tier it falls in and why.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Who may answer a request, from least to most restrictive. The order
/// matters: a compound command gets the most restrictive tier of its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum Tier {
    /// Rust rules allow it without asking anyone.
    Allowed,
    /// The chief may approve it, within the run's limits. What `needs` says
    /// decides which presets let the chief answer.
    Chief { needs: Needs },
    /// Only you can answer, because cos can't tell confidently what it does.
    AskUser,
    /// Only you can answer, whatever the preset: pushes, anything outside the
    /// worktree, secrets, settings and permissions.
    NeverDelegated,
}

/// What a chief-tier request touches beyond the worktree's files. Normal lets
/// the chief approve neither; Trusted lets it approve both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Needs {
    /// Talks to the network (GET-style requests only; anything that sends
    /// data is never delegated).
    pub network: bool,
    /// Installs or downloads packages into the worktree.
    pub install: bool,
}

impl Needs {
    /// Only the worktree: running scripts, builds, edits outside the scope.
    pub const WORKTREE: Needs = Needs {
        network: false,
        install: false,
    };
    pub const NETWORK: Needs = Needs {
        network: true,
        install: false,
    };
    pub const INSTALL: Needs = Needs {
        network: true,
        install: true,
    };

    fn union(self, other: Needs) -> Needs {
        Needs {
            network: self.network || other.network,
            install: self.install || other.install,
        }
    }
}

impl Tier {
    fn rank(self) -> u8 {
        match self {
            Tier::Allowed => 0,
            Tier::Chief { needs } => 1 + u8::from(needs.network) + u8::from(needs.install),
            Tier::AskUser => 4,
            Tier::NeverDelegated => 5,
        }
    }

    /// Whether only you can answer, whatever the preset.
    pub fn is_for_user(self) -> bool {
        matches!(self, Tier::AskUser | Tier::NeverDelegated)
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tier::Allowed => f.write_str("always allowed"),
            Tier::Chief { needs } if needs.install => f.write_str("chief (install)"),
            Tier::Chief { needs } if needs.network => f.write_str("chief (network)"),
            Tier::Chief { .. } => f.write_str("chief (worktree)"),
            Tier::AskUser => f.write_str("ask you"),
            Tier::NeverDelegated => f.write_str("never delegated"),
        }
    }
}

/// A tier and the reason for it, written to be shown to you as-is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(flatten)]
    pub tier: Tier,
    pub reason: String,
}

impl Verdict {
    pub fn new(tier: Tier, reason: impl Into<String>) -> Self {
        Self {
            tier,
            reason: reason.into(),
        }
    }

    pub fn allowed(reason: impl Into<String>) -> Self {
        Self::new(Tier::Allowed, reason)
    }

    pub fn chief(needs: Needs, reason: impl Into<String>) -> Self {
        Self::new(Tier::Chief { needs }, reason)
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self::new(Tier::AskUser, reason)
    }

    pub fn never(reason: impl Into<String>) -> Self {
        Self::new(Tier::NeverDelegated, reason)
    }

    /// Combines two parts of one action. The stricter tier wins and keeps its
    /// reason; on a tie the first reason stays. Two chief tiers merge what
    /// they need, so `curl … && npm install` needs both network and install.
    pub fn and(self, other: Verdict) -> Verdict {
        let tier = match (self.tier, other.tier) {
            (Tier::Chief { needs: a }, Tier::Chief { needs: b }) => Tier::Chief { needs: a.union(b) },
            (a, b) if b.rank() > a.rank() => b,
            (a, _) => a,
        };
        let reason = if other.tier.rank() > self.tier.rank() {
            other.reason
        } else {
            self.reason
        };
        Verdict::new(tier, reason)
    }
}
