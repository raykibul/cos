//! jev in the daemon, in **shadow mode**: at each decision point it asks jev
//! (`cos-judge`), works out what jev would have done, and stores that next to
//! what actually happened. It never changes what happens. See
//! `docs/judge.md`.
//!
//! - **Permission sorting:** every permission request the approvals policy
//!   sends to you ([`AgentManager::permission_requests`]). When you answer
//!   (`approval.decided`), your answer is stored next to jev's would-have.
//! - **Waking a chief:** every item added to a run chief's or a Chief of
//!   Staff's inbox, except your own messages and answers. The inbox tables
//!   are followed by id, so the chiefs' code doesn't know jev exists.
//!
//! Only projects switched on are judged (off by default: inputs go to
//! TypeSafe). Without an API key jev is off. The key is looked up in the
//! daemon's environment, then `daemon.env`, then once in your login shell
//! (see [`cos_judge::key`]); only where it was found is logged.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use cos_core::events::kind;
use cos_core::judge::{DecisionPoint, NewDecision, PointStats, ProjectSwitch};
use cos_core::{ApprovalState, EventLog, Judgments, LoggedEvent, Projects, Runs, Store, Subscription};
use cos_judge::key::{
    EnvFileError, KEY_ENV, KeySource, PROBE_TIMEOUT, Probe, probe_shell, resolve_without_shell,
};
use cos_judge::permission::{Locality, PermissionInput, RawRequest};
use cos_judge::wake::{ChiefKind, WakeInput};
use cos_judge::{JudgeClient, JudgeError, input_hash};
use cos_policy::{Needs, Tier};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, broadcast};

use super::agents::{AgentManager, PermissionAsked};
use crate::home::CosHome;

/// jev calls in flight at once; more wait their turn.
const MAX_CALLS: usize = 4;
/// Inbox items read per poll.
const INBOX_PAGE: usize = 100;
/// Run chief inbox kinds that are your own words, never judged.
const YOURS: &[&str] = &["message", "answer"];

#[derive(Debug, Clone)]
pub(crate) struct JudgeConfig {
    pub endpoint: String,
    pub timeout: Duration,
    /// The daemon's own value of `TYPESAFE_API_KEY`.
    pub env_key: Option<String>,
    /// `daemon.env`.
    pub env_file: PathBuf,
    /// Your login shell, probed for the key; `None` never probes.
    pub shell: Option<PathBuf>,
    pub probe_timeout: Duration,
    /// How often the inboxes are checked for new items.
    pub inbox_poll: Duration,
}

impl JudgeConfig {
    pub(crate) fn for_home(home: &CosHome) -> Self {
        Self {
            endpoint: cos_judge::ENDPOINT.to_owned(),
            timeout: cos_judge::TIMEOUT,
            env_key: std::env::var(KEY_ENV).ok(),
            env_file: crate::daemon_env::path(home),
            shell: Some(login_shell()),
            probe_timeout: PROBE_TIMEOUT,
            inbox_poll: Duration::from_secs(2),
        }
    }

    /// For tests: no key anywhere unless the test adds one, and a fake jev.
    #[cfg(test)]
    pub(crate) fn for_tests(dir: &std::path::Path, endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            timeout: Duration::from_secs(1),
            env_key: None,
            env_file: dir.join(crate::daemon_env::FILE),
            shell: None,
            probe_timeout: Duration::from_secs(3),
            inbox_poll: Duration::from_millis(100),
        }
    }
}

/// `$SHELL`, else the usual login shell of the OS.
pub(crate) fn login_shell() -> PathBuf {
    std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/bash"
            })
        })
}

/// Where the key was found, for `GET /v1/judge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct KeyStatus {
    pub source: KeySource,
    /// Something to know about the lookup, such as `daemon.env` being
    /// refused for its permissions or the shell probe failing.
    pub detail: Option<String>,
    /// The login shell is being asked.
    pub probing: bool,
}

/// `GET /v1/judge`: whether jev runs, the switches, and the numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct JudgeOverview {
    pub key: KeyStatus,
    /// A key was found, so switched-on projects are judged.
    pub active: bool,
    /// Why jev is off, when it is.
    pub off_reason: Option<String>,
    pub endpoint: String,
    pub model: String,
    pub projects: Vec<ProjectSwitch>,
    pub points: Vec<PointStats>,
}

