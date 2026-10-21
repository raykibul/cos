//! The pipeline: takes each task of a run from `ready` to `merged`, with a
//! worker agent, the checks, a reviewer agent and the merge (steps 4 to 7 of
//! `docs/orchestration.md`), and cleans up after cancelled runs.
//!
//! - It follows the event log. A task that becomes `ready`, `failed` or
//!   `cancelled` gets a [driver](driver) if it has none; any other change
//!   to a task pokes its driver, which re-reads the task.
//! - One driver per task does everything for it, one step at a time, and
//!   reads the task's state from the database before each step, so the
//!   chief or you can block, fail or cancel a task at any moment.
//! - On start it reconciles: every task of a running run that is `ready`,
//!   `running`, `checking` or `review` gets a driver, which picks the task
//!   up where it is (see "Restarts" in `docs/daemon.md`).

mod driver;
mod prompts;
#[cfg(test)]
mod tests;
mod verdict;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    EventLog, LoggedEvent, Projects, RunState, Runs, TaskState, TaskWorktrees, TestOptions, WorkerLimiter,
};
use tokio::sync::Notify;

use super::agents::AgentManager;
use crate::home::CosHome;

/// The model run agents use unless a run says otherwise, from the environment.
pub(crate) const RUN_MODEL_ENV: &str = "COS_RUN_MODEL";
/// The longest a cancelled run waits for its drivers before it stops the
/// rest of its agents anyway.
const DRIVERS_GONE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(crate) struct PipelineConfig {
    /// Where task worktrees go: `<cos home>/tasks`.
    pub tasks_dir: PathBuf,
    /// `--model` for workers and reviewers. `None`: Claude's default.
    pub model: Option<String>,
    pub tests: TestOptions,
    pub workers_per_workspace: usize,
    pub workers_total: usize,
    /// The reviewer gets at most this much of the diff in its prompt.
    pub max_diff_bytes: usize,
    /// Test output kept in the checks and sent back to a worker.
    pub max_test_tail_chars: usize,
    /// How long a started or resumed agent gets to be ready for a message.
    pub ready_timeout: Duration,
    /// How long a stopped agent gets to exit before its worktree is removed.
    pub exit_timeout: Duration,
    /// How often a waiting driver checks that its agent is still alive.
    pub liveness_poll: Duration,
    /// How long after an agent's last activity it may be stopped.
    pub stop_settle: Duration,
}

impl PipelineConfig {
    pub(crate) fn for_home(home: &CosHome) -> Self {
        Self {
            tasks_dir: home.tasks(),
            model: std::env::var(RUN_MODEL_ENV)
                .ok()
                .filter(|model| !model.trim().is_empty()),
            tests: TestOptions::default(),
            workers_per_workspace: cos_core::taskgit::DEFAULT_WORKERS_PER_WORKSPACE,
            workers_total: cos_core::taskgit::DEFAULT_WORKERS_TOTAL,
            max_diff_bytes: 24 * 1024,
            max_test_tail_chars: 3_000,
            ready_timeout: Duration::from_secs(90),
            exit_timeout: Duration::from_secs(15),
            liveness_poll: Duration::from_secs(5),
            stop_settle: Duration::from_secs(3),
        }
    }
}

/// Runs the pipeline. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Pipeline {
    shared: Arc<Shared>,
}

pub(super) struct Shared {
    runs: Arc<Runs>,
    agents: AgentManager,
    projects: Arc<Projects>,
    log: EventLog,
    worktrees: TaskWorktrees,
    limiter: WorkerLimiter,
    /// One merge at a time per workspace.
    merges: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Each task's driver, while it has one.
    drivers: Mutex<HashMap<i64, DriverHandle>>,
    /// Agents asked to stop that haven't exited yet. Two `/exit`s typed at
    /// once arrive as one prompt, `/exit/exit`, which Claude answers.
    stopping_agents: Mutex<HashSet<i64>>,
    config: PipelineConfig,
    /// The daemon is stopping: drivers must not touch anything.
    stopping: AtomicBool,
}

struct DriverHandle {
    run_id: i64,
    poke: Arc<Notify>,
    task: tokio::task::AbortHandle,
}

impl Pipeline {
    /// Picks up the tasks of running runs where they are, finishes the
    /// cleanup of cancelled runs, then follows the event log.
    pub(crate) fn start(
        runs: Arc<Runs>,
        agents: AgentManager,
        projects: Arc<Projects>,
        log: EventLog,
        config: PipelineConfig,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new(Shared {
            worktrees: TaskWorktrees::new(config.tasks_dir.clone()),
            limiter: WorkerLimiter::new(config.workers_per_workspace, config.workers_total),
            runs,
            agents,
            projects,
            log,
            merges: Mutex::default(),
            drivers: Mutex::default(),
            stopping_agents: Mutex::default(),
            config,
            stopping: AtomicBool::new(false),
        });
        let (_, last_seq) = shared.log.stats()?;
        shared.reconcile()?;
        tokio::spawn(follow(Arc::downgrade(&shared), shared.log.clone(), last_seq));
        Ok(Self { shared })
    }

