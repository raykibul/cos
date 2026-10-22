//! The Chief of Staff: one permanent Claude session per project that creates
//! workspaces, starts agents, follows them through its inbox and reports
//! back. The contract is `docs/chief-of-staff.md`.
//!
//! - **Starting:** lazily, on `start` or the first message. It runs as an
//!   agent with role `staff` in the project's hidden `<slug>/chief`
//!   worktree, with the user's settings, its standing instructions and brief
//!   appended to the system prompt ([`prompt::system_prompt`]), and the
//!   `cos` MCP server ([`prompt::configure`]). A restart resumes the same
//!   agent and Claude session.
//! - **Tools:** `cos mcp --agent <id>` forwards each call to
//!   `POST /v1/agents/{id}/tools/{name}`, which sends the staff's calls here
//!   ([`Staff::call_tool`]). Every id is checked against its project.
//! - **Inbox:** the agents it started (a turn ended, they exited, they wait
//!   for you) and the runs it started (state changes) add items; so do the
//!   messages you type. When it's idle, the delivery loop types your oldest
//!   message on its own, else one `[cos · N updates]` wakeup under the run
//!   chief's caps.

mod prompt;
#[cfg(test)]
mod tests;
mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use cos_core::chief::{MAX_WAKEUPS_PER_HOUR, next_batch_by};
use cos_core::events::kind;
use cos_core::{
    Agent, AgentState, EventLog, Launch, LoggedEvent, Projects, Role, Runs, StaffBooks, StaffItem,
    StaffItemKind, Store, Subscription,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast};

pub(crate) use prompt::configure;

use super::agents::{AgentManager, AgentSignal, StartAgent};
use super::api::ApiError;
use super::chief::Chief;

/// The model Chiefs of Staff run unless a project sets one; else Claude's
/// default.
pub(crate) const MODEL_ENV: &str = "COS_STAFF_MODEL";
const HOUR_MS: i64 = 3_600_000;
/// The most characters of a message you type to it.
const MAX_MESSAGE_CHARS: usize = 100_000;
/// How many inbox items `GET …/staff/inbox` returns.
const INBOX_PAGE: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct StaffConfig {
    pub model: Option<String>,
    /// How often waiting inboxes are checked even without news.
    pub poll: Duration,
    /// After typing something, how long to wait for it to start working
    /// before typing more.
    pub settle: Duration,
    /// How long `agent_send` waits for a resumed agent to be ready.
    pub ready_wait: Duration,
}

impl StaffConfig {
    pub(crate) fn from_env() -> Self {
        Self {
            model: std::env::var(MODEL_ENV)
                .ok()
                .map(|model| model.trim().to_owned())
                .filter(|model| !model.is_empty()),
            poll: Duration::from_secs(30),
            settle: Duration::from_secs(20),
            ready_wait: Duration::from_secs(60),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            model: None,
            poll: Duration::from_millis(200),
            settle: Duration::from_secs(5),
            ready_wait: Duration::from_secs(10),
        }
    }
}

/// `GET /v1/projects/{id}/staff`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StaffView {
    pub project_id: i64,
    /// `None` before it first starts.
    pub agent_id: Option<i64>,
    pub state: Option<AgentState>,
    /// The model it runs: the project's, else `COS_STAFF_MODEL`. `None`
    /// means Claude's default.
    pub model: Option<String>,
    pub brief: String,
    pub worktree: PathBuf,
    pub branch: String,
    /// Agents it started that are starting or working.
    pub working_agents: usize,
    /// Agents it started that wait for an approval or a trust answer.
    pub waiting_on_you: usize,
}

/// Body of `PUT /v1/projects/{id}/staff`. A null or empty model goes back
/// to the default.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct StaffSettings {
    #[serde(default)]
    pub model: Option<String>,
}

/// Body of `POST /v1/projects/{id}/staff/message`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaffMessage {
    pub text: String,
}

/// `GET /v1/projects/{id}/staff/inbox`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaffInbox {
    pub items: Vec<StaffItem>,
}

/// Runs the Chiefs of Staff. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Staff {
    inner: Arc<Inner>,
}

struct Inner {
    log: EventLog,
    projects: Arc<Projects>,
    runs: Arc<Runs>,
    agents: AgentManager,
    chief: Chief,
    books: StaffBooks,
    config: StaffConfig,
    /// Wakes the delivery loop.
    wake: Arc<Notify>,
    /// Staff agents that got something typed in and haven't started working
    /// on it yet, by agent id.
    woken: Mutex<HashMap<i64, Instant>>,
    /// One start at a time, so two requests can't make two chiefs.
    starting: tokio::sync::Mutex<()>,
}

