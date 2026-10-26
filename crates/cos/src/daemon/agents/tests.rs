//! The agents API over the socket, with `cos-fake-claude` as Claude. Its
//! interactive mode runs hooks the way the real CLI does (see
//! `crates/cos-harness/src/bin/cos-fake-claude/interactive.rs`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    AgentState, Agents, ApprovalState, Base, DecidedBy, Devices, EventLog, NewAgent, Projects, Role, Store,
};
use jiff::Timestamp;
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::watch;

use super::{AgentConfig, AgentManager, StartAgent};
use crate::client::agents::TerminalStream;
use crate::client::{Client, DaemonError, TerminalMessage};
use crate::daemon::{api, serve, tailnet::Tailnet};

const WAIT: Duration = Duration::from_secs(10);

/// `cos-fake-claude`, built next to this test binary by
/// `cargo test --workspace` (`make test`).
pub(crate) fn fake_claude() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    // target/<profile>/deps/cos-… → target/<profile>/cos-fake-claude
    exe.parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("cos-fake-claude"))
        .unwrap_or_default()
}

/// The `cos` binary, built next to this test binary because the crate has
/// integration tests (`crates/cos/tests`).
pub(crate) fn cos_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("cos"))
        .unwrap_or_default()
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    log: EventLog,
    manager: AgentManager,
    client: Client,
    workspace_id: i64,
    projects: Arc<Projects>,
    _stop: watch::Sender<bool>,
}

impl Fixture {
    async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    async fn start_with(configure: impl FnOnce(&mut AgentConfig)) -> Self {
        assert!(
            fake_claude().is_file(),
            "{} is missing; run the tests with `cargo test --workspace` (make test)",
            fake_claude().display()
        );
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        let projects = Arc::new(Projects::new(store.clone(), log.clone(), dir.path().join("w")));
        let repo = git_repo(dir.path());
        let (project, _) = projects.add(&repo).await.unwrap();
        let workspace = projects
            .create_workspace(project.id, Base::Head, None)
            .await
            .unwrap();
        let mut fixture = Self {
            client: Client::new(dir.path().join("cos.sock")),
            manager: start_manager(&store, &log, &projects, dir.path(), configure),
            dir,
            store,
            log,
            workspace_id: workspace.id,
            projects,
            _stop: watch::channel(false).0,
        };
        fixture.serve();
        fixture
    }

    /// Serves the API for the current manager on `cos.sock`.
    fn serve(&mut self) {
        let socket = self.dir.path().join("cos.sock");
        let _ = std::fs::remove_file(&socket);
        let (stop, stopping) = watch::channel(false);
        let staff = crate::daemon::staff::Staff::start(
            self.store.clone(),
            self.log.clone(),
            self.projects.clone(),
            self.manager.clone(),
            crate::daemon::staff::StaffConfig::for_tests(),
        )
        .unwrap();
        let judge = crate::daemon::judge::Judge::off_for_tests(
            &self.store,
            &self.log,
            &self.projects,
            &self.manager,
            self.dir.path(),
        );
        let state = api::AppState {
            forwards: crate::daemon::forward::Forwards::new(),
            shells: crate::daemon::shells::Shells::new(),
            staff,
            judge,
            log: self.log.clone(),
            projects: self.projects.clone(),
            devices: Arc::new(Devices::new(self.store.clone(), self.log.clone())),
            tailnet: Tailnet::load(self.store.clone(), self.log.clone()),
            peers: crate::daemon::peers::Peers::without_tailscale(),
            agents: self.manager.clone(),
            started_at: Timestamp::now(),
            home: self.dir.path().to_path_buf(),
            stopping: stopping.clone(),
        };
        serve(UnixListener::bind(&socket).unwrap(), state, stopping);
        self._stop = stop;
    }

