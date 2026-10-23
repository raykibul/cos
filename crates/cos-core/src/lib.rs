//! Core state for cos: storage, the event log, projects with their
//! workspaces, the agents running in them, the tailnet devices allowed to
//! reach the daemon, and the Chief of Staff's books.

pub mod agents;
mod cities;
pub mod devices;
pub mod env;
pub mod events;
mod git;
pub mod judge;
pub mod projects;
pub mod staff;
mod store;

pub use agents::ApprovalTier;
pub use agents::{
    Agent, AgentState, Agents, Approval, ApprovalState, DecidedBy, Launch, NewAgent, NewApproval, Role,
};
pub use devices::{Device, DeviceError, DeviceState, Devices, SeenDevice};
pub use env::{EnvError, Environment, GlobalEnv, ProjectEnv, Variables};
pub use events::{EventLog, LoggedEvent, Subscription};
pub use judge::{DecisionPoint, Judgments, NewDecision, PointStats, ProjectSwitch};
pub use projects::{Base, Project, ProjectError, Projects, Workspace};
pub use staff::{StaffBooks, StaffError, StaffItem, StaffItemKind, StaffRecord};
pub use store::{Store, StoreError};
