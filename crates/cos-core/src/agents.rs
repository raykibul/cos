//! Agents (Claude sessions the daemon runs in terminals, in a workspace's
//! folder) and their approvals (permission requests waiting for an answer),
//! as stored in the database. The daemon owns the processes; this module keeps
//! the records and logs every change to the event log. The contract with the
//! apps is `docs/agents-api.md`.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::kind;
use crate::{EventLog, Store, StoreError};

/// `last_message` is cut to this many characters.
pub const MAX_LAST_MESSAGE_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: i64,
    pub workspace_id: i64,
    pub run_id: Option<i64>,
    /// The task a worker or reviewer works on.
    #[serde(default)]
    pub task_id: Option<i64>,
    pub role: Role,
    pub title: String,
    /// Where it runs, if not the workspace's folder (a task worktree).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    pub session_id: String,
    pub pid: Option<u32>,
    pub state: AgentState,
    pub model: Option<String>,
    pub permission_mode: String,
    /// Whether Claude loads the user's own settings (allow rules, MCP
    /// servers). Without them only the repo's settings apply.
    pub user_settings: bool,
    pub rows: u16,
    pub cols: u16,
    pub started_ms: i64,
    pub last_activity_ms: i64,
    pub exited_ms: Option<i64>,
    /// `None` if the process was killed, or hasn't exited.
    pub exit_code: Option<i32>,
    /// The last turn's final message, cut to [`MAX_LAST_MESSAGE_CHARS`].
    pub last_message: Option<String>,
    /// The newest approval of this agent that is still pending.
    pub pending_approval_id: Option<i64>,
    /// The agent that started this one: a Chief of Staff.
    #[serde(default)]
    pub spawned_by: Option<i64>,
    /// Claude's transcript (JSONL), from the hooks.
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    /// The names of the extra environment variables (global and project,
    /// see `docs/daemon.md`) it was last launched with, sorted. Never values.
    #[serde(default)]
    pub env_names: Vec<String>,
    /// `--append-system-prompt`, kept so a resume passes it again.
    #[serde(skip)]
    pub system_prompt: Option<String>,
    /// `--mcp-config`, kept so a resume passes it again.
    #[serde(skip)]
    pub mcp_config: Option<String>,
}

/// How a run agent is started beyond [`NewAgent`]: recorded with
/// [`Agents::set_launch`] and used again on every resume.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launch {
    pub task_id: Option<i64>,
    /// `None`: the workspace's folder.
    pub cwd: Option<PathBuf>,
    pub system_prompt: Option<String>,
    pub mcp_config: Option<String>,
    /// The Chief of Staff that started it.
    pub spawned_by: Option<i64>,
}

/// Who started an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Someone started it from an app or the CLI, or a Chief of Staff did
    /// for them.
    User,
    Chief,
    Worker,
    Reviewer,
    /// A project's Chief of Staff.
    Staff,
}

