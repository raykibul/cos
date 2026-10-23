//! jev in the daemon against a fake jev server on 127.0.0.1 and
//! `cos-fake-claude`: permission requests and inbox items are judged only for
//! switched-on projects, your answers land next to jev's, and the key is
//! found where the lookup order says. No real calls.

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use cos_core::events::kind;
use cos_core::judge::Decision;
use cos_core::{
    AgentState, Agents, Base, EventLog, Judgments, Projects, Role, StaffBooks, StaffItemKind, Store,
};
use cos_judge::KEY_ENV;
use cos_judge::key::KeySource;
use serde_json::{Value, json};

use super::{Judge, JudgeConfig};
use crate::daemon::agents::tests::{git_repo, until};
use crate::daemon::agents::{AgentConfig, AgentManager, StartAgent};

/// A fake jev: answers permission and wake questions, and keeps what it
/// was sent.
#[derive(Clone, Default)]
struct FakeJev {
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn decide(State(fake): State<FakeJev>, headers: axum::http::HeaderMap, body: String) -> String {
    let body: Value = serde_json::from_str(&body).unwrap();
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let questions = body["questions"].as_object().unwrap().clone();
    fake.seen.lock().unwrap().push((auth, body));
    let answers = if questions.contains_key("action_kind") {
        json!({
            "action_kind": { "type": "choice", "choice": "read_only", "confidence": 0.96,
                             "probabilities": { "read_only": 0.96, "external": 0.04 } },
            "serves_task": { "type": "noul", "noul": 0.9 },
            "leaves_worktree": { "type": "noul", "noul": 0.03 },
        })
    } else {
        json!({
            "wake": { "type": "choice", "choice": "now", "confidence": 0.88,
                      "probabilities": { "now": 0.88, "batch": 0.1, "none": 0.02 } },
            "is_question_for_chief": { "type": "noul", "noul": 0.2 },
        })
    };
    json!({ "model": "typesafe/jev-fake", "answers": answers }).to_string()
}

async fn fake_jev() -> (String, FakeJev) {
    let fake = FakeJev::default();
    let app = Router::new()
        .route("/v1/systemone", post(decide))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/v1/systemone"), fake)
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    log: EventLog,
    projects: Arc<Projects>,
    manager: AgentManager,
    project_id: i64,
    workspace_id: i64,
    endpoint: String,
    jev: FakeJev,
}

impl Fixture {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        let projects = Arc::new(Projects::new(store.clone(), log.clone(), dir.path().join("w")));
        let (project, _) = projects.add(&git_repo(dir.path())).await.unwrap();
        let workspace = projects
            .create_workspace(project.id, Base::Head, None)
            .await
            .unwrap();
        let manager = AgentManager::start(
            Agents::new(store.clone(), log.clone()),
            projects.clone(),
            cos_core::Environment::new(store.clone(), log.clone()),
            AgentConfig::for_tests(dir.path()),
        )
        .unwrap();
        let (endpoint, jev) = fake_jev().await;
        Self {
            dir,
            store,
            log,
            projects,
            manager,
            project_id: project.id,
            workspace_id: workspace.id,
            endpoint,
            jev,
        }
    }

    fn config(&self) -> JudgeConfig {
        JudgeConfig::for_tests(self.dir.path(), &self.endpoint)
    }

    fn judge(&self, config: JudgeConfig) -> Judge {
        Judge::start(
            self.store.clone(),
            self.log.clone(),
            self.projects.clone(),
            self.manager.clone(),
            config,
        )
        .unwrap()
    }

