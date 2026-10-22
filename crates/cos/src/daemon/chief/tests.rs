//! The chief over the socket, with `cos-fake-claude` as Claude. The fake
//! chief reaches its tools through the real `cos mcp` (its `mcp …` prompts),
//! and the tests stand in for the pipeline by moving tasks with `Runs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    Agent, AgentState, Agents, ApprovalRoute, Base, EventLog, Inbox, InboxKind, Launch, NewRun, Projects,
    QuestionState, Role, Run, RunState, Runs, Store, TaskState,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::net::UnixListener;
use tokio::sync::watch;

use super::{Chief, ChiefConfig};
use crate::client::Client;
use crate::daemon::agents::tests::{cos_binary, fake_claude, git_repo, status, until};
use crate::daemon::agents::{AgentConfig, AgentManager, StartAgent};
use crate::daemon::{api, serve, tailnet::Tailnet};

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    log: EventLog,
    runs: Arc<Runs>,
    projects: Arc<Projects>,
    manager: AgentManager,
    _chief: Chief,
    client: Client,
    workspace_id: i64,
    _stop: watch::Sender<bool>,
}

impl Fixture {
    async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    async fn start_with(configure: impl FnOnce(&mut AgentConfig)) -> Self {
        for binary in [fake_claude(), cos_binary()] {
            assert!(
                binary.is_file(),
                "{} is missing; run the tests with `cargo test --workspace` (make test)",
                binary.display()
            );
        }
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
        let runs = Arc::new(Runs::new(store.clone(), log.clone()));
        let mut config = AgentConfig::for_tests(dir.path());
        configure(&mut config);
        let manager = AgentManager::start(
            Agents::new(store.clone(), log.clone()),
            projects.clone(),
            runs.clone(),
            cos_core::Environment::new(store.clone(), log.clone()),
            config,
        )
        .unwrap();
        let chief = Chief::start(
            store.clone(),
            log.clone(),
            runs.clone(),
            manager.clone(),
            dir.path().to_path_buf(),
            ChiefConfig::for_tests(),
        )
        .unwrap();
        let staff = crate::daemon::staff::Staff::start(
            store.clone(),
            log.clone(),
            projects.clone(),
            runs.clone(),
            manager.clone(),
            chief.clone(),
            crate::daemon::staff::StaffConfig::for_tests(),
        )
        .unwrap();
        let socket = dir.path().join("cos.sock");
        let (stop, stopping) = watch::channel(false);
        let judge =
            crate::daemon::judge::Judge::off_for_tests(&store, &log, &projects, &runs, &manager, dir.path());
        let state = api::AppState {
            staff,
            judge,
            log: log.clone(),
            projects: projects.clone(),
            devices: Arc::new(cos_core::Devices::new(store.clone(), log.clone())),
            tailnet: Tailnet::load(store.clone(), log.clone()),
            peers: crate::daemon::peers::Peers::without_tailscale(),
            runs: runs.clone(),
            agents: manager.clone(),
            chief: chief.clone(),
            started_at: Timestamp::now(),
            home: dir.path().to_path_buf(),
            stopping: stopping.clone(),
        };
        serve(UnixListener::bind(&socket).unwrap(), state, stopping);
        Self {
            client: Client::new(socket),
            dir,
            store,
            log,
            runs,
            projects,
            manager,
            _chief: chief,
            workspace_id: workspace.id,
            _stop: stop,
        }
    }

    async fn new_run(&self, goal: &str) -> Run {
        self.client
            .create_run(&NewRun {
                workspace_id: self.workspace_id,
                goal: goal.into(),
                preset: cos_core::ApprovalPreset::default(),
                approval_cap: None,
            })
            .await
            .unwrap()
    }

    /// A new run whose chief has read its first prompt and gone idle.
    async fn run_with_idle_chief(&self, goal: &str) -> (Run, i64) {
        let run = self.new_run(goal).await;
        let chief = run.chief_agent_id.expect("the run has a chief");
        self.until_says(chief, "echo: You are the chief of cos run").await;
        (run, chief)
    }