impl Staff {
    /// Starts following events and agent signals, and delivering wakeups.
    pub(crate) fn start(
        store: Arc<Store>,
        log: EventLog,
        projects: Arc<Projects>,
        runs: Arc<Runs>,
        agents: AgentManager,
        chief: Chief,
        config: StaffConfig,
    ) -> anyhow::Result<Self> {
        let (_, last_seq) = log.stats()?;
        let subscription = log.subscribe(last_seq);
        let signals = agents.signals();
        let wake = Arc::new(Notify::new());
        let inner = Arc::new(Inner {
            books: StaffBooks::new(store, log.clone()),
            log,
            projects,
            runs,
            agents,
            chief,
            config,
            wake: wake.clone(),
            woken: Mutex::default(),
            starting: tokio::sync::Mutex::new(()),
        });
        tokio::spawn(follow(Arc::downgrade(&inner), subscription));
        tokio::spawn(follow_signals(Arc::downgrade(&inner), signals));
        tokio::spawn(deliver_loop(Arc::downgrade(&inner), wake));
        Ok(Self { inner })
    }

    /// The project's Chief of Staff, started or not.
    pub(crate) async fn view(&self, project_id: i64) -> Result<StaffView, ApiError> {
        let inner = &self.inner;
        let project = inner.projects.get(project_id).await?;
        let record = inner.books.record(project.id)?;
        let agent = record.agent_id.and_then(|id| inner.agents.get(id).ok());
        let (worktree, branch) = inner.projects.staff_location(project.id)?;
        let spawned = match &agent {
            Some(agent) => inner.agents.spawned_by(agent.id)?,
            None => Vec::new(),
        };
        let count = |states: &[AgentState]| {
            spawned
                .iter()
                .filter(|agent| states.contains(&agent.state))
                .count()
        };
        Ok(StaffView {
            project_id: project.id,
            agent_id: agent.as_ref().map(|agent| agent.id),
            state: agent.as_ref().map(|agent| agent.state),
            model: inner.model(record.model.as_deref()),
            brief: record.brief,
            worktree,
            branch,
            working_agents: count(&[AgentState::Starting, AgentState::Working]),
            waiting_on_you: count(&[AgentState::WaitingApproval, AgentState::NeedsTrust]),
        })
    }

    /// Sets the project's model, used from its next start.
    pub(crate) async fn configure(
        &self,
        project_id: i64,
        settings: StaffSettings,
    ) -> Result<StaffView, ApiError> {
        let inner = &self.inner;
        let project = inner.projects.get(project_id).await?;
        let model = settings
            .model
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty());
        if model.as_ref().is_some_and(|model| model.chars().count() > 200) {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "that model name is too long".into(),
            ));
        }
        inner.books.set_model(project.id, model.as_deref())?;
        self.view(project.id).await
    }

    /// Starts or resumes the project's Chief of Staff; nothing if it runs.
    pub(crate) async fn start_staff(&self, project_id: i64) -> Result<StaffView, ApiError> {
        self.inner.ensure_running(project_id).await?;
        self.view(project_id).await
    }

    /// A message from you: it waits in the inbox and goes in on its own,
    /// before any news, as soon as the chief is idle. Starts the chief if
    /// needed.
    pub(crate) async fn message(&self, project_id: i64, text: &str) -> Result<(), ApiError> {
        let inner = &self.inner;
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
        let project = inner.projects.get(project_id).await?;
        inner.ensure_running(project.id).await?;
        inner
            .books
            .add(project.id, StaffItemKind::Message, None, None, text)?;
        inner.wake.notify_one();
        Ok(())
    }

    /// News from agents and runs, newest first.
    pub(crate) async fn inbox(&self, project_id: i64) -> Result<Vec<StaffItem>, ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        Ok(self.inner.books.recent(project.id, INBOX_PAGE)?)
    }

    /// Before a project is removed: its Chief of Staff and worktree go too,
    /// unless the chief is running. Workspaces people see must be removed
    /// first, as before.
    pub(crate) async fn remove_project(&self, project_id: i64) -> Result<(), ApiError> {
        let inner = &self.inner;
        let project = inner.projects.get(project_id).await?;
        if project.workspaces.is_empty()
            && let Some(worktree) = inner.projects.existing_staff_workspace(project.id)?
        {
            inner.agents.remove_workspace(worktree.id).await?;
        }
        inner.projects.remove(project.id).await?;
        Ok(())
    }
}