    fn write_env_file(&self, text: &str, mode: u32) {
        let path = self.dir.path().join(crate::daemon_env::FILE);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn books(&self) -> Judgments {
        Judgments::new(self.store.clone(), self.log.clone())
    }

    fn decisions(&self) -> Vec<Decision> {
        self.books().recent(None, 100).unwrap()
    }

    /// Starts one of your agents that asks for `ask` and waits for the
    /// approval.
    async fn ask(&self, ask: &str) -> i64 {
        let request = StartAgent {
            prompt: Some(format!("ask:{ask}")),
            user_settings: Some(false),
            ..StartAgent::default()
        };
        let agent = self
            .manager
            .start_agent(self.workspace_id, Role::User, &request)
            .unwrap();
        until("a pending approval", || async {
            let agent = self.manager.get(agent.id).ok()?;
            (agent.state == AgentState::WaitingApproval)
                .then_some(())
                .and(agent.pending_approval_id)
        })
        .await
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.jev.seen.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn shadow_mode_judges_switched_on_projects_and_keeps_your_answer() {
    let fixture = Fixture::start().await;
    fixture.write_env_file(&format!("{KEY_ENV}=sk-from-file\n"), 0o600);
    let judge = fixture.judge(fixture.config());

    // Off by default: the request goes to you and jev isn't asked.
    let first = fixture.ask("Bash ls").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fixture.calls().is_empty());
    assert!(fixture.decisions().is_empty());
    fixture.manager.decide(first, true, None).unwrap();

    judge.set_enabled(fixture.project_id, true).unwrap().unwrap();
    let approval_id = fixture.ask("Bash ls src").await;
    let decision = until("jev's decision", || async { fixture.decisions().pop() }).await;

    assert_eq!(decision.approval_id, Some(approval_id));
    assert_eq!(decision.would, "approve", "{decision:?}");
    assert_eq!(decision.actual, None);
    assert!(!decision.no_opinion);
    assert_eq!(decision.model.as_deref(), Some("typesafe/jev-fake"));
    // Still waiting for you: shadow mode changes nothing.
    let approval = fixture.manager.approval(approval_id).unwrap();
    assert_eq!(approval.state, cos_core::ApprovalState::Pending);

    // One call, with the key from daemon.env and no machine paths.
    let calls = fixture.calls();
    assert_eq!(calls.len(), 1);
    let (auth, body) = &calls[0];
    assert_eq!(auth, "Bearer sk-from-file");
    assert_eq!(body["state"]["request"]["input"]["command"], "ls src");
    assert_eq!(body["state"]["request"]["cwd"], ".");
    let workspace = fixture.projects.workspace(fixture.workspace_id).unwrap();
    assert!(!body.to_string().contains(&workspace.path.display().to_string()));

    // You deny it: a disagreement, with a labelled example.
    fixture.manager.decide(approval_id, false, None).unwrap();
    let decided = until("your answer next to jev's", || async {
        fixture
            .books()
            .decision(decision.id)
            .unwrap()
            .filter(|decision| decision.actual.is_some())
    })
    .await;
    assert_eq!(decided.actual.as_deref(), Some("denied"));

    let overview = judge.overview().unwrap();
    assert!(overview.active);
    assert_eq!(overview.key.source, KeySource::DaemonEnv);
    let permission = &overview.points[0];
    assert_eq!((permission.answered, permission.would_approve), (1, 1));
    assert_eq!(permission.disagreed, 1);
    assert_eq!(permission.disagreements[0].summary, "Bash ls src");

    // judge.decided says what jev would do, never what it was sent.
    let events = fixture.log.after(0, 1000).unwrap();
    let decided = events
        .iter()
        .find(|event| event.kind == kind::JUDGE_DECIDED)
        .unwrap();
    assert_eq!(decided.data["would"], "approve");
    assert!(!decided.data.to_string().contains("ls src"));
    assert!(
        !events
            .iter()
            .any(|event| event.data.to_string().contains("sk-from-file"))
    );
}

#[tokio::test]
async fn inbox_items_are_judged_except_your_own_words() {
    let fixture = Fixture::start().await;
    fixture.write_env_file(&format!("{KEY_ENV}=sk-file\n"), 0o600);
    let judge = fixture.judge(fixture.config());
    let staff = StaffBooks::new(fixture.store.clone(), fixture.log.clone());

    // Off: nothing judged, and the items are passed by.
    staff
        .add(
            fixture.project_id,
            StaffItemKind::Agent,
            Some(2),
            "Agent 2 exited.",
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(fixture.decisions().is_empty());

    judge.set_enabled(fixture.project_id, true).unwrap();
    staff
        .add(
            fixture.project_id,
            StaffItemKind::Message,
            None,
            "make a workspace",
        )
        .unwrap();
    staff
        .add(
            fixture.project_id,
            StaffItemKind::Agent,
            Some(3),
            "Agent 3 finished its turn.",
        )
        .unwrap();

    let decisions = until("a wake decision", || async {
        let decisions = fixture.decisions();
        (decisions.len() == 1).then_some(decisions)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fixture.decisions().len(), 1, "your own messages aren't judged");
    let decision = &decisions[0];
    assert!(decision.subject.starts_with("staff_inbox:"), "{decision:?}");
    assert_eq!(decision.would, "now");
    assert_eq!(decision.point, cos_core::DecisionPoint::Wake);
    assert_eq!(decision.input["source"], "staff_chief");
    assert!(decision.input["update"].get("run_goal").is_none());
    let wake = &judge.overview().unwrap().points[1];
    assert_eq!(wake.would["now"], 1);
    // The inbox is untouched: the items still wait for the chief.
    assert_eq!(staff.waiting(fixture.project_id).unwrap().len(), 3);
}

#[tokio::test]
async fn without_a_key_jev_is_off_and_says_why() {
    let fixture = Fixture::start().await;
    let judge = fixture.judge(fixture.config());
    judge.set_enabled(fixture.project_id, true).unwrap();

    let approval_id = fixture.ask("Bash ls").await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let overview = judge.overview().unwrap();
    assert!(!overview.active);
    assert_eq!(overview.key.source, KeySource::Missing);
    assert!(overview.off_reason.unwrap().contains(KEY_ENV));
    assert!(fixture.calls().is_empty());
    assert!(fixture.decisions().is_empty());
    fixture.manager.decide(approval_id, true, None).unwrap();
}

#[tokio::test]
async fn the_key_is_looked_up_in_order() {
    let fixture = Fixture::start().await;

    // The environment wins over daemon.env.
    fixture.write_env_file(&format!("{KEY_ENV}=sk-file\n"), 0o600);
    let env = fixture.judge(JudgeConfig {
        env_key: Some("sk-env".into()),
        ..fixture.config()
    });
    assert_eq!(env.overview().unwrap().key.source, KeySource::Environment);

    // A daemon.env others can read is refused, with the reason.
    fixture.write_env_file(&format!("{KEY_ENV}=sk-file\n"), 0o644);
    let refused = fixture.judge(fixture.config());
    let overview = refused.overview().unwrap();
    assert_eq!(overview.key.source, KeySource::Missing);
    assert!(overview.key.detail.unwrap().contains("chmod 600"));

    // Then the login shell, once, in the background; reload asks again.
    let shell = fixture.dir.path().join("login-shell");
    let script = |value: &str| format!("#!/bin/sh\n{KEY_ENV}={value}; export {KEY_ENV}; eval \"$2\"\n");
    std::fs::write(&shell, script("sk-shell")).unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probed = fixture.judge(JudgeConfig {
        shell: Some(shell.clone()),
        ..fixture.config()
    });
    let overview = until("the shell probe", || async {
        let overview = probed.overview().unwrap();
        (!overview.key.probing).then_some(overview)
    })
    .await;
    assert_eq!(overview.key.source, KeySource::Shell);
    assert!(overview.active);

    std::fs::write(&shell, script("")).unwrap();
    let reloaded = probed.reload().await.unwrap();
    assert_eq!(reloaded.key.source, KeySource::Missing);
    assert!(!reloaded.active);
    let changed = fixture
        .log
        .after(0, 1000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind::JUDGE_CHANGED)
        .count();
    assert!(changed >= 2, "the probe and the reload each log judge.changed");
}