/// jev in shadow mode. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Judge {
    inner: Arc<Inner>,
}

struct Inner {
    books: Judgments,
    log: EventLog,
    projects: Arc<Projects>,
    runs: Arc<Runs>,
    agents: AgentManager,
    config: JudgeConfig,
    key: Mutex<KeyState>,
    /// One lookup at a time.
    resolving: tokio::sync::Mutex<()>,
    calls: Semaphore,
    /// The last inbox ids seen: the run chiefs', the Chiefs of Staff's.
    cursors: Mutex<(i64, i64)>,
}

struct KeyState {
    status: KeyStatus,
    client: Option<JudgeClient>,
}

impl Judge {
    /// Starts following permission requests, approval answers and the
    /// inboxes, and looks for the key (the shell probe runs in the
    /// background).
    pub(crate) fn start(
        store: Arc<Store>,
        log: EventLog,
        projects: Arc<Projects>,
        runs: Arc<Runs>,
        agents: AgentManager,
        config: JudgeConfig,
    ) -> anyhow::Result<Self> {
        let books = Judgments::new(store, log.clone());
        let cursors = books.inbox_cursors()?;
        let (_, last_seq) = log.stats()?;
        let inner = Arc::new(Inner {
            books,
            log: log.clone(),
            projects,
            runs,
            agents: agents.clone(),
            config,
            key: Mutex::new(KeyState {
                status: KeyStatus {
                    source: KeySource::Missing,
                    detail: None,
                    probing: false,
                },
                client: None,
            }),
            resolving: tokio::sync::Mutex::new(()),
            calls: Semaphore::new(MAX_CALLS),
            cursors: Mutex::new(cursors),
        });
        inner.resolve_static();
        if inner.client().is_none() && inner.config.shell.is_some() {
            inner.lock_key().status.probing = true;
            let resolving = inner.clone();
            tokio::spawn(async move { resolving.resolve().await });
        }
        tokio::spawn(follow_requests(
            Arc::downgrade(&inner),
            agents.permission_requests(),
        ));
        tokio::spawn(follow_events(Arc::downgrade(&inner), log.subscribe(last_seq)));
        tokio::spawn(poll_inboxes(Arc::downgrade(&inner)));
        Ok(Self { inner })
    }

    pub(crate) fn overview(&self) -> Result<JudgeOverview, cos_core::StoreError> {
        let inner = &self.inner;
        let (status, active) = {
            let key = inner.lock_key();
            (key.status.clone(), key.client.is_some())
        };
        let off_reason = (!active).then(|| match (status.source, status.probing) {
            (_, true) => "looking for TYPESAFE_API_KEY in your login shell".to_owned(),
            _ => format!(
                "no {KEY_ENV} in the daemon's environment, {} or your login shell",
                inner.config.env_file.display()
            ),
        });
        Ok(JudgeOverview {
            key: status,
            active,
            off_reason,
            endpoint: inner.config.endpoint.clone(),
            model: cos_judge::MODEL.to_owned(),
            projects: inner.books.switches()?,
            points: inner.books.stats()?,
        })
    }

    /// Looks for the key again, the login shell included, and waits for it.
    pub(crate) async fn reload(&self) -> Result<JudgeOverview, cos_core::StoreError> {
        self.inner.lock_key().status.probing = self.inner.config.shell.is_some();
        self.inner.resolve().await;
        self.overview()
    }

    /// Switches jev on or off for a project. `None` if there's no such project.
    pub(crate) fn set_enabled(
        &self,
        project_id: i64,
        enabled: bool,
    ) -> Result<Option<ProjectSwitch>, cos_core::StoreError> {
        self.inner.books.set_enabled(project_id, enabled)
    }
}

impl Inner {
    fn lock_key(&self) -> std::sync::MutexGuard<'_, KeyState> {
        self.key.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn client(&self) -> Option<JudgeClient> {
        self.lock_key().client.clone()
    }