    /// Waits until the agent is idle with a last message containing `text`.
    async fn until_says(&self, id: i64, text: &str) -> Agent {
        until(&format!("agent {id} to be idle saying {text:?}"), || async {
            self.client.agent(id).await.ok().filter(|agent| {
                agent.state == AgentState::Idle
                    && agent
                        .last_message
                        .as_deref()
                        .is_some_and(|message| message.contains(text))
            })
        })
        .await
    }

    async fn until_state(&self, id: i64, state: AgentState) -> Agent {
        until(&format!("agent {id} to be {state:?}"), || async {
            self.client
                .agent(id)
                .await
                .ok()
                .filter(|agent| agent.state == state)
        })
        .await
    }

    /// Types `mcp <tool> <arguments>` into the fake chief, which calls the
    /// tool through `cos mcp`, and returns what it got back.
    async fn chief_calls(&self, chief: i64, tool: &str, arguments: &Value) -> String {
        self.chief_types(chief, &format!("mcp {tool} {arguments}")).await
    }

    /// Types a prompt into the fake chief and returns its next reply.
    async fn chief_types(&self, chief: i64, prompt: &str) -> String {
        let before = self.client.agent(chief).await.unwrap().last_message;
        self.client.say(chief, prompt.into()).await.unwrap();
        let agent = until(&format!("the chief's answer to {prompt:?}"), || async {
            self.client.agent(chief).await.ok().filter(|agent| {
                agent.state == AgentState::Idle
                    && agent.last_message.is_some()
                    && agent.last_message != before
            })
        })
        .await;
        agent.last_message.unwrap()
    }

    async fn tool(&self, agent: i64, name: &str, arguments: Value) -> anyhow::Result<String> {
        self.client.chief_tool(agent, name, &arguments).await
    }

    /// A worker of the task, as the pipeline would start it.
    fn worker(&self, run_id: i64, task_id: i64, prompt: &str) -> Agent {
        let request = StartAgent {
            prompt: Some(prompt.into()),
            permission_mode: Some("acceptEdits".into()),
            user_settings: Some(false),
            ..StartAgent::default()
        };
        let launch = Launch {
            task_id: Some(task_id),
            ..Launch::default()
        };
        self.manager
            .start_agent_with(self.workspace_id, Role::Worker, Some(run_id), &request, &launch)
            .unwrap()
    }

    fn board_file(&self, run_id: i64) -> PathBuf {
        self.dir
            .path()
            .join("runs")
            .join(run_id.to_string())
            .join("board.md")
    }

    fn events(&self, kind: &str) -> Vec<cos_core::LoggedEvent> {
        self.log
            .after(0, 10_000)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == kind)
            .collect()
    }

    /// Proposes and approves a plan with one task per title, each on its
    /// own file. Returns the task ids in order.
    fn approved_plan(&self, run_id: i64, titles: &[&str]) -> Vec<i64> {
        self.runs.propose_plan(run_id, &plan(titles)).unwrap();
        self.runs
            .approve_plan(run_id)
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.id)
            .collect()
    }
}

fn plan(titles: &[&str]) -> cos_core::PlanProposal {
    serde_json::from_value(plan_json(titles)).unwrap()
}

