//! The Chief of Staff over the socket, with `cos-fake-claude` as Claude.
//! The fake reaches the staff tools through the real `cos mcp` (its `mcp …`
//! prompts), so the tests type those as messages, the way you would.

use std::sync::Arc;

use cos_core::events::kind;
use cos_core::{Agent, AgentState, Agents, Base, EventLog, Projects, Role, StaffBooks, StaffItemKind, Store};
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::net::UnixListener;
use tokio::sync::watch;

use super::{Staff, StaffConfig, StaffSettings};
use crate::client::Client;
use crate::daemon::agents::tests::{cos_binary, fake_claude, git_repo, status, until};
use crate::daemon::agents::{AgentConfig, AgentManager};
use crate::daemon::transcript::MessageKind;
use crate::daemon::{api, serve, tailnet::Tailnet};

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    log: EventLog,
    projects: Arc<Projects>,
    books: StaffBooks,
    /// The records, with what the API leaves out (the system prompt).
    records: Agents,
    client: Client,
    project_id: i64,
    workspace_id: i64,
    manager: AgentManager,
    staff: Staff,
    _stop: watch::Sender<bool>,
}

impl Fixture {
    async fn start() -> Self {
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
        let manager = start_manager(&store, &log, &projects, dir.path());
        let (staff, stop) = serve_daemon(&store, &log, &projects, &manager, dir.path());
        Self {
            client: Client::new(dir.path().join("cos.sock")),
            books: StaffBooks::new(store.clone(), log.clone()),
            records: Agents::new(store.clone(), log.clone()),
            store,
            dir,
            log,
            projects,
            project_id: project.id,
            workspace_id: workspace.id,
            manager,
            staff,
            _stop: stop,
        }
    }

    /// What a daemon restart does: shut the agents down, then start the
    /// agents, chiefs and server again on the same database. Chiefs aren't
    /// resumed until the test calls `resume_chiefs`, as the daemon does
    /// once it serves.
    async fn restart(&mut self) {
        self.manager.shutdown().await;
        let _ = self._stop.send(true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.manager = start_manager(&self.store, &self.log, &self.projects, self.dir.path());
        (self.staff, self._stop) = serve_daemon(
            &self.store,
            &self.log,
            &self.projects,
            &self.manager,
            self.dir.path(),
        );
    }

    /// Starts the chief and waits until it's idle.
    async fn idle_staff(&self) -> Agent {
        let view = self.client.start_staff(self.project_id).await.unwrap();
        self.until_state(view.agent_id.unwrap(), AgentState::Idle).await
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

    /// Sends `text` through `…/staff/message` and returns the chief's reply
    /// to it (the fake echoes, or answers `mcp …` with the tool's result).
    async fn say(&self, staff: i64, text: &str) -> String {
        let before = self.client.agent(staff).await.unwrap().last_message;
        self.client
            .message_staff(self.project_id, text.into())
            .await
            .unwrap();
        let agent = until(&format!("the chief's answer to {text:?}"), || async {
            self.client.agent(staff).await.ok().filter(|agent| {
                agent.state == AgentState::Idle
                    && agent.last_message.is_some()
                    && agent.last_message != before
            })
        })
        .await;
        agent.last_message.unwrap()
    }

    /// Calls a staff tool the way `cos mcp` does.
    async fn tool(&self, agent: i64, name: &str, arguments: Value) -> anyhow::Result<String> {
        self.client.staff_tool(agent, name, &arguments).await
    }

    async fn tool_json(&self, agent: i64, name: &str, arguments: Value) -> Value {
        let text = self.tool(agent, name, arguments).await.unwrap();
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} returned {text}"))
    }

    fn events(&self, kind: &str) -> Vec<cos_core::LoggedEvent> {
        self.log
            .after(0, 10_000)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == kind)
            .collect()
    }