    /// What a daemon restart does: shut the agents down, then start a new
    /// manager on the same database.
    async fn restart(&mut self) {
        self.manager.shutdown().await;
        let _ = self._stop.send(true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.manager = start_manager(&self.store, &self.log, &self.projects, self.dir.path(), |_| {});
        self.serve();
    }

    async fn start_agent(&self, prompt: &str) -> cos_core::Agent {
        let request = StartAgent {
            prompt: Some(prompt.into()),
            ..StartAgent::default()
        };
        self.client
            .start_agent(self.workspace_id, &request)
            .await
            .unwrap()
    }

    /// Waits until the agent is in `state` and returns it.
    async fn until_state(&self, id: i64, state: AgentState) -> cos_core::Agent {
        until(&format!("agent {id} to be {state:?}"), || async {
            self.client
                .agent(id)
                .await
                .ok()
                .filter(|agent| agent.state == state)
        })
        .await
    }

    async fn until_idle_with(&self, id: i64, message: &str) -> cos_core::Agent {
        until(&format!("agent {id} to be idle with {message:?}"), || async {
            self.client.agent(id).await.ok().filter(|agent| {
                agent.state == AgentState::Idle && agent.last_message.as_deref() == Some(message)
            })
        })
        .await
    }

    async fn pending_approval(&self, agent_id: i64) -> cos_core::Approval {
        until("a pending approval", || async {
            self.client
                .approvals(Some("pending"))
                .await
                .ok()?
                .into_iter()
                .find(|approval| approval.agent_id == agent_id)
        })
        .await
    }

    fn kinds(&self) -> Vec<String> {
        self.log
            .after(0, 1000)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }
}

fn start_manager(
    store: &Arc<Store>,
    log: &EventLog,
    projects: &Arc<Projects>,
    dir: &Path,
    configure: impl FnOnce(&mut AgentConfig),
) -> AgentManager {
    let mut config = AgentConfig::for_tests(dir);
    configure(&mut config);
    AgentManager::start(
        Agents::new(store.clone(), log.clone()),
        projects.clone(),
        cos_core::Environment::new(store.clone(), log.clone()),
        config,
    )
    .unwrap()
}

/// Polls `check` until it returns something.
pub(crate) async fn until<T, F: Future<Output = Option<T>>>(what: &str, mut check: impl FnMut() -> F) -> T {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) fn status(error: anyhow::Error) -> u16 {
    error
        .downcast::<DaemonError>()
        .expect("a DaemonError")
        .status
        .as_u16()
}

/// A git repo with one commit and no remote.
pub(crate) fn git_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=cos test",
                "-c",
                "user.email=test@cos.local",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&["add", "README"]);
    git(&["commit", "-q", "-m", "first"]);
    repo.canonicalize().unwrap()
}

/// Reads a terminal stream until its accumulated output contains `text`.
async fn output_until(stream: &mut TerminalStream, text: &str) {
    let mut seen = Vec::new();
    tokio::time::timeout(WAIT, async {
        while !String::from_utf8_lossy(&seen).contains(text) {
            match stream.next().await.unwrap() {
                Some(TerminalMessage::Output(bytes)) => seen.extend(bytes),
                Some(_) => {}
                None => panic!("stream ended before {text:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never saw {text:?} in {:?}", String::from_utf8_lossy(&seen)));
}

#[tokio::test]
async fn an_agent_runs_stops_and_resumes() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;

    let agent = fixture.start_agent("hello").await;
    assert_eq!(agent.role, Role::User);
    assert_eq!(agent.title, "hello");
    assert_eq!((agent.rows, agent.cols), (40, 120));
    assert_eq!(agent.permission_mode, "default");
    assert!(agent.user_settings, "your own agent gets your settings");
    assert!(agent.pid.is_some());
    let agent = fixture.until_idle_with(agent.id, "echo: hello").await;

    client.say(agent.id, "again".into()).await.unwrap();
    fixture.until_idle_with(agent.id, "echo: again").await;
    let screen = client.screen(agent.id).await.unwrap();
    assert!(screen.text.contains("echo: again"), "{}", screen.text);
    assert_eq!((screen.rows, screen.cols), (40, 120));

    client.stop_agent(agent.id).await.unwrap();
    let exited = fixture.until_state(agent.id, AgentState::Exited).await;
    assert_eq!(exited.exit_code, Some(0));
    assert!(exited.exited_ms.is_some());
    // Exited: no typing, no stopping again.
    assert_eq!(status(client.say(agent.id, "x".into()).await.unwrap_err()), 409);
    assert_eq!(status(client.stop_agent(agent.id).await.unwrap_err()), 409);

    let resumed = client.resume_agent(agent.id).await.unwrap();
    assert_eq!(
        (resumed.id, resumed.session_id.as_str()),
        (agent.id, agent.session_id.as_str())
    );
    assert_eq!(resumed.exit_code, None);
    fixture.until_state(agent.id, AgentState::Idle).await;
    let screen = client.screen(agent.id).await.unwrap();
    assert!(
        screen
            .text
            .contains(&format!("session {} (resume)", agent.session_id)),
        "{}",
        screen.text
    );
    // Running: can't resume or forget it.
    assert_eq!(status(client.resume_agent(agent.id).await.unwrap_err()), 409);
    assert_eq!(status(client.remove_agent(agent.id).await.unwrap_err()), 409);

    client.stop_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Exited).await;
    client.remove_agent(agent.id).await.unwrap();
    assert_eq!(status(client.agent(agent.id).await.unwrap_err()), 404);

    let kinds = fixture.kinds();
    let count = |kind: &str| kinds.iter().filter(|seen| *seen == kind).count();
    assert_eq!(count(kind::AGENT_STARTED), 2);
    assert_eq!(count(kind::AGENT_EXITED), 2);
    assert_eq!(count(kind::AGENT_REMOVED), 1);
    let first_change = fixture
        .log
        .after(0, 1000)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == kind::AGENT_STATE_CHANGED)
        .unwrap();
    assert_eq!(
        first_change.data,
        json!({ "agent_id": agent.id, "state": "idle", "from": "starting" })
    );
}