    /// The environment and `daemon.env`. Returns whether a key was found.
    fn resolve_static(&self) -> bool {
        let file = cos_judge::key::read_env_file(&self.config.env_file);
        let detail = match &file {
            Err(error @ (EnvFileError::Unsafe { .. } | EnvFileError::Unreadable { .. })) => {
                tracing::warn!("{error}");
                Some(error.to_string())
            }
            _ => None,
        };
        match resolve_without_shell(self.config.env_key.as_deref(), &file) {
            Some((key, source)) => {
                self.set_key(Some(key), source, detail);
                true
            }
            None => {
                self.lock_key().status.detail = detail;
                false
            }
        }
    }

    /// The whole lookup: environment, `daemon.env`, then the login shell.
    async fn resolve(&self) {
        let _one = self.resolving.lock().await;
        if self.resolve_static() {
            return;
        }
        let Some(shell) = self.config.shell.clone() else {
            let detail = self.lock_key().status.detail.clone();
            self.set_key(None, KeySource::Missing, detail);
            return;
        };
        let probe = probe_shell(&shell, KEY_ENV, self.config.probe_timeout).await;
        let detail = self.lock_key().status.detail.clone();
        match probe {
            Probe::Found(key) => self.set_key(Some(key), KeySource::Shell, detail),
            Probe::NotSet => self.set_key(None, KeySource::Missing, detail),
            Probe::Failed(why) => {
                tracing::warn!(shell = %shell.display(), "could not ask the login shell for {KEY_ENV}: {why}");
                let detail = Some(match detail {
                    Some(detail) => format!("{detail}; {why}"),
                    None => why,
                });
                self.set_key(None, KeySource::Missing, detail);
            }
        }
    }

    fn set_key(&self, key: Option<cos_judge::ApiKey>, source: KeySource, detail: Option<String>) {
        match source {
            KeySource::Missing => tracing::info!("jev is off: no {KEY_ENV} found"),
            // Where it was found, never the value.
            source => tracing::info!("jev's key found in {}", source.describe()),
        }
        {
            let mut state = self.lock_key();
            state.client = key.map(|key| {
                JudgeClient::with_endpoint(key, self.config.endpoint.clone()).timeout(self.config.timeout)
            });
            state.status = KeyStatus {
                source,
                detail,
                probing: false,
            };
        }
        let logged = self
            .log
            .append(kind::JUDGE_CHANGED, None, json!({ "key_source": source }));
        if let Err(error) = logged {
            tracing::error!(%error, "could not log judge.changed");
        }
    }

    /// Whether jev should judge something of this project now.
    fn judging(&self, project_id: i64) -> Option<JudgeClient> {
        let client = self.client()?;
        match self.books.enabled(project_id) {
            Ok(true) => Some(client),
            Ok(false) => None,
            Err(error) => {
                tracing::error!(%error, project = project_id, "could not read jev's switch");
                None
            }
        }
    }

    // ---- Permission sorting ----