    /// Another project with a workspace and an agent record in it.
    async fn other_project(&self) -> (i64, i64, i64) {
        let repo = git_repo(&self.dir.path().join("other"));
        let (project, _) = self.projects.add(&repo).await.unwrap();
        let workspace = self
            .projects
            .create_workspace(project.id, Base::Head, Some("Elsewhere"))
            .await
            .unwrap();
        let agent = self
            .client
            .start_agent(
                workspace.id,
                &crate::daemon::agents::StartAgent {
                    prompt: Some("hello".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        (project.id, workspace.id, agent.id)
    }
}

fn start_manager(
    store: &Arc<Store>,
    log: &EventLog,
    projects: &Arc<Projects>,
    dir: &std::path::Path,
) -> AgentManager {
    AgentManager::start(
        Agents::new(store.clone(), log.clone()),
        projects.clone(),
        cos_core::Environment::new(store.clone(), log.clone()),
        AgentConfig::for_tests(dir),
    )
    .unwrap()
}

/// Starts the chiefs and serves the API on `<dir>/cos.sock` until the
/// returned sender is told to stop.
fn serve_daemon(
    store: &Arc<Store>,
    log: &EventLog,
    projects: &Arc<Projects>,
    manager: &AgentManager,
    dir: &std::path::Path,
) -> (Staff, watch::Sender<bool>) {
    let staff = Staff::start(
        store.clone(),
        log.clone(),
        projects.clone(),
        manager.clone(),
        StaffConfig::for_tests(),
    )
    .unwrap();
    let socket = dir.join("cos.sock");
    let _ = std::fs::remove_file(&socket);
    let (stop, stopping) = watch::channel(false);
    let judge = crate::daemon::judge::Judge::off_for_tests(store, log, projects, manager, dir);
    let state = api::AppState {
        forwards: crate::daemon::forward::Forwards::new(),
        shells: crate::daemon::shells::Shells::new(),
        log: log.clone(),
        projects: projects.clone(),
        devices: Arc::new(cos_core::Devices::new(store.clone(), log.clone())),
        tailnet: Tailnet::load(store.clone(), log.clone()),
        peers: crate::daemon::peers::Peers::without_tailscale(),
        agents: manager.clone(),
        staff: staff.clone(),
        judge,
        started_at: Timestamp::now(),
        home: dir.to_path_buf(),
        stopping: stopping.clone(),
    };
    serve(UnixListener::bind(&socket).unwrap(), state, stopping);
    (staff, stop)
}

#[tokio::test]
async fn it_starts_lazily_in_its_own_worktree_and_resumes_the_same_session() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;

    let before = client.staff(fixture.project_id).await.unwrap();
    assert_eq!((before.agent_id, before.state), (None, None));
    assert_eq!(before.branch, "repo/chief");
    assert!(
        before.worktree.ends_with("chief"),
        "{}",
        before.worktree.display()
    );
    assert!(!before.worktree.exists(), "made lazily");
    assert_eq!((before.working_agents, before.waiting_on_you), (0, 0));

    let staff = fixture.idle_staff().await;
    assert_eq!(staff.role, Role::Staff);
    assert_eq!(staff.title, "Chief of Staff · repo");
    assert!(staff.user_settings, "your own assistant gets your settings");
    assert_eq!(staff.permission_mode, "default");
    let view = client.staff(fixture.project_id).await.unwrap();
    assert_eq!(
        (view.agent_id, view.state),
        (Some(staff.id), Some(AgentState::Idle))
    );
    assert!(view.worktree.is_dir());
    assert_eq!(
        staff.workspace_id,
        fixture
            .projects
            .staff_workspace(fixture.project_id)
            .await
            .unwrap()
            .id
    );
    // Its worktree isn't a workspace.
    let projects = client.projects().await.unwrap();
    assert_eq!(projects[0].workspaces.len(), 1);
    assert_eq!(
        status(client.remove_workspace(staff.workspace_id).await.unwrap_err()),
        404
    );
    assert_eq!(fixture.events(kind::STAFF_STARTED).len(), 1);
    assert_eq!(
        fixture.events(kind::STAFF_STARTED)[0].data,
        json!({ "project_id": fixture.project_id, "agent_id": staff.id })
    );

    // Starting it again while it runs does nothing.
    let again = client.start_staff(fixture.project_id).await.unwrap();
    assert_eq!(again.agent_id, Some(staff.id));
    assert_eq!(fixture.events(kind::STAFF_STARTED).len(), 1);

    // Stopped before its first message, Claude saved no session, so it
    // starts afresh with the same session id.
    client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
    let view = client.start_staff(fixture.project_id).await.unwrap();
    assert_eq!(view.agent_id, Some(staff.id));
    fixture.until_state(staff.id, AgentState::Idle).await;
    let screen = client.screen(staff.id).await.unwrap();
    assert!(
        screen
            .text
            .contains(&format!("session {} (startup)", staff.session_id)),
        "{}",
        screen.text
    );

    // After a message it resumes the same session, with the model set for
    // the project.
    assert_eq!(fixture.say(staff.id, "remember me").await, "echo: remember me");
    let set = client
        .set_staff_model(fixture.project_id, Some("haiku".into()))
        .await
        .unwrap();
    assert_eq!(set.model.as_deref(), Some("haiku"));
    client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
    let view = client.start_staff(fixture.project_id).await.unwrap();
    assert_eq!(view.agent_id, Some(staff.id));
    let resumed = fixture.until_state(staff.id, AgentState::Idle).await;
    assert_eq!(resumed.session_id, staff.session_id);
    assert_eq!(resumed.model.as_deref(), Some("haiku"));
    let screen = client.screen(staff.id).await.unwrap();
    assert!(
        screen
            .text
            .contains(&format!("session {} (resume)", staff.session_id)),
        "{}",
        screen.text
    );
    assert_eq!(fixture.events(kind::STAFF_STARTED).len(), 3);

    // Its transcript reads as a chat.
    let transcript = client.transcript(staff.id, 0).await.unwrap();
    let texts: Vec<_> = transcript
        .messages
        .iter()
        .map(|message| message.text.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(texts, ["remember me", "echo: remember me"]);
}

#[tokio::test]
async fn the_brief_and_instructions_go_in_the_system_prompt() {
    let fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    let stored_prompt = |id: i64| {
        fixture
            .records
            .get(id)
            .unwrap()
            .and_then(|agent| agent.system_prompt)
            .unwrap_or_default()
    };
    let prompt = stored_prompt(staff.id);
    assert!(
        prompt.contains("You are the Chief of Staff for the project repo."),
        "{prompt}"
    );
    assert!(prompt.contains("[cos · N updates]"), "{prompt}");
    assert!(
        prompt.ends_with("Your brief:\n(empty)\n\nYour memory notes:\n(none yet)"),
        "{prompt}"
    );

    let saved = fixture
        .tool(
            staff.id,
            "brief_update",
            json!({ "brief": "  Ship the README.  " }),
        )
        .await
        .unwrap();
    assert_eq!(saved, "Brief saved (16 bytes).");
    assert_eq!(
        fixture.client.staff(fixture.project_id).await.unwrap().brief,
        "Ship the README."
    );
    assert!(stored_prompt(staff.id).contains("Your brief:\nShip the README.\n"));

    let too_long = fixture
        .tool(staff.id, "brief_update", json!({ "brief": "x".repeat(4097) }))
        .await
        .unwrap_err();
    assert!(
        too_long.to_string().contains("keep it under 4096"),
        "{too_long:#}"
    );
}

#[tokio::test]
async fn you_can_set_its_brief_and_it_gets_it_on_its_next_start() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;

    // Before it ever starts: saved, and in its first prompt.
    let view = client
        .set_staff_brief(fixture.project_id, "  Release 0.2 by Friday.\n")
        .await
        .unwrap();
    assert_eq!(view.brief, "Release 0.2 by Friday.");
    let staff = fixture.idle_staff().await;
    let stored_prompt = || {
        fixture
            .records
            .get(staff.id)
            .unwrap()
            .and_then(|agent| agent.system_prompt)
            .unwrap_or_default()
    };
    assert!(
        stored_prompt().contains("Your brief:\nRelease 0.2 by Friday.\n"),
        "{}",
        stored_prompt()
    );

    // While it runs: saved for its next launch, and logged.
    let changed = fixture.events(kind::STAFF_CHANGED).len();
    let view = client
        .set_staff_brief(fixture.project_id, "Only the login fix.")
        .await
        .unwrap();
    assert_eq!(view.brief, "Only the login fix.");
    assert!(stored_prompt().contains("Your brief:\nOnly the login fix.\n"));
    assert_eq!(fixture.events(kind::STAFF_CHANGED).len(), changed + 1);

    // Too long: refused, and the old one stays.
    let too_long = client
        .set_staff_brief(fixture.project_id, &"x".repeat(4097))
        .await
        .unwrap_err();
    assert!(
        too_long.to_string().contains("keep it under 4096"),
        "{too_long:#}"
    );
    let view = client.staff(fixture.project_id).await.unwrap();
    assert_eq!(view.brief, "Only the login fix.");

    // Empty clears it.
    let view = client.set_staff_brief(fixture.project_id, "  ").await.unwrap();
    assert_eq!(view.brief, "");
    assert!(stored_prompt().contains("Your brief:\n(empty)\n"));
}

#[tokio::test]
async fn only_you_set_its_identity_and_it_heads_its_prompt() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;

    // Before it ever starts: saved, and in its first prompt, above the rules.
    let view = client
        .set_staff_identity(fixture.project_id, "  You are Ada. Be terse.\n")
        .await
        .unwrap();
    assert_eq!(view.identity, "You are Ada. Be terse.");
    let staff = fixture.idle_staff().await;
    let stored_prompt = || {
        fixture
            .records
            .get(staff.id)
            .unwrap()
            .and_then(|agent| agent.system_prompt)
            .unwrap_or_default()
    };
    let prompt = stored_prompt();
    let identity = prompt
        .find("<identity>\nYou are Ada. Be terse.\n</identity>")
        .unwrap_or_else(|| panic!("{prompt}"));
    assert!(
        identity < prompt.find("- You may create workspaces").unwrap(),
        "{prompt}"
    );
    assert!(prompt.contains("Only the user can change it"), "{prompt}");

    // The chief has no tool for it.
    let refused = fixture
        .tool(staff.id, "identity_update", json!({ "identity": "I am Bob." }))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("no tool"), "{refused:#}");
    assert_eq!(
        client.staff(fixture.project_id).await.unwrap().identity,
        "You are Ada. Be terse."
    );

