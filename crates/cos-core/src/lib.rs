//! Core state for cos: storage, the event log, projects with their
//! workspaces, the agents running in them, the tailnet devices allowed to
//! reach the daemon, and runs with their tasks and board. The rest of the
//! git work lands in later milestones (see `docs/proposal.md`).

pub mod agents;
pub mod chief;
mod cities;
pub mod devices;
pub mod events;
mod git;
pub mod projects;
pub mod runs;
mod store;
pub mod taskgit;

pub use agents::{
    Agent, AgentState, Agents, Approval, ApprovalState, DecidedBy, Launch, NewAgent, NewApproval, Role,
};
pub use agents::{ApprovalRoute, ApprovalTier};
pub use chief::{Inbox, InboxItem, InboxKind, Question, QuestionState, Questions, Wakeup};
pub use devices::{Device, DeviceError, DeviceState, Devices, SeenDevice};
pub use events::{EventLog, LoggedEvent, Subscription};
pub use projects::{Base, Project, ProjectError, Projects, Workspace};
pub use runs::{
    ApprovalPreset, Board, NewRun, PlanProposal, ProposedTask, ReviewVerdict, Run, RunError, RunState,
    RunSummary, Runs, ScopeCheck, Task, TaskChecks, TaskCounts, TaskReview, TaskState, TestsCheck,
    TestsStatus, render_board,
};
pub use runs::{LimitsChange, RunLimits};
pub use store::{Store, StoreError};
pub use taskgit::{
    Change, ChangeStatus, DiffSummary, MergeOutcome, Scope, ScopeReport, TaskGitError, TaskWorktree,
    TaskWorktrees, TestOptions, TestOutcome, TestStatus, WorkerLimiter, WorkerPermit,
};
