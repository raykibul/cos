//! Runs, their tasks and the board. A run is one goal in one workspace. The
//! chief proposes a plan of tasks, you approve it, and the orchestration
//! moves each task along its state machine ([`TaskState`]) through
//! [`Runs::update_task`]. This module only keeps the books: it never starts
//! an agent or touches git. The contract is in `docs/daemon.md`.

mod board;
mod plan;
mod states;
mod storage;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use board::{Board, TaskCounts, render_board};
pub use plan::{MAX_TASKS, PlanProposal, ProposedTask};
pub use states::{RunState, TaskMove, TaskState};

use crate::events::kind;
use crate::{EventLog, Store, StoreError};

/// How many times a task may go back for another attempt.
pub const MAX_RETRIES: u32 = 1;
/// Chief approvals allowed per run unless the run says otherwise.
pub const DEFAULT_APPROVAL_CAP: u32 = 20;
const MAX_APPROVAL_CAP: u32 = 1000;
const MAX_GOAL_CHARS: usize = 10_000;
const MAX_REASON_CHARS: usize = 2_000;

/// One goal in one workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub workspace_id: i64,
    pub goal: String,
    pub state: RunState,
    /// How much the chief may approve on your behalf.
    pub preset: ApprovalPreset,
    /// The most approvals the chief may give in this run; after that they come to you.
    pub approval_cap: u32,
    /// The report, once written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_path: Option<PathBuf>,
    /// Why the run failed or was cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: Timestamp,
    /// Changes whenever the run or one of its tasks does.
    pub updated_at: Timestamp,
}

/// One task of a run, done by one worker in its own worktree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub run_id: i64,
    /// Order in the plan, from 0.
    pub position: u32,
    pub title: String,
    pub instructions: String,
    /// Paths or globs, relative to the repo, the worker may change.
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_command: Option<String>,
    /// Ids of tasks in the same run that must be merged first.
    pub depends_on: Vec<i64>,
    pub state: TaskState,
    /// Attempts after the first. At most [`MAX_RETRIES`].
    pub retries: u32,
    /// Filled in when the task's worktree is created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    /// Why it was last blocked, failed or sent back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A run with how many of its tasks are in each state, for lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    #[serde(flatten)]
    pub run: Run,
    pub tasks: TaskCounts,
}

/// How much the chief may approve on your behalf (see "Approving on your
/// behalf" in `docs/proposal.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalPreset {
    /// Only you approve.
    Strict,
    /// The chief handles the "approved for you" tier.
    #[default]
    Normal,
    /// The chief also handles network access and installs.
    Trusted,
}

impl ApprovalPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Normal => "normal",
            Self::Trusted => "trusted",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Strict, Self::Normal, Self::Trusted]
            .into_iter()
            .find(|preset| preset.as_str() == text)
    }
}

/// What starting a run takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRun {
    pub workspace_id: i64,
    pub goal: String,
    #[serde(default)]
    pub preset: ApprovalPreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_cap: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// Bad input, such as an invalid plan. The message is written for the user.
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    /// Not allowed in the current state.
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for RunError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error.into())
    }
}

/// Creates runs and moves them and their tasks along. Every change is
/// written in one transaction and then logged, with the run id on each event.
pub struct Runs {
    store: Arc<Store>,
    log: EventLog,
}

impl Runs {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// Starts a run in `planning`. A workspace has at most one unfinished run,
    /// because all of a run's tasks merge into the workspace's branch.
    pub fn create(&self, new: NewRun) -> Result<Run, RunError> {
        let goal = new.goal.trim();
        if goal.is_empty() {
            return Err(RunError::Invalid("a run needs a goal".into()));
        }
        if goal.chars().count() > MAX_GOAL_CHARS {
            return Err(RunError::Invalid(format!(
                "a goal can be at most {MAX_GOAL_CHARS} characters"
            )));
        }
        let approval_cap = new.approval_cap.unwrap_or(DEFAULT_APPROVAL_CAP);
        if approval_cap > MAX_APPROVAL_CAP {
            return Err(RunError::Invalid(format!(
                "the approval cap can be at most {MAX_APPROVAL_CAP}"
            )));
        }
        let workspace = self
            .store
            .workspace(new.workspace_id)?
            .ok_or_else(|| RunError::NotFound(format!("there is no workspace {}", new.workspace_id)))?;

        let mut conn = self.store.lock();
        let tx = conn.transaction()?;
        if let Some(busy) = storage::unfinished_run_in(&tx, workspace.id)? {
            return Err(RunError::Conflict(format!(
                "workspace {} already has run {} ({}); cancel it or let it finish first",
                workspace.title, busy.id, busy.state
            )));
        }
        let now = storage::now();
        let mut run = Run {
            id: 0,
            workspace_id: workspace.id,
            goal: goal.to_owned(),
            state: RunState::Planning,
            preset: new.preset,
            approval_cap,
            report_path: None,
            reason: None,
            created_at: now,
            updated_at: now,
        };
        run.id = storage::insert_run(&tx, &run)?;
        tx.commit()?;
        drop(conn);

        self.log.append(
            kind::RUN_CREATED,
            Some(&run.id.to_string()),
            json!({
                "run_id": run.id,
                "workspace_id": run.workspace_id,
                "goal": run.goal,
                "preset": run.preset,
                "approval_cap": run.approval_cap,
            }),
        )?;
        Ok(run)
    }