    // While it runs: saved for its next launch, and logged.
    let changed = fixture.events(kind::STAFF_CHANGED).len();
    client
        .set_staff_identity(fixture.project_id, "You are Ada. Explain your reasoning.")
        .await
        .unwrap();
    assert!(stored_prompt().contains("<identity>\nYou are Ada. Explain your reasoning.\n</identity>"));
    assert_eq!(fixture.events(kind::STAFF_CHANGED).len(), changed + 1);

    // Too long: refused, and the old one stays.
    let too_long = client
        .set_staff_identity(fixture.project_id, &"x".repeat(8193))
        .await
        .unwrap_err();
    assert!(
        too_long.to_string().contains("keep it under 8192"),
        "{too_long:#}"
    );
    assert_eq!(
        client.staff(fixture.project_id).await.unwrap().identity,
        "You are Ada. Explain your reasoning."
    );

    // Empty clears it, and the prompt says nothing about one.
    let view = client.set_staff_identity(fixture.project_id, " ").await.unwrap();
    assert_eq!(view.identity, "");
    assert!(!stored_prompt().contains("<identity>"), "{}", stored_prompt());
}

#[tokio::test]
async fn memory_notes_are_listed_in_its_prompt_and_you_can_edit_them_too() {
    let fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    let stored_prompt = || {
        fixture
            .records
            .get(staff.id)
            .unwrap()
            .and_then(|agent| agent.system_prompt)
            .unwrap_or_default()
    };
    let changed = || fixture.events(kind::STAFF_CHANGED).len();

    let note = json!({
        "name": "release",
        "description": "How cos ships",
        "body": "Tag v*, then wait for CI.",
    });
    let saved = fixture.tool(staff.id, "memory_write", note).await.unwrap();
    assert_eq!(saved, "Saved the note release (25 bytes).");
    assert_eq!(changed(), 1);
    assert!(
        stored_prompt().ends_with("Your memory notes:\n- release: How cos ships"),
        "{}",
        stored_prompt()
    );

    // You write one from an app; the chief reads it.
    let yours = cos_core::StaffNote {
        name: "people".into(),
        description: "Who to ask about what".into(),
        body: "Rakibul owns the apps.".into(),
        created_ms: 0,
        updated_ms: 0,
    };
    let written = fixture
        .client
        .write_staff_note(
            fixture.project_id,
            &yours.name,
            &super::NoteWrite {
                description: yours.description.clone(),
                body: yours.body.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (written.name.as_str(), written.body.as_str()),
        ("people", "Rakibul owns the apps.")
    );
    assert!(stored_prompt().contains("- people: Who to ask about what\n- release: How cos ships"));
    let list = fixture.tool_json(staff.id, "memory_list", json!({})).await;
    assert_eq!(list["notes"][0]["name"], json!("people"), "{list}");
    assert_eq!(list["notes"][1]["bytes"], json!(25), "{list}");
    let read = fixture
        .tool_json(staff.id, "memory_read", json!({ "name": "people" }))
        .await;
    assert_eq!(read["body"], json!("Rakibul owns the apps."), "{read}");

    // Replacing keeps one note under the name.
    let replaced = fixture
        .tool(
            staff.id,
            "memory_write",
            json!({ "name": "release", "description": "How cos ships", "body": "Tag v*." }),
        )
        .await
        .unwrap();
    assert_eq!(replaced, "Replaced the note release (7 bytes).");
    let notes = fixture.client.staff_memory(fixture.project_id).await.unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].body, "Tag v*.");

    // A wrong name says which notes there are; bad notes are refused.
    let missing = fixture
        .tool(staff.id, "memory_read", json!({ "name": "relase" }))
        .await
        .unwrap_err();
    assert!(
        missing.to_string().contains("the notes are: people, release"),
        "{missing:#}"
    );
    let bad = fixture
        .tool(
            staff.id,
            "memory_write",
            json!({ "name": "Two Words", "description": "d", "body": "b" }),
        )
        .await
        .unwrap_err();
    assert!(bad.to_string().contains("lowercase letters"), "{bad:#}");

    // Deleting, by the chief or by you, takes it out of the prompt.
    let deleted = fixture
        .tool(staff.id, "memory_delete", json!({ "name": "release" }))
        .await
        .unwrap();
    assert_eq!(deleted, "Deleted the note release.");
    fixture
        .client
        .delete_staff_note(fixture.project_id, "people")
        .await
        .unwrap();
    assert!(stored_prompt().ends_with("Your memory notes:\n(none yet)"));
    assert_eq!(
        status(
            fixture
                .client
                .delete_staff_note(fixture.project_id, "people")
                .await
                .unwrap_err()
        ),
        404
    );
    assert_eq!(changed(), 5);
}

