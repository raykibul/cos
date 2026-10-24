//! The daemon's agent runtime: Claude sessions running in terminals the
//! daemon owns, one per agent, in a workspace's folder. The contract with the
//! apps is `docs/agents-api.md`.
//!
//! - Each agent is a [`ClaudeTerminal`]. Its output feeds a screen and is
//!   streamed to whoever watches (see [`AgentManager::attach`]); watching
//!   never slows the terminal down.
//! - Every agent's hooks report to one [`HookServer`] at `<home>/hooks.sock`,
//!   routed to agents by session id (see [`hooks`]). That's how the daemon
//!   follows state and turns permission requests into approvals. The screen
//!   is read only to spot the trust dialog and the login screen.
//! - Every launch and resume gets the environment set for the agent's
//!   project (see [`cos_core::Environment`]), as it is at that moment.
//! - Records live in the database ([`cos_core::Agents`]), so they survive a
//!   restart; agents that were running come back as `interrupted`.

mod approvals;
mod hooks;
#[cfg(test)]
pub(crate) mod tests;
mod transcripts;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use cos_core::{
    Agent, AgentState, Agents, Approval, ApprovalState, Environment, Launch, NewAgent, ProjectError,
    Projects, Role, StoreError, Variables,
};
use cos_harness::claude::hooks::{HookEvent, HookServer};
use cos_harness::{
    ClaudeOptions, ClaudeTerminal, HarnessError, HookTarget, PermissionMode, TerminalAttach, TerminalEvent,
    TerminalOptions,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use super::transcript::{self, Transcript};
use crate::home::CosHome;

const DEFAULT_ROWS: u16 = 40;
const DEFAULT_COLS: u16 = 120;
/// Larger terminals are refused as a mistake.
const MAX_SIZE: u16 = 1_000;
/// A missing title becomes this many characters of the prompt.
const TITLE_FROM_PROMPT: usize = 60;
const MAX_TITLE_CHARS: usize = 200;
/// How long `stop` waits for `/exit` before killing the process.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the daemon gives its agents to exit when it stops itself.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(8);
/// Between pressing Esc to end a turn and typing `/exit`.
const INTERRUPT_PAUSE: Duration = Duration::from_millis(300);
/// How often a stopping agent is checked for having exited.
const EXIT_POLL: Duration = Duration::from_millis(50);
/// Signals buffered per subscriber; one that falls further behind gets `Lagged`.
const SIGNAL_BUFFER: usize = 256;

/// What the agents do that the pipeline waits for. In memory only; the
/// event log has the same facts for apps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSignal {
    /// A `Stop` hook: the turn ended, with its full last message.
    TurnEnded { agent_id: i64, message: Option<String> },
    /// The process ended. `shutdown` if the daemon stopped it on its way down.
    Exited {
        agent_id: i64,
        code: Option<i32>,
        shutdown: bool,
    },
}

/// A permission request that went to you, as the approvals policy saw it.
/// jev (see [`crate::daemon::judge`]) follows these in shadow mode.
#[derive(Debug, Clone)]
pub(crate) struct PermissionAsked {
    pub approval_id: i64,
    pub agent_id: i64,
    pub workspace_id: i64,
    pub summary: String,
    pub request: cos_policy::ApprovalRequest,
    pub verdict: cos_policy::Verdict,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentConfig {
    /// The `claude` binary. A bare name is looked up on the daemon's PATH
    /// when an agent starts, never through a shell, so shell functions and
    /// aliases named `claude` don't apply.
    pub claude: PathBuf,
    /// The shell command Claude's hooks run, such as `'/usr/local/bin/cos' hook`.
    pub hook_command: String,
    pub hook_socket: PathBuf,
    /// Extra environment for every agent's process, over the project's and
    /// global variables (tests point the fake's files here).
    pub env: Vec<(String, String)>,
    pub stop_timeout: Duration,
    pub shutdown_timeout: Duration,
    /// The `cos` binary a Chief of Staff runs as its MCP server (`cos mcp`).
    pub cos_exe: PathBuf,
    /// The cos home that server talks to.
    pub cos_home: PathBuf,
}

impl AgentConfig {
    /// The daemon's settings: `claude` from `$COS_CLAUDE_BIN`, PATH or where
    /// its installers put it (see [`crate::which::claude`]), and this
    /// binary's `cos hook` as the relay.
    pub(crate) fn for_home(home: &CosHome) -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let claude = crate::which::claude();
        if claude.is_absolute() {
            tracing::info!(claude = %claude.display(), "agents run this claude");
        } else if crate::which::find_on_path(&claude.to_string_lossy(), std::env::var_os("PATH").as_deref())
            .is_none()
        {
            tracing::warn!(
                "no `{}` on PATH or where Claude Code installs it; agents won't start. \
                 Install Claude Code, or set {} to its path",
                claude.display(),
                crate::which::CLAUDE_BIN_ENV
            );
        }
        Ok(Self {
            claude,
            hook_command: format!("{} hook", crate::try_pty::shell_quote(&exe)),
            hook_socket: home.hook_socket(),
            env: Vec::new(),
            stop_timeout: STOP_TIMEOUT,
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            cos_exe: exe,
            cos_home: home.root().to_path_buf(),
        })
    }

