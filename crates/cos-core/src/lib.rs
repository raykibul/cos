//! Core state for cos: storage, the event log, and projects with their
//! workspaces. The board, task states and the rest of the git work land in
//! later milestones (see `docs/proposal.md`).

mod cities;
pub mod events;
mod git;
pub mod projects;
mod store;
pub mod taskgit;

pub use events::{EventLog, LoggedEvent, Subscription};
pub use projects::{Base, Project, ProjectError, Projects, Workspace};
pub use store::{Store, StoreError};
pub use taskgit::{
    Change, ChangeStatus, DiffSummary, MergeOutcome, Scope, ScopeReport, TaskGitError, TaskWorktree,
    TaskWorktrees, TestOptions, TestOutcome, TestStatus, WorkerLimiter, WorkerPermit,
};