#[tokio::test]
async fn a_new_session_keeps_the_chief_its_agents_and_its_memory() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;

    // Before it ever started, a new session just starts it.
    let view = client.new_staff_session(fixture.project_id).await.unwrap();
    let staff = fixture
        .until_state(view.agent_id.unwrap(), AgentState::Idle)
        .await;
    assert_eq!(fixture.events(kind::STAFF_STARTED).len(), 1);

    assert_eq!(fixture.say(staff.id, "remember me").await, "echo: remember me");
    let spawned = fixture
        .tool_json(
            staff.id,
            "agent_spawn",
            json!({ "prompt": "say hi", "workspace_id": fixture.workspace_id }),
        )
        .await;
    let agent_id = spawned["agent_id"].as_i64().unwrap();
    fixture
        .tool(
            staff.id,
            "memory_write",
            json!({ "name": "release", "description": "How cos ships", "body": "Tag v*." }),
        )
        .await
        .unwrap();
    let transcripts_before = fixture.events(kind::AGENT_TRANSCRIPT).len();

    // While it runs: it is stopped, then started in a new Claude session.
    let view = client.new_staff_session(fixture.project_id).await.unwrap();
    assert_eq!(view.agent_id, Some(staff.id), "the same chief");
    let fresh = fixture.until_state(staff.id, AgentState::Idle).await;
    assert_ne!(fresh.session_id, staff.session_id);
    let screen = client.screen(staff.id).await.unwrap();
    assert!(
        screen
            .text
            .contains(&format!("session {} (startup)", fresh.session_id)),
        "{}",
        screen.text
    );
    let prompt = fixture
        .records
        .get(staff.id)
        .unwrap()
        .and_then(|agent| agent.system_prompt)
        .unwrap_or_default();
    assert!(prompt.ends_with("- release: How cos ships"), "{prompt}");
    assert_eq!(fixture.events(kind::STAFF_STARTED).len(), 2);

    // Its chat starts over, and apps are told to read it from the top.
    let reset = fixture.events(kind::AGENT_TRANSCRIPT)[transcripts_before..]
        .iter()
        .any(|event| event.data == json!({ "agent_id": staff.id, "next": 0 }));
    assert!(reset, "agent.transcript with next 0");
    assert_eq!(fixture.say(staff.id, "who am i").await, "echo: who am i");
    let texts: Vec<String> = client
        .transcript(staff.id, 0)
        .await
        .unwrap()
        .messages
        .iter()
        .filter_map(|message| message.text.clone())
        .collect();
    assert_eq!(texts, ["who am i", "echo: who am i"]);

    // The agent it started is still its own.
    assert_eq!(client.agent(agent_id).await.unwrap().spawned_by, Some(staff.id));
    let read = fixture
        .tool_json(staff.id, "agent_read", json!({ "agent_id": agent_id }))
        .await;
    assert!(read["state"].is_string(), "{read}");

    // A restart after that resumes the new session, not the old one.
    client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
    client.start_staff(fixture.project_id).await.unwrap();
    let resumed = fixture.until_state(staff.id, AgentState::Idle).await;
    assert_eq!(resumed.session_id, fresh.session_id);
}

