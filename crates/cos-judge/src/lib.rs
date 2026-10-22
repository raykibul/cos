//! jev for cos: a fast, cheap judgment layer between the daemon's Rust rules
//! and the chief model. jev is TypeSafe's "System One" model: structured
//! state in, typed probabilistic answers out, never text.
//!
//! - [`JudgeClient`] makes one call per decision point with all of that
//!   point's questions, times out after 1 s, and decodes typed answers
//!   ([`YesNo`], [`Choice`] into a Rust enum, [`Score`]).
//! - Every question and threshold lives in `judge/questions.json`
//!   ([`questions()`]), embedded in the binary.
//! - [`permission`] sorts permission requests; [`wake`] decides when to wake
//!   a chief. Each has a `would_have` that turns answers into an action. On
//!   an error, a timeout or confidence below 0.5 jev has no opinion, and
//!   things go on exactly as without it.
//! - [`key`] finds `TYPESAFE_API_KEY`.
//!
//! cos runs jev in shadow mode: it logs what jev would have done and changes
//! nothing. The design is "jev: a fast judgment layer" in
//! `docs/proposal.md`; how cos uses it is `docs/judge.md`.

mod answers;
mod client;
pub mod key;
pub mod permission;
pub mod questions;
mod trim;
pub mod wake;

pub use answers::{Choice, ChoiceOption, RawAnswers, Score, YesNo};
pub use client::{ENDPOINT, JudgeClient, JudgeError, Judged, MODEL, TIMEOUT};
pub use key::{ApiKey, KEY_ENV, KeySource};
pub use questions::questions;
pub use trim::{cut, input_hash, relative_paths};