#[tokio::test]
async fn the_transcript_reads_as_a_chat_and_its_growth_is_logged() {
    use crate::daemon::transcript::{MessageKind, Speaker};

    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("hello").await;
    let agent = fixture.until_idle_with(agent.id, "echo: hello").await;
    let path = agent
        .transcript_path
        .clone()
        .expect("the hooks gave a transcript path");
    assert!(path.starts_with(fixture.dir.path()), "{}", path.display());

    let transcript = client.transcript(agent.id, 0).await.unwrap();
    let rows: Vec<(Speaker, MessageKind, Option<&str>)> = transcript
        .messages
        .iter()
        .map(|message| (message.role, message.kind, message.text.as_deref()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (Speaker::User, MessageKind::Text, Some("hello")),
            (Speaker::Assistant, MessageKind::Text, Some("echo: hello")),
        ]
    );
    assert_eq!(transcript.next, 2);

    // A message of several lines, typed through the route, goes in as one
    // paste; and a tool call has its result folded in.
    client.say(agent.id, "first\nsecond".into()).await.unwrap();
    fixture.until_idle_with(agent.id, "echo: first\nsecond").await;
    client.say(agent.id, "ask:Bash ls".into()).await.unwrap();
    let approval = fixture.pending_approval(agent.id).await;
    client.answer_approval(approval.id, false, None).await.unwrap();
    until("the denied call's reply", || async {
        let transcript = client.transcript(agent.id, 2).await.ok()?;
        (transcript.next == 7).then_some(transcript)
    })
    .await;
    let later = client.transcript(agent.id, 2).await.unwrap();
    assert_eq!(later.messages.first().map(|message| message.n), Some(3));
    assert_eq!(later.messages[0].text.as_deref(), Some("first\nsecond"));
    assert_eq!(later.messages[2].text.as_deref(), Some("ask:Bash ls"));
    let call = later.messages[3].tool.as_ref().expect("a tool row");
    assert_eq!(
        (call.name.as_str(), call.summary.as_str(), call.is_error),
        ("Bash", "ls", true)
    );

    // Logged once a second at most, and only when the file grew.
    until("agent.transcript up to 7", || async {
        fixture
            .log
            .after(0, 1000)
            .unwrap()
            .into_iter()
            .rev()
            .find(|event| event.kind == kind::AGENT_TRANSCRIPT)
            .filter(|event| event.data == json!({ "agent_id": agent.id, "next": 7 }))
    })
    .await;
    let logged: Vec<_> = fixture
        .log
        .after(0, 1000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind::AGENT_TRANSCRIPT)
        .collect();
    for pair in logged.windows(2) {
        let gap = pair[1].ts.duration_since(pair[0].ts);
        assert!(gap.as_millis() >= 900, "{logged:?}");
    }

    // Unknown agent: 404. No transcript yet: empty.
    assert_eq!(status(client.transcript(999, 0).await.unwrap_err()), 404);
    let agents = Agents::new(fixture.store.clone(), fixture.log.clone());
    let quiet = agents
        .create(NewAgent {
            workspace_id: fixture.workspace_id,
            role: Role::User,
            title: None,
            session_id: "quiet".into(),
            model: None,
            permission_mode: "default".into(),
            user_settings: true,
            rows: 40,
            cols: 120,
        })
        .unwrap();
    let empty = client.transcript(quiet.id, 0).await.unwrap();
    assert_eq!((empty.messages.len(), empty.next), (0, 0));
}