impl Inner {
    /// The model for a project's chief.
    fn model(&self, project_model: Option<&str>) -> Option<String> {
        project_model
            .map(str::to_owned)
            .or_else(|| self.config.model.clone())
    }

    /// Starts the chief if it isn't running and returns its agent.
    async fn ensure_running(&self, project_id: i64) -> Result<Agent, ApiError> {
        let _one_at_a_time = self.starting.lock().await;
        let project = self.projects.get(project_id).await?;
        let record = self.books.record(project.id)?;
        let existing = record.agent_id.and_then(|id| self.agents.get(id).ok());
        if let Some(agent) = existing.as_ref().filter(|agent| agent.state.is_running()) {
            return Ok(agent.clone());
        }
        let worktree = self.projects.staff_workspace(project.id).await?;
        let system_prompt = prompt::system_prompt(&project.name, &record.brief);
        let model = self.model(record.model.as_deref());
        let agent = match existing {
            Some(agent) => {
                self.agents
                    .set_next_launch(agent.id, Some(&system_prompt), model.as_deref())?;
                self.agents.restart(agent.id)?
            }
            None => {
                let request = StartAgent {
                    title: Some(format!("Chief of Staff · {}", project.name)),
                    model,
                    permission_mode: Some("default".into()),
                    user_settings: Some(true),
                    ..StartAgent::default()
                };
                let launch = Launch {
                    system_prompt: Some(system_prompt),
                    ..Launch::default()
                };
                let agent =
                    self.agents
                        .start_agent_with(worktree.id, Role::Staff, None, &request, &launch)?;
                self.books.set_agent(project.id, agent.id)?;
                agent
            }
        };
        self.log.append(
            kind::STAFF_STARTED,
            None,
            json!({ "project_id": project.id, "agent_id": agent.id }),
        )?;
        Ok(agent)
    }

    /// The project whose Chief of Staff started `agent`, with that chief.
    fn spawner(&self, agent: &Agent) -> Option<(i64, i64)> {
        let spawner = agent.spawned_by?;
        let project = self.books.project_of(spawner).ok()??;
        Some((project, spawner))
    }

    fn add(&self, project_id: i64, kind: StaffItemKind, agent: Option<i64>, run: Option<i64>, text: &str) {
        match self.books.add(project_id, kind, agent, run, text) {
            Ok(_) => self.wake.notify_one(),
            Err(failure) => {
                tracing::error!(error = %failure, project = project_id, "the staff inbox missed news")
            }
        }
    }

    fn on_signal(&self, signal: AgentSignal) {
        let (agent_id, news) = match signal {
            AgentSignal::TurnEnded { agent_id, message } => {
                let message = message.unwrap_or_default();
                (agent_id, ("finished a turn".to_owned(), message))
            }
            AgentSignal::Exited { shutdown: true, .. } => return,
            AgentSignal::Exited { agent_id, code, .. } => {
                let code = code.map_or_else(|| "no exit code".into(), |code| format!("exit code {code}"));
                (agent_id, (format!("exited ({code})"), String::new()))
            }
        };
        let Ok(agent) = self.agents.get(agent_id) else {
            return;
        };
        let Some((project, _)) = self.spawner(&agent) else {
            return;
        };
        let (what, body) = news;
        let text = format!("{} {what}\n{body}", describe(&agent));
        self.add(project, StaffItemKind::Agent, Some(agent.id), None, &text);
    }

