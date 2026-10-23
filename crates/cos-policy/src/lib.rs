//! The approvals policy: who may answer an agent's permission request.
//!
//! - [`classify`] sorts an [`ApprovalRequest`] into a [`Tier`] with a reason:
//!   always allowed by Rust rules, the chief may approve it, ask you, or
//!   never delegated.
//! - [`timeout`] refuses requests nobody answers within five minutes.
//!
//! You answer every request today; the tier and reason are shown with it,
//! and jev (`cos-judge`) gets them in shadow mode.
//!
//! Everything here is pure: no database, no clock, no daemon. The only disk
//! access is the optional [`Filesystem`] probe for symlinks. The rules are
//! described in `docs/approvals.md`.

mod bash;
mod classify;
mod paths;
mod request;
mod shell;
pub mod timeout;
mod verdict;

pub use classify::{classify, classify_with};
pub use paths::{Filesystem, Lexical, PathProbe};
pub use request::{ApprovalRequest, Role};
pub use verdict::{Needs, Tier, Verdict};