fn plan_json(titles: &[&str]) -> Value {
    let tasks: Vec<Value> = titles
        .iter()
        .map(|title| {
            json!({
                "title": title,
                "instructions": format!("do {title}"),
                "files": [format!("src/{title}.rs")],
            })
        })
        .collect();
    json!({ "tasks": tasks })
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[tokio::test]
async fn a_new_run_starts_its_chief() {
    let fixture = Fixture::start().await;

    let run = fixture.new_run("add a health check").await;

    let chief_id = run.chief_agent_id.expect("chief_agent_id is set");
    let chief = fixture.client.agent(chief_id).await.unwrap();
    assert_eq!(chief.role, Role::Chief);
    assert_eq!(chief.run_id, Some(run.id));
    assert_eq!(chief.title, format!("Chief of run {}", run.id));
    assert_eq!(chief.permission_mode, "default");
    assert!(!chief.user_settings, "a run's chief doesn't get your settings");
    let workspace = fixture.projects.workspace(fixture.workspace_id).unwrap();
    assert_eq!(chief.workspace_id, workspace.id);
    let chief = fixture
        .until_says(chief_id, "echo: You are the chief of cos run")
        .await;
    let first = chief.last_message.unwrap();
    assert!(first.contains("Goal: add a health check"), "{first}");
    assert!(first.contains("board_propose"), "{first}");

    let started = fixture.events(kind::AGENT_STARTED);
    assert_eq!(started[0].data["run_id"], run.id);
    let board = until("board.md", || async {
        Some(read(&fixture.board_file(run.id))).filter(|board| !board.is_empty())
    })
    .await;
    assert!(board.contains("> add a health check"), "{board}");
    assert_eq!(fixture.runs.run(run.id).unwrap().chief_agent_id, Some(chief_id));
}

#[tokio::test]
async fn a_run_fails_when_its_chief_cant_start() {
    let fixture = Fixture::start_with(|config| config.claude = "/nonexistent/claude".into()).await;

    let error = fixture
        .client
        .create_run(&NewRun {
            workspace_id: fixture.workspace_id,
            goal: "anything".into(),
            preset: cos_core::ApprovalPreset::default(),
            approval_cap: None,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("claude"), "{error}");
    let runs = fixture.runs.list().unwrap();
    assert_eq!(runs.len(), 1);
    let run = fixture.runs.run(runs[0].run.id).unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(
        run.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("the chief could not start")),
        "{run:?}"
    );
}

#[tokio::test]
async fn the_chief_plans_through_its_mcp_server() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("split the parser").await;

    let tools = fixture.chief_types(chief, "mcp tools").await;
    for tool in [
        "board_get",
        "board_propose",
        "task_update",
        "task_send",
        "ask_user",
        "approval_decide",
        "report_write",
    ] {
        assert!(tools.contains(tool), "{tool} missing from {tools}");
    }

    let board = fixture.chief_calls(chief, "board_get", &json!({})).await;
    assert!(board.contains(&format!("# Run {}: planning", run.id)), "{board}");
    assert!(board.contains("```json"), "{board}");

    let bad = json!({ "tasks": [
        { "title": "lexer", "instructions": "", "files": [] },
        { "title": "lexer", "instructions": "x", "files": ["src/a.rs"], "depends_on": ["nope"] },
    ]});
    let refused = fixture.chief_calls(chief, "board_propose", &bad).await;
    assert!(refused.starts_with("error:"), "{refused}");
    assert!(
        refused.matches("- ").count() >= 3,
        "every problem is listed: {refused}"
    );
    assert_eq!(fixture.runs.run(run.id).unwrap().state, RunState::Planning);

    let accepted = fixture
        .chief_calls(chief, "board_propose", &plan_json(&["lexer", "parser"]))
        .await;
    assert!(accepted.contains("Plan proposed: 2 tasks"), "{accepted}");
    assert_eq!(
        fixture.runs.run(run.id).unwrap().state,
        RunState::AwaitingPlanApproval
    );
    let board = until("board.md with the plan", || async {
        Some(read(&fixture.board_file(run.id))).filter(|board| board.contains("awaiting_plan_approval"))
    })
    .await;
    assert!(board.contains("lexer") && board.contains("parser"), "{board}");
    // Nothing is written inside the repo.
    let workspace = fixture.projects.workspace(fixture.workspace_id).unwrap();
    assert!(!workspace.path.join("board.md").exists());

    // The chief's own tools never ask for approval.
    assert!(fixture.events(kind::APPROVAL_REQUESTED).is_empty());
}

