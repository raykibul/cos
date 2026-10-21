//! The chief: the agent that plans and steers a run. The contract is
//! `docs/orchestration.md`.
//!
//! - **Starting:** every new run gets a chief agent (role `chief`) in its
//!   workspace, with the `cos` MCP server ([`prompt::configure`]) and its
//!   role appended to the system prompt. Its id is `runs.chief_agent_id`.
//! - **Tools:** `cos mcp --agent <id>` forwards each tool call to
//!   `POST /v1/agents/{id}/tools/{name}` ([`Chief::call_tool`]), so the
//!   daemon knows which agent and run is calling.
//! - **Inbox:** the follower turns events into inbox items (tasks merged,
//!   blocked or failed; approvals routed to the chief; the run reaching
//!   `reporting`), and the answer and message routes add theirs. When the
//!   chief is `idle` the delivery loop types one wakeup holding up to 20
//!   items or 24 KiB, at most 30 times an hour, with a brief of the run on
//!   top.
//! - **Files:** `<cos home>/runs/<run id>/board.md`, rewritten whenever the
//!   board changes, and `report.md` from `report_write`.

mod prompt;
#[cfg(test)]
mod tests;
mod tools;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use cos_core::chief::{MAX_WAKEUPS_PER_HOUR, next_batch};
use cos_core::events::kind;
use cos_core::{
    AgentState, EventLog, Inbox, InboxKind, LoggedEvent, Question, QuestionState, Questions, Role, Run, Runs,
    Store, Subscription, Task, render_board,
};
use jiff::Timestamp;
use serde_json::Value;
use tokio::sync::Notify;

pub(crate) use prompt::configure;

use super::agents::{AgentManager, StartAgent};
use super::api::ApiError;

/// The model chiefs run, if set; Claude's default otherwise.
pub(crate) const MODEL_ENV: &str = "COS_CHIEF_MODEL";
const HOUR_MS: i64 = 3_600_000;
const MAX_MESSAGE_CHARS: usize = 10_000;

#[derive(Debug, Clone)]
pub(crate) struct ChiefConfig {
    pub model: Option<String>,
    /// How often waiting inboxes are checked even without news, so items
    /// held back by the hourly cap go out once it frees up.
    pub poll: Duration,
    /// After a wakeup, how long to wait for the chief to start working
    /// before another wakeup may go out.
    pub settle: Duration,
    /// How long a finished run's chief may keep working before it's stopped.
    pub stop_wait: Duration,
}

impl ChiefConfig {
    pub(crate) fn from_env() -> Self {
        Self {
            model: std::env::var(MODEL_ENV)
                .ok()
                .map(|model| model.trim().to_owned())
                .filter(|model| !model.is_empty()),
            poll: Duration::from_secs(30),
            settle: Duration::from_secs(20),
            stop_wait: Duration::from_secs(120),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            model: None,
            poll: Duration::from_millis(200),
            settle: Duration::from_secs(5),
            stop_wait: Duration::from_secs(5),
        }
    }
}

/// Finds the worker running a task, for `task_send`. The pipeline owns
/// which agent works on which task; until it plugs in, nothing is found.
pub(crate) type WorkerLookup = Arc<dyn Fn(&Task) -> Option<i64> + Send + Sync>;

/// Runs the chiefs. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Chief {
    inner: Arc<Inner>,
}

struct Inner {
    log: EventLog,
    runs: Arc<Runs>,
    agents: AgentManager,
    inbox: Inbox,
    questions: Questions,
    /// The cos home: run files go in `runs/<id>/` under it.
    home: PathBuf,
    config: ChiefConfig,
    /// Wakes the delivery loop.
    wake: Arc<Notify>,
    /// Chiefs that got a wakeup and haven't started working on it yet.
    woken: Mutex<HashMap<i64, Instant>>,
    workers: Mutex<WorkerLookup>,
}

impl Chief {
    /// Starts following the event log (from now on) and delivering wakeups.
    pub(crate) fn start(
        store: Arc<Store>,
        log: EventLog,
        runs: Arc<Runs>,
        agents: AgentManager,
        home: PathBuf,
        config: ChiefConfig,
    ) -> anyhow::Result<Self> {
        let (_, last_seq) = log.stats()?;
        let subscription = log.subscribe(last_seq);
        let wake = Arc::new(Notify::new());
        let inner = Arc::new(Inner {
            inbox: Inbox::new(store.clone()),
            questions: Questions::new(store, log.clone()),
            log,
            runs,
            agents,
            home,
            config,
            wake: wake.clone(),
            woken: Mutex::default(),
            workers: Mutex::new(Arc::new(|_: &Task| None)),
        });
        tokio::spawn(follow(Arc::downgrade(&inner), subscription));
        tokio::spawn(deliver_loop(Arc::downgrade(&inner), wake));
        Ok(Self { inner })
    }