impl Role {
    /// Whether you answer its permission requests yourself, whenever you
    /// like: your own agents and your Chief of Staff, not a run's.
    pub fn asks_you(self) -> bool {
        matches!(self, Self::User | Self::Staff)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Spawned; no hook yet.
    Starting,
    /// Claude is asking whether to trust the folder.
    NeedsTrust,
    /// Claude is showing its login screen: the agent's Claude config dir
    /// (`CLAUDE_CONFIG_DIR`) isn't logged in. Someone logs in in its terminal.
    NeedsLogin,
    /// A prompt is running.
    Working,
    /// A permission request is open.
    WaitingApproval,
    /// The turn ended; waiting for input.
    Idle,
    /// The process ended.
    Exited,
    /// It was running when the daemon stopped; it can be resumed.
    Interrupted,
}

impl AgentState {
    /// True while the agent's process is (or should be) running.
    pub fn is_running(self) -> bool {
        !matches!(self, Self::Exited | Self::Interrupted)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub id: i64,
    pub agent_id: i64,
    pub workspace_id: i64,
    pub tool_name: String,
    pub tool_input: Value,
    /// From the `PreToolUse` just before the request; the request has none.
    pub tool_use_id: Option<String>,
    /// One line for people, such as `Bash npm install left-pad`.
    pub summary: String,
    pub state: ApprovalState,
    pub decided_by: Option<DecidedBy>,
    /// The message sent with a denial, or the chief's reason.
    pub message: Option<String>,
    pub created_ms: i64,
    pub decided_ms: Option<i64>,
    /// How the approvals policy classified the request. `None` on approvals
    /// recorded before the policy was wired in.
    #[serde(default)]
    pub tier: Option<ApprovalTier>,
    /// Who was asked to answer it.
    #[serde(default)]
    pub route: Option<ApprovalRoute>,
    /// The policy's one-line reason, plus why it didn't go to the chief when
    /// the tier would have let it.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The approvals policy's tier for a request: `cos_policy::Tier` without
/// what a chief-tier request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTier {
    /// Rust rules allow it.
    Allowed,
    /// The chief may approve it, within the run's limits.
    Chief,
    /// Only you, because cos can't tell confidently what it does.
    AskUser,
    /// Only you, whatever the limits.
    NeverDelegated,
}

impl From<cos_policy::Tier> for ApprovalTier {
    fn from(tier: cos_policy::Tier) -> Self {
        match tier {
            cos_policy::Tier::Allowed => Self::Allowed,
            cos_policy::Tier::Chief { .. } => Self::Chief,
            cos_policy::Tier::AskUser => Self::AskUser,
            cos_policy::Tier::NeverDelegated => Self::NeverDelegated,
        }
    }
}

/// Who answers an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRoute {
    /// Rust rules answered it straight away; nobody was asked.
    Auto,
    /// The run's chief, through `approval_decide`.
    Chief,
    /// You.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    /// Allowed through cos.
    Allowed,
    /// Denied through cos.
    Denied,
    /// Answered in the terminal, or the tool went ahead some other way.
    Settled,
    /// Nobody answered in time, so cos denied it.
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidedBy {
    User,
    Terminal,
    Rule,
    Chief,
}

/// What [`Agents::create`] needs.
#[derive(Debug, Clone)]
pub struct NewAgent {
    pub workspace_id: i64,
    pub run_id: Option<i64>,
    pub role: Role,
    /// `None` becomes "Agent N".
    pub title: Option<String>,
    pub session_id: String,
    pub model: Option<String>,
    pub permission_mode: String,
    pub user_settings: bool,
    pub rows: u16,
    pub cols: u16,
}

/// What [`Agents::add_approval`] needs.
#[derive(Debug, Clone, Default)]
pub struct NewApproval {
    pub agent_id: i64,
    pub workspace_id: i64,
    pub tool_name: String,
    pub tool_input: Value,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub tier: Option<ApprovalTier>,
    pub route: Option<ApprovalRoute>,
    pub reason: Option<String>,
}

/// The agents and approvals tables, with every change logged.
#[derive(Clone)]
pub struct Agents {
    store: Arc<Store>,
    log: EventLog,
}

impl Agents {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// Records a new agent in `starting`. Logs nothing yet: the daemon calls
    /// [`Self::started`] once its process is up, or [`Self::discard`].
    pub fn create(&self, new: NewAgent) -> Result<Agent, StoreError> {
        let id = {
            let conn = self.store.lock();
            let now = now_ms();
            conn.execute(
                "INSERT INTO agents (workspace_id, run_id, role, title, session_id, state, model,
                     permission_mode, user_settings, term_rows, term_cols, started_ms, last_activity_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'starting', ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![
                    new.workspace_id,
                    new.run_id,
                    as_text(&new.role),
                    new.title.clone().unwrap_or_default(),
                    new.session_id,
                    new.model,
                    new.permission_mode,
                    new.user_settings,
                    new.rows,
                    new.cols,
                    now,
                ],
            )?;
            let id = conn.last_insert_rowid();
            if new.title.is_none() {
                conn.execute(
                    "UPDATE agents SET title = ?2 WHERE id = ?1",
                    params![id, format!("Agent {id}")],
                )?;
            }
            id
        };
        self.require(id)
    }

