//! The board: a run, its tasks and how many are in each state, as JSON for
//! clients and as markdown for `<repo>/.cos/board.md`.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use super::{Run, RunState, Task, TaskState};
use crate::Workspace;

/// A run as the board shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub run: Run,
    /// `None` if the workspace has since been removed.
    pub workspace: Option<Workspace>,
    /// In plan order.
    pub tasks: Vec<Task>,
    pub counts: TaskCounts,
}

impl Board {
    pub fn new(run: Run, workspace: Option<Workspace>, tasks: Vec<Task>) -> Self {
        let mut counts = TaskCounts::default();
        for task in &tasks {
            counts.add(task.state, 1);
        }
        Self {
            run,
            workspace,
            tasks,
            counts,
        }
    }
}

/// How many tasks are in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskCounts {
    pub proposed: usize,
    pub ready: usize,
    pub running: usize,
    pub checking: usize,
    pub review: usize,
    pub merged: usize,
    pub blocked: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub total: usize,
}

impl TaskCounts {
    pub fn get(&self, state: TaskState) -> usize {
        *self.field(state)
    }

    pub(crate) fn add(&mut self, state: TaskState, count: usize) {
        *self.field_mut(state) += count;
        self.total += count;
    }

    /// "2 merged, 1 running, 1 ready (4 total)": furthest along first.
    pub fn summary(&self) -> String {
        if self.total == 0 {
            return "no tasks yet".into();
        }
        let parts: Vec<String> = TaskState::ALL
            .iter()
            .rev()
            .filter(|&&state| self.get(state) > 0)
            .map(|&state| format!("{} {state}", self.get(state)))
            .collect();
        format!("{} ({} total)", parts.join(", "), self.total)
    }

    fn field(&self, state: TaskState) -> &usize {
        match state {
            TaskState::Proposed => &self.proposed,
            TaskState::Ready => &self.ready,
            TaskState::Running => &self.running,
            TaskState::Checking => &self.checking,
            TaskState::Review => &self.review,
            TaskState::Merged => &self.merged,
            TaskState::Blocked => &self.blocked,
            TaskState::Failed => &self.failed,
            TaskState::Cancelled => &self.cancelled,
        }
    }

    fn field_mut(&mut self, state: TaskState) -> &mut usize {
        match state {
            TaskState::Proposed => &mut self.proposed,
            TaskState::Ready => &mut self.ready,
            TaskState::Running => &mut self.running,
            TaskState::Checking => &mut self.checking,
            TaskState::Review => &mut self.review,
            TaskState::Merged => &mut self.merged,
            TaskState::Blocked => &mut self.blocked,
            TaskState::Failed => &mut self.failed,
            TaskState::Cancelled => &mut self.cancelled,
        }
    }
}

/// The board as markdown. Times are UTC, so the output only depends on the board.
pub fn render_board(board: &Board) -> String {
    let run = &board.run;
    let mut out = String::new();
    let _ = writeln!(out, "# Run {}: {}\n", run.id, run.state);
    for line in run.goal.lines() {
        let _ = writeln!(out, "> {line}");
    }
    out.push('\n');

    let workspace = match &board.workspace {
        Some(workspace) => format!("{} (`{}`)", workspace.title, workspace.branch),
        None => format!("#{} (removed)", run.workspace_id),
    };
    let _ = writeln!(out, "- Workspace: {workspace}");
    let _ = writeln!(
        out,
        "- Approvals: {} preset, the chief may approve up to {}",
        run.preset.as_str(),
        run.approval_cap
    );
    let _ = writeln!(out, "- Tasks: {}", board.counts.summary());
    if let Some(report) = &run.report_path {
        let _ = writeln!(out, "- Report: {}", report.display());
    }
    if let Some(reason) = &run.reason {
        let _ = writeln!(out, "- Why it stopped: {}", one_line(reason));
    }
    let _ = writeln!(
        out,
        "- Updated: {}",
        run.updated_at.strftime("%Y-%m-%d %H:%M UTC")
    );

    if board.tasks.is_empty() {
        let note = match run.state {
            RunState::Planning => "No plan yet; the chief is working on it.",
            _ => "This run has no tasks.",
        };
        let _ = writeln!(out, "\n{note}");
        return out;
    }

    out.push_str("\n| # | Task | State | After | Files | Branch |\n");
    out.push_str("|---|------|-------|-------|-------|--------|\n");
    for task in &board.tasks {
        let state = if task.retries > 0 {
            format!("{} (retry {})", task.state, task.retries)
        } else {
            task.state.to_string()
        };
        let after: Vec<String> = task.depends_on.iter().map(|id| format!("#{id}")).collect();
        let files: Vec<String> = task
            .files
            .iter()
            .map(|file| format!("`{}`", cell(file)))
            .collect();
        let branch = task
            .branch
            .as_deref()
            .map(|branch| format!("`{}`", cell(branch)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "| {} | {} | {state} | {} | {} | {branch} |",
            task.id,
            cell(&task.title),
            after.join(", "),
            files.join(", ")
        );
    }

    let notes: Vec<&Task> = board
        .tasks
        .iter()
        .filter(|task| task.last_failure.is_some())
        .collect();
    if !notes.is_empty() {
        out.push_str("\n## Problems\n\n");
        for task in notes {
            let reason = task.last_failure.as_deref().unwrap_or_default();
            let _ = writeln!(
                out,
                "- #{} {} ({}): {}",
                task.id,
                one_line(&task.title),
                task.state,
                one_line(reason)
            );
        }
    }
    out
}

/// Text safe inside a markdown table cell.
fn cell(text: &str) -> String {
    one_line(text).replace('|', "\\|")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
