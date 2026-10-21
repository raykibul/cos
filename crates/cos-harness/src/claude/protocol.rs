//! The JSON lines Claude Code reads and writes when started with
//! `-p --input-format stream-json --output-format stream-json --permission-prompt-tool stdio`.
//!
//! This is the protocol the Agent SDKs speak, but it isn't a documented
//! contract, so parsing is lenient: anything unexpected becomes
//! [`Event::Other`] rather than an error. Captured examples and notes are in
//! `docs/claude-protocol.md`.

use serde_json::{Value, json};

use crate::{Decision, Event, PermissionRequest, TurnResult};

// ---- Lines cos writes to Claude's stdin ----

pub(crate) fn user_message(text: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": text } })
}

pub(crate) fn control_request(request_id: &str, subtype: &str) -> Value {
    json!({ "type": "control_request", "request_id": request_id, "request": { "subtype": subtype } })
}

pub(crate) fn control_success(request_id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": response },
    })
}

pub(crate) fn control_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
}

/// The answer to a `can_use_tool` request. Claude requires `updatedInput` on
/// an allow, so the original input is sent back unchanged.
pub(crate) fn permission_answer(request: &PermissionRequest, decision: &Decision) -> Value {
    let mut answer = match decision {
        Decision::Allow => json!({ "behavior": "allow", "updatedInput": request.input }),
        Decision::Deny { message } => json!({ "behavior": "deny", "message": message }),
    };
    if let Some(id) = &request.tool_use_id {
        answer["toolUseID"] = json!(id);
    }
    control_success(&request.request_id, answer)
}

// ---- Lines Claude writes to stdout ----

/// One stdout line, sorted by what the session has to do with it.
#[derive(Debug, PartialEq)]
pub(crate) enum Incoming {
    /// Plain output, already turned into events. Can be empty.
    Events(Vec<Event>),
    /// Claude is asking cos something and waits for a `control_response`.
    ControlRequest {
        request_id: String,
        subtype: String,
        body: Value,
    },
    /// Claude's answer to a `control_request` cos sent.
    ControlResponse {
        request_id: String,
        result: Result<Value, String>,
    },
    /// Claude withdrew one of its own control requests.
    ControlCancel {
        request_id: String,
    },
    KeepAlive,
}

pub(crate) fn parse_line(line: &str) -> Incoming {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Incoming::Events(vec![Event::Other(Value::String(line.to_owned()))]);
    };
    match (str_at(&value, "/type"), str_at(&value, "/subtype")) {
        (Some("system"), Some("init")) => Incoming::Events(vec![Event::Started {
            session_id: string_at(&value, "/session_id"),
            model: string_at(&value, "/model"),
            permission_mode: string_at(&value, "/permissionMode"),
        }]),
        (Some("assistant"), _) => Incoming::Events(assistant_events(&value)),
        (Some("user"), _) => Incoming::Events(tool_results(&value)),
        (Some("result"), _) => Incoming::Events(vec![Event::TurnFinished(turn_result(&value))]),
        (Some("control_request"), _) => Incoming::ControlRequest {
            request_id: string_at(&value, "/request_id"),
            subtype: string_at(&value, "/request/subtype"),
            body: value.get("request").cloned().unwrap_or(Value::Null),
        },
        (Some("control_response"), _) => {
            let request_id = string_at(&value, "/response/request_id");
            let result = if str_at(&value, "/response/subtype") == Some("success") {
                Ok(value
                    .pointer("/response/response")
                    .cloned()
                    .unwrap_or(Value::Null))
            } else {
                Err(str_at(&value, "/response/error")
                    .unwrap_or("unknown error")
                    .to_owned())
            };
            Incoming::ControlResponse { request_id, result }
        }
        (Some("control_cancel_request"), _) => Incoming::ControlCancel {
            request_id: string_at(&value, "/request_id"),
        },
        (Some("keep_alive"), _) => Incoming::KeepAlive,
        _ => Incoming::Events(vec![Event::Other(value)]),
    }
}

/// Builds the permission request from the body of a `can_use_tool` control request.
pub(crate) fn permission_request(request_id: String, body: &Value) -> PermissionRequest {
    PermissionRequest {
        request_id,
        tool_name: string_at(body, "/tool_name"),
        input: body.get("input").cloned().unwrap_or(Value::Null),
        tool_use_id: str_at(body, "/tool_use_id").map(str::to_owned),
        description: str_at(body, "/description").map(str::to_owned),
        blocked_path: str_at(body, "/blocked_path").map(str::to_owned),
    }
}

fn assistant_events(value: &Value) -> Vec<Event> {
    blocks(value)
        .iter()
        .filter_map(|block| match str_at(block, "/type")? {
            "text" => {
                let text = str_at(block, "/text")?;
                (!text.is_empty()).then(|| Event::Text(text.to_owned()))
            }
            "tool_use" => Some(Event::ToolUse {
                id: string_at(block, "/id"),
                name: string_at(block, "/name"),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            }),
            // Thinking and other block types aren't shown.
            _ => None,
        })
        .collect()
}

fn tool_results(value: &Value) -> Vec<Event> {
    blocks(value)
        .iter()
        .filter(|block| str_at(block, "/type") == Some("tool_result"))
        .map(|block| Event::ToolResult {
            tool_use_id: string_at(block, "/tool_use_id"),
            is_error: block.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            content: content_text(block.get("content")),
        })
        .collect()
}

