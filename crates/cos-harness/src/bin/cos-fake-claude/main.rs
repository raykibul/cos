//! A stand-in for `claude -p --input-format stream-json ...`, used by the
//! cos-harness integration tests so they need no login and spend no tokens.
//!
//! What it does depends on the user message:
//! - contains `permission`: calls Bash, asks the host, and reports the answer
//! - contains `cancel`: asks the host, then withdraws the request
//! - contains `hook`: sends a control request cos doesn't support
//! - anything else: replies `echo: <message>`
//!
//! With `COS_FAKE_CLAUDE_FAIL` set it prints to stderr and exits 2 at startup.
//! It exits 3 if a flag the real CLI needs to route prompts to cos is missing.
//!
//! Without `-p` it plays the interactive UI instead (see [`interactive`]), and
//! `cos-fake-claude hook` is the hook relay, like `cos hook`.

mod interactive;
mod mcp;
mod script;

use std::io::{BufRead, Write};

use serde_json::{Value, json};

const REQUIRED_FLAGS: [(&str, &str); 3] = [
    ("--input-format", "stream-json"),
    ("--output-format", "stream-json"),
    ("--permission-prompt-tool", "stdio"),
];

fn main() {
    if std::env::var_os("COS_FAKE_CLAUDE_FAIL").is_some() {
        eprintln!("fake-claude: not logged in");
        std::process::exit(2);
    }

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("hook") {
        let socket = std::env::var_os(cos_harness::claude::hooks::SOCKET_ENV).map(std::path::PathBuf::from);
        cos_harness::claude::hooks::relay(socket.as_deref(), &mut std::io::stdin(), &mut std::io::stdout());
        return;
    }
    if !args.iter().any(|arg| arg == "-p") {
        interactive::run(&args);
        return;
    }
    for (flag, value) in REQUIRED_FLAGS {
        if !args.windows(2).any(|pair| pair[0] == flag && pair[1] == value) {
            eprintln!("fake-claude: missing {flag} {value}");
            std::process::exit(3);
        }
    }
    let session_id = args
        .windows(2)
        .find(|pair| pair[0] == "--session-id" || pair[0] == "--resume")
        .map_or("fake-session".to_owned(), |pair| pair[1].clone());

    let stdin = std::io::stdin();
    let mut input = stdin.lock().lines().map_while(Result::ok);

    while let Some(line) = input.next() {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match message["type"].as_str() {
            Some("control_request") => {
                let request_id = message["request_id"].as_str().unwrap_or_default();
                send(json!({
                    "type": "control_response",
                    "response": { "subtype": "success", "request_id": request_id, "response": {} },
                }));
                if message["request"]["subtype"] == "initialize" {
                    send(json!({
                        "type": "system", "subtype": "init", "session_id": session_id,
                        "model": "fake-model", "permissionMode": "default", "tools": ["Bash"],
                    }));
                }
            }
            Some("user") => {
                let text = message["message"]["content"].as_str().unwrap_or_default();
                if text.contains("permission") {
                    permission_turn(&mut input);
                } else if text.contains("cancel") {
                    cancel_turn();
                } else if text.contains("hook") {
                    hook_turn(&mut input);
                } else {
                    say(&format!("echo: {text}"));
                    finish(false, &[]);
                }
            }
            _ => {}
        }
    }
}

fn permission_turn(input: &mut impl Iterator<Item = String>) {
    let command = json!({ "command": "touch x" });
    send(json!({
        "type": "assistant",
        "message": { "role": "assistant", "content": [
            { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": command },
        ] },
    }));
    send(json!({
        "type": "control_request", "request_id": "perm-1",
        "request": {
            "subtype": "can_use_tool", "tool_name": "Bash", "input": command,
            "tool_use_id": "toolu_1", "description": "Create x", "blocked_path": "/work/x",
        },
    }));

    let answer = wait_for_response(input, "perm-1");
    let answer = &answer["response"];
    if answer["behavior"] == "allow" && answer["updatedInput"] == command {
        tool_result("ran: touch x", false);
        say("allowed");
        finish(false, &[]);
    } else {
        tool_result(answer["message"].as_str().unwrap_or("denied"), true);
        say("denied");
        finish(false, &["Bash"]);
    }
}

fn cancel_turn() {
    send(json!({
        "type": "control_request", "request_id": "perm-2",
        "request": { "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "sleep 1" } },
    }));
    send(json!({ "type": "control_cancel_request", "request_id": "perm-2" }));
    finish(true, &[]);
}

fn hook_turn(input: &mut impl Iterator<Item = String>) {
    send(json!({
        "type": "control_request", "request_id": "hook-1",
        "request": { "subtype": "hook_callback", "callback_id": "hook_0", "input": {} },
    }));
    let response = wait_for_response(input, "hook-1");
    say(if response["subtype"] == "error" {
        "hook refused"
    } else {
        "hook accepted"
    });
    finish(false, &[]);
}

/// Reads stdin until the control_response for `request_id` arrives.
fn wait_for_response(input: &mut impl Iterator<Item = String>, request_id: &str) -> Value {
    for line in input {
        if let Ok(message) = serde_json::from_str::<Value>(&line)
            && message["type"] == "control_response"
            && message["response"]["request_id"] == request_id
        {
            return message["response"].clone();
        }
    }
    eprintln!("fake-claude: input closed while waiting for {request_id}");
    std::process::exit(4);
}

fn say(text: &str) {
    send(json!({
        "type": "assistant",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
    }));
}

fn tool_result(content: &str, is_error: bool) {
    send(json!({
        "type": "user",
        "message": { "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": content, "is_error": is_error },
        ] },
    }));
}

fn finish(is_error: bool, denied: &[&str]) {
    let denials: Vec<Value> = denied.iter().map(|tool| json!({ "tool_name": tool })).collect();
    send(json!({
        "type": "result",
        "subtype": if is_error { "error_during_execution" } else { "success" },
        "is_error": is_error, "result": "", "total_cost_usd": 0.0, "duration_ms": 1,
        "permission_denials": denials,
    }));
}

fn send(message: Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}