    /// Records how a new agent is started beyond [`NewAgent`]. Call it
    /// between [`Self::create`] and the launch. Logs nothing.
    pub fn set_launch(&self, id: i64, launch: &Launch) -> Result<Agent, StoreError> {
        self.store.lock().execute(
            "UPDATE agents SET task_id = ?2, cwd = ?3, system_prompt = ?4, mcp_config = ?5, spawned_by = ?6
             WHERE id = ?1",
            params![
                id,
                launch.task_id,
                launch.cwd.as_ref().map(|cwd| cwd.to_string_lossy()),
                launch.system_prompt,
                launch.mcp_config,
                launch.spawned_by,
            ],
        )?;
        self.require(id)
    }

    /// Replaces `--append-system-prompt` for the next launch. Logs nothing.
    pub fn set_system_prompt(&self, id: i64, prompt: Option<&str>) -> Result<(), StoreError> {
        self.store.lock().execute(
            "UPDATE agents SET system_prompt = ?2 WHERE id = ?1",
            params![id, prompt],
        )?;
        Ok(())
    }

    /// Replaces the model for the next launch. Logs nothing.
    pub fn set_model(&self, id: i64, model: Option<&str>) -> Result<(), StoreError> {
        self.store
            .lock()
            .execute("UPDATE agents SET model = ?2 WHERE id = ?1", params![id, model])?;
        Ok(())
    }

    /// Records where Claude keeps the agent's transcript. Logs nothing.
    pub fn set_transcript_path(&self, id: i64, path: &std::path::Path) -> Result<(), StoreError> {
        self.store.lock().execute(
            "UPDATE agents SET transcript_path = ?2 WHERE id = ?1",
            params![id, path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Logs `agent.transcript`: the transcript grew to `next` messages.
    pub fn log_transcript(&self, id: i64, next: usize) -> Result<(), StoreError> {
        self.log.append(
            kind::AGENT_TRANSCRIPT,
            None,
            json!({ "agent_id": id, "next": next }),
        )?;
        Ok(())
    }

    /// The agents `spawner` started, oldest first.
    pub fn spawned_by(&self, spawner: i64) -> Result<Vec<Agent>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!("{AGENT_SELECT} WHERE a.spawned_by = ?1 ORDER BY a.id"))?
            .query_map([spawner], read_agent)?
            .collect::<Result<_, _>>()?)
    }