#[tokio::test]
async fn every_tool_works_on_its_own_project_only() {
    let fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    let (other_project, other_workspace, other_agent) = fixture.other_project().await;

    // workspaces_list
    let list = fixture.tool_json(staff.id, "workspaces_list", json!({})).await;
    let workspaces = list["workspaces"].as_array().unwrap();
    assert_eq!(workspaces.len(), 1, "{list}");
    assert_eq!(workspaces[0]["id"], json!(fixture.workspace_id));
    assert_eq!(workspaces[0]["title"], Value::Null);
    assert!(
        workspaces[0]["branch"].as_str().unwrap().starts_with("repo/"),
        "{list}"
    );

    // workspace_create: a city name, the project's branch prefix, no origin so HEAD.
    let created = fixture
        .tool_json(staff.id, "workspace_create", json!({ "title": "Docs" }))
        .await;
    assert_eq!(created["title"], "Docs");
    assert_eq!(
        created["branch"],
        format!("repo/{}", created["city"].as_str().unwrap())
    );
    assert_eq!(created["base"], "head");

    // agent_spawn: needs exactly one place.
    let both = fixture
        .tool(
            staff.id,
            "agent_spawn",
            json!({ "prompt": "x", "workspace_id": fixture.workspace_id, "new_workspace": {} }),
        )
        .await
        .unwrap_err();
    assert!(both.to_string().contains("exactly one"), "{both:#}");
    let spawned = fixture
        .tool_json(
            staff.id,
            "agent_spawn",
            json!({ "prompt": "Add a line to README", "new_workspace": { "title": "Readme" } }),
        )
        .await;
    let agent_id = spawned["agent_id"].as_i64().unwrap();
    let workspace_id = spawned["workspace_id"].as_i64().unwrap();
    let agent = fixture.client.agent(agent_id).await.unwrap();
    assert_eq!(agent.role, Role::User);
    assert_eq!(agent.spawned_by, Some(staff.id));
    assert_eq!(agent.workspace_id, workspace_id);
    assert_eq!(agent.title, "Add a line to README");
    let reply = fixture.until_says(agent_id, "echo: Add a line to README").await;
    assert_eq!(
        reply.last_message.as_deref(),
        Some(
            "echo: Add a line to README\n\n[cos] The Chief of Staff of repo started you. When you finish, end with \
             a short summary of what you did and anything left open."
        )
    );
    let view = fixture.client.staff(fixture.project_id).await.unwrap();
    assert_eq!((view.working_agents, view.waiting_on_you), (0, 0));

    // agent_read: the full last message, and the screen if asked.
    let read = fixture
        .tool_json(
            staff.id,
            "agent_read",
            json!({ "agent_id": agent_id, "screen": true }),
        )
        .await;
    assert_eq!(read["state"], "idle");
    assert!(
        read["last_message"]
            .as_str()
            .unwrap()
            .ends_with("anything left open."),
        "{read}"
    );
    assert!(read["screen"].as_str().unwrap().contains("fake claude"), "{read}");

    // agent_send, agent_stop, and agent_send resuming a stopped agent.
    let sent = fixture
        .tool(
            staff.id,
            "agent_send",
            json!({ "agent_id": agent_id, "message": "and another" }),
        )
        .await
        .unwrap();
    assert_eq!(sent, format!("Sent to agent {agent_id}."));
    fixture.until_says(agent_id, "echo: and another").await;
    let stopped = fixture
        .tool(staff.id, "agent_stop", json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(stopped, format!("Stopping agent {agent_id}."));
    fixture.until_state(agent_id, AgentState::Exited).await;
    let resumed = fixture
        .tool(
            staff.id,
            "agent_send",
            json!({ "agent_id": agent_id, "message": "once more" }),
        )
        .await
        .unwrap();
    assert_eq!(resumed, format!("Resumed agent {agent_id} and sent the message."));
    fixture.until_says(agent_id, "echo: once more").await;

    // There is no run_start any more.
    let gone = fixture
        .tool(
            staff.id,
            "run_start",
            json!({ "workspace_id": fixture.workspace_id, "goal": "Tidy up" }),
        )
        .await
        .unwrap_err();
    assert_eq!(status(gone), 404);

    // Ids from another project, or the chief's own worktree, are refused.
    for (tool, arguments) in [
        ("agent_read", json!({ "agent_id": other_agent })),
        ("agent_send", json!({ "agent_id": other_agent, "message": "hi" })),
        ("agent_stop", json!({ "agent_id": other_agent })),
        ("agent_read", json!({ "agent_id": staff.id })),
        (
            "agent_spawn",
            json!({ "prompt": "x", "workspace_id": other_workspace }),
        ),
        (
            "agent_spawn",
            json!({ "prompt": "x", "workspace_id": staff.workspace_id }),
        ),
    ] {
        let refused = fixture.tool(staff.id, tool, arguments.clone()).await.unwrap_err();
        assert!(
            [403, 404].contains(&status(refused)),
            "{tool} {arguments} should be refused"
        );
    }
    assert_eq!(fixture.client.agent(other_agent).await.unwrap().spawned_by, None);
    let list = fixture.tool_json(staff.id, "workspaces_list", json!({})).await;
    assert!(
        !list.to_string().contains("Elsewhere"),
        "project {other_project}'s workspace leaked: {list}"
    );

    // Bad arguments and unknown tools are tool errors; other agents can't
    // call staff tools.
    let bad = fixture
        .tool(staff.id, "agent_read", json!({ "id": 1 }))
        .await
        .unwrap_err();
    assert!(
        bad.to_string().contains("bad arguments for agent_read"),
        "{bad:#}"
    );
    assert_eq!(
        status(fixture.tool(staff.id, "rm_rf", json!({})).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(
            fixture
                .tool(agent_id, "workspaces_list", json!({}))
                .await
                .unwrap_err()
        ),
        403
    );
}

#[tokio::test]
async fn a_spawned_agents_news_wakes_the_chief_through_its_inbox() {
    let fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;

    // Through the real MCP server: the staff's tool list, then a spawn.
    let tools = fixture.say(staff.id, "mcp tools").await;
    assert_eq!(
        tools,
        "workspaces_list,workspace_create,agent_spawn,agent_send,agent_read,agent_stop,brief_update,memory_list,\
         memory_read,memory_write,memory_delete"
    );
    let spawn = json!({ "prompt": "say hi", "workspace_id": fixture.workspace_id });
    let spawned = fixture.say(staff.id, &format!("mcp agent_spawn {spawn}")).await;
    let agent_id = serde_json::from_str::<Value>(&spawned).unwrap()["agent_id"]
        .as_i64()
        .unwrap();

    // Its turn ends: the chief is woken with one update.
    let woken = fixture.until_says(staff.id, "[cos · 1 updates]").await;
    let wakeup = woken.last_message.unwrap();
    assert!(
        wakeup.starts_with(&format!(
            "echo: [cos · 1 updates]\n\n### Agent {agent_id} \"say hi\" finished a turn\necho: say hi"
        )),
        "{wakeup}"
    );
    assert!(wakeup.ends_with("then end your turn."), "{wakeup}");
    let inbox = fixture.client.staff_inbox(fixture.project_id).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(
        (inbox[0].kind, inbox[0].agent_id),
        (StaffItemKind::Agent, Some(agent_id))
    );
    assert!(inbox[0].delivered_ms.is_some());
    assert!(!fixture.events(kind::STAFF_INBOX).is_empty());

    // The wakeup shows as an update in the chief's transcript.
    let transcript = fixture.client.transcript(staff.id, 0).await.unwrap();
    let update = transcript
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::Update)
        .expect("an update row");
    assert!(update.text.as_deref().unwrap().starts_with("[cos · 1 updates]"));
    let tool = transcript
        .messages
        .iter()
        .find_map(|message| message.tool.as_ref())
        .expect("a tool row");
    assert_eq!(tool.name, "mcp__cos__agent_spawn");

    // An approval waiting for you, and an exit, are news too.
    fixture.client.say(agent_id, "ask:Bash ls".into()).await.unwrap();
    let woken = fixture
        .until_says(staff.id, "waits for the user to approve")
        .await;
    assert!(woken.last_message.unwrap().contains("Bash"));
    let approval = fixture
        .client
        .approvals(Some("pending"))
        .await
        .unwrap()
        .into_iter()
        .find(|approval| approval.agent_id == agent_id)
        .unwrap();
    fixture
        .client
        .answer_approval(approval.id, false, None)
        .await
        .unwrap();
    fixture.until_says(staff.id, "finished a turn\ndenied").await;
    fixture.client.stop_agent(agent_id).await.unwrap();
    fixture.until_says(staff.id, "exited (exit code 0)").await;
}

#[tokio::test]
async fn an_agent_started_with_report_false_sends_the_chief_no_news() {
    let mut fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    let inbox_len = |fixture: &Fixture| {
        let books = &fixture.books;
        books.recent(fixture.project_id, 100).unwrap().len()
    };

    // A silent agent's turn ends: nothing reaches the inbox.
    let spawn = json!({ "prompt": "quiet job", "workspace_id": fixture.workspace_id, "report": false });
    let spawned = fixture.say(staff.id, &format!("mcp agent_spawn {spawn}")).await;
    let silent = serde_json::from_str::<Value>(&spawned).unwrap()["agent_id"]
        .as_i64()
        .unwrap();
    let agent = fixture.until_says(silent, "echo: quiet job").await;
    assert_eq!((agent.spawned_by, agent.report), (Some(staff.id), false));

    // A default one's does, and wakes the chief with its update alone.
    let spawned = fixture
        .tool_json(
            staff.id,
            "agent_spawn",
            json!({ "prompt": "say hi", "workspace_id": fixture.workspace_id }),
        )
        .await;
    let loud = spawned["agent_id"].as_i64().unwrap();
    assert!(fixture.client.agent(loud).await.unwrap().report);
    let woken = fixture.until_says(staff.id, "[cos · 1 updates]").await;
    assert!(
        woken
            .last_message
            .unwrap()
            .contains(&format!("### Agent {loud} \"say hi\"")),
        "only the default agent's news"
    );
    let inbox = fixture.client.staff_inbox(fixture.project_id).await.unwrap();
    assert_eq!(
        inbox.iter().map(|item| item.agent_id).collect::<Vec<_>>(),
        [Some(loud)]
    );

    // The chief still sees it, and which one is silent.
    let list = fixture.tool_json(staff.id, "workspaces_list", json!({})).await;
    let agents = list["workspaces"][0]["agents"].as_array().unwrap();
    let report_of = |id: i64| agents.iter().find(|agent| agent["id"] == json!(id)).unwrap()["report"].clone();
    assert_eq!(
        (report_of(silent), report_of(loud)),
        (json!(false), json!(true)),
        "{list}"
    );

    // An approval and an exit are no news either.
    fixture.client.say(silent, "ask:Bash ls".into()).await.unwrap();
    fixture.until_state(silent, AgentState::WaitingApproval).await;
    let approval = fixture
        .client
        .agent(silent)
        .await
        .unwrap()
        .pending_approval_id
        .unwrap();
    fixture
        .client
        .answer_approval(approval, false, None)
        .await
        .unwrap();
    fixture.until_says(silent, "denied").await;
    fixture.client.stop_agent(silent).await.unwrap();
    fixture.until_state(silent, AgentState::Exited).await;

    // It stays silent across a daemon restart and a resume.
    fixture.restart().await;
    fixture.staff.resume_chiefs().await.unwrap();
    fixture.until_state(staff.id, AgentState::Idle).await;
    assert!(!fixture.client.agent(silent).await.unwrap().report);
    let sent = fixture
        .tool(
            staff.id,
            "agent_send",
            json!({ "agent_id": silent, "message": "still quiet" }),
        )
        .await
        .unwrap();
    assert_eq!(sent, format!("Resumed agent {silent} and sent the message."));
    fixture.until_says(silent, "echo: still quiet").await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert_eq!(inbox_len(&fixture), 1);

    // agent_send can make it report again.
    let sent = fixture
        .tool(
            staff.id,
            "agent_send",
            json!({ "agent_id": silent, "message": "speak up", "report": true }),
        )
        .await
        .unwrap();
    assert_eq!(sent, format!("Sent to agent {silent}. It reports to you again."));
    fixture
        .until_says(
            staff.id,
            &format!("### Agent {silent} \"quiet job\" finished a turn\necho: speak up"),
        )
        .await;
    assert!(fixture.client.agent(silent).await.unwrap().report);
    assert_eq!(inbox_len(&fixture), 2);
}

#[tokio::test]
async fn your_messages_go_in_alone_and_before_news() {
    let fixture = Fixture::start().await;
    // News waiting before the chief ever started.
    for n in 1..=2 {
        fixture
            .books
            .add(
                fixture.project_id,
                StaffItemKind::Agent,
                None,
                &format!("News {n}\nbody {n}"),
            )
            .unwrap();
    }
    fixture
        .client
        .message_staff(fixture.project_id, "first, from you".into())
        .await
        .unwrap();
    let staff_id = fixture
        .client
        .staff(fixture.project_id)
        .await
        .unwrap()
        .agent_id
        .unwrap();
    let woken = fixture.until_says(staff_id, "[cos · 2 updates]").await;
    assert_eq!(
        woken
            .last_message
            .as_deref()
            .unwrap()
            .lines()
            .take(5)
            .collect::<Vec<_>>(),
        ["echo: [cos · 2 updates]", "", "### News 1", "body 1", ""]
    );
    let transcript = fixture.client.transcript(staff_id, 0).await.unwrap();
    let rows: Vec<(MessageKind, String)> = transcript
        .messages
        .iter()
        .map(|message| (message.kind, message.text.clone().unwrap_or_default()))
        .collect();
    assert_eq!(rows[0], (MessageKind::Text, "first, from you".to_owned()));
    assert_eq!(rows[1], (MessageKind::Text, "echo: first, from you".to_owned()));
    assert_eq!(rows[2].0, MessageKind::Update);

    // Past the hourly cap news waits, but your messages still go in.
    for _ in 0..cos_core::staff::MAX_WAKEUPS_PER_HOUR {
        fixture.books.deliver(fixture.project_id, staff_id, &[]).unwrap();
    }
    fixture
        .books
        .add(fixture.project_id, StaffItemKind::Agent, None, "Held back")
        .unwrap();
    assert_eq!(fixture.say(staff_id, "still here?").await, "echo: still here?");
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert_eq!(fixture.books.waiting(fixture.project_id).unwrap().len(), 1);
    let inbox = fixture.client.staff_inbox(fixture.project_id).await.unwrap();
    let texts: Vec<_> = inbox.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        ["Held back", "News 2\nbody 2", "News 1\nbody 1"],
        "newest first, no messages"
    );

    // Empty and unknown.
    assert_eq!(
        status(
            fixture
                .client
                .message_staff(fixture.project_id, "  ".into())
                .await
                .unwrap_err()
        ),
        400
    );
    assert_eq!(
        status(fixture.client.message_staff(99, "hi".into()).await.unwrap_err()),
        404
    );
    assert_eq!(status(fixture.client.staff(99).await.unwrap_err()), 404);
}