    async fn on_permission(self: Arc<Self>, asked: PermissionAsked) {
        let Ok(workspace) = self.projects.workspace(asked.workspace_id) else {
            return;
        };
        let Some(client) = self.judging(workspace.project_id) else {
            return;
        };
        let agent = self.agents.get(asked.agent_id).ok();
        let task = agent.as_ref().and_then(|agent| {
            let task = self.runs.task(agent.task_id?).ok()?;
            (Some(task.run_id) == agent.run_id).then_some(task)
        });
        // Without a task, the agent's title (from its first prompt) says
        // what it's for.
        let (title, instructions) = match (&task, &agent) {
            (Some(task), _) => (Some(task.title.as_str()), Some(task.instructions.as_str())),
            (None, Some(agent)) => (Some(agent.title.as_str()), None),
            (None, None) => (None, None),
        };
        let request = &asked.request;
        let role = serde_json::to_value(request.role)
            .ok()
            .and_then(|role| role.as_str().map(str::to_owned))
            .unwrap_or_default();
        let tier = asked.verdict.tier.to_string();
        let input = PermissionInput::from_request(&RawRequest {
            tool: &request.tool,
            input: &request.input,
            cwd: Some(&request.cwd),
            worktree_root: &request.worktree_root,
            home: request.home.as_deref(),
            agent_role: &role,
            task_title: title,
            task_instructions: instructions,
            policy_tier: &tier,
            policy_reason: &asked.verdict.reason,
        });
        // Rust's call, never jev's: only what the policy knows stays in the
        // worktree, with no network and no install.
        let locality = match asked.verdict.tier {
            Tier::Allowed => Locality::Worktree,
            Tier::Chief { needs } if needs == Needs::WORKTREE => Locality::Worktree,
            _ => Locality::Beyond,
        };

        let Ok(_permit) = self.calls.acquire().await else {
            return;
        };
        let started = std::time::Instant::now();
        let result = client.permission(&input).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let answers = result.as_ref().map(|judged| &judged.answers);
        let verdict = cos_judge::permission::would_have(locality, answers);
        let (answers, error, model) = split(&result);
        tracing::info!(
            approval = asked.approval_id,
            would = verdict.action.as_str(),
            latency_ms,
            "jev judged a permission request (shadow mode)"
        );
        let recorded = self.books.record(NewDecision {
            point: DecisionPoint::Permission,
            project_id: Some(workspace.project_id),
            subject: format!("approval:{}", asked.approval_id),
            approval_id: Some(asked.approval_id),
            summary: asked.summary.clone(),
            input_hash: input_hash(&input),
            input: serde_json::to_value(&input).unwrap_or(Value::Null),
            answers,
            would: verdict.action.as_str().to_owned(),
            why: verdict.why,
            no_opinion: verdict.no_opinion,
            latency_ms,
            error,
            model,
        });
        if let Err(error) = recorded {
            tracing::error!(%error, approval = asked.approval_id, "could not record jev's decision");
            return;
        }
        // You may have answered while jev was thinking.
        if let Ok(approval) = self.agents.approval(asked.approval_id)
            && approval.state != ApprovalState::Pending
        {
            let decided_by = serde_json::to_value(approval.decided_by).unwrap_or(Value::Null);
            let state = serde_json::to_value(approval.state).unwrap_or(Value::Null);
            self.note_answer(asked.approval_id, &state, &decided_by);
        }
    }

    fn on_event(&self, event: &LoggedEvent) {
        if event.kind != kind::APPROVAL_DECIDED {
            return;
        }
        let Some(approval_id) = event.data["approval_id"].as_i64() else {
            return;
        };
        self.note_answer(approval_id, &event.data["state"], &event.data["decided_by"]);
    }

    /// Stores what happened to an approval jev judged.
    fn note_answer(&self, approval_id: i64, state: &Value, decided_by: &Value) {
        let Some(actual) = actual(state.as_str(), decided_by.as_str()) else {
            return;
        };
        if let Err(error) = self.books.record_actual(approval_id, actual) {
            tracing::error!(%error, approval = approval_id, "could not record your answer next to jev's");
        }
    }

    // ---- Waking a chief ----

    /// Judges what was added to the inboxes since the last look.
    fn check_inboxes(self: &Arc<Self>) {
        let (run_after, staff_after) = *self.cursors.lock().unwrap_or_else(PoisonError::into_inner);
        let runs = self.books.run_inbox_after(run_after, INBOX_PAGE);
        let staff = self.books.staff_inbox_after(staff_after, INBOX_PAGE);
        let (runs, staff) = match (runs, staff) {
            (Ok(runs), Ok(staff)) => (runs, staff),
            (Err(error), _) | (_, Err(error)) => {
                tracing::error!(%error, "jev could not read the inboxes");
                return;
            }
        };
        {
            let mut cursors = self.cursors.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(last) = runs.last() {
                cursors.0 = last.id;
            }
            if let Some(last) = staff.last() {
                cursors.1 = last.id;
            }
        }
        for item in runs {
            if YOURS.contains(&item.kind.as_str()) {
                continue;
            }
            let input = WakeInput::new(ChiefKind::RunChief, &item.kind, &item.text, Some(&item.goal));
            let subject = format!("inbox:{}", item.id);
            tokio::spawn(self.clone().on_inbox(item.project_id, subject, input));
        }
        for item in staff {
            if item.kind == "message" {
                continue;
            }
            let input = WakeInput::new(ChiefKind::StaffChief, &item.kind, &item.text, None);
            let subject = format!("staff_inbox:{}", item.id);
            tokio::spawn(self.clone().on_inbox(item.project_id, subject, input));
        }
    }