    /// The agents of a task, oldest first.
    pub fn of_task(&self, task_id: i64) -> Result<Vec<Agent>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!("{AGENT_SELECT} WHERE a.task_id = ?1 ORDER BY a.id"))?
            .query_map([task_id], read_agent)?
            .collect::<Result<_, _>>()?)
    }

    /// The agents of a run, oldest first.
    pub fn of_run(&self, run_id: i64) -> Result<Vec<Agent>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!("{AGENT_SELECT} WHERE a.run_id = ?1 ORDER BY a.id"))?
            .query_map([run_id], read_agent)?
            .collect::<Result<_, _>>()?)
    }

    /// Forgets an agent whose process never started. Logs nothing.
    pub fn discard(&self, id: i64) -> Result<(), StoreError> {
        self.store
            .lock()
            .execute("DELETE FROM agents WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The process is up: records its pid and the names of the extra
    /// environment variables it got, and logs `agent.started`.
    pub fn started(&self, id: i64, pid: Option<u32>, env_names: &[String]) -> Result<Agent, StoreError> {
        let names = serde_json::to_string(env_names).expect("strings serialize");
        self.store.lock().execute(
            "UPDATE agents SET pid = ?2, env_names = ?3 WHERE id = ?1",
            params![id, pid, names],
        )?;
        let agent = self.require(id)?;
        self.log.append(
            kind::AGENT_STARTED,
            None,
            json!({
                "agent_id": id,
                "workspace_id": agent.workspace_id,
                "run_id": agent.run_id,
                "role": agent.role,
                "session_id": agent.session_id,
            }),
        )?;
        Ok(agent)
    }

    /// Puts an exited or interrupted agent back in `starting` for a resume.
    /// Logs `agent.state_changed`; [`Self::started`] follows.
    pub fn restart(&self, id: i64) -> Result<Agent, StoreError> {
        let from = self.require(id)?.state;
        {
            let conn = self.store.lock();
            let now = now_ms();
            conn.execute(
                "UPDATE agents SET state = 'starting', pid = NULL, started_ms = ?2, last_activity_ms = ?2,
                     exited_ms = NULL, exit_code = NULL
                 WHERE id = ?1",
                params![id, now],
            )?;
        }
        self.log_state_change(id, AgentState::Starting, from)?;
        self.require(id)
    }

    pub fn get(&self, id: i64) -> Result<Option<Agent>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!("{AGENT_SELECT} WHERE a.id = ?1"))?
            .query_row([id], read_agent)
            .optional()?)
    }

    /// Agents, newest first, optionally only those in one workspace or state.
    pub fn list(
        &self,
        workspace_id: Option<i64>,
        state: Option<AgentState>,
    ) -> Result<Vec<Agent>, StoreError> {
        let conn = self.store.lock();
        let state = state.map(|state| as_text(&state));
        Ok(conn
            .prepare_cached(&format!(
                "{AGENT_SELECT} WHERE (?1 IS NULL OR a.workspace_id = ?1) AND (?2 IS NULL OR a.state = ?2)
                 ORDER BY a.id DESC"
            ))?
            .query_map(params![workspace_id, state], read_agent)?
            .collect::<Result<_, _>>()?)
    }

    /// Moves a running agent to `state` and counts it as activity. Logs
    /// `agent.state_changed` and returns the previous state if it changed.
    /// Exited and interrupted agents are left alone; see [`Self::finish`].
    pub fn set_state(&self, id: i64, state: AgentState) -> Result<Option<AgentState>, StoreError> {
        let from = {
            let conn = self.store.lock();
            let Some(from) = current_state(&conn, id)? else {
                return Ok(None);
            };
            if !from.is_running() {
                return Ok(None);
            }
            conn.execute(
                "UPDATE agents SET state = ?2, last_activity_ms = ?3 WHERE id = ?1",
                params![id, as_text(&state), now_ms()],
            )?;
            from
        };
        if from == state {
            return Ok(None);
        }
        self.log_state_change(id, state, from)?;
        Ok(Some(from))
    }

    /// Records activity without a state change.
    pub fn touch(&self, id: i64) -> Result<(), StoreError> {
        self.store.lock().execute(
            "UPDATE agents SET last_activity_ms = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    pub fn set_last_message(&self, id: i64, message: &str) -> Result<(), StoreError> {
        let message: String = message.chars().take(MAX_LAST_MESSAGE_CHARS).collect();
        self.store.lock().execute(
            "UPDATE agents SET last_message = ?2 WHERE id = ?1",
            params![id, message],
        )?;
        Ok(())
    }

    pub fn set_size(&self, id: i64, rows: u16, cols: u16) -> Result<(), StoreError> {
        self.store.lock().execute(
            "UPDATE agents SET term_rows = ?2, term_cols = ?3 WHERE id = ?1",
            params![id, rows, cols],
        )?;
        Ok(())
    }

    /// The agent's process ended. `state` is `Exited`, or `Interrupted` when
    /// the daemon stopped it on its way down. Logs `agent.state_changed` and
    /// `agent.exited`.
    pub fn finish(&self, id: i64, state: AgentState, exit_code: Option<i32>) -> Result<(), StoreError> {
        let from = {
            let conn = self.store.lock();
            let Some(from) = current_state(&conn, id)? else {
                return Ok(());
            };
            let now = now_ms();
            conn.execute(
                "UPDATE agents SET state = ?2, exited_ms = ?3, exit_code = ?4, last_activity_ms = ?3
                 WHERE id = ?1",
                params![id, as_text(&state), now, exit_code],
            )?;
            from
        };
        if from != state {
            self.log_state_change(id, state, from)?;
        }
        self.log.append(
            kind::AGENT_EXITED,
            None,
            json!({ "agent_id": id, "exit_code": exit_code }),
        )?;
        Ok(())
    }

    /// Marks every agent that was running as `interrupted`, for a daemon
    /// that starts after one that didn't stop cleanly. Returns their ids.
    pub fn interrupt_running(&self) -> Result<Vec<i64>, StoreError> {
        let running: Vec<(i64, AgentState)> = self
            .list(None, None)?
            .into_iter()
            .filter(|agent| agent.state.is_running())
            .map(|agent| (agent.id, agent.state))
            .collect();
        for &(id, from) in &running {
            self.store
                .lock()
                .execute("UPDATE agents SET state = 'interrupted' WHERE id = ?1", [id])?;
            self.log_state_change(id, AgentState::Interrupted, from)?;
        }
        Ok(running.into_iter().map(|(id, _)| id).collect())
    }

    /// Forgets an agent and its approvals. Logs `agent.removed`.
    pub fn remove(&self, id: i64) -> Result<(), StoreError> {
        self.store
            .lock()
            .execute("DELETE FROM agents WHERE id = ?1", [id])?;
        self.log_removed(id)
    }

    /// Logs `agent.removed` for an agent the database already dropped, such
    /// as one whose workspace was removed.
    pub fn log_removed(&self, id: i64) -> Result<(), StoreError> {
        self.log
            .append(kind::AGENT_REMOVED, None, json!({ "agent_id": id }))?;
        Ok(())
    }

    // ---- Approvals ----

    /// Records a pending approval. Logs `approval.requested`, and
    /// `approval.routed` when it goes to the chief.
    pub fn add_approval(&self, new: NewApproval) -> Result<Approval, StoreError> {
        let id = self.insert_approval(&new, None)?;
        let approval = self.require_approval(id)?;
        self.log.append(
            kind::APPROVAL_REQUESTED,
            None,
            json!({
                "approval_id": id,
                "agent_id": new.agent_id,
                "summary": approval.summary,
                "tier": approval.tier,
                "route": approval.route,
            }),
        )?;
        if approval.route == Some(ApprovalRoute::Chief) {
            self.log_routed(&approval)?;
        }
        Ok(approval)
    }

    /// Records an approval that Rust rules allowed straight away: `allowed`
    /// by `rule`, route `auto`. Nobody is asked, so it logs only
    /// `approval.decided`, never `approval.requested`.
    pub fn add_ruled_approval(&self, new: NewApproval) -> Result<Approval, StoreError> {
        let new = NewApproval {
            route: Some(ApprovalRoute::Auto),
            ..new
        };
        let id = self.insert_approval(&new, Some(DecidedBy::Rule))?;
        self.log.append(
            kind::APPROVAL_DECIDED,
            None,
            json!({
                "approval_id": id,
                "state": ApprovalState::Allowed,
                "decided_by": DecidedBy::Rule,
                "route": ApprovalRoute::Auto,
            }),
        )?;
        self.require_approval(id)
    }

    /// Sends a pending approval to someone else, such as to you once the
    /// limits refused the chief's answer. `reason` replaces the stored one
    /// if given. `None` if it was no longer pending. Logs `approval.routed`.
    pub fn route_approval(
        &self,
        id: i64,
        route: ApprovalRoute,
        reason: Option<&str>,
    ) -> Result<Option<Approval>, StoreError> {
        let changed = self.store.lock().execute(
            "UPDATE approvals SET route = ?2, reason = COALESCE(?3, reason)
             WHERE id = ?1 AND state = 'pending'",
            params![id, as_text(&route), reason],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let approval = self.require_approval(id)?;
        self.log_routed(&approval)?;
        Ok(Some(approval))
    }

    /// The run an agent belongs to, if any.
    pub fn run_of(&self, agent_id: i64) -> Result<Option<i64>, StoreError> {
        Ok(self
            .store
            .lock()
            .query_row("SELECT run_id FROM agents WHERE id = ?1", [agent_id], |row| {
                row.get(0)
            })
            .optional()?
            .flatten())
    }

    /// Inserts an approval: pending, or already allowed by `decided_by`.
    fn insert_approval(&self, new: &NewApproval, decided_by: Option<DecidedBy>) -> Result<i64, StoreError> {
        let conn = self.store.lock();
        let now = now_ms();
        let (state, decided_ms) = match decided_by {
            Some(_) => (ApprovalState::Allowed, Some(now)),
            None => (ApprovalState::Pending, None),
        };
        conn.execute(
            "INSERT INTO approvals (agent_id, workspace_id, tool_name, tool_input, tool_use_id, summary,
                 state, decided_by, created_ms, decided_ms, tier, route, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                new.agent_id,
                new.workspace_id,
                new.tool_name,
                new.tool_input.to_string(),
                new.tool_use_id,
                new.summary,
                as_text(&state),
                decided_by.map(|by| as_text(&by)),
                now,
                decided_ms,
                new.tier.map(|tier| as_text(&tier)),
                new.route.map(|route| as_text(&route)),
                new.reason,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Logs `approval.routed`, under the agent's run so the chief's inbox
    /// can pick it up.
    fn log_routed(&self, approval: &Approval) -> Result<(), StoreError> {
        let run_id = self.run_of(approval.agent_id)?;
        self.log.append(
            kind::APPROVAL_ROUTED,
            run_id.map(|id| id.to_string()).as_deref(),
            json!({
                "approval_id": approval.id,
                "route": approval.route,
                "tier": approval.tier,
                "run_id": run_id,
                "agent_id": approval.agent_id,
                "summary": approval.summary,
                "reason": approval.reason,
            }),
        )?;
        Ok(())
    }

    /// Answers a pending approval. Returns `None` if it was no longer
    /// pending, in which case nothing changes. Logs `approval.decided`.
    pub fn decide_approval(
        &self,
        id: i64,
        state: ApprovalState,
        decided_by: Option<DecidedBy>,
        message: Option<&str>,
    ) -> Result<Option<Approval>, StoreError> {
        let changed = self.store.lock().execute(
            "UPDATE approvals SET state = ?2, decided_by = ?3, message = ?4, decided_ms = ?5
             WHERE id = ?1 AND state = 'pending'",
            params![
                id,
                as_text(&state),
                decided_by.map(|by| as_text(&by)),
                message,
                now_ms()
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.log.append(
            kind::APPROVAL_DECIDED,
            None,
            json!({ "approval_id": id, "state": state, "decided_by": decided_by }),
        )?;
        self.approval(id)
    }

    pub fn approval(&self, id: i64) -> Result<Option<Approval>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!("SELECT {APPROVAL_COLUMNS} FROM approvals WHERE id = ?1"))?
            .query_row([id], read_approval)
            .optional()?)
    }

    /// Approvals, oldest first, optionally only those in one state.
    pub fn approvals(&self, state: Option<ApprovalState>) -> Result<Vec<Approval>, StoreError> {
        let conn = self.store.lock();
        let state = state.map(|state| as_text(&state));
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE (?1 IS NULL OR state = ?1) ORDER BY id"
            ))?
            .query_map(params![state], read_approval)?
            .collect::<Result<_, _>>()?)
    }

    /// The ids of an agent's pending approvals, oldest first.
    pub fn pending_approvals_of(&self, agent_id: i64) -> Result<Vec<i64>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached("SELECT id FROM approvals WHERE agent_id = ?1 AND state = 'pending' ORDER BY id")?
            .query_map([agent_id], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    fn require(&self, id: i64) -> Result<Agent, StoreError> {
        self.get(id)?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    fn require_approval(&self, id: i64) -> Result<Approval, StoreError> {
        self.approval(id)?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    fn log_state_change(&self, id: i64, state: AgentState, from: AgentState) -> Result<(), StoreError> {
        self.log.append(
            kind::AGENT_STATE_CHANGED,
            None,
            json!({ "agent_id": id, "state": state, "from": from }),
        )?;
        Ok(())
    }
}

// ---- Storage ----

const AGENT_SELECT: &str =
    "SELECT a.id, a.workspace_id, a.run_id, a.role, a.title, a.session_id, a.pid, a.state,
        a.model, a.permission_mode, a.user_settings, a.term_rows, a.term_cols, a.started_ms, a.last_activity_ms, a.exited_ms,
        a.exit_code, a.last_message,
        (SELECT MAX(p.id) FROM approvals p WHERE p.agent_id = a.id AND p.state = 'pending'),
        a.task_id, a.cwd, a.system_prompt, a.mcp_config, a.spawned_by, a.transcript_path, a.env_names
    FROM agents a";

const APPROVAL_COLUMNS: &str =
    "id, agent_id, workspace_id, tool_name, tool_input, tool_use_id, summary, state,
    decided_by, message, created_ms, decided_ms, tier, route, reason";

fn current_state(conn: &Connection, id: i64) -> rusqlite::Result<Option<AgentState>> {
    conn.query_row("SELECT state FROM agents WHERE id = ?1", [id], |row| {
        from_text(row, 0)
    })
    .optional()
}

fn read_agent(row: &Row<'_>) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        run_id: row.get(2)?,
        role: from_text(row, 3)?,
        title: row.get(4)?,
        session_id: row.get(5)?,
        pid: row.get(6)?,
        state: from_text(row, 7)?,
        model: row.get(8)?,
        permission_mode: row.get(9)?,
        user_settings: row.get(10)?,
        rows: row.get(11)?,
        cols: row.get(12)?,
        started_ms: row.get(13)?,
        last_activity_ms: row.get(14)?,
        exited_ms: row.get(15)?,
        exit_code: row.get(16)?,
        last_message: row.get(17)?,
        pending_approval_id: row.get(18)?,
        task_id: row.get(19)?,
        cwd: row.get::<_, Option<String>>(20)?.map(PathBuf::from),
        system_prompt: row.get(21)?,
        mcp_config: row.get(22)?,
        spawned_by: row.get(23)?,
        transcript_path: row.get::<_, Option<String>>(24)?.map(PathBuf::from),
        // Written by us; NULL before an agent's first launch with the column.
        env_names: row
            .get::<_, Option<String>>(25)?
            .and_then(|names| serde_json::from_str(&names).ok())
            .unwrap_or_default(),
    })
}