#[tokio::test]
async fn tools_check_who_calls_and_what_state_the_run_is_in() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("tidy up").await;
    let user_agent = fixture
        .client
        .start_agent(
            fixture.workspace_id,
            &StartAgent {
                prompt: Some("hi".into()),
                ..StartAgent::default()
            },
        )
        .await
        .unwrap();

    // Who may call.
    assert_eq!(
        status(
            fixture
                .tool(user_agent.id, "board_get", json!({}))
                .await
                .unwrap_err()
        ),
        403
    );
    assert_eq!(
        status(fixture.tool(999, "board_get", json!({})).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(fixture.tool(chief, "make_coffee", json!({})).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(
            fixture
                .tool(chief, "ask_user", json!({ "question": "x", "colour": "red" }))
                .await
                .unwrap_err()
        ),
        400
    );
    assert_eq!(
        status(
            fixture
                .tool(chief, "approval_decide", json!({}))
                .await
                .unwrap_err()
        ),
        400
    );

    // Not before the run reaches the report.
    assert_eq!(
        status(
            fixture
                .tool(chief, "report_write", json!({ "markdown": "# Done" }))
                .await
                .unwrap_err()
        ),
        409
    );
    assert_eq!(status(fixture.client.report(run.id).await.unwrap_err()), 404);

    let proposed = fixture
        .tool(chief, "board_propose", plan_json(&["a", "b"]))
        .await
        .unwrap();
    assert!(proposed.contains("Plan proposed: 2 tasks"), "{proposed}");
    let tasks: Vec<i64> = fixture
        .client
        .approve_plan(run.id)
        .await
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.id)
        .collect();
    let (a, b) = (tasks[0], tasks[1]);

    // task_update: what each action needs.
    let update = |task: i64, action: &str, reason: &str| {
        fixture.tool(
            chief,
            "task_update",
            json!({ "task_id": task, "action": action, "reason": reason }),
        )
    };
    assert_eq!(status(update(a, "retry", "again").await.unwrap_err()), 409);
    assert_eq!(status(update(a, "cancel", " ").await.unwrap_err()), 400);
    assert_eq!(status(update(a, "pause", "why not").await.unwrap_err()), 400);
    assert_eq!(status(update(999, "cancel", "x").await.unwrap_err()), 404);

    fixture.runs.update_task(a, TaskState::Running, None).unwrap();
    fixture
        .runs
        .update_task(a, TaskState::Blocked, Some("tests fail"))
        .unwrap();
    let retried = update(a, "retry", "use the fixture in tests/").await.unwrap();
    assert!(retried.contains("is now ready"), "{retried}");
    assert_eq!(fixture.runs.task(a).unwrap().state, TaskState::Ready);
    assert!(
        update(a, "cancel", "not needed after all")
            .await
            .unwrap()
            .contains("cancelled")
    );

    fixture.runs.update_task(b, TaskState::Running, None).unwrap();
    fixture
        .runs
        .update_task(b, TaskState::Blocked, Some("conflict"))
        .unwrap();
    assert!(
        update(b, "fail", "can't be done here")
            .await
            .unwrap()
            .contains("failed")
    );
    assert_eq!(fixture.runs.run(run.id).unwrap().state, RunState::Reporting);

    // The report.
    assert_eq!(
        status(
            fixture
                .tool(chief, "report_write", json!({ "markdown": "  " }))
                .await
                .unwrap_err()
        ),
        400
    );
    let saved = fixture
        .tool(
            chief,
            "report_write",
            json!({ "markdown": "# Run report\n\nNothing merged." }),
        )
        .await
        .unwrap();
    let path = fixture
        .dir
        .path()
        .join("runs")
        .join(run.id.to_string())
        .join("report.md");
    assert!(saved.contains(&path.display().to_string()), "{saved}");
    let markdown = read(&path);
    assert!(
        markdown.starts_with("# Run report\n\nNothing merged.\n"),
        "{markdown}"
    );
    assert!(markdown.contains("## Approved on your behalf (0)"), "{markdown}");
    let done = fixture.runs.run(run.id).unwrap();
    assert_eq!(done.state, RunState::Done);
    assert_eq!(done.report_path.as_deref(), Some(path.as_path()));
    let report = fixture.client.report(run.id).await.unwrap();
    assert_eq!((report.markdown, report.path), (markdown, path.clone()));
    let written = fixture.events(kind::RUN_REPORT_WRITTEN);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].run_id.as_deref(), Some(run.id.to_string().as_str()));
    assert_eq!(written[0].data["path"], path.display().to_string());
    assert_eq!(
        status(
            fixture
                .tool(chief, "report_write", json!({ "markdown": "again" }))
                .await
                .unwrap_err()
        ),
        409
    );

    // A finished run's chief is stopped once it's idle.
    fixture.until_state(chief, AgentState::Exited).await;
}

