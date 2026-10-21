//! What the pipeline found about a task: its checks after the worker said
//! it was done, and the reviewer's verdict. Stored on the task as JSON and
//! shown on the board.

use serde::{Deserialize, Serialize};

/// The checks after a worker's turn: the scope, the tests, the commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskChecks {
    pub scope: ScopeCheck,
    /// `None` when the tests didn't run because the scope check failed.
    pub tests: Option<TestsCheck>,
    /// Commits on the task branch since its base.
    #[serde(default)]
    pub commits_ahead: u32,
}

/// The scope check, as `cos_core::taskgit::ScopeReport` found it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScopeCheck {
    pub ok: bool,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    #[serde(default)]
    pub protected: Vec<String>,
    #[serde(default)]
    pub uncommitted: Vec<String>,
    /// The branch the worktree was on, if it wasn't the task branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrong_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestsCheck {
    pub status: TestsStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The end of the output, stdout and stderr together.
    #[serde(default)]
    pub output_tail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestsStatus {
    Passed,
    Failed,
    TimedOut,
    /// The task has no test command.
    Skipped,
}

impl TestsStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Skipped => "skipped",
        }
    }
}

/// The reviewer's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReview {
    pub verdict: ReviewVerdict,
    /// What to fix, for `changes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The reviewer agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    Changes,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Changes => "changes",
        }
    }
}