#[tokio::test]
async fn a_project_goes_with_its_chief_of_staff_once_it_is_stopped() {
    let fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    let worktree = fixture.client.staff(fixture.project_id).await.unwrap().worktree;
    fixture
        .client
        .remove_workspace(fixture.workspace_id)
        .await
        .unwrap();
    let running = fixture
        .client
        .remove_project(fixture.project_id)
        .await
        .unwrap_err();
    assert_eq!(status(running), 409);
    fixture.client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
    until("staff.stopped", || async {
        fixture
            .events(kind::STAFF_STOPPED)
            .into_iter()
            .find(|event| event.data == json!({ "project_id": fixture.project_id, "agent_id": staff.id }))
    })
    .await;
    fixture.client.remove_project(fixture.project_id).await.unwrap();
    assert!(!worktree.exists());
    assert!(fixture.client.projects().await.unwrap().is_empty());
    assert_eq!(status(fixture.client.agent(staff.id).await.unwrap_err()), 404);
}

#[tokio::test]
async fn the_model_comes_from_the_project_else_the_daemon() {
    let fixture = Fixture::start().await;
    let view = fixture.client.staff(fixture.project_id).await.unwrap();
    assert_eq!(view.model, None);
    let set = fixture
        .client
        .set_staff_model(fixture.project_id, Some(" sonnet ".into()))
        .await
        .unwrap();
    assert_eq!(set.model.as_deref(), Some("sonnet"));
    assert_eq!(
        fixture.events(kind::STAFF_CHANGED)[0].data,
        json!({ "project_id": fixture.project_id })
    );
    let staff = fixture.idle_staff().await;
    assert_eq!(staff.model.as_deref(), Some("sonnet"));
    let cleared = fixture
        .client
        .set_staff_model(fixture.project_id, None)
        .await
        .unwrap();
    assert_eq!(cleared.model, None);
    let _ = StaffSettings::default();
}