#[tokio::test]
async fn task_send_types_into_the_tasks_worker() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("send things").await;
    let task = fixture.approved_plan(run.id, &["one"])[0];
    // Stands in for the pipeline's worker.
    let worker_id = fixture.worker(run.id, task, "ready").id;
    fixture.until_says(worker_id, "echo: ready").await;
    let send =
        |message: &str| fixture.tool(chief, "task_send", json!({ "task_id": task, "message": message }));

    assert_eq!(status(send("too early").await.unwrap_err()), 409);
    fixture.runs.update_task(task, TaskState::Running, None).unwrap();
    assert_eq!(status(send("").await.unwrap_err()), 400);
    let sent = send("use serde for this").await.unwrap();

    assert!(sent.contains(&format!("agent {worker_id}")), "{sent}");
    fixture
        .until_says(worker_id, "echo: Message from the chief: use serde for this")
        .await;
}

#[tokio::test]
async fn questions_go_to_you_and_answers_wake_the_chief() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("pick a cache").await;

    let asked = fixture
        .chief_calls(
            chief,
            "ask_user",
            &json!({ "question": "Redis or in-memory?", "choices": ["redis", "memory"] }),
        )
        .await;
    assert!(asked.contains("Asked the user (question"), "{asked}");
    let open = fixture.client.questions(Some("open")).await.unwrap();
    assert_eq!(open.len(), 1);
    let question = &open[0];
    assert_eq!(
        (question.run_id, question.agent_id, question.state),
        (run.id, Some(chief), QuestionState::Open)
    );
    assert_eq!(question.choices, ["redis", "memory"]);
    let asked_events = fixture.events(kind::QUESTION_ASKED);
    assert_eq!(asked_events[0].data["question_id"], question.id);
    assert_eq!(
        asked_events[0].run_id.as_deref(),
        Some(run.id.to_string().as_str())
    );

    let answered = fixture
        .client
        .answer_question(question.id, "redis".into())
        .await
        .unwrap();
    assert_eq!(answered.state, QuestionState::Answered);
    assert_eq!(answered.answer.as_deref(), Some("redis"));
    assert_eq!(
        status(
            fixture
                .client
                .answer_question(question.id, "memory".into())
                .await
                .unwrap_err()
        ),
        409
    );
    assert_eq!(fixture.events(kind::QUESTION_ANSWERED).len(), 1);
    let woken = fixture.until_says(chief, "The user answered: \"redis\"").await;
    let wakeup = woken.last_message.unwrap();
    assert!(
        wakeup.contains(&format!("[cos wakeup] Run {} is planning", run.id)),
        "{wakeup}"
    );
    assert!(fixture.client.questions(Some("open")).await.unwrap().is_empty());

    // Messages.
    fixture
        .client
        .message_chief(run.id, "use the staging db".into())
        .await
        .unwrap();
    fixture
        .until_says(chief, "The user says: use the staging db")
        .await;
    assert_eq!(
        status(
            fixture
                .client
                .message_chief(run.id, "  ".into())
                .await
                .unwrap_err()
        ),
        400
    );
    assert_eq!(
        status(fixture.client.message_chief(999, "hi".into()).await.unwrap_err()),
        404
    );

    // Once the run is cancelled, its open questions stop waiting for you,
    // and its chief takes no more answers or messages.
    fixture
        .tool(chief, "ask_user", json!({ "question": "Still there?" }))
        .await
        .unwrap();
    let still_open = fixture.client.questions(Some("open")).await.unwrap();
    assert_eq!(still_open.len(), 1);
    fixture
        .client
        .cancel_run(run.id, Some("changed my mind".into()))
        .await
        .unwrap();
    assert!(fixture.client.questions(Some("open")).await.unwrap().is_empty());
    assert_eq!(fixture.client.questions(None).await.unwrap().len(), 2);
    assert_eq!(
        status(
            fixture
                .client
                .answer_question(still_open[0].id, "yes".into())
                .await
                .unwrap_err()
        ),
        409
    );
    assert_eq!(
        status(
            fixture
                .client
                .message_chief(run.id, "hello?".into())
                .await
                .unwrap_err()
        ),
        409
    );
}