    /// Every run, newest first, with task counts.
    pub fn list(&self) -> Result<Vec<RunSummary>, RunError> {
        let conn = self.store.lock();
        let mut counts = storage::task_counts(&conn)?;
        Ok(storage::runs(&conn)?
            .into_iter()
            .map(|run| RunSummary {
                tasks: counts.remove(&run.id).unwrap_or_default(),
                run,
            })
            .collect())
    }

    pub fn run(&self, id: i64) -> Result<Run, RunError> {
        storage::run(&self.store.lock(), id)?.ok_or_else(|| no_run(id))
    }

    pub fn task(&self, id: i64) -> Result<Task, RunError> {
        storage::task(&self.store.lock(), id)?.ok_or_else(|| no_task(id))
    }

    /// The run, its workspace (if it still exists) and its tasks in plan order.
    pub fn board(&self, id: i64) -> Result<Board, RunError> {
        let (run, tasks) = {
            let conn = self.store.lock();
            let run = storage::run(&conn, id)?.ok_or_else(|| no_run(id))?;
            (run, storage::tasks_of(&conn, id)?)
        };
        let workspace = self.store.workspace(run.workspace_id)?;
        Ok(Board::new(run, workspace, tasks))
    }

    /// Puts the chief's plan on the board as `proposed` tasks and waits for
    /// your approval. Proposing again before approval replaces the plan.
    pub fn propose_plan(&self, run_id: i64, plan: &PlanProposal) -> Result<Board, RunError> {
        let checked = plan::validate(plan)
            .map_err(|problems| RunError::Invalid(format!("the plan was not accepted: {problems}")))?;
        self.edit(run_id, |edit, tx| {
            let state = edit.run.state;
            if !matches!(state, RunState::Planning | RunState::AwaitingPlanApproval) {
                return Err(RunError::Conflict(format!(
                    "run {run_id} is {state}; a plan can only be proposed before it is approved"
                )));
            }
            let replaced = edit.tasks.len();
            storage::delete_tasks(tx, run_id)?;
            edit.tasks = storage::insert_plan(tx, run_id, &checked, edit.now)?;
            edit.event(
                kind::PLAN_PROPOSED,
                json!({
                    "run_id": run_id,
                    "tasks": edit.tasks.len(),
                    "replaced": replaced,
                }),
            );
            if state == RunState::Planning {
                edit.set_run_state(RunState::AwaitingPlanApproval, None)?;
            }
            Ok(())
        })
    }

    /// Approves the plan: the run starts, and tasks without dependencies
    /// become `ready`.
    pub fn approve_plan(&self, run_id: i64) -> Result<Board, RunError> {
        self.edit(run_id, |edit, _| {
            if edit.run.state != RunState::AwaitingPlanApproval {
                return Err(RunError::Conflict(format!(
                    "run {run_id} has no plan waiting for approval (it is {})",
                    edit.run.state
                )));
            }
            edit.event(
                kind::PLAN_APPROVED,
                json!({ "run_id": run_id, "tasks": edit.tasks.len() }),
            );
            edit.set_run_state(RunState::Running, None)?;
            edit.promote_ready_tasks();
            Ok(())
        })
    }

    /// Cancels the run and every task that hasn't finished. Stopping the
    /// agents working on them is up to whoever follows the events.
    pub fn cancel(&self, run_id: i64, reason: Option<&str>) -> Result<Board, RunError> {
        let reason = clean(reason);
        self.stop(run_id, RunState::Cancelled, reason.as_deref())
    }

    /// Marks the run failed, cancelling every task that hasn't finished.
    pub fn fail(&self, run_id: i64, reason: &str) -> Result<Board, RunError> {
        let reason = clean(Some(reason)).ok_or_else(|| RunError::Invalid("say why the run failed".into()))?;
        self.stop(run_id, RunState::Failed, Some(&reason))
    }

    /// Records the report and finishes the run. Only from `reporting`.
    pub fn finish(&self, run_id: i64, report: &Path) -> Result<Board, RunError> {
        self.edit(run_id, |edit, _| {
            edit.run.report_path = Some(report.to_path_buf());
            edit.set_run_state(RunState::Done, None)
        })
    }