    fn on_event(&self, event: &LoggedEvent) {
        let data = &event.data;
        match event.kind.as_str() {
            kind::AGENT_STATE_CHANGED => {
                let agent_id = data["agent_id"].as_i64().unwrap_or_default();
                match data["state"].as_str() {
                    Some("idle") => self.wake.notify_one(),
                    Some(state) => {
                        self.woken
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&agent_id);
                        if state == "needs_trust" {
                            self.on_needs_trust(agent_id);
                        }
                    }
                    None => {}
                }
            }
            kind::APPROVAL_REQUESTED => self.on_approval(data),
            kind::RUN_STATE_CHANGED => self.on_run(data),
            _ => {}
        }
    }

    fn on_needs_trust(&self, agent_id: i64) {
        let Ok(agent) = self.agents.get(agent_id) else {
            return;
        };
        if let Some((project, _)) = self.spawner(&agent) {
            let text = format!(
                "{} waits for the user to trust its folder\nThe user answers this in the app.",
                describe(&agent)
            );
            self.add(project, StaffItemKind::Agent, Some(agent.id), None, &text);
        }
    }

    fn on_approval(&self, data: &Value) {
        let Some(agent) = data["agent_id"].as_i64().and_then(|id| self.agents.get(id).ok()) else {
            return;
        };
        if let Some((project, _)) = self.spawner(&agent) {
            let text = format!(
                "{} waits for the user to approve: {}\nApproval {}. The user answers it in the app.",
                describe(&agent),
                data["summary"].as_str().unwrap_or("a tool call"),
                data["approval_id"].as_i64().unwrap_or_default(),
            );
            self.add(project, StaffItemKind::Agent, Some(agent.id), None, &text);
        }
    }

    fn on_run(&self, data: &Value) {
        let Some(run_id) = data["run_id"].as_i64() else {
            return;
        };
        let Some(spawner) = self.runs.spawned_by(run_id).ok().flatten() else {
            return;
        };
        let Some(project) = self.books.project_of(spawner).ok().flatten() else {
            return;
        };
        let to = data["to"].as_str().unwrap_or("?");
        let mut text = format!(
            "Run {run_id} is now {to} (was {})",
            data["from"].as_str().unwrap_or("?")
        );
        if let Some(reason) = data["reason"].as_str() {
            text.push('\n');
            text.push_str(reason);
        }
        if let Some(report) = data["report"].as_str() {
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("\nReport: {report}"));
        }
        self.add(project, StaffItemKind::Run, None, Some(run_id), &text);
    }

    /// Delivers to every idle chief with items waiting.
    async fn deliver_all(&self) {
        let projects = match self.books.projects_waiting() {
            Ok(projects) => projects,
            Err(failure) => {
                tracing::error!(error = %failure, "could not read the staff inbox");
                return;
            }
        };
        for project_id in projects {
            if let Err(failure) = self.deliver(project_id).await {
                tracing::warn!(error = %failure.message, project = project_id, "could not wake the Chief of Staff");
            }
        }
    }

    /// Types your oldest message, or else one wakeup of news, into the
    /// project's chief if it is idle and not still picking up the last
    /// thing typed. Returns whether it typed anything.
    async fn deliver(&self, project_id: i64) -> Result<bool, ApiError> {
        let Some(chief) = self.books.record(project_id)?.agent_id else {
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
        let waiting = self.books.waiting(project_id)?;
        if let Some(message) = waiting.iter().find(|item| item.kind == StaffItemKind::Message) {
            self.type_in(chief, &message.text).await?;
            self.books.delivered(message)?;
            return Ok(true);
        }
        let hour_ago = Timestamp::now().as_millisecond() - HOUR_MS;
        if self.books.wakeups_since(project_id, hour_ago)? >= MAX_WAKEUPS_PER_HOUR {
            return Ok(false);
        }
        let batch = next_batch_by(&waiting, |item| item.text.len());
        if batch.is_empty() {
            return Ok(false);
        }
        let text = prompt::wakeup(batch, waiting.len() - batch.len());
        self.type_in(chief, &text).await?;
        self.books.deliver(project_id, chief, batch)?;
        Ok(true)
    }

    /// Pastes `text` into the chief and presses Return, marking it woken
    /// first: it may start working (which clears the mark) before typing
    /// returns.
    async fn type_in(&self, chief: i64, text: &str) -> Result<(), ApiError> {
        self.woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(chief, Instant::now());
        self.agents.paste_message(chief, text).await.map_err(|failure| {
            self.woken
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&chief);
            failure.into()
        })
    }
}

/// `Agent 15 "Add a README line"`, for inbox items.
fn describe(agent: &Agent) -> String {
    format!("Agent {} \"{}\"", agent.id, agent.title)
}

/// Feeds every new event to the inbox.
async fn follow(inner: Weak<Inner>, mut subscription: Subscription) {
    loop {
        let event = match subscription.next().await {
            Ok(Some(event)) => event,
            Ok(None) => return,
            Err(failure) => {
                tracing::error!(error = %failure, "the Chief of Staff stopped following events");
                return;
            }
        };
        let Some(inner) = inner.upgrade() else { return };
        inner.on_event(&event);
    }
}

/// Feeds turn ends and exits, with their full last message, to the inbox.
async fn follow_signals(inner: Weak<Inner>, mut signals: broadcast::Receiver<AgentSignal>) {
    loop {
        let signal = match signals.recv().await {
            Ok(signal) => signal,
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "the Chief of Staff missed agent signals");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let Some(inner) = inner.upgrade() else { return };
        inner.on_signal(signal);
    }
}

/// Delivers whenever something changes, and every `poll`.
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

fn error(status: StatusCode, message: String) -> ApiError {
    ApiError { status, message }
}
