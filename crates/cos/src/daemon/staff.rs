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
//!   for you) add items; so do the messages you type. When it's idle, the
//!   delivery loop types your oldest message on its own, else one
//!   `[cos · N updates]` wakeup under the caps in [`cos_core::staff`]. While coordination is paused, news waits and only your
//!   messages go in. An agent started with `report: false` adds nothing.
//! - **Restarts:** when the daemon starts, every chief that was running
//!   (now `interrupted`) is resumed, one a second, unless you stopped it
//!   yourself ([`Staff::resume_chiefs`]).

mod prompt;
#[cfg(test)]
mod tests;
mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use cos_core::events::kind;
use cos_core::staff::{MAX_WAKEUPS_PER_HOUR, next_batch};
use cos_core::{
    Agent, AgentState, EventLog, Launch, LoggedEvent, Projects, Role, StaffBooks, StaffItem, StaffItemKind,
    StaffNote, StaffRecord, Store, Subscription, Workspace,
};
use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast};

pub(crate) use prompt::configure;

use super::agents::{AgentManager, AgentSignal, StartAgent};
use super::api::ApiError;
use super::attachments;

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
    /// Between chiefs resumed after a daemon restart.
    pub stagger: Duration,
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
            stagger: Duration::from_secs(1),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            model: None,
            poll: Duration::from_millis(200),
            settle: Duration::from_secs(5),
            ready_wait: Duration::from_secs(10),
            stagger: Duration::from_millis(100),
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
    /// Who it is, written by you; empty when not set. It has no tool to
    /// change it.
    #[serde(default)]
    pub identity: String,
    pub worktree: PathBuf,
    pub branch: String,
    /// Agents it started that are starting or working.
    pub working_agents: usize,
    /// Agents it started that wait for an approval or a trust answer.
    pub waiting_on_you: usize,
    /// Coordination is paused: news waits in the inbox, and only your
    /// messages go in.
    #[serde(default)]
    pub paused: bool,
    /// Items of news waiting to be delivered.
    #[serde(default)]
    pub updates_waiting: usize,
}

/// Body of `PUT /v1/projects/{id}/staff`. A field left out stays as it is.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct StaffSettings {
    /// A null or empty model goes back to the default.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    /// Replaces its brief, trimmed; empty clears it. At most 4 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Replaces its identity, trimmed; empty clears it. At most 8 KiB.
    /// Only this route sets it: the chief has no tool for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

/// A field that is there, even as null, is `Some`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(field: D) -> Result<Option<T>, D::Error> {
    T::deserialize(field).map(Some)
}

/// Body of `POST /v1/projects/{id}/staff/message`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaffMessage {
    #[serde(default)]
    pub text: String,
    /// Files uploaded for the chief (`POST /v1/projects/{id}/staff/attachments`),
    /// by name, named at the end of the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

/// `GET /v1/projects/{id}/staff/inbox`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaffInbox {
    pub items: Vec<StaffItem>,
}

/// `GET /v1/projects/{id}/staff/memory`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaffMemory {
    /// By name.
    pub notes: Vec<StaffNote>,
}

/// Body of `PUT /v1/projects/{id}/staff/memory/{name}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteWrite {
    pub description: String,
    pub body: String,
}

/// Runs the Chiefs of Staff. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Staff {
    inner: Arc<Inner>,
}

