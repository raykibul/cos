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
//!
//! Reads go through a [`Cache`]: a file that only grew since the last read
//! has just its new lines parsed. The context meter ([`Context`]) comes from
//! the newest reply's token usage.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::attachments::{self, Attached};

/// Wakeups the daemon types into a Chief of Staff start with this.
pub(crate) const UPDATE_PREFIX: &str = "[cos ·";
/// A tool summary is cut to this many characters.
const SUMMARY_CHARS: usize = 160;
/// Tools that are Claude's plumbing rather than work (loading deferred tools).
const HIDDEN_TOOLS: [&str; 1] = ["ToolSearch"];
/// cos's tool that starts an agent; its result names the agent.
const SPAWN_TOOL: &str = "mcp__cos__agent_spawn";
/// cos's tools that act on an agent named by `agent_id` in their input.
const AGENT_TOOLS: [&str; 3] = [
    "mcp__cos__agent_send",
    "mcp__cos__agent_read",
    "mcp__cos__agent_stop",
];
/// How many transcripts the cache keeps parsed.
const CACHED_FILES: usize = 64;
/// Context windows, in tokens.
const WINDOW_200K: u64 = 200_000;
const WINDOW_1M: u64 = 1_000_000;

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
    /// Files you attached through cos, read back from the block cos
    /// appended to your message (which `text` leaves out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attached>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ToolCall {
    pub name: String,
    /// Its main argument on one line, such as the command or the file.
    pub summary: String,
    pub is_error: bool,
    /// The agent a cos agent tool acted on: `agent_spawn`'s new agent
    /// (from its result), or `agent_send`, `agent_read` and `agent_stop`'s
    /// `agent_id`. Apps open that agent from the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<i64>,
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
    /// How full the conversation is, as of the newest reply; `None` before
    /// the first one.
    #[serde(default)]
    pub context: Option<Context>,
}

/// How much of the model's context window the conversation fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Context {
    /// The newest reply's input: fresh, cache-read and cache-written tokens.
    pub used_tokens: u64,
    /// The model's window, when cos knows it.
    pub window_tokens: Option<u64>,
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

/// Parses the complete lines of a transcript.
#[cfg(test)]
pub(crate) fn parse(bytes: &[u8]) -> Transcript {
    let mut parser = Parser::default();
    parser.feed(bytes);
    parser.transcript(None)
}

/// A transcript parsed line by line, so more lines can be fed as the file
/// grows.
#[derive(Debug, Clone, Default)]
pub(crate) struct Parser {
    messages: Vec<Message>,
    /// Tool calls by their tool_use id, for folding results in.
    calls: HashMap<String, usize>,
    /// Bytes taken so far: everything up to and including the last newline.
    consumed: u64,
    /// The newest reply's input tokens, and the model that wrote it.
    usage: Option<(u64, Option<String>)>,
}

