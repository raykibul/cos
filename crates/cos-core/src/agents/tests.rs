use super::*;

/// Agents on an in-memory store with one project and workspace (id 1).
fn agents() -> (Agents, EventLog, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store
        .lock()
        .execute_batch(
            "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/r', 'r', 'main', 0);
             INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
             VALUES (1, 'lisbon', 'lisbon', 'cos/lisbon', '/w', 'head', 'main', 'abc', 0);",
        )
        .unwrap();
    let log = EventLog::new(store.clone());
    (Agents::new(store.clone(), log.clone()), log, store)
}

fn new_agent(title: Option<&str>) -> NewAgent {
    NewAgent {
        workspace_id: 1,
        run_id: None,
        role: Role::User,
        title: title.map(str::to_owned),
        session_id: uuid_like(),
        model: Some("haiku".into()),
        permission_mode: "default".into(),
        user_settings: true,
        rows: 40,
        cols: 120,
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("session-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn kinds(log: &EventLog) -> Vec<String> {
    log.after(0, 100)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

#[test]
fn an_agent_goes_from_starting_to_exited() {
    let (agents, log, _) = agents();
    let agent = agents.create(new_agent(None)).unwrap();
    assert_eq!(agent.title, format!("Agent {}", agent.id));
    assert_eq!(agent.state, AgentState::Starting);
    assert!(
        kinds(&log).is_empty(),
        "nothing is logged before the process is up"
    );

    let agent = agents.started(agent.id, Some(42)).unwrap();
    assert_eq!(agent.pid, Some(42));
    assert_eq!(
        agents.set_state(agent.id, AgentState::Working).unwrap(),
        Some(AgentState::Starting)
    );
    assert_eq!(agents.set_state(agent.id, AgentState::Working).unwrap(), None);
    agents.set_last_message(agent.id, &"x".repeat(3_000)).unwrap();
    agents.finish(agent.id, AgentState::Exited, Some(0)).unwrap();
    // A finished agent ignores late hooks.
    assert_eq!(agents.set_state(agent.id, AgentState::Idle).unwrap(), None);

    let agent = agents.get(agent.id).unwrap().unwrap();
    assert_eq!(agent.state, AgentState::Exited);
    assert_eq!(agent.exit_code, Some(0));
    assert!(agent.exited_ms.is_some());
    assert_eq!(agent.last_message.unwrap().len(), MAX_LAST_MESSAGE_CHARS);
    assert_eq!(
        kinds(&log),
        [
            kind::AGENT_STARTED,
            kind::AGENT_STATE_CHANGED,
            kind::AGENT_STATE_CHANGED,
            kind::AGENT_EXITED
        ]
    );
    let changes: Vec<Value> = log
        .after(0, 100)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind::AGENT_STATE_CHANGED)
        .map(|event| event.data)
        .collect();
    assert_eq!(
        changes,
        [
            json!({ "agent_id": agent.id, "state": "working", "from": "starting" }),
            json!({ "agent_id": agent.id, "state": "exited", "from": "working" }),
        ]
    );
}

#[test]
fn running_agents_are_interrupted_and_can_restart() {
    let (agents, _, _) = agents();
    let running = agents.create(new_agent(Some("a"))).unwrap();
    let done = agents.create(new_agent(Some("b"))).unwrap();
    agents.finish(done.id, AgentState::Exited, None).unwrap();

    assert_eq!(agents.interrupt_running().unwrap(), [running.id]);
    assert_eq!(
        agents.get(running.id).unwrap().unwrap().state,
        AgentState::Interrupted
    );

    let again = agents.restart(running.id).unwrap();
    assert_eq!(again.state, AgentState::Starting);
    assert_eq!(again.session_id, running.session_id);
    assert_eq!(again.exited_ms, None);
}

#[test]
fn lists_newest_first_with_filters() {
    let (agents, _, _) = agents();
    let first = agents.create(new_agent(Some("first"))).unwrap();
    let second = agents.create(new_agent(Some("second"))).unwrap();
    agents.set_state(second.id, AgentState::Idle).unwrap();

    let ids = |list: Vec<Agent>| list.into_iter().map(|agent| agent.id).collect::<Vec<_>>();
    assert_eq!(ids(agents.list(None, None).unwrap()), [second.id, first.id]);
    assert_eq!(
        ids(agents.list(Some(1), Some(AgentState::Idle)).unwrap()),
        [second.id]
    );
    assert!(agents.list(Some(2), None).unwrap().is_empty());
}

#[test]
fn approvals_are_decided_once() {
    let (agents, log, _) = agents();
    let agent = agents.create(new_agent(None)).unwrap();
    let approval = agents
        .add_approval(NewApproval {
            agent_id: agent.id,
            workspace_id: 1,
            tool_name: "Bash".into(),
            tool_input: json!({ "command": "touch x" }),
            tool_use_id: Some("toolu_1".into()),
            summary: "Bash touch x".into(),
            ..NewApproval::default()
        })
        .unwrap();
    assert_eq!(approval.state, ApprovalState::Pending);
    assert_eq!(
        agents.get(agent.id).unwrap().unwrap().pending_approval_id,
        Some(approval.id)
    );
    assert_eq!(agents.pending_approvals_of(agent.id).unwrap(), [approval.id]);

    let denied = agents
        .decide_approval(
            approval.id,
            ApprovalState::Denied,
            Some(DecidedBy::User),
            Some("no"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        (denied.state, denied.decided_by, denied.message.as_deref()),
        (ApprovalState::Denied, Some(DecidedBy::User), Some("no"))
    );
    assert_eq!(denied.tool_input, json!({ "command": "touch x" }));
    assert!(
        agents
            .decide_approval(approval.id, ApprovalState::Allowed, Some(DecidedBy::User), None)
            .unwrap()
            .is_none()
    );
    assert_eq!(agents.get(agent.id).unwrap().unwrap().pending_approval_id, None);
    assert!(agents.approvals(Some(ApprovalState::Pending)).unwrap().is_empty());
    assert_eq!(agents.approvals(None).unwrap().len(), 1);

    let events = log.after(0, 100).unwrap();
    assert_eq!(
        events.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(),
        [kind::APPROVAL_REQUESTED, kind::APPROVAL_DECIDED]
    );
    assert_eq!(
        events[1].data,
        json!({ "approval_id": approval.id, "state": "denied", "decided_by": "user" })
    );
}

#[test]
fn removing_an_agent_or_its_workspace_drops_its_approvals() {
    let (agents, log, store) = agents();
    let agent = agents.create(new_agent(None)).unwrap();
    let approval = |agent_id| NewApproval {
        agent_id,
        workspace_id: 1,
        tool_name: "Write".into(),
        tool_input: json!({}),
        tool_use_id: None,
        summary: "Write".into(),
        ..NewApproval::default()
    };
    agents.add_approval(approval(agent.id)).unwrap();

    agents.remove(agent.id).unwrap();
    assert!(agents.get(agent.id).unwrap().is_none());
    assert!(agents.approvals(None).unwrap().is_empty());
    assert_eq!(kinds(&log).last().unwrap(), kind::AGENT_REMOVED);

    let other = agents.create(new_agent(None)).unwrap();
    agents.add_approval(approval(other.id)).unwrap();
    store
        .lock()
        .execute("DELETE FROM workspaces WHERE id = 1", [])
        .unwrap();
    assert!(agents.list(None, None).unwrap().is_empty());
    assert!(agents.approvals(None).unwrap().is_empty());
}

#[test]
fn states_parse_from_their_names() {
    assert_eq!("waiting_approval".parse(), Ok(AgentState::WaitingApproval));
    assert!("busy".parse::<AgentState>().is_err());
    assert_eq!("settled".parse(), Ok(ApprovalState::Settled));
}

#[test]
fn approvals_carry_their_tier_and_route() {
    let (agents, log, _) = agents();
    let agent = agents.create(new_agent(None)).unwrap();
    let new = |route| NewApproval {
        agent_id: agent.id,
        workspace_id: 1,
        tool_name: "Bash".into(),
        summary: "Bash npm install".into(),
        tier: Some(ApprovalTier::Chief),
        route: Some(route),
        reason: Some("installs packages".into()),
        ..NewApproval::default()
    };

    let ruled = agents.add_ruled_approval(new(ApprovalRoute::User)).unwrap();
    assert_eq!(
        (ruled.state, ruled.decided_by, ruled.route),
        (
            ApprovalState::Allowed,
            Some(DecidedBy::Rule),
            Some(ApprovalRoute::Auto)
        )
    );
    assert!(ruled.decided_ms.is_some());
    assert_eq!(kinds(&log), [kind::APPROVAL_DECIDED], "nobody is asked");

    let chief = agents.add_approval(new(ApprovalRoute::Chief)).unwrap();
    assert_eq!(
        (chief.tier, chief.route, chief.reason.as_deref()),
        (
            Some(ApprovalTier::Chief),
            Some(ApprovalRoute::Chief),
            Some("installs packages")
        )
    );
    let rerouted = agents
        .route_approval(chief.id, ApprovalRoute::User, Some("the chief may not"))
        .unwrap()
        .unwrap();
    assert_eq!(rerouted.route, Some(ApprovalRoute::User));
    assert_eq!(rerouted.reason.as_deref(), Some("the chief may not"));
    let events = log.after(0, 100).unwrap();
    assert_eq!(
        events.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(),
        [
            kind::APPROVAL_DECIDED,
            kind::APPROVAL_REQUESTED,
            kind::APPROVAL_ROUTED,
            kind::APPROVAL_ROUTED
        ]
    );
    assert_eq!(events[2].data["route"], "chief");
    assert_eq!(events[3].data["route"], "user");
    assert_eq!(events[3].data["tier"], "chief");

    agents
        .decide_approval(chief.id, ApprovalState::Denied, Some(DecidedBy::User), None)
        .unwrap();
    assert!(
        agents
            .route_approval(chief.id, ApprovalRoute::Chief, None)
            .unwrap()
            .is_none()
    );
    let json = serde_json::to_value(agents.approval(chief.id).unwrap().unwrap()).unwrap();
    assert_eq!((&json["tier"], &json["route"]), (&json!("chief"), &json!("user")));
}
