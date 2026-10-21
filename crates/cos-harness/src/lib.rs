//! Runs coding agents as child processes and turns what they print into [`Event`]s.
//!
//! Only Claude Code is supported so far (see [`claude`]). Codex comes later; that is
//! when the shared surface (start, send, answer, interrupt, events) becomes a trait.

pub mod claude;
mod error;
mod event;

pub use claude::{
    ClaudeOptions, ClaudeSession, ClaudeTerminal, HookTarget, PermissionMode, TerminalAttach, TerminalEvent,
    TerminalOptions, TerminalUpdate,
};
pub use error::HarnessError;
pub use event::{Decision, Event, PermissionRequest, TurnResult};
