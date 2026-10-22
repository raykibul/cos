//! A task's driver: one tokio task that moves one task along, one step at a
//! time. Before each step it reads the task and its run from the database,
//! so it always acts on the current state, whoever changed it:
//!
//! | State | Step |
//! |---|---|
//! | `ready` | worker slot, worktree, worker agent (new, or the old one resumed), then `running` |
//! | `running` | wait for the worker's turn to end, read `DONE` / `STUCK`, then `checking` or `blocked` |
//! | `checking` | scope check, tests, commits; then `review`, a retry, or `blocked` |
//! | `review` | a reviewer agent, its verdict, then the merge, a retry, or `blocked` |
//! | anything else | stop the task's agents, clean up, and leave |
//!
//! A step waits on agents through [`AgentSignal`]s and wakes up when the
//! task is poked (another change to it), so a cancel lands at once.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use cos_core::runs::MAX_RETRIES;
use cos_core::{
    Agent, AgentState, Launch, MergeOutcome, ReviewVerdict, Role, Run, RunError, RunState, Scope, ScopeCheck,
    ScopeReport, Task, TaskChecks, TaskReview, TaskState, TaskWorktree, TestStatus, TestsCheck, TestsStatus,
    WorkerPermit,
};
use tokio::sync::Notify;
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};

use super::verdict::{ReviewerVerdict, WorkerVerdict};
use super::{Shared, prompts, verdict};
use crate::daemon::agents::{AgentSignal, StartAgent};

/// Errors in a row before a driver gives up on its task for now.
const MAX_ERRORS: u32 = 3;
const READY_POLL: Duration = Duration::from_millis(100);
const EXIT_POLL: Duration = Duration::from_millis(50);

/// Whether a finished task may still have a worktree to clean up.
pub(super) fn needs_cleanup(task: &Task) -> bool {
    task.state.is_finished() && task.worktree.as_ref().is_some_and(|path| path.is_dir())
}