fn read_approval(row: &Row<'_>) -> rusqlite::Result<Approval> {
    let tool_input: String = row.get(4)?;
    let decided_by: Option<String> = row.get(8)?;
    Ok(Approval {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        workspace_id: row.get(2)?,
        tool_name: row.get(3)?,
        // Written by us as JSON; anything else is kept as a string.
        tool_input: serde_json::from_str(&tool_input).unwrap_or(Value::String(tool_input)),
        tool_use_id: row.get(5)?,
        summary: row.get(6)?,
        state: from_text(row, 7)?,
        decided_by: decided_by
            .map(|text| parse_text(&text).map_err(|error| conversion_error(8, error)))
            .transpose()?,
        message: row.get(9)?,
        created_ms: row.get(10)?,
        decided_ms: row.get(11)?,
        tier: optional_text(row, 12)?,
        route: optional_text(row, 13)?,
        reason: row.get(14)?,
    })
}

fn optional_text<T: for<'de> Deserialize<'de>>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = row.get(index)?;
    text.map(|text| parse_text(&text).map_err(|error| conversion_error(index, error)))
        .transpose()
}

/// The serde name of a unit enum variant, as stored in the database.
fn as_text(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => text,
        other => unreachable!("stored enums serialize to strings, got {other:?}"),
    }
}

fn parse_text<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(text.to_owned())).map_err(|error| error.to_string())
}

fn from_text<T: for<'de> Deserialize<'de>>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    parse_text(&text).map_err(|error| conversion_error(index, error))
}

fn conversion_error(index: usize, error: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, error.into())
}

impl FromStr for AgentState {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_text(text).map_err(|_| format!("unknown agent state `{text}`"))
    }
}

impl FromStr for ApprovalState {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_text(text).map_err(|_| format!("unknown approval state `{text}`"))
    }
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests;