impl Parser {
    /// Takes the complete lines of `bytes`, which follow what was fed
    /// before. A line without its newline is left for the next feed.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
            return;
        };
        for line in bytes[..end].split(|byte| *byte == b'\n') {
            self.line(line);
        }
        self.consumed += end as u64 + 1;
    }

    /// The chat so far. `launch_model` is the model the agent was started
    /// with (`opus[1m]`), which says more about the window than the model
    /// in the file.
    pub(crate) fn transcript(&self, launch_model: Option<&str>) -> Transcript {
        let context = self.usage.as_ref().map(|(used, model)| {
            let window = context_window(launch_model, model.as_deref())
                // A guess the conversation outgrew was wrong.
                .filter(|window| window >= used);
            Context {
                used_tokens: *used,
                window_tokens: window,
            }
        });
        Transcript {
            messages: self.messages.clone(),
            next: settled(&self.messages),
            context,
        }
    }

    fn line(&mut self, line: &[u8]) {
        let messages = &mut self.messages;
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if flag(&entry, "isSidechain") || flag(&entry, "isMeta") || flag(&entry, "isCompactSummary") {
            return;
        }
        let ts_ms = entry["timestamp"]
            .as_str()
            .and_then(|ts| ts.parse::<Timestamp>().ok())
            .map_or(0, Timestamp::as_millisecond);
        let content = &entry["message"]["content"];
        match entry["type"].as_str() {
            Some("user") => {
                if let Some(text) = content.as_str() {
                    push_user_text(messages, text, ts_ms);
                    return;
                }
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            push_user_text(messages, block["text"].as_str().unwrap_or_default(), ts_ms);
                        }
                        Some("tool_result") => {
                            let call = block["tool_use_id"]
                                .as_str()
                                .and_then(|id| self.calls.get(id))
                                .and_then(|&index| messages[index].tool.as_mut());
                            if let Some(call) = call {
                                call.done = true;
                                call.is_error = block["is_error"].as_bool().unwrap_or(false);
                                if call.name == SPAWN_TOOL && !call.is_error {
                                    call.agent_id = spawned_agent(&block["content"]);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                note_usage(&mut self.usage, &entry["message"]);
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or_default().trim();
                            if !text.is_empty() {
                                push(
                                    messages,
                                    Speaker::Assistant,
                                    MessageKind::Text,
                                    Some(text),
                                    None,
                                    ts_ms,
                                );
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
                                agent_id: AGENT_TOOLS
                                    .contains(&name)
                                    .then(|| block["input"]["agent_id"].as_i64())
                                    .flatten(),
                                id: id.clone(),
                                done: false,
                            };
                            push(
                                messages,
                                Speaker::Assistant,
                                MessageKind::Tool,
                                None,
                                Some(call),
                                ts_ms,
                            );
                            self.calls.insert(id, messages.len() - 1);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Keeps a reply's usage: what its request put in the context. Replies
/// cos can't count (Claude's own `<synthetic>` notices) are skipped.
fn note_usage(kept: &mut Option<(u64, Option<String>)>, message: &Value) {
    let usage = &message["usage"];
    let model = message["model"].as_str();
    if !usage.is_object() || model == Some("<synthetic>") {
        return;
    }
    let tokens = |key: &str| usage[key].as_u64().unwrap_or(0);
    let used =
        tokens("input_tokens") + tokens("cache_read_input_tokens") + tokens("cache_creation_input_tokens");
    if used > 0 {
        *kept = Some((used, model.map(str::to_owned)));
    }
}

/// The context window of a Claude model, when cos knows it: 1M for a
/// `[1m]` model and for the models whose window is 1M by default (Opus and
/// Sonnet from 4.6, Haiku from 5, Fable, Mythos), 200K for older ones.
/// `launch` is the model the agent was started with, `model` the one the
/// transcript names.
pub(crate) fn context_window(launch: Option<&str>, model: Option<&str>) -> Option<u64> {
    let one_million =
        |name: Option<&str>| name.is_some_and(|name| name.to_ascii_lowercase().contains("[1m]"));
    if one_million(launch) || one_million(model) {
        return Some(WINDOW_1M);
    }
    let model = model?.to_ascii_lowercase();
    let mut parts = model.strip_prefix("claude-")?.split('-');
    let family = parts.next()?;
    if family.chars().all(|c| c.is_ascii_digit()) {
        // claude-3-5-sonnet-…: the Claude 3 models.
        return Some(WINDOW_200K);
    }
    let major: u32 = parts.next()?.parse().ok()?;
    // A dated snapshot (`claude-sonnet-4-20250514`) has no minor version.
    let minor: u32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .filter(|minor| *minor < 100)
        .unwrap_or(0);
    let version = (major, minor);
    let million = match family {
        "fable" | "mythos" => true,
        "opus" | "sonnet" => version >= (4, 6),
        "haiku" => version >= (5, 0),
        _ => return None,
    };
    Some(if million { WINDOW_1M } else { WINDOW_200K })
}

/// Parsed transcripts by file. A file that only grew since the last read
/// has just the new lines parsed; one that shrank or was rewritten is
/// parsed again from the top.
#[derive(Default)]
pub(crate) struct Cache {
    files: Mutex<HashMap<PathBuf, Cached>>,
}

struct Cached {
    /// The file's length and modification time when it was last read.
    len: u64,
    modified: Option<SystemTime>,
    parser: Parser,
    /// When it was last used, for dropping the oldest.
    used: u64,
}

impl Cache {
    /// Reads the transcript at `path`. A missing file is an empty
    /// transcript.
    pub(crate) fn read(&self, path: &Path, launch_model: Option<&str>) -> std::io::Result<Transcript> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.lock().remove(path);
                return Ok(Transcript::default());
            }
            Err(error) => return Err(error),
        };
        let modified = meta.modified().ok();
        // Taken out while reading, so the lock isn't held for the file.
        let cached = self.lock().remove(path);
        let (len, parser) = match cached {
            Some(cached) if cached.len == meta.len() && cached.modified == modified => {
                (cached.len, cached.parser)
            }
            Some(cached) if meta.len() > cached.len => match grow(path, cached.parser)? {
                Some(grown) => grown,
                None => from_top(path)?,
            },
            _ => from_top(path)?,
        };
        let transcript = parser.transcript(launch_model);
        let mut files = self.lock();
        let used = files.values().map(|cached| cached.used).max().unwrap_or(0) + 1;
        files.insert(
            path.to_owned(),
            Cached {
                len,
                modified,
                parser,
                used,
            },
        );
        if files.len() > CACHED_FILES
            && let Some(oldest) = files
                .iter()
                .min_by_key(|(_, cached)| cached.used)
                .map(|(path, _)| path.clone())
        {
            files.remove(&oldest);
        }
        Ok(transcript)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Cached>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Parses the whole file. Returns how many bytes it read, with the parser.
fn from_top(path: &Path) -> std::io::Result<(u64, Parser)> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut parser = Parser::default();
    parser.feed(&bytes);
    Ok((bytes.len() as u64, parser))
}

/// Feeds `parser` what follows its last complete line, if the file still
/// ends that line where it did (it grew rather than being rewritten).
/// Returns how far it read, with the parser; `None` means read it again
/// from the top.
fn grow(path: &Path, mut parser: Parser) -> std::io::Result<Option<(u64, Parser)>> {
    let mut file = std::fs::File::open(path)?;
    // From the newline that ended the last line taken, to check it's
    // still there.
    let from = parser.consumed.saturating_sub(1);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let rest = match parser.consumed {
        0 => &bytes[..],
        _ if bytes.first() == Some(&b'\n') => &bytes[1..],
        _ => return Ok(None),
    };
    parser.feed(rest);
    Ok(Some((from + bytes.len() as u64, parser)))
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
    let (text, attached) = match kind {
        MessageKind::Text => attachments::split(text),
        _ => (text.to_owned(), Vec::new()),
    };
    push(messages, Speaker::User, kind, Some(&text), None, ts_ms);
    if let Some(message) = messages.last_mut() {
        message.attachments = attached;
    }
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
        attachments: Vec::new(),
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
        SPAWN_TOOL => field("title").or_else(|| field("prompt")),
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

/// The agent id in `agent_spawn`'s result, `{"agent_id": 7, …}`, which
/// Claude writes as a string or as text blocks.
fn spawned_agent(content: &Value) -> Option<i64> {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => return None,
    };
    serde_json::from_str::<Value>(&text).ok()?["agent_id"].as_i64()
}

#[cfg(test)]
mod tests;