    /// Plugs in how `task_send` finds a task's worker.
    #[cfg_attr(not(test), expect(dead_code, reason = "the pipeline plugs in here"))]
    pub(crate) fn set_worker_lookup(&self, lookup: WorkerLookup) {
        *self.inner.workers.lock().unwrap_or_else(PoisonError::into_inner) = lookup;
    }

    /// Starts the chief of a new run and records it on the run.
    pub(crate) fn start_for_run(&self, run: &Run) -> Result<Run, ApiError> {
        let inner = &self.inner;
        let board = inner.runs.board(run.id)?;
        let workspace = board.workspace.ok_or_else(|| {
            error(
                StatusCode::CONFLICT,
                format!("run {}'s workspace is gone", run.id),
            )
        })?;
        let request = StartAgent {
            prompt: Some(prompt::first_prompt(run, &workspace.path, &workspace.branch)),
            title: Some(format!("Chief of run {}", run.id)),
            model: inner.config.model.clone(),
            permission_mode: Some("default".into()),
            user_settings: Some(false),
            ..StartAgent::default()
        };
        let agent = inner
            .agents
            .start_agent(run.workspace_id, Role::Chief, Some(run.id), &request)?;
        Ok(inner.runs.set_chief_agent(run.id, agent.id)?)
    }

    /// Questions, oldest first. Open questions of finished runs are left
    /// out: nobody is waiting for them any more.
    pub(crate) fn questions(
        &self,
        state: Option<QuestionState>,
        run_id: Option<i64>,
    ) -> Result<Vec<Question>, ApiError> {
        let inner = &self.inner;
        let mut questions = inner.questions.list(state, run_id)?;
        if state == Some(QuestionState::Open) {
            let mut finished = HashMap::new();
            questions.retain(|question| {
                !*finished.entry(question.run_id).or_insert_with(|| {
                    inner
                        .runs
                        .run(question.run_id)
                        .is_ok_and(|run| run.state.is_finished())
                })
            });
        }
        Ok(questions)
    }

    pub(crate) fn question(&self, id: i64) -> Result<Question, ApiError> {
        Ok(self.inner.questions.get(id)?)
    }

    /// Records your answer and puts it in the chief's inbox.
    pub(crate) fn answer(&self, id: i64, answer: &str) -> Result<Question, ApiError> {
        let inner = &self.inner;
        let question = inner.questions.get(id)?;
        let run = inner.runs.run(question.run_id)?;
        if run.state.is_finished() {
            return Err(error(
                StatusCode::CONFLICT,
                format!(
                    "run {} is {}; nobody is waiting for this answer",
                    run.id, run.state
                ),
            ));
        }
        let question = inner.questions.answer(id, answer)?;
        let text = format!(
            "You asked (question {}): {} The user answered: {}",
            question.id,
            quote(&question.question),
            quote(question.answer.as_deref().unwrap_or_default())
        );
        inner.add(run.id, InboxKind::Answer, &text)?;
        Ok(question)
    }

    /// A message from you to the run's chief, through its inbox.
    pub(crate) fn message(&self, run_id: i64, text: &str) -> Result<(), ApiError> {
        let inner = &self.inner;
        let run = inner.runs.run(run_id)?;
        if run.state.is_finished() {
            return Err(error(
                StatusCode::CONFLICT,
                format!("run {run_id} is {}; its chief has stopped", run.state),
            ));
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(error(StatusCode::BAD_REQUEST, "the message is empty".into()));
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!("a message can be at most {MAX_MESSAGE_CHARS} characters"),
            ));
        }
        inner.add(run_id, InboxKind::Message, &format!("The user says: {text}"))?;
        Ok(())
    }

    /// The run's report, once written.
    pub(crate) fn report(&self, run_id: i64) -> Result<(String, PathBuf), ApiError> {
        let inner = &self.inner;
        let run = inner.runs.run(run_id)?;
        let path = run
            .report_path
            .unwrap_or_else(|| report_path(&inner.home, run_id));
        match std::fs::read_to_string(&path) {
            Ok(markdown) => Ok((markdown, path)),
            Err(read) if read.kind() == std::io::ErrorKind::NotFound => {
                Err(ApiError::not_found(&format!("run {run_id} has no report yet")))
            }
            Err(read) => Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not read {}: {read}", path.display()),
            )),
        }
    }
}