#[tokio::test]
async fn the_chief_of_staff_and_the_agents_it_spawns_get_the_project_env() {
    use crate::daemon::agents::tests::{home, launched_env, var};

    let fixture = Fixture::start().await;
    let client = &fixture.client;
    client
        .set_global_env("CLAUDE_CONFIG_DIR", "~/.claude".into())
        .await
        .unwrap();
    client
        .set_project_env(
            fixture.project_id,
            "CLAUDE_CONFIG_DIR",
            "~/.claude-personal".into(),
        )
        .await
        .unwrap();
    let expected = format!("{}/.claude-personal", home());

    let staff = fixture.idle_staff().await;
    let env = launched_env(fixture.dir.path(), &staff.session_id);
    assert_eq!(var(&env, "CLAUDE_CONFIG_DIR"), Some(expected.as_str()));
    assert_eq!(staff.env_names, ["CLAUDE_CONFIG_DIR"]);

    let spawn = json!({ "prompt": "say hi", "workspace_id": fixture.workspace_id });
    let spawned = fixture.say(staff.id, &format!("mcp agent_spawn {spawn}")).await;
    let agent_id = serde_json::from_str::<Value>(&spawned).unwrap()["agent_id"]
        .as_i64()
        .unwrap();
    let agent = fixture.until_says(agent_id, "echo: say hi").await;
    assert_eq!(agent.spawned_by, Some(staff.id));
    let env = launched_env(fixture.dir.path(), &agent.session_id);
    assert_eq!(var(&env, "CLAUDE_CONFIG_DIR"), Some(expected.as_str()));
}

#[tokio::test]
async fn paused_coordination_holds_news_but_not_your_messages() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let staff = fixture.idle_staff().await;
    client
        .set_staff_model(fixture.project_id, Some("sonnet".into()))
        .await
        .unwrap();
    let changed = fixture.events(kind::STAFF_CHANGED).len();

    let paused = client.set_staff_paused(fixture.project_id, true).await.unwrap();
    assert!(paused.paused);
    assert_eq!(
        paused.model.as_deref(),
        Some("sonnet"),
        "a pause leaves the model"
    );
    assert_eq!(fixture.events(kind::STAFF_CHANGED).len(), changed + 1);
    fixture
        .books
        .add(fixture.project_id, StaffItemKind::Agent, None, "News 1\nbody")
        .unwrap();

    // Your messages still go in; the news waits.
    assert_eq!(
        fixture.say(staff.id, "still listening?").await,
        "echo: still listening?"
    );
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let view = client.staff(fixture.project_id).await.unwrap();
    assert_eq!((view.paused, view.updates_waiting), (true, 1));
    assert_eq!(
        client.agent(staff.id).await.unwrap().last_message.as_deref(),
        Some("echo: still listening?")
    );

    // Resuming lets it in.
    let resumed = client.set_staff_paused(fixture.project_id, false).await.unwrap();
    assert!(!resumed.paused);
    fixture.until_says(staff.id, "[cos · 1 updates]").await;
    let view = client.staff(fixture.project_id).await.unwrap();
    assert_eq!((view.paused, view.updates_waiting), (false, 0));
    assert_eq!(view.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn chiefs_come_back_after_a_daemon_restart_unless_you_stopped_them() {
    let mut fixture = Fixture::start().await;
    let staff = fixture.idle_staff().await;
    assert_eq!(fixture.say(staff.id, "remember me").await, "echo: remember me");
    // Another project's chief, to see the second one wait its turn.
    let (other, _, mine) = fixture.other_project().await;
    let other_staff = fixture.client.start_staff(other).await.unwrap().agent_id.unwrap();
    fixture.until_state(other_staff, AgentState::Idle).await;
    fixture.until_state(mine, AgentState::Idle).await;
    let started = fixture.events(kind::STAFF_STARTED).len();

    fixture.restart().await;
    for id in [staff.id, other_staff, mine] {
        assert_eq!(
            fixture.client.agent(id).await.unwrap().state,
            AgentState::Interrupted
        );
    }
    fixture.staff.resume_chiefs().await.unwrap();

    let back = fixture.until_state(staff.id, AgentState::Idle).await;
    assert_eq!(back.session_id, staff.session_id, "the same Claude session");
    let transcript = fixture.client.transcript(staff.id, 0).await.unwrap();
    assert!(
        transcript
            .messages
            .iter()
            .any(|message| message.text.as_deref() == Some("remember me")),
        "{transcript:?}"
    );
    fixture.until_state(other_staff, AgentState::Idle).await;
    let restarts: Vec<_> = fixture.events(kind::STAFF_STARTED)[started..].to_vec();
    assert_eq!(restarts.len(), 2);
    let gap = restarts[1].ts.as_millisecond() - restarts[0].ts.as_millisecond();
    assert!(gap >= 90, "staggered, {gap} ms apart");
    // Your own agent stays interrupted, and `cos agent resume` brings it back.
    assert_eq!(
        fixture.client.agent(mine).await.unwrap().state,
        AgentState::Interrupted
    );
    fixture.client.resume_agent(mine).await.unwrap();
    fixture.until_state(mine, AgentState::Idle).await;
    fixture.client.stop_agent(mine).await.unwrap();
    fixture.until_state(mine, AgentState::Exited).await;

    // A chief you stop stays stopped, even if a crash left it interrupted.
    fixture.client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
    assert!(fixture.books.record(fixture.project_id).unwrap().stopped);
    fixture.client.stop_agent(other_staff).await.unwrap();
    fixture.until_state(other_staff, AgentState::Exited).await;
    // As if the daemon crashed with its record still running: the next
    // start marks it interrupted.
    fixture.records.restart(staff.id).unwrap();
    fixture.restart().await;
    assert_eq!(
        fixture.client.agent(staff.id).await.unwrap().state,
        AgentState::Interrupted
    );
    fixture.staff.resume_chiefs().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        fixture.client.agent(staff.id).await.unwrap().state,
        AgentState::Interrupted
    );
    assert_eq!(
        fixture.client.agent(other_staff).await.unwrap().state,
        AgentState::Exited
    );

    // Starting it again clears that.
    fixture.client.start_staff(fixture.project_id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Idle).await;
    assert!(!fixture.books.record(fixture.project_id).unwrap().stopped);
    fixture.client.stop_agent(staff.id).await.unwrap();
    fixture.until_state(staff.id, AgentState::Exited).await;
}

