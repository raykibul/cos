//! Core state for cos: storage, the event log, projects with their
//! workspaces, and the tailnet devices allowed to reach the daemon. The board, task states and the rest of the git work land in
//! later milestones (see `docs/proposal.md`).

mod cities;
pub mod devices;
pub mod events;
mod git;
pub mod projects;
mod store;

pub use devices::{Device, DeviceError, DeviceState, Devices, SeenDevice};
pub use events::{EventLog, LoggedEvent, Subscription};
pub use projects::{Base, Project, ProjectError, Projects, Workspace};
pub use store::{Store, StoreError};