fn turn_result(value: &Value) -> TurnResult {
    let denied_tools = value
        .get("permission_denials")
        .and_then(Value::as_array)
        .map(|denials| denials.iter().map(|d| string_at(d, "/tool_name")).collect())
        .unwrap_or_default();
    TurnResult {
        is_error: value.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        subtype: string_at(value, "/subtype"),
        text: str_at(value, "/result").map(str::to_owned),
        cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
        duration_ms: value.get("duration_ms").and_then(Value::as_u64),
        denied_tools,
    }
}

/// The content blocks of an assistant or user message. A plain-string
/// content (a user prompt being echoed back) has none.
fn blocks(value: &Value) -> &[Value] {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Tool result content is either a string or a list of text blocks.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| str_at(part, "/text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn string_at(value: &Value, pointer: &str) -> String {
    str_at(value, pointer).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOW: &str = include_str!("../../tests/fixtures/claude-2.1.306-allow.jsonl");
    const DENY: &str = include_str!("../../tests/fixtures/claude-2.1.306-deny.jsonl");

    fn parse_all(capture: &str) -> Vec<Incoming> {
        capture.lines().map(parse_line).collect()
    }

    fn events(capture: &str) -> Vec<Event> {
        parse_all(capture)
            .into_iter()
            .flat_map(|incoming| match incoming {
                Incoming::Events(events) => events,
                _ => Vec::new(),
            })
            .filter(|event| !matches!(event, Event::Other(_)))
            .collect()
    }

    #[test]
    fn real_allow_capture_reads_as_a_full_turn() {
        let parsed = parse_all(ALLOW);

        assert!(matches!(
            &parsed[0],
            Incoming::ControlResponse { request_id, result: Ok(_) } if request_id == "init-1"
        ));
        let permission = parsed
            .iter()
            .find_map(|incoming| match incoming {
                Incoming::ControlRequest {
                    request_id,
                    subtype,
                    body,
                } if subtype == "can_use_tool" => Some(permission_request(request_id.clone(), body)),
                _ => None,
            })
            .expect("capture has a permission request");
        assert_eq!(permission.tool_name, "Bash");
        assert_eq!(permission.input["command"], "touch probe_file.txt && echo done");
        assert_eq!(
            permission.blocked_path.as_deref(),
            Some("/work/sandbox/probe_file.txt")
        );
        assert!(
            permission
                .tool_use_id
                .as_deref()
                .is_some_and(|id| id.starts_with("toolu_"))
        );

        let events = events(ALLOW);
        assert!(matches!(&events[0], Event::Started { model, .. } if model == "claude-haiku-5-5"));
        assert!(matches!(&events[1], Event::ToolUse { name, .. } if name == "Bash"));
        assert!(matches!(
            &events[2],
            Event::ToolResult { is_error: false, content, .. } if content == "done"
        ));
        assert!(matches!(&events[3], Event::Text(_)));
        let Event::TurnFinished(result) = &events[4] else {
            panic!("expected TurnFinished, got {:?}", events[4]);
        };
        assert!(!result.is_error);
        assert_eq!(result.subtype, "success");
        assert!(result.denied_tools.is_empty());
        assert!(result.cost_usd.is_some_and(|cost| cost > 0.0));
    }

    #[test]
    fn real_deny_capture_shows_the_refusal() {
        let events = events(DENY);

        assert!(events.iter().any(|event| matches!(
            event,
            Event::ToolResult { is_error: true, content, .. } if content == "cos probe: denied by host"
        )));
        let Some(Event::TurnFinished(result)) = events.last() else {
            panic!("expected the turn to finish");
        };
        assert_eq!(result.denied_tools, vec!["Bash".to_owned()]);
    }

    #[test]
    fn allow_echoes_the_input_and_tool_use_id() {
        let request = PermissionRequest {
            request_id: "req-1".into(),
            tool_name: "Bash".into(),
            input: json!({ "command": "ls" }),
            tool_use_id: Some("toolu_1".into()),
            description: None,
            blocked_path: None,
        };

        assert_eq!(
            permission_answer(&request, &Decision::Allow),
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "req-1",
                    "response": { "behavior": "allow", "updatedInput": { "command": "ls" }, "toolUseID": "toolu_1" },
                },
            })
        );
        assert_eq!(
            permission_answer(&request, &Decision::Deny { message: "no".into() })["response"]["response"],
            json!({ "behavior": "deny", "message": "no", "toolUseID": "toolu_1" })
        );
    }

    #[test]
    fn unknown_and_broken_lines_are_kept_not_dropped() {
        assert_eq!(
            parse_line("not json"),
            Incoming::Events(vec![Event::Other(json!("not json"))])
        );
        assert_eq!(
            parse_line(r#"{"type":"rate_limit_event"}"#),
            Incoming::Events(vec![Event::Other(json!({ "type": "rate_limit_event" }))])
        );
        assert_eq!(
            parse_line(
                r#"{"type":"control_response","response":{"subtype":"error","request_id":"x","error":"nope"}}"#
            ),
            Incoming::ControlResponse {
                request_id: "x".into(),
                result: Err("nope".into())
            }
        );
    }
}