/// Drives the task until it needs nobody (merged, blocked, failed or
/// cancelled, or its run stopped), then cleans up and leaves.
pub(super) async fn drive(shared: Arc<Shared>, task_id: i64, poke: Arc<Notify>) {
    let signals = shared.agents.signals();
    let mut driver = Driver {
        shared: shared.clone(),
        task_id,
        poke,
        signals,
        permit: None,
        worktree: None,
        prompted: false,
        asked: false,
        errors: 0,
    };
    loop {
        let flow = match driver.step().await {
            Ok(flow) => {
                driver.errors = 0;
                flow
            }
            Err(error) => driver.on_error(error).await,
        };
        if flow == Flow::Continue {
            continue;
        }
        if shared.stopping() {
            return;
        }
        driver.finish().await;
        // Leave, unless the task was sent back to `ready` meanwhile; under
        // the lock, so a `ready` seen after this starts a new driver.
        let mut drivers = shared.drivers();
        let again = driver.errors < MAX_ERRORS
            && shared
                .runs
                .task(task_id)
                .is_ok_and(|task| task.state == TaskState::Ready);
        if again {
            continue;
        }
        drivers.remove(&task_id);
        return;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// Read the task again and take the next step.
    Continue,
    /// Nothing more to do for now.
    Done,
}

/// How waiting for an agent's turn ended.
enum Waited {
    Turn(Option<String>),
    Exited(Option<i32>),
    /// The task or its run changed, or the daemon is stopping.
    Moved,
}

struct Driver {
    shared: Arc<Shared>,
    task_id: i64,
    poke: Arc<Notify>,
    signals: broadcast::Receiver<AgentSignal>,
    permit: Option<WorkerPermit>,
    worktree: Option<TaskWorktree>,
    /// The worker has been given its work for the current `running` spell.
    prompted: bool,
    /// The worker was asked once for its verdict line.
    asked: bool,
    errors: u32,
}

impl Driver {
    async fn step(&mut self) -> anyhow::Result<Flow> {
        let task = self.shared.runs.task(self.task_id)?;
        let run = self.shared.runs.run(task.run_id)?;
        let active = matches!(
            task.state,
            TaskState::Ready | TaskState::Running | TaskState::Checking | TaskState::Review
        );
        if self.shared.stopping() || run.state != RunState::Running || !active {
            return Ok(Flow::Done);
        }
        if self.permit.is_none() {
            tokio::select! {
                permit = self.shared.limiter.acquire(run.workspace_id) => self.permit = Some(permit),
                () = self.poke.notified() => return Ok(Flow::Continue),
            }
        }
        match task.state {
            TaskState::Ready => self.start(&task, &run).await,
            TaskState::Running => self.work(&task, &run).await,
            TaskState::Checking => self.check(&task, &run).await,
            TaskState::Review => self.review(&task, &run).await,
            _ => Ok(Flow::Done),
        }
    }

    /// `ready`: a worktree and a worker, then `running`.
    async fn start(&mut self, task: &Task, run: &Run) -> anyhow::Result<Flow> {
        let worktree = self.worktree(task, run).await?;
        match self.latest(Role::Worker)? {
            None => {
                self.start_worker(task, run, &worktree)?;
            }
            Some(worker) => {
                let message = match (&task.last_failure, task.retries) {
                    (Some(reason), 1..) => prompts::worker_retry(task, reason),
                    _ => prompts::WORKER_CONTINUE.to_owned(),
                };
                self.wake(&worker, &message).await?;
            }
        }
        self.prompted = true;
        self.asked = false;
        self.update(TaskState::Running, None)
    }

    /// `running`: wait for the worker to say it's done.
    async fn work(&mut self, task: &Task, run: &Run) -> anyhow::Result<Flow> {
        if !self.prompted {
            // Picked up after a restart: the worker was cut off mid-turn.
            match self.latest(Role::Worker)? {
                None => {
                    let worktree = self.worktree(task, run).await?;
                    self.start_worker(task, run, &worktree)?;
                }
                Some(worker) if worker.state == AgentState::Idle => {
                    // Its turn ended while nobody was listening.
                    self.prompted = true;
                    return self
                        .on_worker_turn(task, &worker, worker.last_message.clone())
                        .await;
                }
                Some(worker) if worker.state.is_running() => {}
                Some(worker) => self.wake(&worker, prompts::WORKER_CONTINUE).await?,
            }
            self.prompted = true;
        }
        let worker = self
            .latest(Role::Worker)?
            .ok_or_else(|| anyhow!("task {} has no worker", task.id))?;
        match self.wait_turn(worker.id, TaskState::Running).await? {
            Waited::Moved => Ok(Flow::Continue),
            Waited::Turn(message) => self.on_worker_turn(task, &worker, message).await,
            Waited::Exited(code) => {
                let reason = format!(
                    "the worker exited{} before it finished",
                    code.map(|code| format!(" with code {code}")).unwrap_or_default()
                );
                self.prompted = false;
                if task.retries < MAX_RETRIES {
                    self.update(TaskState::Ready, Some(&reason))
                } else {
                    self.update(TaskState::Blocked, Some(&reason))
                }
            }
        }
    }

    async fn on_worker_turn(
        &mut self,
        task: &Task,
        worker: &Agent,
        message: Option<String>,
    ) -> anyhow::Result<Flow> {
        match message.as_deref().and_then(verdict::worker) {
            Some(WorkerVerdict::Done) => {
                self.prompted = false;
                self.update(TaskState::Checking, None)
            }
            Some(WorkerVerdict::Stuck(why)) => {
                self.update(TaskState::Blocked, Some(&format!("the worker is stuck: {why}")))
            }
            None if !self.asked => {
                self.asked = true;
                self.drain();
                self.shared
                    .agents
                    .send_message(worker.id, prompts::WORKER_ASK_VERDICT)
                    .await?;
                Ok(Flow::Continue)
            }
            None => {
                tracing::info!(task = task.id, "the worker gave no verdict line");
                self.update(
                    TaskState::Blocked,
                    Some("the worker is stuck: no verdict line (it ended without DONE or STUCK)"),
                )
            }
        }
    }

    /// `checking`: the scope, the tests and the commits.
    async fn check(&mut self, task: &Task, run: &Run) -> anyhow::Result<Flow> {
        let worktree = self.worktree(task, run).await?;
        let scope = match Scope::new(&task.files) {
            Ok(scope) => scope,
            Err(error) => {
                return self.update(TaskState::Blocked, Some(&format!("the task's files: {error}")));
            }
        };
        let report = worktree.check_scope(&scope).await?;
        let tests = if report.is_ok() {
            Some(match &task.test_command {
                Some(command) => {
                    let outcome = worktree.run_tests(command, &self.shared.config.tests).await?;
                    TestsCheck {
                        status: match outcome.status {
                            TestStatus::Passed => TestsStatus::Passed,
                            TestStatus::Failed => TestsStatus::Failed,
                            TestStatus::TimedOut => TestsStatus::TimedOut,
                        },
                        exit_code: outcome.exit_code,
                        output_tail: prompts::tail(&outcome.output, self.shared.config.max_test_tail_chars),
                    }
                }
                None => TestsCheck {
                    status: TestsStatus::Skipped,
                    exit_code: None,
                    output_tail: String::new(),
                },
            })
        } else {
            None
        };
        let commits_ahead = worktree.commits_ahead().await?;
        let checks = TaskChecks {
            scope: scope_check(&report),
            tests: tests.clone(),
            commits_ahead,
        };
        self.shared.runs.record_checks(task.id, &checks)?;

        if !report.is_ok() {
            return self.retry_or_block(task, run, &scope_reason(&report)).await;
        }
        if let Some(tests) =
            tests.filter(|tests| matches!(tests.status, TestsStatus::Failed | TestsStatus::TimedOut))
        {
            let command = task.test_command.as_deref().unwrap_or_default();
            let how = match (tests.status, tests.exit_code) {
                (TestsStatus::TimedOut, _) => "timed out".to_owned(),
                (_, Some(code)) => format!("failed with exit code {code}"),
                _ => "failed".to_owned(),
            };
            let reason = format!(
                "The test command `{command}` {how}. The end of its output:\n{}",
                tests.output_tail
            );
            return self.retry_or_block(task, run, &reason).await;
        }
        if commits_ahead == 0 {
            return self.update(TaskState::Blocked, Some("the worker made no changes"));
        }
        self.update(TaskState::Review, None)
    }

    /// `review`: a reviewer, its verdict, then the merge.
    async fn review(&mut self, task: &Task, run: &Run) -> anyhow::Result<Flow> {
        let worktree = self.worktree(task, run).await?;
        if worktree.is_merged().await? {
            // Merged before a restart, with no time to record it.
            return self.merged(None);
        }
        let summary = worktree.diff_summary().await?;
        let (diff, cut) = worktree.diff_text(self.shared.config.max_diff_bytes).await?;
        let prompt = prompts::reviewer_task(task, &worktree.base_commit, &summary, &diff, cut);
        self.drain();
        let reviewer = self.start_agent(
            run,
            Role::Reviewer,
            "plan",
            &format!("Review {}: {}", task.id, task.title),
            prompts::REVIEWER_SYSTEM,
            &worktree,
            prompt,
        )?;

        let mut asked = false;
        let verdict = loop {
            match self.wait_turn(reviewer.id, TaskState::Review).await? {
                Waited::Moved => {
                    tokio::spawn(stop_settled(self.shared.clone(), vec![reviewer.id]));
                    return Ok(Flow::Continue);
                }
                Waited::Turn(message) => match message.as_deref().and_then(verdict::reviewer) {
                    Some(verdict) => break verdict,
                    None if !asked => {
                        asked = true;
                        self.drain();
                        self.shared
                            .agents
                            .send_message(reviewer.id, prompts::REVIEWER_ASK_VERDICT)
                            .await?;
                    }
                    None => break ReviewerVerdict::Changes("no verdict line".into()),
                },
                Waited::Exited(_) => {
                    break ReviewerVerdict::Changes("the reviewer exited without a verdict".into());
                }
            }
        };
        tokio::spawn(stop_settled(self.shared.clone(), vec![reviewer.id]));
        let review = match &verdict {
            ReviewerVerdict::Approve => TaskReview {
                verdict: ReviewVerdict::Approve,
                notes: None,
                agent_id: Some(reviewer.id),
            },
            ReviewerVerdict::Changes(notes) => TaskReview {
                verdict: ReviewVerdict::Changes,
                notes: Some(notes.clone()),
                agent_id: Some(reviewer.id),
            },
        };
        self.shared.runs.record_review(task.id, &review)?;
        match verdict {
            ReviewerVerdict::Approve => self.merge(task, run, &worktree).await,
            ReviewerVerdict::Changes(notes) => {
                let reason = format!("The reviewer asked for changes:\n{notes}");
                self.retry_or_block(task, run, &reason).await
            }
        }
    }

    /// Merges into the workspace, one merge per workspace at a time.
    async fn merge(&mut self, task: &Task, run: &Run, worktree: &TaskWorktree) -> anyhow::Result<Flow> {
        let lock = self.shared.merge_lock(run.workspace_id);
        let _one_at_a_time = lock.lock().await;
        if self.shared.runs.task(task.id)?.state != TaskState::Review {
            return Ok(Flow::Continue);
        }
        match worktree.merge(&task.title).await {
            Ok(MergeOutcome::Merged { commit }) => self.merged(Some(&commit)),
            Ok(MergeOutcome::AlreadyMerged) => self.merged(None),
            Ok(MergeOutcome::Conflict { paths }) => self.update(
                TaskState::Blocked,
                Some(&format!(
                    "merging into the workspace conflicts in: {}",
                    paths.join(", ")
                )),
            ),
            Err(error) => self.update(TaskState::Blocked, Some(&format!("could not merge: {error}"))),
        }
    }

    fn merged(&mut self, commit: Option<&str>) -> anyhow::Result<Flow> {
        match self
            .shared
            .runs
            .update_task(self.task_id, TaskState::Merged, None)
        {
            Ok(_) => {
                self.shared.runs.log_merged(self.task_id, commit)?;
                Ok(Flow::Continue)
            }
            Err(RunError::Conflict(_)) => Ok(Flow::Continue),
            Err(error) => Err(error.into()),
        }
    }

    /// The task's one retry, with `reason` sent to the worker; `blocked`
    /// once it's used.
    async fn retry_or_block(&mut self, task: &Task, run: &Run, reason: &str) -> anyhow::Result<Flow> {
        if task.retries >= MAX_RETRIES {
            return self.update(TaskState::Blocked, Some(reason));
        }
        match self
            .shared
            .runs
            .update_task(task.id, TaskState::Running, Some(reason))
        {
            Ok(_) => {}
            Err(RunError::Conflict(_)) => return Ok(Flow::Continue),
            Err(error) => return Err(error.into()),
        }
        let task = self.shared.runs.task(task.id)?;
        match self.latest(Role::Worker)? {
            Some(worker) => self.wake(&worker, &prompts::worker_retry(&task, reason)).await?,
            None => {
                let worktree = self.worktree(&task, run).await?;
                self.start_worker(&task, run, &worktree)?;
            }
        }
        self.prompted = true;
        self.asked = false;
        Ok(Flow::Continue)
    }

    /// Moves the task. A move refused because the task moved meanwhile is
    /// fine: the next step reads where it is now.
    fn update(&self, to: TaskState, reason: Option<&str>) -> anyhow::Result<Flow> {
        match self.shared.runs.update_task(self.task_id, to, reason) {
            Ok(_) | Err(RunError::Conflict(_)) => Ok(Flow::Continue),
            Err(error) => Err(error.into()),
        }
    }

    /// A step failed: block the task with the error, so the chief or you
    /// can decide.
    async fn on_error(&mut self, error: anyhow::Error) -> Flow {
        self.errors += 1;
        tracing::error!(task = self.task_id, error = %format!("{error:#}"), "a pipeline step failed");
        if self.shared.stopping() {
            return Flow::Done;
        }
        let blocked =
            self.shared
                .runs
                .update_task(self.task_id, TaskState::Blocked, Some(&format!("{error:#}")));
        if blocked.is_err() && self.errors >= MAX_ERRORS {
            return Flow::Done;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        Flow::Continue
    }

    /// The task needs nobody now: stop its agents, free its slot, and clean
    /// up its worktree if it merged, failed or was cancelled.
    async fn finish(&mut self) {
        self.prompted = false;
        self.asked = false;
        let agents = self.shared.agents.of_task(self.task_id).unwrap_or_default();
        let running: Vec<i64> = agents
            .iter()
            .filter(|agent| agent.state.is_running())
            .map(|agent| agent.id)
            .collect();
        self.permit = None;
        stop_settled(self.shared.clone(), running.clone()).await;
        let Ok(task) = self.shared.runs.task(self.task_id) else {
            return;
        };
        if !task.state.is_finished() {
            return;
        }
        self.until_exited(&running).await;
        let worktree = match self.worktree.take() {
            Some(worktree) => Some(worktree),
            None => self.reopen(&task).await,
        };
        let Some(worktree) = worktree else { return };
        if let Err(error) = clean_up(&task, &worktree).await {
            tracing::warn!(task = task.id, %error, "could not clean up a task's worktree");
        }
    }

    // ---- Worktrees and agents ----

    /// The task's worktree: the one it has, or a new one.
    async fn worktree(&mut self, task: &Task, run: &Run) -> anyhow::Result<TaskWorktree> {
        if let Some(worktree) = &self.worktree {
            return Ok(worktree.clone());
        }
        let existing = match &task.worktree {
            Some(path) if path.is_dir() => self.reopen(task).await,
            _ => None,
        };
        let worktree = match existing {
            Some(worktree) => worktree,
            None => {
                let workspace = self.shared.projects.workspace(run.workspace_id)?;
                let worktree = self
                    .shared
                    .worktrees
                    .create(&workspace, task.id, &task.title)
                    .await
                    .context("could not create the task's worktree")?;
                self.shared
                    .runs
                    .assign_worktree(task.id, &worktree.branch, &worktree.path)?;
                worktree
            }
        };
        self.worktree = Some(worktree.clone());
        Ok(worktree)
    }

    /// The worktree recorded on the task, if its branch still exists.
    async fn reopen(&self, task: &Task) -> Option<TaskWorktree> {
        let (Some(branch), Some(path)) = (&task.branch, &task.worktree) else {
            return None;
        };
        let run = self.shared.runs.run(task.run_id).ok()?;
        let workspace = self.shared.projects.workspace(run.workspace_id).ok()?;
        self.shared
            .worktrees
            .reopen(&workspace, task.id, branch, path)
            .await
            .inspect_err(|error| tracing::warn!(task = task.id, %error, "could not reopen a task's worktree"))
            .ok()
    }

    /// The task's newest agent in `role`.
    fn latest(&self, role: Role) -> anyhow::Result<Option<Agent>> {
        Ok(self
            .shared
            .agents
            .of_task(self.task_id)?
            .into_iter()
            .rev()
            .find(|agent| agent.role == role))
    }

    fn start_worker(&mut self, task: &Task, run: &Run, worktree: &TaskWorktree) -> anyhow::Result<Agent> {
        self.drain();
        self.start_agent(
            run,
            Role::Worker,
            "acceptEdits",
            &format!("Task {}: {}", task.id, task.title),
            prompts::WORKER_SYSTEM,
            worktree,
            prompts::worker_task(task),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_agent(
        &self,
        run: &Run,
        role: Role,
        permission_mode: &str,
        title: &str,
        system_prompt: &str,
        worktree: &TaskWorktree,
        prompt: String,
    ) -> anyhow::Result<Agent> {
        let request = StartAgent {
            prompt: Some(prompt),
            title: Some(title.to_owned()),
            model: self.shared.config.model.clone(),
            permission_mode: Some(permission_mode.to_owned()),
            user_settings: Some(false),
            rows: None,
            cols: None,
        };
        let launch = Launch {
            task_id: Some(self.task_id),
            cwd: Some(worktree.path.clone()),
            system_prompt: Some(system_prompt.to_owned()),
            mcp_config: None,
            spawned_by: None,
        };
        let agent = self
            .shared
            .agents
            .start_agent_with(run.workspace_id, role, Some(run.id), &request, &launch)
            .with_context(|| format!("could not start the {}", role_name(role)))?;
        tracing::info!(
            task = self.task_id,
            agent = agent.id,
            role = role_name(role),
            "started a run agent"
        );
        Ok(agent)
    }

    /// Brings an agent back if it has exited, waits until it can take a
    /// message, and sends `message`.
    async fn wake(&mut self, agent: &Agent, message: &str) -> anyhow::Result<()> {
        if !agent.state.is_running() {
            self.shared
                .agents
                .resume(agent.id)
                .with_context(|| format!("could not resume agent {}", agent.id))?;
        }
        self.until_ready(agent.id).await?;
        self.drain();
        self.shared.agents.send_message(agent.id, message).await?;
        Ok(())
    }

    /// Waits until the agent has started and isn't busy starting up.
    async fn until_ready(&self, id: i64) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + self.shared.config.ready_timeout;
        loop {
            let agent = self.shared.agents.get(id)?;
            match agent.state {
                AgentState::Idle | AgentState::Working | AgentState::WaitingApproval => return Ok(()),
                AgentState::Exited | AgentState::Interrupted => {
                    return Err(anyhow!("agent {id} exited before it could take a message"));
                }
                AgentState::Starting | AgentState::NeedsTrust => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("agent {id} didn't start in time"));
            }
            tokio::time::sleep(READY_POLL).await;
        }
    }

    async fn until_exited(&self, ids: &[i64]) {
        let deadline = tokio::time::Instant::now() + self.shared.config.exit_timeout;
        for &id in ids {
            while self
                .shared
                .agents
                .get(id)
                .is_ok_and(|agent| agent.state.is_running())
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(EXIT_POLL).await;
            }
        }
    }

    /// Waits for the agent's turn to end, for it to exit, or for the task
    /// to leave `expected`.
    async fn wait_turn(&mut self, agent_id: i64, expected: TaskState) -> anyhow::Result<Waited> {
        let mut liveness = tokio::time::interval(self.shared.config.liveness_poll);
        liveness.tick().await;
        loop {
            tokio::select! {
                signal = self.signals.recv() => match signal {
                    Ok(AgentSignal::TurnEnded { agent_id: id, message }) if id == agent_id => {
                        return Ok(Waited::Turn(message));
                    }
                    Ok(AgentSignal::Exited { agent_id: id, code, shutdown }) if id == agent_id => {
                        return Ok(if shutdown { Waited::Moved } else { Waited::Exited(code) });
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => {
                        // Missed signals; the agent's record says where it is.
                        let agent = self.shared.agents.get(agent_id)?;
                        if !agent.state.is_running() {
                            return Ok(Waited::Exited(agent.exit_code));
                        }
                        if agent.state == AgentState::Idle {
                            return Ok(Waited::Turn(agent.last_message));
                        }
                    }
                    Err(RecvError::Closed) => return Ok(Waited::Moved),
                },
                () = self.poke.notified() => {
                    if self.moved(expected)? {
                        return Ok(Waited::Moved);
                    }
                }
                _ = liveness.tick() => {
                    let agent = self.shared.agents.get(agent_id)?;
                    if !agent.state.is_running() {
                        return Ok(if self.shared.stopping() { Waited::Moved } else { Waited::Exited(agent.exit_code) });
                    }
                }
            }
        }
    }

    fn moved(&self, expected: TaskState) -> anyhow::Result<bool> {
        let task = self.shared.runs.task(self.task_id)?;
        let run = self.shared.runs.run(task.run_id)?;
        Ok(self.shared.stopping() || task.state != expected || run.state != RunState::Running)
    }

    /// Forgets signals so far, before sending a message whose answer we wait for.
    fn drain(&mut self) {
        loop {
            match self.signals.try_recv() {
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty | TryRecvError::Closed) => return,
            }
        }
    }
}

/// Stops agents, first giving any whose turn has only just ended a moment
/// to settle: Claude takes text typed straight after a turn as the next
/// prompt, `/exit` included, and would answer it instead of exiting.
async fn stop_settled(shared: Arc<Shared>, ids: Vec<i64>) {
    let settle = shared.config.stop_settle.as_millis() as i64;
    let newest = ids
        .iter()
        .filter_map(|&id| shared.agents.get(id).ok())
        .map(|agent| agent.last_activity_ms)
        .max();
    if let Some(newest) = newest {
        let idle_for = jiff::Timestamp::now().as_millisecond() - newest;
        if idle_for < settle {
            tokio::time::sleep(Duration::from_millis((settle - idle_for) as u64)).await;
        }
    }
    for id in ids {
        shared.stop_once(id);
    }
}

/// After a task finished: a merged task's worktree and branch go; a failed
/// or cancelled task's go only if it has no commits, otherwise both stay.
async fn clean_up(task: &Task, worktree: &TaskWorktree) -> anyhow::Result<()> {
    match task.state {
        TaskState::Merged => {
            worktree.remove(true).await?;
            worktree.delete_branch().await?;
        }
        TaskState::Failed | TaskState::Cancelled => {
            if worktree.commits_ahead().await? == 0 {
                worktree.remove(true).await?;
                worktree.delete_branch().await?;
            } else {
                tracing::info!(task = task.id, branch = %worktree.branch, "keeping a cancelled task's commits");
            }
        }
        _ => {}
    }
    Ok(())
}

fn scope_check(report: &ScopeReport) -> ScopeCheck {
    ScopeCheck {
        ok: report.is_ok(),
        out_of_scope: report.out_of_scope.clone(),
        protected: report.protected.clone(),
        uncommitted: report.uncommitted.clone(),
        wrong_branch: report.wrong_branch.clone(),
    }
}

/// The scope violations, for the worker and the board.
fn scope_reason(report: &ScopeReport) -> String {
    let mut reason = String::from("The changes broke the task's file scope:");
    let list = |reason: &mut String, what: &str, paths: &[String]| {
        if !paths.is_empty() {
            reason.push_str(&format!("\n- {what}: {}", paths.join(", ")));
        }
    };
    list(
        &mut reason,
        "changed outside the task's files (undo these)",
        &report.out_of_scope,
    );
    list(
        &mut reason,
        "protected paths no task may touch",
        &report.protected,
    );
    list(
        &mut reason,
        "left uncommitted (commit or delete them)",
        &report.uncommitted,
    );
    if let Some(branch) = &report.wrong_branch {
        reason.push_str(&format!(
            "\n- the worktree is on {branch}, not the task's branch (switch back)"
        ));
    }
    reason
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Worker => "worker",
        Role::Reviewer => "reviewer",
        Role::Chief => "chief",
        Role::User => "agent",
        Role::Staff => "chief of staff",
    }
}
