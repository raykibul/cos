//! What an agent asked to do, in a form that doesn't depend on how it asked.
//! Claude's `PermissionRequest` hook and its headless `can_use_tool` message
//! both carry a tool name and its input; the daemon fills in the rest from the
//! run it knows the agent belongs to.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One permission request, ready to classify.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Claude's tool name: `Bash`, `Read`, `Edit`, `WebFetch`, `mcp__server__tool`, …
    pub tool: String,
    /// The tool's input exactly as the agent sent it, for example Bash's
    /// `{command, description}` or Write's `{file_path, content}`.
    pub input: Value,
    /// The directory the tool runs in. For Bash this is the shell's current
    /// directory, which can differ from where the agent started.
    pub cwd: PathBuf,
    /// The worktree the agent works in. Nothing outside it is ever approved
    /// without you.
    pub worktree_root: PathBuf,
    /// The files the task may change, as globs relative to the worktree root
    /// (`src/limits/**`, `docs/*.md`). `None` means the whole worktree.
    #[serde(default)]
    pub scope: Option<Vec<String>>,
    #[serde(default)]
    pub role: Role,
    /// The task's test command, such as `cargo test -p cos-core`. Running it
    /// (with extra arguments) inside the worktree is always allowed.
    #[serde(default)]
    pub test_command: Option<String>,
    /// Your home folder, used to expand `~` and `$HOME`. Without it, any path
    /// under `~` counts as outside the worktree.
    #[serde(default)]
    pub home: Option<PathBuf>,
}

impl ApprovalRequest {
    /// A worker's request with no task scope, test command or home folder.
    pub fn new(
        tool: impl Into<String>,
        input: Value,
        cwd: impl Into<PathBuf>,
        worktree_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            tool: tool.into(),
            input,
            cwd: cwd.into(),
            worktree_root: worktree_root.into(),
            scope: None,
            role: Role::Worker,
            test_command: None,
            home: None,
        }
    }
}

/// Which kind of agent asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Works on one task in its own worktree.
    #[default]
    Worker,
    /// Plans and coordinates. It can't approve its own requests.
    Chief,
    /// Reads a worker's diff. It has no reason to edit files.
    Reviewer,
    /// A session you started and are watching, so its requests come to you.
    UserSession,
}