    /// Moves a task to `to`, if that's legal, then lets the run react:
    /// dependents of a merged task may become ready, dependents of a failed
    /// or cancelled one are blocked, and once every task has finished the
    /// run moves to `reporting`.
    pub fn update_task(&self, task_id: i64, to: TaskState, reason: Option<&str>) -> Result<Board, RunError> {
        let run_id = storage::task_run(&self.store.lock(), task_id)?.ok_or_else(|| no_task(task_id))?;
        let reason = clean(reason);
        self.edit(run_id, |edit, _| {
            if edit.run.state != RunState::Running {
                return Err(RunError::Conflict(format!(
                    "run {run_id} is {}; its tasks only move while it is running",
                    edit.run.state
                )));
            }
            let index = edit.index_of(task_id);
            edit.set_task_state(index, to, reason.as_deref())?;
            match to {
                TaskState::Merged => edit.promote_ready_tasks(),
                TaskState::Failed | TaskState::Cancelled => edit.block_dependents(index),
                _ => {}
            }
            if edit.tasks.iter().all(|task| task.state.is_finished()) {
                edit.set_run_state(RunState::Reporting, None)?;
            }
            Ok(())
        })
    }

    /// Records the branch and worktree the git side made for a task.
    pub fn assign_worktree(&self, task_id: i64, branch: &str, worktree: &Path) -> Result<Task, RunError> {
        let run_id = storage::task_run(&self.store.lock(), task_id)?.ok_or_else(|| no_task(task_id))?;
        let board = self.edit(run_id, |edit, _| {
            let index = edit.index_of(task_id);
            let task = &mut edit.tasks[index];
            task.branch = Some(branch.to_owned());
            task.worktree = Some(worktree.to_path_buf());
            edit.dirty.insert(task_id);
            edit.event(
                kind::TASK_WORKTREE_ASSIGNED,
                json!({ "run_id": run_id, "task_id": task_id, "branch": branch, "worktree": worktree }),
            );
            Ok(())
        })?;
        Ok(board
            .tasks
            .into_iter()
            .find(|task| task.id == task_id)
            .expect("the task belongs to the run"))
    }

    fn stop(&self, run_id: i64, to: RunState, reason: Option<&str>) -> Result<Board, RunError> {
        self.edit(run_id, |edit, _| {
            if edit.run.state.is_finished() {
                return Err(RunError::Conflict(format!(
                    "run {run_id} is already {}",
                    edit.run.state
                )));
            }
            let task_reason = match to {
                RunState::Failed => "the run failed",
                _ => "the run was cancelled",
            };
            for index in 0..edit.tasks.len() {
                if !edit.tasks[index].state.is_finished() {
                    edit.set_task_state(index, TaskState::Cancelled, Some(task_reason))?;
                }
            }
            edit.set_run_state(to, reason)
        })
    }

    /// Loads the run and its tasks, applies `change` to them in one
    /// transaction, saves what changed, then logs the events. If `change`
    /// fails, nothing is saved or logged.
    fn edit(
        &self,
        run_id: i64,
        change: impl FnOnce(&mut Edit, &Transaction<'_>) -> Result<(), RunError>,
    ) -> Result<Board, RunError> {
        let events = {
            let mut conn = self.store.lock();
            let tx = conn.transaction()?;
            let run = storage::run(&tx, run_id)?.ok_or_else(|| no_run(run_id))?;
            let tasks = storage::tasks_of(&tx, run_id)?;
            let mut edit = Edit {
                run,
                tasks,
                dirty: HashSet::new(),
                events: Vec::new(),
                now: storage::now(),
            };
            change(&mut edit, &tx)?;
            if !edit.events.is_empty() {
                edit.run.updated_at = edit.now;
                storage::save_run(&tx, &edit.run)?;
                for task in edit.tasks.iter().filter(|task| edit.dirty.contains(&task.id)) {
                    storage::save_task(&tx, task)?;
                }
                tx.commit()?;
            }
            edit.events
        };
        let run_key = run_id.to_string();
        for (kind, data) in events {
            self.log.append(kind, Some(&run_key), data)?;
        }
        self.board(run_id)
    }
}

/// A run and its tasks being changed in memory, with the events to log.
struct Edit {
    run: Run,
    tasks: Vec<Task>,
    /// Ids of tasks to save.
    dirty: HashSet<i64>,
    events: Vec<(&'static str, Value)>,
    now: Timestamp,
}

impl Edit {
    fn event(&mut self, kind: &'static str, data: Value) {
        self.events.push((kind, data));
    }

    fn index_of(&self, task_id: i64) -> usize {
        self.tasks
            .iter()
            .position(|task| task.id == task_id)
            .expect("the task was looked up in this run")
    }

