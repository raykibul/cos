//! The MCP bridge spoken to over an in-memory pipe, the way Claude speaks to
//! it over stdio, against a stub daemon on a Unix socket that stands in for
//! the daemon's tool route. The real daemon is exercised through a fake
//! Chief of Staff in `daemon::staff::tests`.

use std::time::Duration;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};
use tokio::net::UnixListener;

use super::*;

fn stub() -> Router {
    Router::new().route(
        "/v1/agents/{id}/tools/{name}",
        post(
            |Path((id, name)): Path<(i64, String)>, Json(body): Json<Value>| async move {
                match name.as_str() {
                    "workspace_create" => (
                        StatusCode::CONFLICT,
                        Json(json!({ "error": "the repo folder is gone" })),
                    ),
                    _ => (
                        StatusCode::OK,
                        Json(json!({ "text": format!("{name} for agent {id} with {body}") })),
                    ),
                }
            },
        ),
    )
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
async fn connect() -> (McpClient, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("cos.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let app = stub();
    tokio::spawn(async move { axum::serve(listener, app).await });

    let (server, client) = tokio::io::duplex(1 << 16);
    let bridge = Bridge {
        client: Arc::new(Client::new(socket)),
        agent: 1,
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
    (client, dir)
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn initialize_list_and_call_round_trip() {
    let (mut client, _dir) = connect().await;

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
            "brief_update"
        ]
    );

    let result = client.call("agent_read", json!({ "agent_id": 2 })).await;
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(text(&result), "agent_read for agent 1 with {\"agent_id\":2}");

    // The daemon's refusal reaches the chief as a tool error, word for word.
    let result = client.call("workspace_create", json!({})).await;
    assert_eq!(result["isError"], json!(true), "{result}");
    assert_eq!(text(&result), "the repo folder is gone");

    // An unknown tool, a run's included, is a protocol error.
    for name in ["rm_rf", "run_start", "approval_decide"] {
        let reply = client
            .request("tools/call", json!({ "name": name, "arguments": {} }))
            .await;
        assert!(reply["error"].is_object(), "{reply}");
    }
}

#[test]
fn every_tool_has_a_description_and_an_object_schema() {
    for tool in tools() {
        assert!(
            tool.description.as_deref().is_some_and(|text| text.len() > 40),
            "{}",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert_eq!(tool.input_schema["additionalProperties"], false, "{}", tool.name);
    }
}