#[tokio::test]
async fn a_reply_written_after_stop_is_still_logged() {
    let fixture = Fixture::start().await;
    let agent = fixture.start_agent("hello").await;
    fixture.until_idle_with(agent.id, "echo: hello").await;
    fixture.client.say(agent.id, "a late reply".into()).await.unwrap();
    // The reply lands in the file 800 ms after `Stop`, with no hook after it:
    // the apps still hear that the transcript grew to hold it.
    until("agent.transcript up to 4", || async {
        fixture.log.after(0, 1000).unwrap().into_iter().find(|event| {
            event.kind == kind::AGENT_TRANSCRIPT && event.data == json!({ "agent_id": agent.id, "next": 4 })
        })
    })
    .await;
}

#[tokio::test]
async fn an_agent_follows_its_terminal_into_a_new_session_after_clear() {
    use crate::daemon::transcript::{MessageKind, Speaker};

    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("hello").await;
    let agent = fixture.until_idle_with(agent.id, "echo: hello").await;
    let old_path = agent.transcript_path.clone().expect("a transcript path");

    client.say(agent.id, "/clear".into()).await.unwrap();
    let new_session = format!("{}-cleared", agent.session_id);
    let cleared =
        until("the agent to move to the new session", || async {
            client.agent(agent.id).await.ok().filter(|seen| {
                seen.session_id == new_session && seen.transcript_path.as_ref() != Some(&old_path)
            })
        })
        .await;
    assert!(
        cleared
            .transcript_path
            .as_ref()
            .is_some_and(|path| path.ends_with(format!("{new_session}.jsonl"))),
        "{:?}",
        cleared.transcript_path
    );
    // The apps are told to read the transcript from the top, and it's empty.
    until("agent.transcript back to 0", || async {
        fixture.log.after(0, 1000).unwrap().into_iter().find(|event| {
            event.kind == kind::AGENT_TRANSCRIPT && event.data == json!({ "agent_id": agent.id, "next": 0 })
        })
    })
    .await;
    assert_eq!(client.transcript(agent.id, 0).await.unwrap().messages.len(), 0);

    // The new session's hooks reach the agent: its state, its transcript…
    client.say(agent.id, "after".into()).await.unwrap();
    fixture.until_idle_with(agent.id, "echo: after").await;
    let transcript = client.transcript(agent.id, 0).await.unwrap();
    let rows: Vec<(Speaker, MessageKind, Option<&str>)> = transcript
        .messages
        .iter()
        .map(|message| (message.role, message.kind, message.text.as_deref()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (Speaker::User, MessageKind::Text, Some("after")),
            (Speaker::Assistant, MessageKind::Text, Some("echo: after")),
        ]
    );

    // …and its permission requests.
    client.say(agent.id, "needs permission".into()).await.unwrap();
    let approval = fixture.pending_approval(agent.id).await;
    client.answer_approval(approval.id, true, None).await.unwrap();
    fixture.until_idle_with(agent.id, "allowed by hook").await;

    // A resume continues the new session.
    client.stop_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Exited).await;
    let resumed = client.resume_agent(agent.id).await.unwrap();
    assert_eq!(resumed.session_id, new_session);
    fixture.until_state(agent.id, AgentState::Idle).await;
    let screen = client.screen(agent.id).await.unwrap();
    assert!(
        screen.text.contains(&format!("session {new_session} (resume)")),
        "{}",
        screen.text
    );
}