#[tokio::test]
async fn wakeups_hold_at_most_20_items() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("batch").await;
    let inbox = Inbox::new(fixture.store.clone());

    // All at once, with no await in between, so the delivery loop sees 25.
    for n in 0..25 {
        inbox
            .add(run.id, InboxKind::Message, &format!("note {n}."))
            .unwrap();
    }

    fixture.until_says(chief, "note 24.").await;
    let items = inbox.items(run.id).unwrap();
    assert!(items.iter().all(|item| item.wakeup_id.is_some()));
    let first = items[0].wakeup_id;
    let sizes = [
        items.iter().filter(|item| item.wakeup_id == first).count(),
        items.iter().filter(|item| item.wakeup_id != first).count(),
    ];
    assert_eq!(sizes, [20, 5]);
    assert!(
        items[..20].iter().all(|item| item.wakeup_id == first),
        "oldest first"
    );
    assert_eq!(inbox.wakeups_since(run.id, 0).unwrap(), 2);
}

#[tokio::test]
async fn wakeups_wait_for_an_idle_chief() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("busy").await;
    let inbox = Inbox::new(fixture.store.clone());
    fixture.client.say(chief, "hang forever".into()).await.unwrap();
    fixture.until_state(chief, AgentState::Working).await;

    fixture
        .client
        .message_chief(run.id, "are you there".into())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    assert_eq!(inbox.waiting(run.id).unwrap().len(), 1);
    assert_eq!(inbox.wakeups_since(run.id, 0).unwrap(), 0);
    fixture.client.stop_agent(chief).await.unwrap();
    fixture.until_state(chief, AgentState::Exited).await;
}

#[tokio::test]
async fn wakeups_are_capped_at_30_an_hour() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("chatty").await;
    let inbox = Inbox::new(fixture.store.clone());
    for _ in 0..30 {
        inbox.deliver(run.id, chief, &[]).unwrap();
    }

    fixture
        .client
        .message_chief(run.id, "one more".into())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    assert_eq!(inbox.waiting(run.id).unwrap().len(), 1);
    let chief = fixture.client.agent(chief).await.unwrap();
    assert!(
        !chief.last_message.unwrap_or_default().contains("one more"),
        "past the cap, items wait"
    );
}

#[tokio::test]
async fn approvals_routed_to_the_chief_wake_it_and_it_decides() {
    let fixture = Fixture::start().await;
    let (run, chief) = fixture.run_with_idle_chief("document limits").await;
    let task = fixture.approved_plan(run.id, &["limits"])[0];
    let root = fixture.projects.workspace(fixture.workspace_id).unwrap().path;
    fixture.runs.assign_worktree(task, "cos/oslo-1", &root).unwrap();
    let worker = fixture.worker(
        run.id,
        task,
        &format!("ask:Write {}", root.join("docs/limits.md").display()),
    );

    let routed = until("an approval routed to the chief", || async {
        fixture
            .client
            .approvals(Some("pending"))
            .await
            .ok()?
            .into_iter()
            .find(|approval| approval.agent_id == worker.id)
    })
    .await;
    assert_eq!(routed.route, Some(ApprovalRoute::Chief));
    fixture
        .until_says(chief, &format!("Approval {} waits for your decision", routed.id))
        .await;

    let decided = fixture
        .chief_calls(
            chief,
            "approval_decide",
            &json!({ "approval_id": routed.id, "decision": "allow", "reason": "the task documents the limits" }),
        )
        .await;

    assert!(decided.contains("is now allowed"), "{decided}");
    fixture.until_says(worker.id, "allowed by hook").await;
    let section = fixture.runs.approvals_report_section(run.id).unwrap();
    assert!(section.contains("## Approved on your behalf (1)"), "{section}");
}

#[tokio::test]
async fn a_runs_chief_gets_the_project_env() {
    use crate::daemon::agents::tests::{home, launched_env, var};

    let fixture = Fixture::start().await;
    let project = fixture
        .projects
        .workspace(fixture.workspace_id)
        .unwrap()
        .project_id;
    fixture
        .client
        .set_project_env(project, "CLAUDE_CONFIG_DIR", "~/.claude-work".into())
        .await
        .unwrap();

    let (_, chief) = fixture.run_with_idle_chief("use the work account").await;
    let chief = fixture.client.agent(chief).await.unwrap();
    assert_eq!(chief.role, Role::Chief);
    let env = launched_env(fixture.dir.path(), &chief.session_id);
    assert_eq!(
        var(&env, "CLAUDE_CONFIG_DIR"),
        Some(format!("{}/.claude-work", home()).as_str())
    );
    assert_eq!(chief.env_names, ["CLAUDE_CONFIG_DIR"]);
}
