//! Claude Code hooks: how cos follows a session that runs in a terminal.
//!
//! cos starts the session with `--settings` holding hooks for that session
//! only, so nothing is written to the user's settings. Each hook runs a relay
//! command (`cos hook`) that forwards the hook's input to a [`HookServer`] over
//! a Unix socket and prints whatever cos answers.
//!
//! Only `PermissionRequest` waits for an answer. Every other hook is answered
//! with nothing straight away, so Claude never waits on cos for them. If cos is
//! not running, the relay prints nothing and Claude carries on as if there were
//! no hook: the permission prompt shows up in the terminal as usual.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream as AsyncUnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::Decision;

/// The environment variable that tells the relay where the hook socket is.
pub const SOCKET_ENV: &str = "COS_HOOK_SOCKET";

/// The hook events cos listens to.
pub const EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// How long Claude lets a permission hook wait for cos, in seconds. After
/// that it kills the relay and asks in the terminal instead.
pub const PERMISSION_TIMEOUT_SECS: u64 = 24 * 60 * 60;
/// The other hooks are answered at once; this only guards against a stuck relay.
const EVENT_TIMEOUT_SECS: u64 = 10;

/// The `--settings` value that sends every hook in [`EVENTS`] to `command`.
pub fn settings(command: &str) -> Value {
    let mut hooks = Map::new();
    for event in EVENTS {
        let timeout = if event == "PermissionRequest" {
            PERMISSION_TIMEOUT_SECS
        } else {
            EVENT_TIMEOUT_SECS
        };
        let mut entry = json!({
            "hooks": [{ "type": "command", "command": command, "timeout": timeout }],
        });
        if is_tool_event(event) {
            entry["matcher"] = json!("*");
        }
        hooks.insert(event.to_owned(), json!([entry]));
    }
    json!({ "hooks": hooks })
}

fn is_tool_event(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse")
}

/// Runs inside a hook: forwards the hook input on `input` to the socket and
/// writes cos's answer, if any, to `output`.
///
/// Never fails. Whatever goes wrong, the hook prints nothing and exits 0 so
/// Claude behaves as if cos weren't there.
pub fn relay(socket: Option<&Path>, input: &mut impl Read, output: &mut impl Write) {
    let _ = try_relay(socket, input, output);
}

fn try_relay(socket: Option<&Path>, input: &mut impl Read, output: &mut impl Write) -> std::io::Result<()> {
    let mut text = String::new();
    input.read_to_string(&mut text)?;
    let Some(socket) = socket else { return Ok(()) };
    // Re-encode so the request is exactly one line.
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };

    let mut stream = UnixStream::connect(socket)?;
    writeln!(stream, "{value}")?;
    // The write half stays open while we wait: cos treats end-of-file as the
    // hook having been killed.
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer)?;
    let answer = answer.trim();
    if !answer.is_empty() {
        writeln!(output, "{answer}")?;
        output.flush()?;
    }
    Ok(())
}

/// One hook that ran in a Claude session.
#[derive(Debug)]
pub struct Hook {
    /// Unique within one [`HookServer`].
    pub id: u64,
    /// `SessionStart`, `PreToolUse`, `Stop` and so on.
    pub event: String,
    pub session_id: Option<String>,
    /// Everything Claude passed to the hook.
    pub input: Value,
    reply: Option<oneshot::Sender<Value>>,
}

impl Hook {
    fn from_input(id: u64, input: Value) -> Self {
        let field = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            id,
            event: field("hook_event_name").unwrap_or_default(),
            session_id: field("session_id"),
            input,
            reply: None,
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        self.input.get("tool_name").and_then(Value::as_str)
    }

    pub fn tool_input(&self) -> Option<&Value> {
        self.input.get("tool_input")
    }

    /// True for a permission request that is waiting for [`Hook::answer`].
    pub fn awaits_answer(&self) -> bool {
        self.reply.is_some()
    }

    /// Answers a permission request. Dropping the hook instead leaves the
    /// decision to Claude's own prompt in the terminal.
    pub fn answer(mut self, decision: &Decision) {
        let decision = match decision {
            Decision::Allow => json!({ "behavior": "allow" }),
            Decision::Deny { message } => json!({ "behavior": "deny", "message": message }),
        };
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(json!({
                "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision },
            }));
        }
    }
}