#[tokio::test]
async fn approvals_are_answered_through_cos() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("needs permission").await;

    let approval = fixture.pending_approval(agent.id).await;
    assert_eq!(approval.tool_name, "Bash");
    assert_eq!(approval.tool_input, json!({ "command": "touch x" }));
    assert_eq!(approval.tool_use_id.as_deref(), Some("toolu_1"));
    assert_eq!(approval.summary, "Bash touch x");
    assert_eq!(approval.workspace_id, fixture.workspace_id);
    let waiting = fixture.until_state(agent.id, AgentState::WaitingApproval).await;
    assert_eq!(waiting.pending_approval_id, Some(approval.id));

    let allowed = client.answer_approval(approval.id, true, None).await.unwrap();
    assert_eq!(
        (allowed.state, allowed.decided_by),
        (ApprovalState::Allowed, Some(DecidedBy::User))
    );
    assert!(allowed.decided_ms.is_some());
    assert_eq!(
        status(
            client
                .answer_approval(approval.id, false, None)
                .await
                .unwrap_err()
        ),
        409
    );
    let idle = fixture.until_idle_with(agent.id, "allowed by hook").await;
    assert_eq!(idle.pending_approval_id, None);

    client.say(agent.id, "permission again".into()).await.unwrap();
    let second = fixture.pending_approval(agent.id).await;
    let denied = client
        .answer_approval(second.id, false, Some("not now".into()))
        .await
        .unwrap();
    assert_eq!(denied.state, ApprovalState::Denied);
    assert_eq!(denied.message.as_deref(), Some("not now"));
    fixture.until_idle_with(agent.id, "denied by hook: not now").await;

    assert_eq!(
        status(client.answer_approval(999, true, None).await.unwrap_err()),
        404
    );
    assert!(client.approvals(Some("pending")).await.unwrap().is_empty());
    assert_eq!(client.approvals(None).await.unwrap().len(), 2);
    let kinds = fixture.kinds();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.starts_with("approval."))
            .collect::<Vec<_>>(),
        [
            kind::APPROVAL_REQUESTED,
            kind::APPROVAL_DECIDED,
            kind::APPROVAL_REQUESTED,
            kind::APPROVAL_DECIDED
        ]
    );
}

#[tokio::test]
async fn an_answer_typed_in_the_terminal_settles_the_approval() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("needs permission").await;
    let approval = fixture.pending_approval(agent.id).await;

    client.keys(agent.id, b"1\r").await.unwrap();

    fixture.until_idle_with(agent.id, "allowed in terminal").await;
    let settled = until("the approval to settle", || async {
        client
            .approvals(None)
            .await
            .ok()?
            .into_iter()
            .find(|found| found.id == approval.id && found.state == ApprovalState::Settled)
    })
    .await;
    assert_eq!(settled.decided_by, Some(DecidedBy::Terminal));
    assert_eq!(
        status(client.answer_approval(approval.id, true, None).await.unwrap_err()),
        409
    );
}

#[tokio::test]
async fn the_trust_dialog_is_spotted_and_answered() {
    let fixture = Fixture::start_with(|config| {
        config.env.push(("COS_FAKE_CLAUDE_UNTRUSTED".into(), "1".into()));
    })
    .await;
    let client = &fixture.client;
    let agent = fixture.start_agent("hello").await;

    fixture.until_state(agent.id, AgentState::NeedsTrust).await;
    client.trust_agent(agent.id).await.unwrap();
    fixture.until_idle_with(agent.id, "echo: hello").await;
    assert_eq!(status(client.trust_agent(agent.id).await.unwrap_err()), 409);
}

