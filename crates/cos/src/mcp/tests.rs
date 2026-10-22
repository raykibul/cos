//! The MCP bridge spoken to over an in-memory pipe, the way Claude speaks to
//! it over stdio, against a stub daemon on a Unix socket. The stub stands in
//! for the daemon's tool route and for the approvals component's
//! `POST /v1/approvals/{id}/chief`. The real daemon is exercised through a
//! fake chief in `daemon::chief::tests`.

use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};
use tokio::net::UnixListener;

use super::*;

/// What the stub saw: decisions sent to the approvals route.
#[derive(Default)]
struct Seen {
    decisions: Mutex<Vec<(i64, Value)>>,
}

fn agent(id: i64, run_id: i64) -> Value {
    json!({
        "id": id, "workspace_id": 1, "run_id": run_id, "role": if id == 1 { "chief" } else { "worker" },
        "title": format!("Agent {id}"), "session_id": format!("s{id}"), "pid": null, "state": "working",
        "model": null, "permission_mode": "default", "user_settings": false, "rows": 40, "cols": 120,
        "started_ms": 0, "last_activity_ms": 0, "exited_ms": null, "exit_code": null,
        "last_message": null, "pending_approval_id": null
    })
}

fn approval(id: i64, agent_id: i64, state: &str) -> Value {
    json!({
        "id": id, "agent_id": agent_id, "workspace_id": 1, "tool_name": "Bash",
        "tool_input": {"command": "npm install left-pad"}, "tool_use_id": null,
        "summary": "Bash npm install left-pad", "state": state, "decided_by": null, "message": null,
        "created_ms": 0, "decided_ms": null
    })
}

type Reply = (StatusCode, Json<Value>);

fn stub(seen: Arc<Seen>) -> Router {
    Router::new()
        .route(
            "/v1/agents/{id}/tools/{name}",
            post(
                |Path((id, name)): Path<(i64, String)>, Json(body): Json<Value>| async move {
                    match name.as_str() {
                        "board_propose" => (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "error": "The plan was not accepted.\n- task 1 has no title" })),
                        ),
                        _ => (
                            StatusCode::OK,
                            Json(json!({ "text": format!("{name} for agent {id} with {body}") })),
                        ),
                    }
                },
            ),
        )
        // Agent 1 is the chief of run 5, agent 2 a worker of run 5, agent 3 of run 6.
        .route(
            "/v1/agents/{id}",
            get(|Path(id): Path<i64>| async move {
                (StatusCode::OK, Json(agent(id, if id == 3 { 6 } else { 5 })))
            }),
        )
        // Approval 10 is from agent 2, 11 from agent 3.
        .route(
            "/v1/approvals/{id}",
            get(|Path(id): Path<i64>| async move {
                (
                    StatusCode::OK,
                    Json(approval(id, if id == 11 { 3 } else { 2 }, "pending")),
                )
            }),
        )
        .route(
            "/v1/approvals/{id}/chief",
            post(
                |State(seen): State<Arc<Seen>>, Path(id): Path<i64>, Json(body): Json<Value>| async move {
                    seen.decisions.lock().unwrap().push((id, body.clone()));
                    let reply: Reply = if body["reason"].as_str() == Some("over the cap") {
                        (
                            StatusCode::FORBIDDEN,
                            Json(json!({ "error": "the chief's approval cap for this run is used up" })),
                        )
                    } else {
                        let state = if body["decision"] == "allow" {
                            "allowed"
                        } else {
                            "denied"
                        };
                        (StatusCode::OK, Json(approval(id, 2, state)))
                    };
                    reply
                },
            ),
        )
        .with_state(seen)
}

/// Claude's side of the pipe.
struct McpClient {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
    next_id: i64,
}

impl McpClient {
    async fn send(&mut self, message: Value) {
        self.writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
                .await
                .expect("the server answers")
                .unwrap()
                .expect("the server keeps the pipe open");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let reply = self
            .request("tools/call", json!({ "name": tool, "arguments": arguments }))
            .await;
        reply["result"].clone()
    }
}

/// A bridge for agent 1 on a stub daemon, initialized.
async fn connect() -> (McpClient, Arc<Seen>, tempfile::TempDir) {
    connect_as(false).await
}

