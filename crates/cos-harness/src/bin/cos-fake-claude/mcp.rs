//! A tiny MCP client for the fake's interactive mode, so a fake chief can
//! call the tools of the server named in `--mcp-config` the way Claude does:
//! start the server, `initialize`, then `tools/list` or `tools/call` over
//! stdio, one JSON-RPC message per line.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// The first server in an `--mcp-config` value (inline JSON).
pub(crate) struct Server {
    pub name: String,
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl Server {
    pub(crate) fn from_config(config: &str) -> Option<Self> {
        let config: Value = serde_json::from_str(config).ok()?;
        let (name, server) = config["mcpServers"].as_object()?.iter().next()?;
        Some(Self {
            name: name.clone(),
            command: server["command"].as_str()?.to_owned(),
            args: server["args"]
                .as_array()
                .map(|args| {
                    args.iter()
                        .filter_map(|arg| arg.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            env: server["env"]
                .as_object()
                .map(|env| {
                    env.iter()
                        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// The tools' names, comma-separated.
    pub(crate) fn list(&self) -> Result<String, String> {
        let result = self.exchange("tools/list", json!({}))?;
        let names: Vec<&str> = result["tools"]
            .as_array()
            .map(|tools| tools.iter().filter_map(|tool| tool["name"].as_str()).collect())
            .unwrap_or_default();
        Ok(names.join(","))
    }

    /// Calls a tool and returns its text, starting `error: ` if the tool failed.
    pub(crate) fn call(&self, tool: &str, arguments: Value) -> Result<String, String> {
        let result = self.exchange("tools/call", json!({ "name": tool, "arguments": arguments }))?;
        let text: Vec<&str> = result["content"]
            .as_array()
            .map(|content| {
                content
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let text = text.join("\n");
        Ok(if result["isError"] == json!(true) {
            format!("error: {text}")
        } else {
            text
        })
    }

    /// Starts the server, initializes, sends one request and returns its result.
    fn exchange(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", self.command))?;
        let result = talk(&mut child, method, params);
        drop(child.stdin.take());
        let _ = child.wait();
        result
    }
}

fn talk(child: &mut Child, method: &str, params: Value) -> Result<Value, String> {
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "cos-fake-claude", "version": "0" }
            }
        }),
    )?;
    let init = response(&mut stdout, 1)?;
    if init.get("error").is_some() {
        return Err(format!("initialize failed: {init}"));
    }
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )?;
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": method, "params": params }),
    )?;
    let reply = response(&mut stdout, 2)?;
    child.stdin = Some(stdin);
    match reply.get("result") {
        Some(result) => Ok(result.clone()),
        None => Err(format!("{method} failed: {}", reply["error"])),
    }
}

fn send(stdin: &mut ChildStdin, message: Value) -> Result<(), String> {
    writeln!(stdin, "{message}")
        .and_then(|()| stdin.flush())
        .map_err(|error| error.to_string())
}

/// Reads messages until the response with `id`.
fn response(stdout: &mut BufReader<ChildStdout>, id: i64) -> Result<Value, String> {
    let mut line = String::new();
    loop {
        line.clear();
        if stdout.read_line(&mut line).map_err(|error| error.to_string())? == 0 {
            return Err("the MCP server closed its output".into());
        }
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if message["id"] == json!(id) {
            return Ok(message);
        }
    }
}
