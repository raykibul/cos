//! Core state for cos: storage, the event log, and projects with their
//! workspaces. The board, task states and the rest of the git work land in
//! later milestones (see `docs/proposal.md`).

mod cities;
pub mod events;
mod git;
pub mod projects;
mod store;

pub use events::{EventLog, LoggedEvent, Subscription};
pub use projects::{Base, Project, ProjectError, Projects, Workspace};
pub use store::{Store, StoreError};