/// [`connect`], as a Chief of Staff's bridge if `staff`.
async fn connect_as(staff: bool) -> (McpClient, Arc<Seen>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("cos.sock");
    let seen = Arc::new(Seen::default());
    let listener = UnixListener::bind(&socket).unwrap();
    let app = stub(seen.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let (server, client) = tokio::io::duplex(1 << 16);
    let bridge = Bridge {
        client: Arc::new(Client::new(socket)),
        agent: 1,
        staff,
    };
    tokio::spawn(async move {
        let (read, write) = tokio::io::split(server);
        if let Ok(running) = bridge.serve((read, write)).await {
            let _ = running.waiting().await;
        }
    });
    let (read, writer) = tokio::io::split(client);
    let mut client = McpClient {
        lines: BufReader::new(read).lines(),
        writer,
        next_id: 0,
    };
    let init = client
        .request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }),
        )
        .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "cos", "{init}");
    assert!(init["result"]["capabilities"]["tools"].is_object(), "{init}");
    client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    (client, seen, dir)
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn initialize_list_and_call_round_trip() {
    let (mut client, _, _dir) = connect().await;

    let list = client.request("tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "board_get",
            "board_propose",
            "task_update",
            "task_send",
            "ask_user",
            "approval_decide",
            "report_write"
        ]
    );

    let result = client.call("board_get", json!({})).await;
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(text(&result), "board_get for agent 1 with {}");

    // The daemon's refusal reaches the chief as a tool error, word for word.
    let result = client.call("board_propose", json!({ "tasks": [] })).await;
    assert_eq!(result["isError"], json!(true), "{result}");
    assert_eq!(text(&result), "The plan was not accepted.\n- task 1 has no title");

    // An unknown tool is a protocol error.
    let reply = client
        .request("tools/call", json!({ "name": "rm_rf", "arguments": {} }))
        .await;
    assert!(reply["error"].is_object(), "{reply}");
}

#[tokio::test]
async fn a_chief_of_staff_gets_its_own_tools() {
    let (mut client, seen, _dir) = connect_as(true).await;
    let list = client.request("tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "workspaces_list",
            "workspace_create",
            "agent_spawn",
            "agent_send",
            "agent_read",
            "agent_stop",
            "run_start",
            "brief_update"
        ]
    );
    let result = client.call("agent_read", json!({ "agent_id": 2 })).await;
    assert_eq!(text(&result), "agent_read for agent 1 with {\"agent_id\":2}");
    // A run chief's tools aren't there.
    let reply = client
        .request(
            "tools/call",
            json!({ "name": "approval_decide", "arguments": {} }),
        )
        .await;
    assert!(reply["error"].is_object(), "{reply}");
    assert!(seen.decisions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn approval_decide_goes_to_the_approvals_route_for_this_run_only() {
    let (mut client, seen, _dir) = connect().await;

    let result = client
        .call(
            "approval_decide",
            json!({ "approval_id": 10, "decision": "allow", "reason": "the task installs it" }),
        )
        .await;
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(
        text(&result),
        "Approval 10 (Bash npm install left-pad) is now allowed."
    );
    assert_eq!(
        seen.decisions.lock().unwrap().clone(),
        [(
            10,
            json!({ "decision": "allow", "reason": "the task installs it" })
        )]
    );

    // From another run: refused before it reaches the approvals route.
    let result = client
        .call(
            "approval_decide",
            json!({ "approval_id": 11, "decision": "allow", "reason": "x" }),
        )
        .await;
    assert_eq!(result["isError"], json!(true), "{result}");
    assert!(text(&result).contains("not from your run"), "{result}");

    // The limits refuse it: the chief reads why.
    let result = client
        .call(
            "approval_decide",
            json!({ "approval_id": 10, "decision": "allow", "reason": "over the cap" }),
        )
        .await;
    assert_eq!(result["isError"], json!(true), "{result}");
    assert!(text(&result).contains("cap"), "{result}");

    // Bad arguments.
    for arguments in [
        json!({ "approval_id": 10, "decision": "maybe", "reason": "x" }),
        json!({ "approval_id": 10, "decision": "deny", "reason": " " }),
        json!({ "decision": "deny", "reason": "x" }),
    ] {
        let result = client.call("approval_decide", arguments).await;
        assert_eq!(result["isError"], json!(true), "{result}");
    }
    assert_eq!(seen.decisions.lock().unwrap().len(), 2);
}

#[test]
fn every_tool_has_a_description_and_an_object_schema() {
    for tool in tools().into_iter().chain(staff_tools()) {
        assert!(
            tool.description.as_deref().is_some_and(|text| text.len() > 40),
            "{}",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert_eq!(tool.input_schema["additionalProperties"], false, "{}", tool.name);
    }
    let propose = tools()
        .into_iter()
        .find(|tool| tool.name == "board_propose")
        .unwrap();
    assert_eq!(
        propose.input_schema["properties"]["tasks"]["items"]["required"],
        json!(["title", "instructions", "files"])
    );
}