    /// Stops every driver where it is, for a daemon on its way down. Call it
    /// before the agents shut down, so their exits aren't taken as crashes.
    /// The next start picks the tasks up again.
    pub(crate) fn shutdown(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        for (_, driver) in self.shared.drivers().drain() {
            driver.task.abort();
        }
    }
}

impl Shared {
    fn drivers(&self) -> MutexGuard<'_, HashMap<i64, DriverHandle>> {
        self.drivers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn merge_lock(&self, workspace_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        self.merges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(workspace_id)
            .or_default()
            .clone()
    }

    /// Asks an agent to exit, unless it already was and hasn't yet.
    fn stop_once(self: &Arc<Self>, id: i64) {
        let first = self
            .stopping_agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
        if !first {
            return;
        }
        let _ = self.agents.stop(id);
        let shared = self.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            while shared.agents.get(id).is_ok_and(|agent| agent.state.is_running())
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            shared
                .stopping_agents
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        });
    }

    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// After a restart: drivers for every task of a running run that is on
    /// its way, and the cleanup of cancelled or failed runs.
    fn reconcile(self: &Arc<Self>) -> anyhow::Result<()> {
        for summary in self.runs.list()? {
            let run = summary.run;
            match run.state {
                RunState::Running => {
                    for task in self.runs.board(run.id)?.tasks {
                        if matches!(
                            task.state,
                            TaskState::Ready | TaskState::Running | TaskState::Checking | TaskState::Review
                        ) {
                            tracing::info!(task = task.id, state = %task.state, "picking a task up again");
                            self.ensure_driver(run.id, task.id);
                        }
                    }
                }
                RunState::Cancelled | RunState::Failed => {
                    // Worktrees a cancelled run left behind, if the daemon
                    // stopped before it cleaned up. Usually there are none.
                    for task in self.runs.board(run.id)?.tasks {
                        if driver::needs_cleanup(&task) {
                            self.ensure_driver(run.id, task.id);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Starts a driver for the task, or pokes the one it has.
    fn ensure_driver(self: &Arc<Self>, run_id: i64, task_id: i64) {
        if self.stopping() {
            return;
        }
        let mut drivers = self.drivers();
        if let Some(driver) = drivers.get(&task_id) {
            driver.poke.notify_one();
            return;
        }
        let poke = Arc::new(Notify::new());
        let task = tokio::spawn(driver::drive(self.clone(), task_id, poke.clone()));
        drivers.insert(
            task_id,
            DriverHandle {
                run_id,
                poke,
                task: task.abort_handle(),
            },
        );
    }

    fn poke(&self, task_id: i64) {
        if let Some(driver) = self.drivers().get(&task_id) {
            driver.poke.notify_one();
        }
    }

    fn on_event(self: &Arc<Self>, event: &LoggedEvent) {
        let data = &event.data;
        let id = |key: &str| data.get(key).and_then(serde_json::Value::as_i64);
        match event.kind.as_str() {
            kind::TASK_STATE_CHANGED => {
                let (Some(run_id), Some(task_id)) = (id("run_id"), id("task_id")) else {
                    return;
                };
                let to = data
                    .get("to")
                    .and_then(|to| to.as_str()?.parse::<TaskState>().ok());
                match to {
                    Some(TaskState::Ready | TaskState::Failed | TaskState::Cancelled) => {
                        self.ensure_driver(run_id, task_id);
                    }
                    _ => self.poke(task_id),
                }
            }
            kind::RUN_STATE_CHANGED => {
                let to = data
                    .get("to")
                    .and_then(|to| to.as_str()?.parse::<RunState>().ok());
                if let (Some(run_id), Some(RunState::Cancelled | RunState::Failed)) = (id("run_id"), to) {
                    tokio::spawn(stop_run(self.clone(), run_id));
                }
            }
            _ => {}
        }
    }
}

/// Follows the event log from `after` until the daemon goes away.
async fn follow(shared: std::sync::Weak<Shared>, log: EventLog, after: u64) {
    let mut events = log.subscribe(after);
    drop(log);
    loop {
        let event = match events.next().await {
            Ok(Some(event)) => event,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(%error, "the pipeline stopped following events");
                return;
            }
        };
        let Some(shared) = shared.upgrade() else { return };
        if shared.stopping() {
            return;
        }
        shared.on_event(&event);
    }
}

/// A run was cancelled or failed. Its tasks' drivers clean up their tasks;
/// once they're gone, every agent of the run still running (the chief too)
/// is stopped.
async fn stop_run(shared: Arc<Shared>, run_id: i64) {
    let deadline = tokio::time::Instant::now() + DRIVERS_GONE_TIMEOUT;
    while shared.drivers().values().any(|driver| driver.run_id == run_id)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if shared.stopping() {
        return;
    }
    match shared.agents.of_run(run_id) {
        Ok(agents) => {
            for agent in agents.into_iter().filter(|agent| agent.state.is_running()) {
                tracing::info!(
                    agent = agent.id,
                    run = run_id,
                    "stopping an agent of a cancelled run"
                );
                shared.stop_once(agent.id);
            }
        }
        Err(error) => tracing::error!(%error, run = run_id, "could not list a run's agents"),
    }
}