impl Inner {
    /// Adds an inbox item and nudges the delivery loop.
    fn add(&self, run_id: i64, kind: InboxKind, text: &str) -> Result<(), ApiError> {
        self.inbox.add(run_id, kind, text)?;
        self.wake.notify_one();
        Ok(())
    }

    fn on_event(&self, event: &LoggedEvent) {
        let run_id = event.run_id.as_deref().and_then(|id| id.parse::<i64>().ok());
        if let Some(run_id) = run_id
            && ["run.", "plan.", "task."]
                .iter()
                .any(|prefix| event.kind.starts_with(prefix))
            && event.kind != kind::RUN_REPORT_WRITTEN
        {
            self.write_board(run_id);
        }
        let result = match event.kind.as_str() {
            kind::TASK_STATE_CHANGED => self.on_task(&event.data),
            kind::RUN_STATE_CHANGED => self.on_run(&event.data),
            kind::APPROVAL_ROUTED => self.on_routed(run_id, &event.data),
            kind::AGENT_STATE_CHANGED => {
                let agent = event.data["agent_id"].as_i64().unwrap_or_default();
                match event.data["state"].as_str() {
                    Some("idle") => self.wake.notify_one(),
                    Some(_) => {
                        self.woken
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&agent);
                    }
                    None => {}
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(failure) = result {
            tracing::error!(error = %failure.message, kind = %event.kind, "the chief's inbox missed an event");
        }
    }

    fn on_task(&self, data: &Value) -> Result<(), ApiError> {
        let (Some(run_id), Some(task_id)) = (data["run_id"].as_i64(), data["task_id"].as_i64()) else {
            return Ok(());
        };
        let title = quote(data["title"].as_str().unwrap_or_default());
        let reason = data["reason"].as_str().unwrap_or("no reason given");
        let text = match data["to"].as_str() {
            Some("merged") => format!("Task {task_id} {title} merged."),
            Some("failed") => format!("Task {task_id} {title} failed: {reason}"),
            Some("blocked") => {
                let next = if data["retries"].as_u64().unwrap_or_default()
                    >= u64::from(cos_core::runs::MAX_RETRIES)
                {
                    "It has used its retry: fail or cancel it with task_update, or ask_user."
                } else {
                    "Retry it once with task_update (say what to do differently), fail or cancel it, or ask_user."
                };
                format!("Task {task_id} {title} is blocked: {reason} {next}")
            }
            _ => return Ok(()),
        };
        self.add(run_id, InboxKind::Task, &text)
    }

    fn on_run(&self, data: &Value) -> Result<(), ApiError> {
        let Some(run_id) = data["run_id"].as_i64() else {
            return Ok(());
        };
        match data["to"].as_str() {
            Some("reporting") => self.add(
                run_id,
                InboxKind::Reporting,
                "Every task is merged, failed or cancelled. Write the report now with report_write: \
                 what was done, what wasn't and why, and anything the user should check.",
            ),
            Some("done") => self.stop_chief(run_id, true),
            Some("failed" | "cancelled") => self.stop_chief(run_id, false),
            _ => Ok(()),
        }
    }

    fn on_routed(&self, run_id: Option<i64>, data: &Value) -> Result<(), ApiError> {
        if data["route"].as_str() != Some("chief") {
            return Ok(());
        }
        let Some(approval_id) = data["approval_id"].as_i64() else {
            return Ok(());
        };
        let approval = self.agents.approval(approval_id)?;
        let asker = self.agents.get(approval.agent_id)?;
        let Some(run_id) = run_id.or(asker.run_id) else {
            return Ok(());
        };
        let role = serde_json::to_value(asker.role)
            .ok()
            .and_then(|role| role.as_str().map(str::to_owned))
            .unwrap_or_default();
        let why = data["reason"]
            .as_str()
            .filter(|reason| !reason.is_empty())
            .map(|reason| format!(" Policy: {reason}."))
            .unwrap_or_default();
        let text = format!(
            "Approval {approval_id} waits for your decision: {} (asked by {role} agent {}, {}).{why} \
             Call approval_decide with allow or deny and a one-line reason.",
            approval.summary, asker.id, asker.title
        );
        self.add(run_id, InboxKind::Approval, &text)
    }

    /// Stops a finished run's chief: at once, or once it ends its turn
    /// (`wait`), so it finishes writing the report.
    fn stop_chief(&self, run_id: i64, wait: bool) -> Result<(), ApiError> {
        let Some(chief) = self.runs.run(run_id)?.chief_agent_id else {
            return Ok(());
        };
        let agents = self.agents.clone();
        let wait_for = if wait {
            self.config.stop_wait
        } else {
            Duration::ZERO
        };
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + wait_for;
            while tokio::time::Instant::now() < deadline
                && agents.get(chief).is_ok_and(|agent| {
                    matches!(
                        agent.state,
                        AgentState::Starting | AgentState::Working | AgentState::WaitingApproval
                    )
                })
            {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if agents.get(chief).is_ok_and(|agent| agent.state.is_running()) {
                let _ = agents.stop(chief);
            }
        });
        Ok(())
    }

    /// Rewrites `<home>/runs/<id>/board.md`.
    fn write_board(&self, run_id: i64) {
        let result = self
            .runs
            .board(run_id)
            .map_err(anyhow::Error::from)
            .and_then(|board| {
                let dir = run_dir(&self.home, run_id);
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("board.md"), render_board(&board))?;
                Ok(())
            });
        if let Err(failure) = result {
            tracing::warn!(error = %failure, run = run_id, "could not write the board file");
        }
    }