    fn set_run_state(&mut self, to: RunState, reason: Option<&str>) -> Result<(), RunError> {
        let from = self.run.state;
        if !from.can_become(to) {
            return Err(RunError::Conflict(format!(
                "run {} can't go from {from} to {to}",
                self.run.id
            )));
        }
        self.run.state = to;
        if let Some(reason) = reason {
            self.run.reason = Some(reason.to_owned());
        }
        let mut data = json!({ "run_id": self.run.id, "from": from, "to": to });
        if let Some(reason) = reason {
            data["reason"] = json!(reason);
        }
        if let (RunState::Done, Some(report)) = (to, &self.run.report_path) {
            data["report"] = json!(report);
        }
        self.event(kind::RUN_STATE_CHANGED, data);
        Ok(())
    }

    /// Applies the task state machine, the retry limit and dependency gating.
    fn set_task_state(&mut self, index: usize, to: TaskState, reason: Option<&str>) -> Result<(), RunError> {
        let task = &self.tasks[index];
        let (id, from) = (task.id, task.state);
        let Some(step) = from.transition(to) else {
            return Err(RunError::Conflict(format!(
                "task {id} can't go from {from} to {to}"
            )));
        };
        if TaskState::needs_reason(from, to) && reason.is_none() {
            return Err(RunError::Invalid(format!(
                "say why task {id} is going from {from} to {to}"
            )));
        }
        if step == TaskMove::Retry && task.retries >= MAX_RETRIES {
            return Err(RunError::Conflict(format!(
                "task {id} has used its retry; mark it failed or blocked instead"
            )));
        }
        if to == TaskState::Ready
            && let Some(waiting) = self.unmerged_dependency(index)
        {
            return Err(RunError::Conflict(format!(
                "task {id} can't be ready yet: it waits for task {} \"{}\" ({})",
                waiting.id, waiting.title, waiting.state
            )));
        }

        let task = &mut self.tasks[index];
        task.state = to;
        task.updated_at = self.now;
        if step == TaskMove::Retry {
            task.retries += 1;
        }
        if TaskState::needs_reason(from, to) {
            task.last_failure = reason.map(str::to_owned);
        }
        let mut data = json!({
            "run_id": task.run_id,
            "task_id": id,
            "title": task.title,
            "from": from,
            "to": to,
            "retries": task.retries,
        });
        if let Some(reason) = reason {
            data["reason"] = json!(reason);
        }
        self.dirty.insert(id);
        self.event(kind::TASK_STATE_CHANGED, data);
        Ok(())
    }

    /// The first dependency of the task that isn't merged yet.
    fn unmerged_dependency(&self, index: usize) -> Option<&Task> {
        self.tasks[index].depends_on.iter().find_map(|&dep| {
            self.tasks
                .iter()
                .find(|task| task.id == dep && task.state != TaskState::Merged)
        })
    }

    /// Proposed tasks whose dependencies are all merged become ready.
    fn promote_ready_tasks(&mut self) {
        for index in 0..self.tasks.len() {
            if self.tasks[index].state == TaskState::Proposed && self.unmerged_dependency(index).is_none() {
                self.set_task_state(index, TaskState::Ready, None)
                    .expect("proposed → ready is legal once dependencies are merged");
            }
        }
    }

    /// Blocks every proposed task that depends, directly or not, on the task
    /// at `index`, which will now never merge.
    fn block_dependents(&mut self, index: usize) {
        let failed = &self.tasks[index];
        let reason = format!(
            "depends on task {} \"{}\", which {}",
            failed.id,
            failed.title,
            if failed.state == TaskState::Failed {
                "failed"
            } else {
                "was cancelled"
            }
        );
        let mut doomed = HashSet::from([failed.id]);
        // Plans are small (at most MAX_TASKS), so repeat until nothing changes.
        loop {
            let before = doomed.len();
            for task in &self.tasks {
                if task.depends_on.iter().any(|dep| doomed.contains(dep)) {
                    doomed.insert(task.id);
                }
            }
            if doomed.len() == before {
                break;
            }
        }
        for index in 0..self.tasks.len() {
            let task = &self.tasks[index];
            if task.state == TaskState::Proposed && doomed.contains(&task.id) {
                self.set_task_state(index, TaskState::Blocked, Some(&reason))
                    .expect("proposed → blocked is legal");
            }
        }
    }
}

fn no_run(id: i64) -> RunError {
    RunError::NotFound(format!("there is no run {id}"))
}

fn no_task(id: i64) -> RunError {
    RunError::NotFound(format!("there is no task {id}"))
}

/// Trims a free-text reason; empty means none. Long ones are cut short.
fn clean(reason: Option<&str>) -> Option<String> {
    let reason = reason?.trim();
    (!reason.is_empty()).then(|| reason.chars().take(MAX_REASON_CHARS).collect())
}

#[cfg(test)]
mod tests;