    /// For tests: `cos-fake-claude` as both Claude and the hook relay, files
    /// in `dir`, short timeouts.
    #[cfg(test)]
    pub(crate) fn for_tests(dir: &Path) -> Self {
        let fake = tests::fake_claude();
        Self {
            hook_command: format!("{} hook", crate::try_pty::shell_quote(&fake)),
            claude: fake,
            hook_socket: dir.join("hooks.sock"),
            // The fake's transcripts stay in the test's folder, and it
            // writes what it was launched with next to them.
            env: vec![
                (
                    "COS_FAKE_CLAUDE_TRANSCRIPTS".into(),
                    dir.join("transcripts").display().to_string(),
                ),
                (
                    "COS_FAKE_CLAUDE_ENV".into(),
                    dir.join("env").display().to_string(),
                ),
            ],
            stop_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
            cos_exe: tests::cos_binary(),
            cos_home: dir.to_path_buf(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentError {
    /// Bad input. The message is written for the user.
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// Claude couldn't be started or driven.
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<ProjectError> for AgentError {
    fn from(error: ProjectError) -> Self {
        match error {
            ProjectError::Invalid(message) => Self::Invalid(message),
            ProjectError::NotFound(message) => Self::NotFound(message),
            ProjectError::Conflict(message) => Self::Conflict(message),
            ProjectError::Store(error) => Self::Store(error),
        }
    }
}

impl From<HarnessError> for AgentError {
    fn from(error: HarnessError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Body of `POST /v1/workspaces/{id}/agents`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct StartAgent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Load the user's own settings. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_settings: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

/// `GET /v1/agents/{id}/screen`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Screen {
    pub rows: u16,
    pub cols: u16,
    pub text: String,
    /// `[row, col]`, counted from 0.
    pub cursor: [u16; 2],
}

/// What someone opening an agent's terminal gets.
pub(crate) enum TerminalView {
    /// The terminal the daemon holds, running or exited.
    Live(TerminalAttach),
    /// The daemon has no terminal for it (it ran before a restart).
    Gone {
        rows: u16,
        cols: u16,
        exit_code: Option<i32>,
    },
}

/// Runs the agents. Cheap to clone.
#[derive(Clone)]
pub(crate) struct AgentManager {
    inner: Arc<Inner>,
}

struct Inner {
    agents: Agents,
    projects: Arc<Projects>,
    /// The variables set for agents, global and per project.
    env: Environment,
    config: AgentConfig,
    live: Mutex<Live>,
    signals: broadcast::Sender<AgentSignal>,
    /// Permission requests routed to you.
    asked: broadcast::Sender<PermissionAsked>,
    /// Transcripts parsed so far, by file.
    parsed: transcript::Cache,
    /// Kept for its socket; dropping it stops the hooks.
    _hook_server: HookServer,
}

/// The daemon's in-memory side of its agents. Changes to an agent's record
/// are made while this is locked, so they happen in a clear order.
#[derive(Default)]
struct Live {
    /// Every agent with a terminal in this daemon, running or exited.
    terminals: HashMap<i64, Running>,
    /// Running agents by session id, for routing hooks.
    by_session: HashMap<String, i64>,
    /// Permission hooks waiting for an answer, by approval id.
    waiting: HashMap<i64, hooks::Waiting>,
    /// Approval ids by the hook id of their waiting hook.
    hook_approvals: HashMap<u64, i64>,
    /// Each agent's transcript, by agent id.
    transcripts: HashMap<i64, transcripts::Note>,
    /// The daemon is stopping: no new agents.
    stopping: bool,
}

struct Running {
    terminal: Arc<ClaudeTerminal>,
    /// The daemon answers its trust dialog: a Chief of Staff, or an agent
    /// one started, in a worktree cos made for a registered project.
    auto_trust: bool,
    workspace_id: i64,
    /// A hook has arrived, so Claude is past any trust dialog.
    heard: bool,
    /// The trust dialog was spotted, or answered through cos.
    trust_seen: bool,
    /// Claude's login screen was spotted.
    login_seen: bool,
    /// Tools that started (`PreToolUse`) and haven't finished, newest last.
    open_tools: Vec<hooks::OpenTool>,
    exited: bool,
    /// Stopped by the daemon on its way down: it ends `interrupted`.
    shutdown: bool,
}

impl AgentManager {
    /// Listens for hooks and marks agents left running by an earlier daemon
    /// as interrupted.
    pub(crate) fn start(
        agents: Agents,
        projects: Arc<Projects>,
        env: Environment,
        config: AgentConfig,
    ) -> anyhow::Result<Self> {
        let (hook_server, hook_events) = HookServer::bind(&config.hook_socket).map_err(|error| {
            anyhow::anyhow!("could not listen on {}: {error}", config.hook_socket.display())
        })?;
        agents.interrupt_running()?;
        // Their hooks died with the daemon that heard them.
        for approval in agents.approvals(Some(ApprovalState::Pending))? {
            agents.decide_approval(approval.id, ApprovalState::Settled, None, None)?;
        }
        let inner = Arc::new(Inner {
            agents,
            projects,
            env,
            config,
            live: Mutex::default(),
            signals: broadcast::channel(SIGNAL_BUFFER).0,
            asked: broadcast::channel(SIGNAL_BUFFER).0,
            parsed: transcript::Cache::default(),
            _hook_server: hook_server,
        });
        tokio::spawn(follow_hooks(Arc::downgrade(&inner), hook_events));
        Ok(Self { inner })
    }

    /// The variables set for agents, global and per project.
    pub(crate) fn env(&self) -> &Environment {
        &self.inner.env
    }

    pub(crate) fn list(
        &self,
        workspace_id: Option<i64>,
        state: Option<AgentState>,
    ) -> Result<Vec<Agent>, AgentError> {
        Ok(self.inner.agents.list(workspace_id, state)?)
    }

    pub(crate) fn get(&self, id: i64) -> Result<Agent, AgentError> {
        self.inner.agent(id)
    }

    /// Every [`AgentSignal`] from now on.
    pub(crate) fn signals(&self) -> broadcast::Receiver<AgentSignal> {
        self.inner.signals.subscribe()
    }

    /// Every permission request routed to you from now on.
    pub(crate) fn permission_requests(&self) -> broadcast::Receiver<PermissionAsked> {
        self.inner.asked.subscribe()
    }

    /// Starts an agent in a workspace's folder, with `prompt` submitted
    /// straight away if given.
    pub(crate) fn start_agent(
        &self,
        workspace_id: i64,
        role: Role,
        request: &StartAgent,
    ) -> Result<Agent, AgentError> {
        self.start_agent_with(workspace_id, role, request, &Launch::default())
    }

    /// [`Self::start_agent`] with more: `launch` can give it another folder,
    /// `--append-system-prompt`, `--mcp-config` and who started it. They're
    /// recorded, so a resume uses them again.
    pub(crate) fn start_agent_with(
        &self,
        workspace_id: i64,
        role: Role,
        request: &StartAgent,
        launch: &Launch,
    ) -> Result<Agent, AgentError> {
        let inner = &self.inner;
        let workspace = inner.projects.workspace(workspace_id)?;
        let mode = request.permission_mode.as_deref().unwrap_or("default");
        mode.parse::<PermissionMode>().map_err(AgentError::Invalid)?;
        let (rows, cols) = (
            check_size(request.rows.unwrap_or(DEFAULT_ROWS))?,
            check_size(request.cols.unwrap_or(DEFAULT_COLS))?,
        );
        let title = clean(request.title.as_deref(), MAX_TITLE_CHARS)
            .or_else(|| clean(request.prompt.as_deref(), TITLE_FROM_PROMPT));
        let session_id = ClaudeOptions::new(&workspace.path).session_id;

        let agent = inner.agents.create(NewAgent {
            workspace_id,
            role,
            title,
            session_id,
            model: clean(request.model.as_deref(), MAX_TITLE_CHARS),
            permission_mode: mode.to_owned(),
            user_settings: request.user_settings.unwrap_or(true),
            rows,
            cols,
        })?;
        let agent = if *launch == Launch::default() {
            agent
        } else {
            inner.agents.set_launch(agent.id, launch)?
        };
        let prompt = request.prompt.clone().filter(|prompt| !prompt.trim().is_empty());
        let cwd = agent.cwd.clone().unwrap_or(workspace.path);
        inner.launch(&agent, &cwd, prompt, false).inspect_err(|_| {
            let _ = inner.agents.discard(agent.id);
        })
    }

    /// Starts `claude --resume` for an exited or interrupted agent, as the
    /// same agent.
    pub(crate) fn resume(&self, id: i64) -> Result<Agent, AgentError> {
        self.relaunch(id, false)
    }

    /// [`Self::resume`], except that a session Claude never saved (no
    /// transcript yet, as when it was stopped before its first message)
    /// starts afresh with the same session id, since `--resume` would fail.
    pub(crate) fn restart(&self, id: i64) -> Result<Agent, AgentError> {
        self.relaunch(id, true)
    }

    fn relaunch(&self, id: i64, fresh_if_unsaved: bool) -> Result<Agent, AgentError> {
        let inner = &self.inner;
        let agent = inner.agent(id)?;
        if agent.state.is_running() {
            return Err(AgentError::Conflict(format!(
                "agent {id} is still running; only an exited or interrupted agent can be resumed"
            )));
        }
        let workspace = inner.projects.workspace(agent.workspace_id)?;
        let cwd = agent.cwd.clone().unwrap_or(workspace.path);
        if !cwd.is_dir() {
            return Err(AgentError::Conflict(format!(
                "agent {id}'s folder {} is gone, so it can't be resumed",
                cwd.display()
            )));
        }
        let saved = agent.transcript_path.as_deref().is_some_and(Path::is_file);
        let resume = saved || !fresh_if_unsaved;
        let agent = inner.agents.restart(id)?;
        inner.launch(&agent, &cwd, None, resume).inspect_err(|_| {
            let _ = inner.agents.finish(id, AgentState::Exited, None);
        })
    }

    /// Types `text` and presses Return.
    pub(crate) async fn input(&self, id: i64, text: &str) -> Result<(), AgentError> {
        let terminal = self.inner.running_terminal(id)?.0;
        Ok(terminal.submit(text).await?)
    }

    /// Sends a message as one bracketed paste, then Return, whatever its length.
    pub(crate) async fn paste_message(&self, id: i64, text: &str) -> Result<(), AgentError> {
        let terminal = self.inner.running_terminal(id)?.0;
        Ok(terminal.submit_pasted(text).await?)
    }

    /// Replaces the system prompt and model the agent gets on its next
    /// launch. A running agent keeps what it has until then.
    pub(crate) fn set_next_launch(
        &self,
        id: i64,
        system_prompt: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), AgentError> {
        self.inner.agents.set_system_prompt(id, system_prompt)?;
        self.inner.agents.set_model(id, model)?;
        Ok(())
    }

    /// The agents a Chief of Staff started, oldest first.
    pub(crate) fn spawned_by(&self, spawner: i64) -> Result<Vec<Agent>, AgentError> {
        Ok(self.inner.agents.spawned_by(spawner)?)
    }

    /// Sends raw keystrokes.
    pub(crate) fn keys(&self, id: i64, bytes: &[u8]) -> Result<(), AgentError> {
        let terminal = self.inner.running_terminal(id)?.0;
        Ok(terminal.write(bytes)?)
    }

    pub(crate) fn resize(&self, id: i64, rows: u16, cols: u16) -> Result<(), AgentError> {
        let (rows, cols) = (check_size(rows)?, check_size(cols)?);
        let terminal = self.inner.running_terminal(id)?.0;
        terminal.resize(rows, cols)?;
        self.inner.agents.set_size(id, rows, cols)?;
        Ok(())
    }

    /// Answers Claude's trust dialog with yes.
    pub(crate) async fn trust(&self, id: i64) -> Result<(), AgentError> {
        let (terminal, state) = self.inner.running_terminal(id)?;
        if state != AgentState::NeedsTrust {
            return Err(AgentError::Conflict(format!(
                "agent {id} isn't asking whether to trust its folder"
            )));
        }
        terminal.trust_folder().await?;
        let _live = self.inner.live();
        self.inner.agents.set_state(id, AgentState::Starting)?;
        Ok(())
    }

    /// Asks the agent to exit (`/exit`), and kills it if it hasn't after the
    /// stop timeout. Returns at once.
    pub(crate) fn stop(&self, id: i64) -> Result<(), AgentError> {
        let (terminal, state) = self.inner.running_terminal(id)?;
        tokio::spawn(end_session(terminal, state, self.inner.config.stop_timeout));
        Ok(())
    }

    /// Forgets an exited or interrupted agent.
    pub(crate) fn remove(&self, id: i64) -> Result<(), AgentError> {
        let mut live = self.inner.live();
        let agent = self.inner.agent(id)?;
        if agent.state.is_running() {
            return Err(AgentError::Conflict(format!(
                "agent {id} is still running; stop it first"
            )));
        }
        live.terminals.remove(&id);
        live.transcripts.remove(&id);
        self.inner.agents.remove(id)?;
        Ok(())
    }

    pub(crate) fn screen(&self, id: i64) -> Result<Screen, AgentError> {
        let agent = self.inner.agent(id)?;
        let terminal = self.inner.terminal(id);
        Ok(match terminal {
            Some(terminal) => {
                let (rows, cols) = terminal.size();
                let (row, col) = terminal.cursor();
                Screen {
                    rows,
                    cols,
                    text: terminal.screen(),
                    cursor: [row, col],
                }
            }
            None => Screen {
                rows: agent.rows,
                cols: agent.cols,
                text: String::new(),
                cursor: [0, 0],
            },
        })
    }

    /// The agent's transcript as chat messages, those after `after`, and how
    /// full its context is (see [`transcript`]). Empty while Claude hasn't
    /// said where it is.
    pub(crate) async fn transcript(&self, id: i64, after: usize) -> Result<Transcript, AgentError> {
        let agent = self.inner.agent(id)?;
        let Some(path) = agent.transcript_path else {
            return Ok(Transcript::default());
        };
        let inner = self.inner.clone();
        let read = tokio::task::spawn_blocking(move || inner.parsed.read(&path, agent.model.as_deref()))
            .await
            .map_err(|error| AgentError::Failed(error.to_string()))?;
        let transcript = read.map_err(|error| {
            AgentError::Failed(format!("could not read agent {id}'s transcript: {error}"))
        })?;
        Ok(transcript.after(after))
    }

    /// The terminal for someone who starts watching it.
    pub(crate) fn attach(&self, id: i64) -> Result<TerminalView, AgentError> {
        let agent = self.inner.agent(id)?;
        Ok(match self.inner.terminal(id) {
            Some(terminal) => TerminalView::Live(terminal.attach()),
            None => TerminalView::Gone {
                rows: agent.rows,
                cols: agent.cols,
                exit_code: agent.exit_code,
            },
        })
    }

    pub(crate) fn approvals(&self, state: Option<ApprovalState>) -> Result<Vec<Approval>, AgentError> {
        Ok(self.inner.agents.approvals(state)?)
    }

    pub(crate) fn approval(&self, id: i64) -> Result<Approval, AgentError> {
        self.inner
            .agents
            .approval(id)?
            .ok_or_else(|| AgentError::NotFound(format!("there is no approval {id}")))
    }

    /// Answers a pending approval for the user.
    pub(crate) fn decide(
        &self,
        id: i64,
        allow: bool,
        message: Option<String>,
    ) -> Result<Approval, AgentError> {
        self.inner.answer(id, allow, message)
    }

    /// Removes a workspace, refusing while any of its agents runs. Its
    /// exited agents are forgotten with it.
    pub(crate) async fn remove_workspace(&self, id: i64) -> Result<(), AgentError> {
        let agents = self.inner.agents.list(Some(id), None)?;
        if let Some(agent) = agents.iter().find(|agent| agent.state.is_running()) {
            return Err(AgentError::Conflict(format!(
                "agent {} ({}) is still running in this workspace; stop it first",
                agent.id, agent.title
            )));
        }
        self.inner.projects.remove_workspace(id).await?;
        let mut live = self.inner.live();
        for agent in agents {
            live.terminals.remove(&agent.id);
            self.inner.agents.log_removed(agent.id)?;
        }
        Ok(())
    }

    /// Stops every running agent for a daemon on its way down, killing any
    /// that haven't exited in time. They end up `interrupted`.
    pub(crate) async fn shutdown(&self) {
        let inner = &self.inner;
        let stopping: Vec<(i64, Arc<ClaudeTerminal>)> = {
            let mut live = inner.live();
            live.stopping = true;
            live.terminals
                .iter_mut()
                .filter(|(_, running)| !running.exited)
                .map(|(id, running)| {
                    running.shutdown = true;
                    (*id, running.terminal.clone())
                })
                .collect()
        };
        let ends: Vec<_> = stopping
            .iter()
            .map(|(id, terminal)| {
                let state = inner
                    .agents
                    .get(*id)
                    .ok()
                    .flatten()
                    .map_or(AgentState::Idle, |agent| agent.state);
                tokio::spawn(end_session(
                    terminal.clone(),
                    state,
                    inner.config.shutdown_timeout,
                ))
            })
            .collect();
        for end in ends {
            let _ = end.await;
        }
        // Each exit is recorded by its watcher; give them a moment, then
        // record whatever is left.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        for (id, _) in &stopping {
            while inner
                .live()
                .terminals
                .get(id)
                .is_some_and(|running| !running.exited)
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(EXIT_POLL).await;
            }
            let _live = inner.live();
            if inner.agent(*id).is_ok_and(|agent| agent.state.is_running()) {
                let _ = inner.agents.finish(*id, AgentState::Interrupted, None);
            }
        }
    }
}

impl Inner {
    fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn agent(&self, id: i64) -> Result<Agent, AgentError> {
        self.agents
            .get(id)?
            .ok_or_else(|| AgentError::NotFound(format!("there is no agent {id}")))
    }

    fn terminal(&self, id: i64) -> Option<Arc<ClaudeTerminal>> {
        self.live()
            .terminals
            .get(&id)
            .map(|running| running.terminal.clone())
    }

    /// The terminal of a running agent, and its state. 409 once it exited.
    fn running_terminal(&self, id: i64) -> Result<(Arc<ClaudeTerminal>, AgentState), AgentError> {
        let agent = self.agent(id)?;
        let terminal = self
            .terminal(id)
            .filter(|terminal| agent.state.is_running() && terminal.exit_status().is_none());
        terminal.map(|terminal| (terminal, agent.state)).ok_or_else(|| {
            AgentError::Conflict(format!(
                "agent {id} is not running ({}); resume it first",
                serde_json::to_value(agent.state)
                    .ok()
                    .and_then(|state| state.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ))
        })
    }

    /// Spawns Claude for `agent` (already recorded as `starting`) and starts
    /// following it.
    fn launch(
        self: &Arc<Self>,
        agent: &Agent,
        cwd: &Path,
        prompt: Option<String>,
        resume: bool,
    ) -> Result<Agent, AgentError> {
        if !cwd.is_dir() {
            return Err(AgentError::Conflict(format!(
                "the workspace folder {} is missing",
                cwd.display()
            )));
        }
        let mut claude = ClaudeOptions::new(cwd);
        claude.binary = resolve_binary(&self.config.claude, std::env::var_os("PATH").as_deref())
            .unwrap_or_else(|| self.config.claude.clone());
        claude.session_id = agent.session_id.clone();
        claude.resume = resume;
        claude.model = agent.model.clone();
        claude.append_system_prompt = agent.system_prompt.clone();
        claude.mcp_config = agent.mcp_config.clone();
        claude.permission_mode = agent.permission_mode.parse().map_err(AgentError::Invalid)?;
        // The daemon's environment < global < project < cos's own: the
        // terminal adds TERM, the hook socket and so on last.
        let extra = self.agent_env(agent)?;
        let env_names: Vec<String> = extra.keys().cloned().collect();
        claude.env = extra.into_iter().chain(self.config.env.clone()).collect();
        // The chief's MCP server names the agent, so it's added here, where
        // the id is known, on every launch and resume.
        if agent.role == Role::Staff {
            super::staff::configure(&mut claude, &self.config.cos_exe, &self.config.cos_home, agent.id);
        }
        let mut options = TerminalOptions::new(claude);
        options.prompt = prompt;
        // An agent you start is your own session, so by default your
        // settings apply: allow rules, MCP servers, theme.
        options.user_settings = agent.user_settings;
        options.rows = agent.rows;
        options.cols = agent.cols;
        options.hooks = Some(HookTarget {
            // The agent id goes with every hook, so a new session in the
            // same terminal (`/clear`) is still known to be this agent's.
            command: format!("{} --agent {}", self.config.hook_command, agent.id),
            socket: self.config.hook_socket.clone(),
        });

        // Locked from before the spawn until the agent is registered, so its
        // first hook can't arrive before the daemon knows the session.
        let mut live = self.live();
        if live.stopping {
            return Err(AgentError::Conflict("the daemon is stopping".into()));
        }
        let (terminal, output) = ClaudeTerminal::spawn(options)?;
        let terminal = Arc::new(terminal);
        live.by_session.insert(agent.session_id.clone(), agent.id);
        live.terminals.insert(
            agent.id,
            Running {
                terminal: terminal.clone(),
                auto_trust: agent.role != Role::User || agent.spawned_by.is_some(),
                workspace_id: agent.workspace_id,
                heard: false,
                trust_seen: false,
                login_seen: false,
                open_tools: Vec::new(),
                exited: false,
                shutdown: false,
            },
        );
        let started = self.agents.started(agent.id, terminal.pid(), &env_names);
        drop(live);
        tokio::spawn(watch_output(Arc::downgrade(self), agent.id, output));
        Ok(started?)
    }

    /// The extra variables for an agent: its project's on top of the global
    /// ones, as they are now, with a leading `~` expanded to the daemon
    /// user's home. Names cos keeps for itself are skipped, in case the
    /// database holds one from before they were refused.
    fn agent_env(&self, agent: &Agent) -> Result<Variables, AgentError> {
        let project_id = self.projects.workspace(agent.workspace_id)?.project_id;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut env = Variables::new();
        for (name, value) in self.env.effective(project_id)? {
            if cos_harness::claude::is_reserved_env(&name) {
                // The name only: values are secrets.
                tracing::warn!(agent = agent.id, %name, "skipping a variable cos sets itself");
                continue;
            }
            let value = cos_core::env::expand_home(&value, home.as_deref());
            env.insert(name, value);
        }
        Ok(env)
    }

    /// Output arrived. Before the first hook, check for the login screen
    /// (the agent's Claude config dir isn't logged in) and the trust dialog.
    /// A Chief of Staff, or an agent one started, works in a registered
    /// project, which counts as trusted, so the daemon answers the dialog for
    /// it (the one exception to "cos never answers the trust dialog by
    /// itself").
    fn on_output(&self, id: i64) {
        let mut live = self.live();
        let Some(running) = live.terminals.get_mut(&id) else {
            return;
        };
        if running.heard {
            return;
        }
        if !running.login_seen && running.terminal.asks_for_login() {
            // Nobody can log in for it; whoever opens the terminal does.
            running.login_seen = true;
            tracing::info!(agent = id, "Claude is asking to log in");
            if let Err(error) = self.agents.set_state(id, AgentState::NeedsLogin) {
                tracing::error!(%error, agent = id, "could not record the login screen");
            }
            return;
        }
        if running.trust_seen || !running.terminal.asks_for_trust() {
            return;
        }
        running.trust_seen = true;
        if running.auto_trust {
            let terminal = running.terminal.clone();
            let agents = self.agents.clone();
            tracing::info!(agent = id, "answering the trust dialog for an agent cos started");
            tokio::spawn(async move {
                if let Err(error) = terminal.trust_folder().await {
                    // Leave it to the user rather than stuck where nobody looks.
                    tracing::error!(%error, agent = id, "could not answer the trust dialog");
                    if let Err(error) = agents.set_state(id, AgentState::NeedsTrust) {
                        tracing::error!(%error, agent = id, "could not record the trust dialog");
                    }
                }
            });
            return;
        }
        if let Err(error) = self.agents.set_state(id, AgentState::NeedsTrust) {
            tracing::error!(%error, agent = id, "could not record the trust dialog");
        }
    }

    fn on_exit(&self, id: i64, code: Option<i32>) {
        let mut live = self.live();
        let shutdown = live.terminals.get_mut(&id).is_some_and(|running| {
            running.exited = true;
            running.shutdown
        });
        live.by_session.retain(|_, agent| *agent != id);
        let state = if shutdown {
            AgentState::Interrupted
        } else {
            AgentState::Exited
        };
        let result = self
            .settle_all(&mut live, id, None)
            .and_then(|()| self.agents.finish(id, state, code));
        if let Err(error) = result {
            tracing::error!(%error, agent = id, "could not record an agent's exit");
        }
        let _ = self.signals.send(AgentSignal::Exited {
            agent_id: id,
            code,
            shutdown,
        });
    }
}

/// Routes every hook to its agent until the hook server stops.
async fn follow_hooks(inner: Weak<Inner>, mut events: mpsc::UnboundedReceiver<HookEvent>) {
    while let Some(event) = events.recv().await {
        let Some(inner) = inner.upgrade() else { return };
        inner.on_hook(event);
    }
}

/// Feeds an agent's output to the trust check and records its exit.
async fn watch_output(inner: Weak<Inner>, id: i64, mut output: mpsc::UnboundedReceiver<TerminalEvent>) {
    while let Some(event) = output.recv().await {
        let Some(inner) = inner.upgrade() else { return };
        match event {
            TerminalEvent::Output(_) => inner.on_output(id),
            TerminalEvent::Exited { code } => {
                inner.on_exit(id, code);
                return;
            }
        }
    }
}

/// Ends a Claude session: Esc to end a running turn, then `/exit`, then a
/// kill if it hasn't exited within `timeout`.
async fn end_session(terminal: Arc<ClaudeTerminal>, state: AgentState, timeout: Duration) {
    if matches!(state, AgentState::NeedsTrust | AgentState::NeedsLogin) {
        // The dialog takes no commands, and nothing has run yet.
        terminal.kill();
        return;
    }
    if matches!(state, AgentState::Working | AgentState::WaitingApproval) {
        let _ = terminal.write(b"\x1b");
        tokio::time::sleep(INTERRUPT_PAUSE).await;
    }
    let _ = terminal.submit("/exit").await;
    let deadline = tokio::time::Instant::now() + timeout;
    while terminal.exit_status().is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(EXIT_POLL).await;
    }
    if terminal.exit_status().is_none() {
        terminal.kill();
    }
}

/// `name` as is if it is a path, else the first executable file called
/// `name` in `path` (a PATH value). `None` if there is none.
fn resolve_binary(name: &Path, path: Option<&OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    if name.components().count() > 1 {
        return Some(name.to_owned());
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

fn check_size(value: u16) -> Result<u16, AgentError> {
    if value == 0 || value > MAX_SIZE {
        return Err(AgentError::Invalid(format!(
            "a terminal size must be between 1 and {MAX_SIZE}, not {value}"
        )));
    }
    Ok(value)
}

/// The text on one line, trimmed and cut to `max` characters; `None` if empty.
fn clean(text: Option<&str>, max: usize) -> Option<String> {
    let text = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| text.chars().take(max).collect())
}