    async fn on_inbox(self: Arc<Self>, project_id: i64, subject: String, input: WakeInput) {
        let Some(client) = self.judging(project_id) else {
            return;
        };
        let Ok(_permit) = self.calls.acquire().await else {
            return;
        };
        let started = std::time::Instant::now();
        let result = client.wake(&input).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let verdict = cos_judge::wake::would_have(result.as_ref().map(|judged| &judged.answers));
        let (answers, error, model) = split(&result);
        let recorded = self.books.record(NewDecision {
            point: DecisionPoint::Wake,
            project_id: Some(project_id),
            summary: format!(
                "{} {}: {}",
                match input.source {
                    ChiefKind::RunChief => "run chief",
                    ChiefKind::StaffChief => "Chief of Staff",
                },
                input.update.kind,
                cos_judge::cut(&input.update.text, 120)
            ),
            subject,
            approval_id: None,
            input_hash: input_hash(&input),
            input: serde_json::to_value(&input).unwrap_or(Value::Null),
            answers,
            would: verdict.as_str().to_owned(),
            no_opinion: verdict.wake.is_none(),
            why: verdict.why,
            latency_ms,
            error,
            model,
        });
        if let Err(error) = recorded {
            tracing::error!(%error, "could not record jev's decision");
        }
    }
}

/// The answers as JSON, the error and the model of one call.
fn split<T: Serialize>(
    result: &Result<cos_judge::Judged<T>, JudgeError>,
) -> (Option<Value>, Option<String>, Option<String>) {
    match result {
        Ok(judged) => (
            serde_json::to_value(&judged.answers).ok(),
            None,
            judged.model.clone(),
        ),
        Err(error) => (None, Some(error.to_string()), None),
    }
}

/// What you did, from an approval's state and who decided it: `allowed` or
/// `denied` when you answered through cos, `terminal` when it was answered
/// in the terminal (allow or deny unknown), `expired`, or the chief's.
fn actual(state: Option<&str>, decided_by: Option<&str>) -> Option<&'static str> {
    Some(match (state?, decided_by) {
        ("allowed", Some("user")) => "allowed",
        ("denied", Some("user")) => "denied",
        ("allowed", Some("chief")) => "chief_allowed",
        ("denied", Some("chief")) => "chief_denied",
        ("settled", _) => "terminal",
        ("expired", _) => "expired",
        _ => return None,
    })
}

async fn follow_requests(inner: Weak<Inner>, mut requests: broadcast::Receiver<PermissionAsked>) {
    loop {
        let asked = match requests.recv().await {
            Ok(asked) => asked,
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "jev missed permission requests");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let Some(inner) = inner.upgrade() else { return };
        tokio::spawn(inner.on_permission(asked));
    }
}

async fn follow_events(inner: Weak<Inner>, mut subscription: Subscription) {
    loop {
        let event = match subscription.next().await {
            Ok(Some(event)) => event,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(%error, "jev stopped following events");
                return;
            }
        };
        let Some(inner) = inner.upgrade() else { return };
        inner.on_event(&event);
    }
}

async fn poll_inboxes(inner: Weak<Inner>) {
    loop {
        let poll = {
            let Some(inner) = inner.upgrade() else { return };
            inner.check_inboxes();
            inner.config.inbox_poll
        };
        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
impl Judge {
    /// For tests that don't use jev: no key, so it's off.
    pub(crate) fn off_for_tests(
        store: &Arc<Store>,
        log: &EventLog,
        projects: &Arc<Projects>,
        runs: &Arc<Runs>,
        agents: &AgentManager,
        dir: &std::path::Path,
    ) -> Self {
        let config = JudgeConfig::for_tests(dir, "http://127.0.0.1:9/v1/systemone");
        Self::start(
            store.clone(),
            log.clone(),
            projects.clone(),
            runs.clone(),
            agents.clone(),
            config,
        )
        .expect("jev starts")
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn your_answers_are_told_apart() {
        assert_eq!(actual(Some("allowed"), Some("user")), Some("allowed"));
        assert_eq!(actual(Some("denied"), Some("user")), Some("denied"));
        assert_eq!(actual(Some("settled"), Some("terminal")), Some("terminal"));
        assert_eq!(actual(Some("expired"), None), Some("expired"));
        assert_eq!(actual(Some("denied"), Some("chief")), Some("chief_denied"));
        assert_eq!(actual(Some("pending"), None), None);
    }
}