    /// Delivers a wakeup to every idle chief with items waiting.
    async fn deliver_all(&self) {
        let runs = match self.inbox.runs_waiting() {
            Ok(runs) => runs,
            Err(failure) => {
                tracing::error!(error = %failure, "could not read the inbox");
                return;
            }
        };
        for run_id in runs {
            if let Err(failure) = self.deliver(run_id).await {
                tracing::warn!(error = %failure.message, run = run_id, "could not wake the chief");
            }
        }
    }

    /// Types one wakeup into the run's chief if it is idle, under the cap,
    /// and not still picking up the last one. Returns whether it did.
    async fn deliver(&self, run_id: i64) -> Result<bool, ApiError> {
        let run = self.runs.run(run_id)?;
        let Some(chief) = run.chief_agent_id.filter(|_| !run.state.is_finished()) else {
            return Ok(false);
        };
        if self.agents.get(chief)?.state != AgentState::Idle {
            return Ok(false);
        }
        let settling = self
            .woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&chief)
            .is_some_and(|at| at.elapsed() < self.config.settle);
        if settling {
            return Ok(false);
        }
        let hour_ago = Timestamp::now().as_millisecond() - HOUR_MS;
        if self.inbox.wakeups_since(run_id, hour_ago)? >= MAX_WAKEUPS_PER_HOUR {
            return Ok(false);
        }
        let waiting = self.inbox.waiting(run_id)?;
        let batch = next_batch(&waiting);
        if batch.is_empty() {
            return Ok(false);
        }
        let board = self.runs.board(run_id)?;
        let open = self
            .questions
            .list(Some(QuestionState::Open), Some(run_id))?
            .len();
        let text = prompt::wakeup(&board, open, batch, waiting.len() - batch.len());
        // Marked before typing: the chief may start working (which clears
        // the mark) before typing returns.
        self.woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(chief, Instant::now());
        if let Err(failure) = self.agents.input(chief, &text).await {
            self.woken
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&chief);
            return Err(failure.into());
        }
        self.inbox.deliver(run_id, chief, batch)?;
        Ok(true)
    }
}

/// Feeds every new event to the inbox and the board files.
async fn follow(inner: Weak<Inner>, mut subscription: Subscription) {
    loop {
        let event = match subscription.next().await {
            Ok(Some(event)) => event,
            Ok(None) => return,
            Err(failure) => {
                tracing::error!(error = %failure, "the chief stopped following events");
                return;
            }
        };
        let Some(inner) = inner.upgrade() else { return };
        inner.on_event(&event);
    }
}

/// Delivers wakeups whenever something changes, and every `poll`.
async fn deliver_loop(inner: Weak<Inner>, wake: Arc<Notify>) {
    loop {
        let poll = {
            let Some(inner) = inner.upgrade() else { return };
            inner.deliver_all().await;
            inner.config.poll
        };
        tokio::select! {
            () = wake.notified() => {}
            () = tokio::time::sleep(poll) => {}
        }
    }
}

/// `<home>/runs/<id>`: the run's board and report.
pub(crate) fn run_dir(home: &Path, run_id: i64) -> PathBuf {
    home.join("runs").join(run_id.to_string())
}

pub(crate) fn report_path(home: &Path, run_id: i64) -> PathBuf {
    run_dir(home, run_id).join("report.md")
}

fn error(status: StatusCode, message: String) -> ApiError {
    ApiError { status, message }
}

/// Text in double quotes, on one line.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.split_whitespace().collect::<Vec<_>>().join(" "))
}
