//! Claude's own transcript (`transcript_path` from the hooks, one JSON entry
//! per line) read as a chat: what you typed, what Claude said, and one row per
//! tool call with its result folded in. The app shows the Chief of Staff this
//! way. The contract is "Transcript" in `docs/chief-of-staff.md`.
//!
//! The format is undocumented, so parsing is lenient: a line that isn't JSON,
//! or an entry this module doesn't know, is skipped, never an error. Left out:
//! thinking, system and meta entries, sidechains (subagents), bookkeeping
//! entries (`attachment`, `file-history-snapshot`, `api-request-*`…), local
//! command echoes and interruption markers. A line still being written (no
//! newline yet) waits for the next read, so message numbers never change.

use std::collections::HashMap;
use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wakeups the daemon types into a Chief of Staff start with this.
pub(crate) const UPDATE_PREFIX: &str = "[cos ·";
/// A tool summary is cut to this many characters.
const SUMMARY_CHARS: usize = 160;
/// Tools that are Claude's plumbing rather than work (loading deferred tools).
const HIDDEN_TOOLS: [&str; 1] = ["ToolSearch"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Speaker {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MessageKind {
    /// What you typed or Claude said.
    Text,
    /// One tool call, with its result folded in.
    Tool,
    /// A cos wakeup typed into a Chief of Staff (`[cos · N updates]`).
    Update,
}

/// One row of the chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Message {
    /// 1, 2, 3… in the order of the file. Stable: the same file always
    /// gives the same numbers.
    pub n: usize,
    pub role: Speaker,
    pub kind: MessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCall>,
    /// When Claude wrote it; 0 if the entry had no time.
    pub ts_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ToolCall {
    pub name: String,
    /// Its main argument on one line, such as the command or the file.
    pub summary: String,
    pub is_error: bool,
    #[serde(skip)]
    id: String,
    /// Its result is in.
    #[serde(skip)]
    done: bool,
}

/// A transcript read so far.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Transcript {
    pub messages: Vec<Message>,
    /// Every message up to `next` is final. A tool call still waiting for
    /// its result is after `next`, so a client asking `after=next` gets it
    /// again, with its result once it's in.
    pub next: usize,
}

impl Transcript {
    /// The messages after `after`, with `next` as for the whole file.
    pub(crate) fn after(mut self, after: usize) -> Self {
        self.messages.retain(|message| message.n > after);
        self
    }

    /// The text of the last thing Claude said, if anything.
    pub(crate) fn last_reply(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == Speaker::Assistant && message.kind == MessageKind::Text)
            .and_then(|message| message.text.as_deref())
    }
}

/// Reads and parses the file. A missing file is an empty transcript.
pub(crate) fn read(path: &Path) -> std::io::Result<Transcript> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(parse(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Transcript::default()),
        Err(error) => Err(error),
    }
}