#[derive(Debug)]
pub enum HookEvent {
    /// A hook ran. Answer it with [`Hook::answer`] if it
    /// [awaits an answer](Hook::awaits_answer).
    Fired(Hook),
    /// A permission hook stopped waiting before cos answered: it was
    /// answered in the terminal, Claude timed it out, or the session ended.
    Withdrawn { id: u64 },
}

/// Listens on a Unix socket for the relay. One server can serve any number
/// of sessions; [`Hook::session_id`] says which one a hook came from.
pub struct HookServer {
    path: PathBuf,
    /// The socket file this server created, so dropping it never removes a
    /// newer server's socket at the same path.
    file: Option<(u64, u64)>,
    task: JoinHandle<()>,
}

/// A file's device and inode.
fn file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

impl HookServer {
    /// Listens at `path`, replacing any socket file already there. Must be
    /// called inside a Tokio runtime.
    pub fn bind(path: impl Into<PathBuf>) -> std::io::Result<(Self, mpsc::UnboundedReceiver<HookEvent>)> {
        let path = path.into();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let file = file_id(&path);

        let (events, receiver) = mpsc::unbounded_channel();
        let ids = Arc::new(AtomicU64::new(1));
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, ids.clone(), events.clone()));
            }
        });
        Ok((Self { path, file, task }, receiver))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HookServer {
    fn drop(&mut self) {
        self.task.abort();
        if self.file.is_some() && file_id(&self.path) == self.file {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn serve(stream: AsyncUnixStream, ids: Arc<AtomicU64>, events: mpsc::UnboundedSender<HookEvent>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let Ok(input) = serde_json::from_str::<Value>(&line) else {
        let _ = writer.write_all(b"\n").await;
        return;
    };
    let mut hook = Hook::from_input(ids.fetch_add(1, Ordering::Relaxed), input);

    if hook.event != "PermissionRequest" {
        let _ = writer.write_all(b"\n").await;
        let _ = events.send(HookEvent::Fired(hook));
        return;
    }

    let id = hook.id;
    let (reply, answer) = oneshot::channel();
    hook.reply = Some(reply);
    if events.send(HookEvent::Fired(hook)).is_err() {
        let _ = writer.write_all(b"\n").await;
        return;
    }
    // The relay sends nothing more, so a read only returns when it is gone.
    let mut rest = [0u8; 64];
    tokio::select! {
        answer = answer => {
            let line = answer.map(|value| value.to_string()).unwrap_or_default();
            let _ = writer.write_all(format!("{line}\n").as_bytes()).await;
        }
        _ = reader.read(&mut rest) => {
            let _ = events.send(HookEvent::Withdrawn { id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_send_every_event_to_the_relay() {
        let settings = settings("'/bin/cos' hook");

        for event in EVENTS {
            let entry = &settings["hooks"][event][0];
            assert_eq!(entry["hooks"][0]["command"], "'/bin/cos' hook", "{event}");
            assert_eq!(entry["hooks"][0]["type"], "command");
            assert_eq!(entry.get("matcher").is_some(), is_tool_event(event), "{event}");
        }
        assert_eq!(
            settings["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            PERMISSION_TIMEOUT_SECS
        );
    }

    #[test]
    fn relay_without_cos_prints_nothing() {
        let mut output = Vec::new();
        relay(None, &mut &b"{}"[..], &mut output);
        relay(
            Some(Path::new("/nonexistent/cos.sock")),
            &mut &b"{}"[..],
            &mut output,
        );
        assert!(output.is_empty());
    }

    #[tokio::test]
    async fn dropping_an_old_server_keeps_a_newer_ones_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.sock");
        let (old, _) = HookServer::bind(&path).unwrap();
        let (new, _) = HookServer::bind(&path).unwrap();

        drop(old);
        assert!(path.exists());
        drop(new);
        assert!(!path.exists());
    }

    #[test]
    fn answers_become_hook_output() {
        let (reply, mut answer) = oneshot::channel();
        let mut hook = Hook::from_input(1, json!({ "hook_event_name": "PermissionRequest" }));
        hook.reply = Some(reply);

        hook.answer(&Decision::Deny { message: "no".into() });

        assert_eq!(
            answer.try_recv().unwrap(),
            json!({ "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny", "message": "no" },
            } })
        );
    }
}
