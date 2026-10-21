//! Your limits on the chief: which tiers it may answer under each preset, a
//! cap on how many requests it approves per run, and the check every chief
//! decision passes before it takes effect. A manipulated chief can ask for
//! anything; only what these rules permit is applied.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::request::Role;
use crate::verdict::{Tier, Verdict};

/// The suggested cap on chief approvals per run.
pub const DEFAULT_CHIEF_CAP: u32 = 20;

/// How much you let the chief approve, chosen per run and changeable while
/// it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    /// Only you approve.
    Strict,
    /// The chief approves actions contained in the worktree: running
    /// scripts, builds, edits outside the task's scope.
    #[default]
    Normal,
    /// The chief also approves network GETs and installs into the worktree.
    Trusted,
}

/// A run's limits on the chief.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub preset: Preset,
    /// After this many chief approvals in the run, everything goes to you.
    pub chief_cap: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            preset: Preset::Normal,
            chief_cap: DEFAULT_CHIEF_CAP,
        }
    }
}

/// What a run has used of its limits. The daemon stores it with the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunLedger {
    pub chief_approvals: u32,
}

/// Who should answer a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Route {
    /// Rust rules allow it; answer straight away.
    Allow,
    /// Ask the chief through `approval_decide`.
    Chief,
    /// Ask you, and say why it didn't go to the chief.
    User { why: String },
}

/// The chief's answer, as its `approval_decide(id, allow|deny, reason)` tool
/// sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChiefDecision {
    pub answer: ChiefAnswer,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChiefAnswer {
    Allow,
    Deny,
}

/// Why a chief decision wasn't applied. The request stays open and goes to
/// you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChiefRefusal {
    pub why: String,
}

impl fmt::Display for ChiefRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.why)
    }
}

impl std::error::Error for ChiefRefusal {}

impl Limits {
    /// Who answers a request with this verdict from an agent with this role,
    /// given what the run has used.
    pub fn route(&self, verdict: &Verdict, role: Role, ledger: &RunLedger) -> Route {
        let user = |why: &str| Route::User { why: why.to_owned() };
        match verdict.tier {
            Tier::Allowed => Route::Allow,
            Tier::NeverDelegated => user("only you can approve this"),
            Tier::AskUser => user("cos can't tell confidently what this does"),
            Tier::Chief { needs } => {
                if role == Role::Chief {
                    user("the chief can't approve its own requests")
                } else if role == Role::UserSession {
                    user("you started this session")
                } else if self.preset == Preset::Strict {
                    user("the run's preset is Strict")
                } else if self.preset == Preset::Normal && needs.install {
                    user("installing packages needs the Trusted preset")
                } else if self.preset == Preset::Normal && needs.network {
                    user("network access needs the Trusted preset")
                } else if ledger.chief_approvals >= self.chief_cap {
                    user("the chief has used all its approvals for this run")
                } else {
                    Route::Chief
                }
            }
        }
    }

    /// Checks a chief decision against the limits and applies it to the
    /// ledger. Deny is always accepted. Allow is accepted only when the
    /// request would route to the chief right now, with a reason; it then
    /// counts against the cap. Re-checking here (not only when the request
    /// was routed) means a preset change or a cap reached in the meantime
    /// still holds.
    pub fn apply_chief_decision(
        &self,
        verdict: &Verdict,
        role: Role,
        decision: &ChiefDecision,
        ledger: &mut RunLedger,
    ) -> Result<(), ChiefRefusal> {
        if decision.answer == ChiefAnswer::Deny {
            return Ok(());
        }
        if decision.reason.trim().is_empty() {
            return Err(ChiefRefusal {
                why: "an approval needs a reason for the report".to_owned(),
            });
        }
        match self.route(verdict, role, ledger) {
            Route::Chief => {
                ledger.chief_approvals += 1;
                Ok(())
            }
            Route::Allow => Ok(()),
            Route::User { why } => Err(ChiefRefusal {
                why: format!("the chief may not approve this: {why}"),
            }),
        }
    }
}