/// Parses the complete lines of a transcript.
pub(crate) fn parse(bytes: &[u8]) -> Transcript {
    let complete = match bytes.iter().rposition(|byte| *byte == b'\n') {
        Some(end) => &bytes[..end],
        None => &[],
    };
    let mut messages: Vec<Message> = Vec::new();
    // Tool calls by their tool_use id, for folding results in.
    let mut calls: HashMap<String, usize> = HashMap::new();
    for line in complete.split(|byte| *byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if flag(&entry, "isSidechain") || flag(&entry, "isMeta") || flag(&entry, "isCompactSummary") {
            continue;
        }
        let ts_ms = entry["timestamp"]
            .as_str()
            .and_then(|ts| ts.parse::<Timestamp>().ok())
            .map_or(0, Timestamp::as_millisecond);
        let content = &entry["message"]["content"];
        match entry["type"].as_str() {
            Some("user") => {
                if let Some(text) = content.as_str() {
                    push_user_text(&mut messages, text, ts_ms);
                    continue;
                }
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            push_user_text(&mut messages, block["text"].as_str().unwrap_or_default(), ts_ms);
                        }
                        Some("tool_result") => {
                            let call = block["tool_use_id"]
                                .as_str()
                                .and_then(|id| calls.get(id))
                                .and_then(|&index| messages[index].tool.as_mut());
                            if let Some(call) = call {
                                call.done = true;
                                call.is_error = block["is_error"].as_bool().unwrap_or(false);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or_default().trim();
                            if !text.is_empty() {
                                push(&mut messages, Speaker::Assistant, MessageKind::Text, Some(text), None, ts_ms);
                            }
                        }
                        Some("tool_use") => {
                            let name = block["name"].as_str().unwrap_or("?");
                            if HIDDEN_TOOLS.contains(&name) {
                                continue;
                            }
                            let id = block["id"].as_str().unwrap_or_default().to_owned();
                            let call = ToolCall {
                                name: name.to_owned(),
                                summary: summarize(name, &block["input"]),
                                is_error: false,
                                id: id.clone(),
                                done: false,
                            };
                            push(&mut messages, Speaker::Assistant, MessageKind::Tool, None, Some(call), ts_ms);
                            calls.insert(id, messages.len() - 1);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let next = settled(&messages);
    Transcript { messages, next }
}

/// How many messages are final: all of them, unless the newest tool calls
/// (after the last text) still wait for their results.
fn settled(messages: &[Message]) -> usize {
    let last_text = messages
        .iter()
        .rposition(|message| message.kind != MessageKind::Tool)
        .map_or(0, |index| index + 1);
    messages[last_text..]
        .iter()
        .find(|message| message.tool.as_ref().is_some_and(|call| !call.done))
        .map_or(messages.len(), |open| open.n - 1)
}

fn push_user_text(messages: &mut Vec<Message>, text: &str, ts_ms: i64) {
    let text = unwrap_pastes(text);
    let text = text.trim();
    let noise = ["<command-", "<local-command-", "[Request interrupted"];
    if text.is_empty() || noise.iter().any(|prefix| text.starts_with(prefix)) {
        return;
    }
    let kind = if text.starts_with(UPDATE_PREFIX) {
        MessageKind::Update
    } else {
        MessageKind::Text
    };
    push(messages, Speaker::User, kind, Some(text), None, ts_ms);
}

fn push(
    messages: &mut Vec<Message>,
    role: Speaker,
    kind: MessageKind,
    text: Option<&str>,
    tool: Option<ToolCall>,
    ts_ms: i64,
) {
    messages.push(Message {
        n: messages.len() + 1,
        role,
        kind,
        text: text.map(str::to_owned),
        tool,
        ts_ms,
    });
}

/// Claude records a bracketed paste wrapped in
/// `<pasted_content id="…">` … `</pasted_content id="…">`; this is the text
/// as typed.
fn unwrap_pastes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let tag = ["<pasted_content", "</pasted_content"]
            .iter()
            .filter_map(|tag| rest.find(tag))
            .min();
        let Some((start, end)) = tag.and_then(|start| Some((start, start + rest[start..].find('>')?))) else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn flag(entry: &Value, key: &str) -> bool {
    entry[key].as_bool().unwrap_or(false)
}

/// The tool's main argument on one line.
fn summarize(name: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let text = match name {
        "Bash" => field("command"),
        "Task" | "Agent" => field("description"),
        "WebSearch" => field("query"),
        _ => field("file_path")
            .or_else(|| field("notebook_path"))
            .or_else(|| field("pattern"))
            .or_else(|| field("url")),
    };
    match text {
        Some(text) => crate::try_cmd::first_line(text, SUMMARY_CHARS),
        None if input.as_object().is_some_and(|input| input.is_empty()) => String::new(),
        None if input.is_null() => String::new(),
        None => crate::try_cmd::first_line(&input.to_string(), SUMMARY_CHARS),
    }
}

#[cfg(test)]
mod tests;
