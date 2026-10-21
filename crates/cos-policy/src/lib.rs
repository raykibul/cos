//! The approvals policy: who may answer an agent's permission request.
//!
//! - [`classify`] sorts an [`ApprovalRequest`] into a [`Tier`] with a reason:
//!   always allowed by Rust rules, the chief may approve it, ask you, or
//!   never delegated.
//! - [`Limits::route`] turns that into who answers, given the run's
//!   [`Preset`], the per-run cap and the asking agent's [`Role`].
//!   [`Limits::apply_chief_decision`] checks the chief's
//!   `approval_decide` call before it takes effect.
//! - [`timeout`] refuses requests nobody answers within five minutes.
//! - [`audit`] records every answer and renders the report section
//!   "Approved on your behalf (N)".
//!
//! Everything here is pure: no database, no clock, no daemon. The only disk
//! access is the optional [`Filesystem`] probe for symlinks. The rules are
//! described in `docs/approvals.md`.

pub mod audit;
mod bash;
mod classify;
mod limits;
mod paths;
mod request;
mod shell;
pub mod timeout;
mod verdict;

pub use audit::{AuditRecord, Decider, Outcome, render_report_section, summarize};
pub use classify::{classify, classify_with};
pub use limits::{
    ChiefAnswer, ChiefDecision, ChiefRefusal, DEFAULT_CHIEF_CAP, Limits, Preset, Route, RunLedger,
};
pub use paths::{Filesystem, Lexical, PathProbe};
pub use request::{ApprovalRequest, Role};
pub use verdict::{Needs, Tier, Verdict};