struct Inner {
    log: EventLog,
    projects: Arc<Projects>,
    agents: AgentManager,
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
        agents: AgentManager,
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
            agents,
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
            identity: record.identity,
            worktree,
            branch,
            working_agents: count(&[AgentState::Starting, AgentState::Working]),
            waiting_on_you: count(&[
                AgentState::WaitingApproval,
                AgentState::NeedsTrust,
                AgentState::NeedsLogin,
            ]),
            paused: record.paused,
            updates_waiting: inner.books.news_waiting(project.id)?,
        })
    }

    /// Sets the project's model and its brief, both used from its next start
    /// or new session, and pauses or resumes its coordination. Fields left
    /// out stay as they are.
    pub(crate) async fn configure(
        &self,
        project_id: i64,
        settings: StaffSettings,
    ) -> Result<StaffView, ApiError> {
        let inner = &self.inner;
        let project = inner.projects.get(project_id).await?;
        let model = settings.model.map(|model| {
            model
                .map(|model| model.trim().to_owned())
                .filter(|model| !model.is_empty())
        });
        if model.iter().flatten().any(|model| model.chars().count() > 200) {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "that model name is too long".into(),
            ));
        }
        // First, as they're the ones that can be refused (too long).
        if let Some(brief) = &settings.brief {
            inner.books.set_brief(project.id, brief)?;
        }
        if let Some(identity) = &settings.identity {
            inner.books.set_identity(project.id, identity)?;
        }
        if let Some(model) = &model {
            inner.books.set_model(project.id, model.as_deref())?;
        }
        if let Some(paused) = settings.paused {
            inner.books.set_paused(project.id, paused)?;
            if !paused {
                // What waited goes in now.
                inner.wake.notify_one();
            }
        }
        if settings.brief.is_some() || settings.identity.is_some() {
            // Logs `staff.changed` too, and saves the new prompt for its next launch.
            inner.brief_or_notes_changed(project.id).await?;
        } else if model.is_some() || settings.paused.is_some() {
            inner.log_changed(project.id)?;
        }
        self.view(project.id).await
    }

    /// Notes that you stopped this agent, if it is a Chief of Staff, so a
    /// daemon restart leaves it stopped.
    pub(crate) fn stopped_by_user(&self, agent_id: i64) {
        self.inner.set_stopped(agent_id, true);
    }

    /// Notes that you resumed this agent, if it is a Chief of Staff.
    pub(crate) fn resumed_by_user(&self, agent_id: i64) {
        self.inner.set_stopped(agent_id, false);
    }

    /// After a daemon restart: resumes every Chief of Staff that was
    /// running when the daemon stopped or crashed (it's `interrupted` now),
    /// one every `stagger`, unless you stopped it yourself. Your agents and
    /// the ones it started stay `interrupted`. Call it once the daemon
    /// serves.
    pub(crate) fn resume_chiefs(&self) -> tokio::task::JoinHandle<()> {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let chiefs = {
                let Some(inner) = inner.upgrade() else { return };
                match inner.chiefs_to_resume() {
                    Ok(chiefs) => chiefs,
                    Err(failure) => {
                        tracing::error!(error = %failure.message, "could not read the Chiefs of Staff to resume");
                        return;
                    }
                }
            };
            for (index, project_id) in chiefs.into_iter().enumerate() {
                if index > 0 {
                    let Some(stagger) = inner.upgrade().map(|inner| inner.config.stagger) else {
                        return;
                    };
                    tokio::time::sleep(stagger).await;
                }
                let Some(inner) = inner.upgrade() else { return };
                tracing::info!(
                    project = project_id,
                    "resuming the Chief of Staff after a restart"
                );
                if let Err(failure) = inner.ensure_running(project_id).await {
                    tracing::warn!(error = %failure.message, project = project_id, "could not resume the Chief of Staff");
                }
            }
        })
    }

    /// Starts or resumes the project's Chief of Staff; nothing if it runs.
    pub(crate) async fn start_staff(&self, project_id: i64) -> Result<StaffView, ApiError> {
        self.inner.ensure_running(project_id).await?;
        self.view(project_id).await
    }

    /// A message from you: it waits in the inbox and goes in on its own,
    /// before any news, as soon as the chief is idle. Starts the chief if
    /// needed.
    pub(crate) async fn message(
        &self,
        project_id: i64,
        text: &str,
        attachments: &[String],
    ) -> Result<(), ApiError> {
        let inner = &self.inner;
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(error(StatusCode::BAD_REQUEST, "the message is empty".into()));
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!("a message can be at most {MAX_MESSAGE_CHARS} characters"),
            ));
        }
        let project = inner.projects.get(project_id).await?;
        let text = if attachments.is_empty() {
            text.to_owned()
        } else {
            let worktree = inner.projects.staff_workspace(project.id).await?;
            let attached = attachments::find_all(&worktree.path, attachments)?;
            attachments::with_attachments(text, &attached)
        };
        inner.ensure_running(project.id).await?;
        inner.books.add(project.id, StaffItemKind::Message, None, &text)?;
        inner.wake.notify_one();
        Ok(())
    }

    /// The Chief of Staff's worktree, made if needed, for files you attach
    /// before the chief has started.
    pub(crate) async fn attachments_folder(&self, project_id: i64) -> Result<Workspace, ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        Ok(self.inner.projects.staff_workspace(project.id).await?)
    }

    /// News from agents, newest first.
    pub(crate) async fn inbox(&self, project_id: i64) -> Result<Vec<StaffItem>, ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        Ok(self.inner.books.recent(project.id, INBOX_PAGE)?)
    }

    /// The chief's memory notes, by name.
    pub(crate) async fn memory(&self, project_id: i64) -> Result<Vec<StaffNote>, ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        Ok(self.inner.books.notes(project.id)?)
    }

    /// Writes one of the chief's notes for you: it reads the new text on its
    /// next start or new session.
    pub(crate) async fn write_note(
        &self,
        project_id: i64,
        name: &str,
        note: &NoteWrite,
    ) -> Result<StaffNote, ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        let note = self
            .inner
            .books
            .write_note(project.id, name, &note.description, &note.body)?;
        self.inner.brief_or_notes_changed(project.id).await?;
        Ok(note)
    }

    /// Deletes one of the chief's notes for you. 404 if there is none.
    pub(crate) async fn delete_note(&self, project_id: i64, name: &str) -> Result<(), ApiError> {
        let project = self.inner.projects.get(project_id).await?;
        if !self.inner.books.delete_note(project.id, name)? {
            return Err(ApiError::not_found(&format!(
                "there is no note {:?}",
                name.trim()
            )));
        }
        self.inner.brief_or_notes_changed(project.id).await
    }

    /// Drops the chief's Claude session and starts it in a new one, with its
    /// brief and notes as they are now. It stays the same agent, so the
    /// agents it started are still its own. A running chief is stopped
    /// first; one that never started just starts.
    pub(crate) async fn new_session(&self, project_id: i64) -> Result<StaffView, ApiError> {
        self.inner.new_session(project_id).await?;
        self.view(project_id).await
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
        if record.stopped {
            self.books.set_stopped(project.id, false)?;
        }
        let system_prompt = self.system_prompt(&project.name, &record)?;
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
                let agent = self
                    .agents
                    .start_agent_with(worktree.id, Role::Staff, &request, &launch)?;
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

    /// Its standing instructions, with its brief and notes as they are now.
    fn system_prompt(&self, project: &str, record: &StaffRecord) -> Result<String, ApiError> {
        let notes = self.books.notes(record.project_id)?;
        Ok(prompt::system_prompt(
            project,
            &record.identity,
            &record.brief,
            &notes,
        ))
    }

    /// After its brief or notes changed: the chief gets them on its next
    /// start or new session. A running chief keeps what it has until then,
    /// and already knows what it wrote. Logs `staff.changed`.
    async fn brief_or_notes_changed(&self, project_id: i64) -> Result<(), ApiError> {
        self.log_changed(project_id)?;
        let record = self.books.record(project_id)?;
        let Some(agent_id) = record.agent_id else {
            return Ok(());
        };
        if self.agents.get(agent_id).is_err() {
            return Ok(());
        }
        let project = self.projects.get(project_id).await?;
        let system_prompt = self.system_prompt(&project.name, &record)?;
        self.agents.set_next_launch(
            agent_id,
            Some(&system_prompt),
            self.model(record.model.as_deref()).as_deref(),
        )?;
        Ok(())
    }

    async fn new_session(&self, project_id: i64) -> Result<Agent, ApiError> {
        let one_at_a_time = self.starting.lock().await;
        let project = self.projects.get(project_id).await?;
        let record = self.books.record(project.id)?;
        let Some(agent) = record.agent_id.and_then(|id| self.agents.get(id).ok()) else {
            drop(one_at_a_time);
            return self.ensure_running(project.id).await;
        };
        if agent.state.is_running() {
            self.agents.stop(agent.id)?;
            self.until_stopped(agent.id).await?;
        }
        self.woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&agent.id);
        if record.stopped {
            self.books.set_stopped(project.id, false)?;
        }
        let system_prompt = self.system_prompt(&project.name, &record)?;
        let model = self.model(record.model.as_deref());
        self.agents
            .set_next_launch(agent.id, Some(&system_prompt), model.as_deref())?;
        let agent = self.agents.new_session(agent.id)?;
        tracing::info!(
            project = project.id,
            agent = agent.id,
            "the Chief of Staff started a new session"
        );
        self.log.append(
            kind::STAFF_STARTED,
            None,
            json!({ "project_id": project.id, "agent_id": agent.id }),
        )?;
        Ok(agent)
    }

    /// Waits for a stopping agent's process to end.
    async fn until_stopped(&self, agent_id: i64) -> Result<(), ApiError> {
        let deadline = tokio::time::Instant::now() + self.config.ready_wait;
        while self.agents.get(agent_id)?.state.is_running() {
            if tokio::time::Instant::now() >= deadline {
                return Err(error(
                    StatusCode::CONFLICT,
                    format!("agent {agent_id} didn't stop in time; try again"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Projects whose chief the daemon stopped or lost (`interrupted`) and
    /// you didn't stop.
    fn chiefs_to_resume(&self) -> Result<Vec<i64>, ApiError> {
        let mut chiefs = Vec::new();
        for record in self.books.records()? {
            let Some(agent) = record.agent_id.and_then(|id| self.agents.get(id).ok()) else {
                continue;
            };
            if agent.state == AgentState::Interrupted && !record.stopped {
                chiefs.push(record.project_id);
            }
        }
        Ok(chiefs)
    }

    /// Records your intent for a chief: stopped, or wanted running.
    fn set_stopped(&self, agent_id: i64, stopped: bool) {
        let Ok(Some(project_id)) = self.books.project_of(agent_id) else {
            return;
        };
        if let Err(failure) = self.books.set_stopped(project_id, stopped) {
            tracing::error!(error = %failure, project = project_id, "could not record the Chief of Staff's stop");
        }
    }

    /// The project whose Chief of Staff started `agent`, with that chief, if
    /// the agent reports to it.
    fn spawner(&self, agent: &Agent) -> Option<(i64, i64)> {
        if !agent.report {
            return None;
        }
        let spawner = agent.spawned_by?;
        let project = self.books.project_of(spawner).ok()??;
        Some((project, spawner))
    }

    fn add(&self, project_id: i64, kind: StaffItemKind, agent: Option<i64>, text: &str) {
        match self.books.add(project_id, kind, agent, text) {
            Ok(_) => self.wake.notify_one(),
            Err(failure) => {
                tracing::error!(error = %failure, project = project_id, "the staff inbox missed news")
            }
        }
    }

    /// Logs `staff.stopped` when a Chief of Staff's process ends.
    fn log_stopped(&self, agent_id: i64) {
        let Ok(Some(project_id)) = self.books.project_of(agent_id) else {
            return;
        };
        let logged = self.log.append(
            kind::STAFF_STOPPED,
            None,
            json!({ "project_id": project_id, "agent_id": agent_id }),
        );
        if let Err(failure) = logged {
            tracing::error!(error = %failure, project = project_id, "could not log staff.stopped");
        }
    }

    /// Logs `staff.changed`: its model, brief, notes or pause changed.
    fn log_changed(&self, project_id: i64) -> Result<(), ApiError> {
        self.log
            .append(kind::STAFF_CHANGED, None, json!({ "project_id": project_id }))?;
        Ok(())
    }

    fn on_signal(&self, signal: AgentSignal) {
        let (agent_id, news) = match signal {
            AgentSignal::TurnEnded { agent_id, message } => {
                let message = message.unwrap_or_default();
                (agent_id, ("finished a turn".to_owned(), message))
            }
            AgentSignal::Exited {
                agent_id,
                shutdown: true,
                ..
            } => {
                if self
                    .agents
                    .get(agent_id)
                    .is_ok_and(|agent| agent.role == Role::Staff)
                {
                    self.log_stopped(agent_id);
                }
                return;
            }
            AgentSignal::Exited { agent_id, code, .. } => {
                let code = code.map_or_else(|| "no exit code".into(), |code| format!("exit code {code}"));
                (agent_id, (format!("exited ({code})"), String::new()))
            }
        };
        let Ok(agent) = self.agents.get(agent_id) else {
            return;
        };
        if agent.role == Role::Staff && news.0.starts_with("exited") {
            self.log_stopped(agent.id);
            return;
        }
        let Some((project, _)) = self.spawner(&agent) else {
            return;
        };
        let (what, body) = news;
        let text = format!("{} {what}\n{body}", describe(&agent));
        self.add(project, StaffItemKind::Agent, Some(agent.id), &text);
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
            self.add(project, StaffItemKind::Agent, Some(agent.id), &text);
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
            self.add(project, StaffItemKind::Agent, Some(agent.id), &text);
        }
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

    /// Types your oldest message, or else one wakeup of news (unless
    /// coordination is paused), into the project's chief if it is idle and not still picking up the last
    /// thing typed. Returns whether it typed anything.
    async fn deliver(&self, project_id: i64) -> Result<bool, ApiError> {
        let record = self.books.record(project_id)?;
        let Some(chief) = record.agent_id else {
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
        if record.paused {
            return Ok(false);
        }
        let hour_ago = Timestamp::now().as_millisecond() - HOUR_MS;
        if self.books.wakeups_since(project_id, hour_ago)? >= MAX_WAKEUPS_PER_HOUR {
            return Ok(false);
        }
        let batch = next_batch(&waiting);
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