#[tokio::test]
async fn files_you_attach_reach_the_chief_and_the_agents_it_starts() {
    use crate::client::{DaemonError, Folder};

    let fixture = Fixture::start().await;
    let git_status = |dir: &std::path::Path| {
        let out = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    // Uploaded before the chief ever started: its worktree is made for it.
    let shot = fixture
        .client
        .upload(
            Folder::Staff(fixture.project_id),
            "Screen Shot.png",
            b"png bytes".to_vec(),
        )
        .await
        .unwrap();
    let view = fixture.client.staff(fixture.project_id).await.unwrap();
    assert_eq!(view.agent_id, None);
    assert_eq!(shot.name, "Screen-Shot.png");
    assert_eq!(shot.content_type, "image/png");
    assert_eq!(shot.size, 9);
    assert_eq!(shot.path, view.worktree.join(".cos/attachments/Screen-Shot.png"));
    assert_eq!(git_status(&view.worktree), "", "attachments stay out of git");
    let added = fixture.events(kind::ATTACHMENT_ADDED);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].data["name"], "Screen-Shot.png");

    // A message names it, by path, at its end.
    let staff = fixture.idle_staff().await;
    let missing = fixture
        .client
        .message_staff_with(fixture.project_id, "Look".into(), vec!["nope.png".into()])
        .await
        .unwrap_err();
    assert_eq!(missing.downcast_ref::<DaemonError>().unwrap().status, 404);
    fixture
        .client
        .message_staff_with(
            fixture.project_id,
            "Fix this layout".into(),
            vec![shot.name.clone()],
        )
        .await
        .unwrap();
    let echoed = fixture
        .until_says(staff.id, &shot.path.display().to_string())
        .await
        .last_message
        .unwrap();
    assert!(
        echoed.starts_with(
            "echo: Fix this layout\n\nAttached files (saved by cos; read them with the Read tool):\n- "
        ),
        "{echoed}"
    );
    assert!(echoed.ends_with("Screen-Shot.png (image/png, 9 B)"), "{echoed}");

    // The chat shows the text, and the file apart from it.
    let transcript = fixture.client.transcript(staff.id, 0).await.unwrap();
    let mine = transcript
        .messages
        .iter()
        .find(|message| !message.attachments.is_empty())
        .expect("a message with attachments");
    assert_eq!(mine.text.as_deref(), Some("Fix this layout"));
    assert_eq!(mine.attachments[0].name, "Screen-Shot.png");
    // And the app can fetch it to show it.
    let (content_type, bytes) = fixture
        .client
        .download(staff.id, "Screen-Shot.png")
        .await
        .unwrap();
    assert_eq!(
        (content_type.as_str(), bytes.as_slice()),
        ("image/png", &b"png bytes"[..])
    );
    assert!(fixture.client.download(staff.id, ".gitignore").await.is_err());

    // The chief hands it to an agent it starts: copied into the workspace.
    let workspace = fixture.projects.workspace(fixture.workspace_id).unwrap();
    let copy = workspace.path.join(".cos/attachments/Screen-Shot.png");
    let spawn = json!({
        "prompt": "use the screenshot",
        "workspace_id": fixture.workspace_id,
        "attachments": [shot.path],
    });
    let spawned = fixture.say(staff.id, &format!("mcp agent_spawn {spawn}")).await;
    let agent_id = serde_json::from_str::<Value>(&spawned).unwrap()["agent_id"]
        .as_i64()
        .unwrap();
    fixture.until_says(agent_id, &copy.display().to_string()).await;
    assert_eq!(std::fs::read(&copy).unwrap(), b"png bytes");
    assert_eq!(git_status(&workspace.path), "");

    // Sending it again reuses the copy rather than making another.
    let send = json!({ "agent_id": agent_id, "message": "again", "attachments": ["Screen-Shot.png"] });
    fixture.say(staff.id, &format!("mcp agent_send {send}")).await;
    let again = fixture
        .until_says(agent_id, "echo: again")
        .await
        .last_message
        .unwrap();
    assert!(again.contains(&copy.display().to_string()), "{again}");
    let names: Vec<_> = std::fs::read_dir(workspace.path.join(".cos/attachments"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    assert_eq!(names, ["Screen-Shot.png"]);

    // Straight to an agent, and to a new agent in a workspace.
    let log = fixture
        .client
        .upload(Folder::Agent(agent_id), "build log", b"error: oops".to_vec())
        .await
        .unwrap();
    assert_eq!(log.name, "build-log");
    fixture
        .client
        .say_with(agent_id, "why did this fail?".into(), vec![log.name.clone()])
        .await
        .unwrap();
    fixture
        .until_says(
            agent_id,
            &workspace
                .path
                .join(".cos/attachments/build-log")
                .display()
                .to_string(),
        )
        .await;
    let notes = fixture
        .client
        .upload(
            Folder::Workspace(fixture.workspace_id),
            "notes.md",
            b"# notes".to_vec(),
        )
        .await
        .unwrap();
    let started = fixture
        .client
        .start_agent(
            fixture.workspace_id,
            &crate::daemon::agents::StartAgent {
                attachments: vec![notes.name.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(started.title, "notes.md");
    fixture
        .until_says(started.id, "notes.md (text/markdown, 7 B)")
        .await;
}