#[tokio::test]
async fn the_terminal_streams_a_snapshot_then_output() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("hello").await;
    fixture.until_idle_with(agent.id, "echo: hello").await;

    let mut stream = client.terminal(agent.id).await.unwrap();
    match stream.next().await.unwrap() {
        Some(TerminalMessage::Snapshot { rows, cols, data }) => {
            assert_eq!((rows, cols), (40, 120));
            assert!(String::from_utf8_lossy(&data).contains("echo: hello"));
        }
        other => panic!("expected a snapshot, got {other:?}"),
    }

    client.resize_agent(agent.id, 30, 100).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap(),
        Some(TerminalMessage::Resize { rows: 30, cols: 100 })
    );
    assert_eq!(client.agent(agent.id).await.unwrap().rows, 30);
    assert_eq!(
        status(client.resize_agent(agent.id, 0, 100).await.unwrap_err()),
        400
    );

    client.say(agent.id, "again".into()).await.unwrap();
    output_until(&mut stream, "echo: again").await;

    client.stop_agent(agent.id).await.unwrap();
    let exit = tokio::time::timeout(WAIT, async {
        loop {
            match stream.next().await.unwrap() {
                Some(TerminalMessage::Exit { exit_code }) => return exit_code,
                Some(_) => {}
                None => panic!("stream ended without exit"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(exit, Some(0));
    assert_eq!(stream.next().await.unwrap(), None);

    // Opened after the exit: a snapshot, then the exit.
    let mut late = client.terminal(agent.id).await.unwrap();
    assert!(matches!(
        late.next().await.unwrap(),
        Some(TerminalMessage::Snapshot { .. })
    ));
    assert_eq!(
        late.next().await.unwrap(),
        Some(TerminalMessage::Exit { exit_code: Some(0) })
    );
}

#[tokio::test]
async fn stop_kills_an_agent_that_ignores_exit() {
    let fixture = Fixture::start().await;
    let agent = fixture.start_agent("hang forever").await;
    fixture.until_state(agent.id, AgentState::Working).await;

    fixture.client.stop_agent(agent.id).await.unwrap();

    let exited = fixture.until_state(agent.id, AgentState::Exited).await;
    assert_eq!(exited.exit_code, None, "killed, so no exit code");
}

#[tokio::test]
async fn a_daemon_restart_interrupts_agents_and_resume_brings_them_back() {
    let mut fixture = Fixture::start().await;
    let agent = fixture.start_agent("hello").await;
    fixture.until_idle_with(agent.id, "echo: hello").await;

    fixture.restart().await;

    let interrupted = fixture.client.agent(agent.id).await.unwrap();
    assert_eq!(interrupted.state, AgentState::Interrupted);
    let screen = fixture.client.screen(agent.id).await.unwrap();
    assert_eq!((screen.text.as_str(), screen.rows), ("", 40));
    let mut stream = fixture.client.terminal(agent.id).await.unwrap();
    assert!(matches!(
        stream.next().await.unwrap(),
        Some(TerminalMessage::Snapshot { .. })
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        Some(TerminalMessage::Exit { .. })
    ));

    fixture.client.resume_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Idle).await;
    fixture.client.stop_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Exited).await;
}

#[tokio::test]
async fn agents_left_running_by_a_crash_come_back_interrupted() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let log = EventLog::new(store.clone());
    let dir = tempfile::tempdir().unwrap();
    let projects = Arc::new(Projects::new(store.clone(), log.clone(), dir.path().join("w")));
    let (project, _) = projects.add(&git_repo(dir.path())).await.unwrap();
    let workspace = projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();
    let agents = Agents::new(store.clone(), log.clone());
    let agent = agents
        .create(NewAgent {
            workspace_id: workspace.id,
            role: Role::User,
            title: None,
            session_id: "crashed".into(),
            model: None,
            permission_mode: "default".into(),
            user_settings: true,
            rows: 40,
            cols: 120,
        })
        .unwrap();
    agents.set_state(agent.id, AgentState::Working).unwrap();

    let manager = start_manager(&store, &log, &projects, dir.path(), |_| {});

    assert_eq!(manager.get(agent.id).unwrap().state, AgentState::Interrupted);
}

#[tokio::test]
async fn a_workspace_with_a_running_agent_cannot_be_removed() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let agent = fixture.start_agent("hello").await;
    fixture.until_idle_with(agent.id, "echo: hello").await;

    assert_eq!(
        status(client.remove_workspace(fixture.workspace_id).await.unwrap_err()),
        409
    );

    client.stop_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Exited).await;
    client.remove_workspace(fixture.workspace_id).await.unwrap();
    assert_eq!(status(client.agent(agent.id).await.unwrap_err()), 404);
    assert_eq!(fixture.kinds().last().unwrap(), kind::AGENT_REMOVED);
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let start = |permission_mode: Option<&str>| StartAgent {
        permission_mode: permission_mode.map(str::to_owned),
        user_settings: Some(false),
        ..StartAgent::default()
    };

    assert_eq!(status(client.agent(999).await.unwrap_err()), 404);
    assert_eq!(
        status(client.start_agent(999, &start(None)).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(
            client
                .start_agent(fixture.workspace_id, &start(Some("yolo")))
                .await
                .unwrap_err()
        ),
        400
    );
    assert_eq!(status(client.stop_agent(999).await.unwrap_err()), 404);
    let no_terminal = client.terminal(999).await.err().expect("there is no agent 999");
    assert_eq!(status(no_terminal), 404);

    // No prompt: Claude waits, and the agent is named by number.
    let agent = client
        .start_agent(fixture.workspace_id, &start(Some("acceptEdits")))
        .await
        .unwrap();
    assert_eq!(agent.title, format!("Agent {}", agent.id));
    assert!(!agent.user_settings);
    fixture.until_state(agent.id, AgentState::Idle).await;
    let raw = crate::daemon::agents_api::Keys { data: "%%%".into() };
    let response = reqwest_like_post(&fixture, &format!("/v1/agents/{}/keys", agent.id), &raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert_eq!(status(client.approvals(Some("busy")).await.unwrap_err()), 400);
}

/// Posts JSON over the socket and returns the raw response.
async fn reqwest_like_post(fixture: &Fixture, path: &str, body: &impl serde::Serialize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let body = serde_json::to_string(body).unwrap();
    let mut stream = tokio::net::UnixStream::connect(fixture.dir.path().join("cos.sock"))
        .await
        .unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: cos\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

mod approvals;

// ---- Environment ----

/// The environment the fake was last launched with for `session_id`: it
/// writes it to `<dir>/env/<session id>.json` before its first hook.
pub(crate) fn launched_env(dir: &Path, session_id: &str) -> serde_json::Map<String, serde_json::Value> {
    let path = dir.join("env").join(format!("{session_id}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// `name`'s value in a launched environment.
pub(crate) fn var<'a>(env: &'a serde_json::Map<String, serde_json::Value>, name: &str) -> Option<&'a str> {
    env.get(name).and_then(serde_json::Value::as_str)
}

/// The daemon user's home, as `~` expands to.
pub(crate) fn home() -> String {
    std::env::var("HOME").expect("HOME is set")
}

impl Fixture {
    fn project_id(&self) -> i64 {
        self.projects.workspace(self.workspace_id).unwrap().project_id
    }
}

#[tokio::test]
async fn an_agent_gets_the_project_env_over_the_global_one_and_never_over_cos() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let project = fixture.project_id();
    client
        .set_global_env("CLAUDE_CONFIG_DIR", "~/.claude".into())
        .await
        .unwrap();
    client
        .set_global_env("SHARED", "from-global".into())
        .await
        .unwrap();
    client.set_global_env("ONLY_GLOBAL", "g".into()).await.unwrap();
    client
        .set_project_env(project, "CLAUDE_CONFIG_DIR", "~/.claude-personal".into())
        .await
        .unwrap();
    client
        .set_project_env(project, "SHARED", "sk-project-secret".into())
        .await
        .unwrap();
    let path = format!("/opt/cos-test/bin:{}", std::env::var("PATH").unwrap());
    client
        .set_project_env(project, "PATH", path.clone())
        .await
        .unwrap();
    // Names cos sets itself are refused by the API; if the database holds
    // one anyway (written before they were refused), the launch skips it.
    fixture.manager.env().set(Some(project), "TERM", "dumb").unwrap();
    fixture
        .manager
        .env()
        .set(None, "COS_HOOK_SOCKET", "/nowhere")
        .unwrap();

    let agent = fixture.start_agent("hello").await;
    let agent = fixture.until_idle_with(agent.id, "echo: hello").await;
    let env = launched_env(fixture.dir.path(), &agent.session_id);
    let home = home();
    assert_eq!(
        var(&env, "CLAUDE_CONFIG_DIR"),
        Some(format!("{home}/.claude-personal").as_str())
    );
    assert_eq!(var(&env, "SHARED"), Some("sk-project-secret"));
    assert_eq!(var(&env, "ONLY_GLOBAL"), Some("g"));
    assert_eq!(var(&env, "PATH"), Some(path.as_str()), "PATH is replaced");
    assert_eq!(var(&env, "TERM"), Some("xterm-256color"));
    assert_eq!(var(&env, "COLORTERM"), Some("truecolor"));
    assert_eq!(
        var(&env, "COS_HOOK_SOCKET"),
        Some(fixture.dir.path().join("hooks.sock").to_str().unwrap())
    );
    // The fake's own settings (cos's test config) still reach it.
    assert!(var(&env, "COS_FAKE_CLAUDE_TRANSCRIPTS").is_some());

    // The agent shows the names it got, never the values.
    assert_eq!(
        agent.env_names,
        ["CLAUDE_CONFIG_DIR", "ONLY_GLOBAL", "PATH", "SHARED"]
    );
    let json = serde_json::to_string(&client.agent(agent.id).await.unwrap()).unwrap();
    assert!(json.contains("\"env_names\":[\"CLAUDE_CONFIG_DIR\""), "{json}");
    assert!(!json.contains("sk-project-secret"), "{json}");
    for event in fixture.log.after(0, 1000).unwrap() {
        let data = event.data.to_string();
        assert!(
            !data.contains("sk-project-secret") && !data.contains("claude-personal"),
            "{event:?}"
        );
    }
}

#[tokio::test]
async fn a_resume_gets_the_env_as_it_is_now() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let project = fixture.project_id();
    client
        .set_project_env(project, "FOO", "one".into())
        .await
        .unwrap();
    client
        .set_project_env(project, "BAR", "gone soon".into())
        .await
        .unwrap();
    let agent = fixture.start_agent("hello").await;
    let agent = fixture.until_idle_with(agent.id, "echo: hello").await;
    assert_eq!(agent.env_names, ["BAR", "FOO"]);

    client
        .set_project_env(project, "FOO", "two".into())
        .await
        .unwrap();
    client.unset_project_env(project, "BAR").await.unwrap();
    client.set_global_env("NEW", "~".into()).await.unwrap();
    // Running agents keep what they started with.
    let env = launched_env(fixture.dir.path(), &agent.session_id);
    assert_eq!(var(&env, "FOO"), Some("one"));
    assert_eq!(client.agent(agent.id).await.unwrap().env_names, ["BAR", "FOO"]);

    client.stop_agent(agent.id).await.unwrap();
    fixture.until_state(agent.id, AgentState::Exited).await;
    client.resume_agent(agent.id).await.unwrap();
    let resumed = fixture.until_state(agent.id, AgentState::Idle).await;
    let env = launched_env(fixture.dir.path(), &agent.session_id);
    assert_eq!(var(&env, "FOO"), Some("two"));
    assert_eq!(var(&env, "BAR"), None);
    assert_eq!(
        var(&env, "NEW"),
        Some(home().as_str()),
        "`~` alone is the home folder"
    );
    assert_eq!(resumed.env_names, ["FOO", "NEW"]);
}

#[tokio::test]
async fn a_logged_out_claude_config_dir_needs_a_login() {
    let fixture = Fixture::start_with(|config| {
        config.env.push(("COS_FAKE_CLAUDE_LOGGED_OUT".into(), "1".into()));
    })
    .await;
    let client = &fixture.client;

    let agent = fixture.start_agent("hello").await;
    fixture.until_state(agent.id, AgentState::NeedsLogin).await;
    let screen = client.screen(agent.id).await.unwrap();
    assert!(screen.text.contains("Select login method"), "{}", screen.text);

    // Someone logs in in its terminal; then it carries on.
    client.say(agent.id, "login".into()).await.unwrap();
    fixture.until_idle_with(agent.id, "echo: hello").await;

    // Stopping it while it shows the login screen kills it.
    let agent = fixture.start_agent("again").await;
    fixture.until_state(agent.id, AgentState::NeedsLogin).await;
    client.stop_agent(agent.id).await.unwrap();
    let stopped = fixture.until_state(agent.id, AgentState::Exited).await;
    assert_eq!(stopped.exit_code, None);
}
