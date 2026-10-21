//! Claude Code as a child process, in one of two ways:
//!
//! - [`ClaudeSession`]: headless. cos writes prompts and answers to Claude's
//!   stdin and reads events from its stdout, one JSON object per line. Every
//!   permission prompt comes to cos as an [`Event::PermissionRequest`].
//! - [`ClaudeTerminal`]: Claude's own UI in a pseudo-terminal, for people to
//!   watch and type into. cos follows it through [`hooks`].
//!
//! [`registry`] lists the Claude sessions running on the machine, including
//! ones cos didn't start.

pub mod hooks;
mod protocol;
pub mod registry;
mod terminal;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{Decision, Event, HarnessError, PermissionRequest};
use protocol::Incoming;
pub use terminal::{
    ClaudeTerminal, HookTarget, TerminalAttach, TerminalEvent, TerminalOptions, TerminalUpdate,
};

/// How long Claude gets to start up and answer the handshake.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// How long Claude gets to answer an interrupt.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to keep reading output after the process has exited.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long to wait for a process to be reaped after a write to it failed.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// Stderr lines kept for error messages.
const STDERR_TAIL: usize = 40;

/// Variables a Claude session sets for its own children. When cos runs from
/// inside Claude (or is started by something that did), passing them on makes
/// the new Claude think it is a child session: it stops saving its transcript,
/// among other things. Settings such as `CLAUDE_CONFIG_DIR` are kept.
pub(crate) const SESSION_MARKERS: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_CODE_TMPDIR",
];

/// Claude's `--permission-mode`. Whatever the mode doesn't settle on its own
/// becomes a permission request to cos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    /// Ask before every edit and every command that isn't read-only.
    #[default]
    Manual,
    /// Edits inside the working directory go through; commands still ask.
    AcceptEdits,
    /// Read-only planning.
    Plan,
    /// Anything that would ask is refused instead.
    DontAsk,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::DontAsk => "dontAsk",
        }
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual" | "default" => Ok(Self::Manual),
            "acceptEdits" => Ok(Self::AcceptEdits),
            "plan" => Ok(Self::Plan),
            "dontAsk" => Ok(Self::DontAsk),
            other => Err(format!(
                "unknown permission mode `{other}` (expected manual, acceptEdits, plan or dontAsk)"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeOptions {
    /// The `claude` binary. A bare name is looked up on PATH.
    pub binary: PathBuf,
    pub cwd: PathBuf,
    /// A model alias such as `sonnet`, or a full model name. `None` uses Claude's default.
    pub model: Option<String>,
    pub session_id: String,
    /// Continue the existing session `session_id` instead of starting a new one.
    pub resume: bool,
    pub permission_mode: PermissionMode,
    /// Rules that never need approval, such as `Bash(cargo test:*)`.
    pub allowed_tools: Vec<String>,
    pub append_system_prompt: Option<String>,
    /// MCP servers as a JSON string or file path. No other MCP servers are loaded.
    pub mcp_config: Option<String>,
    /// Keep the transcript on disk so the session can be resumed.
    pub persist: bool,
    /// Extra environment variables for the process.
    pub env: Vec<(String, String)>,
}

impl ClaudeOptions {
    /// A new session in `cwd` with a fresh session id.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            binary: PathBuf::from("claude"),
            cwd: cwd.into(),
            model: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            resume: false,
            permission_mode: PermissionMode::default(),
            allowed_tools: Vec::new(),
            append_system_prompt: None,
            mcp_config: None,
            persist: true,
            env: Vec::new(),
        }
    }

    /// The command-line arguments for `claude`.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-prompts",
            "host",
            // Hidden flag, and the one that matters: without it Claude refuses
            // anything that needs approval instead of asking cos.
            "--permission-prompt-tool",
            "stdio",
            // Only the repo's own settings. User settings would let personal
            // allow rules and auto mode approve actions before cos sees them.
            "--setting-sources",
            "project",
            "--strict-mcp-config",
            "--permission-mode",
            self.permission_mode.as_str(),
        ]
        .map(String::from)
        .into();

        args.push(if self.resume { "--resume" } else { "--session-id" }.into());
        args.push(self.session_id.clone());
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if !self.allowed_tools.is_empty() {
            args.extend(["--allowedTools".into(), self.allowed_tools.join(",")]);
        }
        if let Some(prompt) = &self.append_system_prompt {
            args.extend(["--append-system-prompt".into(), prompt.clone()]);
        }
        if let Some(config) = &self.mcp_config {
            args.extend(["--mcp-config".into(), config.clone()]);
        }
        if !self.persist {
            args.push("--no-session-persistence".into());
        }
        args
    }
}

/// A running Claude Code process.
///
/// Events arrive on the receiver returned by [`ClaudeSession::spawn`], ending
/// with [`Event::Exited`]. Dropping the session kills the process.
pub struct ClaudeSession {
    session_id: String,
    shared: Arc<Shared>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
}

/// State the session shares with its reader tasks.
struct Shared {
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// Permission requests waiting for an answer, by request id.
    permissions: Mutex<HashMap<String, PermissionRequest>>,
    /// Control requests cos sent, waiting for Claude's response.
    controls: Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>,
    stderr_tail: Mutex<VecDeque<String>>,
}

impl ClaudeSession {
    /// Starts Claude and waits for its handshake.
    pub async fn spawn(
        options: ClaudeOptions,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Event>), HarnessError> {
        let mut command = Command::new(&options.binary);
        for name in SESSION_MARKERS {
            command.env_remove(name);
        }
        let mut child = command
            .args(options.args())
            .current_dir(&options.cwd)
            .envs(options.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| HarnessError::Spawn {
                binary: options.binary.display().to_string(),
                source,
            })?;

        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            unreachable!("all three pipes were requested");
        };

        let shared = Arc::new(Shared {
            stdin: tokio::sync::Mutex::new(Some(stdin)),
            permissions: Mutex::default(),
            controls: Mutex::default(),
            stderr_tail: Mutex::default(),
        });
        let (events, receiver) = mpsc::unbounded_channel();
        let (kill, killed) = oneshot::channel();

        let readers = [
            tokio::spawn(read_stdout(stdout, shared.clone(), events.clone())),
            tokio::spawn(read_stderr(stderr, shared.clone(), events.clone())),
        ];
        tokio::spawn(wait_for_exit(child, killed, readers, shared.clone(), events));

        let session = Self {
            session_id: options.session_id,
            shared,
            kill: Mutex::new(Some(kill)),
        };
        session.control("initialize", STARTUP_TIMEOUT).await?;
        Ok((session, receiver))
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Sends a user message. Claude starts a turn, or queues it if one is running.
    pub async fn send(&self, text: &str) -> Result<(), HarnessError> {
        self.shared.write(&protocol::user_message(text)).await
    }

    /// Answers a permission request. Each request can be answered once.
    pub async fn answer(&self, request_id: &str, decision: Decision) -> Result<(), HarnessError> {
        let request = lock(&self.shared.permissions)
            .remove(request_id)
            .ok_or_else(|| HarnessError::UnknownRequest(request_id.to_owned()))?;
        self.shared
            .write(&protocol::permission_answer(&request, &decision))
            .await
    }

    /// Stops the current turn. Claude stays up for the next message.
    pub async fn interrupt(&self) -> Result<(), HarnessError> {
        self.control("interrupt", CONTROL_TIMEOUT).await.map(drop)
    }

    /// Closes Claude's input. It finishes what it is doing and exits.
    pub async fn close_input(&self) {
        self.shared.stdin.lock().await.take();
    }

    /// Kills the process right away.
    pub fn kill(&self) {
        if let Some(kill) = lock(&self.kill).take() {
            let _ = kill.send(());
        }
    }

    /// Sends a control request and waits for Claude's response.
    async fn control(&self, subtype: &str, timeout: Duration) -> Result<Value, HarnessError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (reply, response) = oneshot::channel();
        lock(&self.shared.controls).insert(request_id.clone(), reply);

        let sent = self
            .shared
            .write(&protocol::control_request(&request_id, subtype))
            .await;
        // A failed write usually means the process already died. Give it a
        // moment to be reaped so the error can carry its stderr.
        let wait = if sent.is_ok() { timeout } else { EXIT_GRACE };

        match tokio::time::timeout(wait, response).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(HarnessError::Rejected {
                request: subtype.to_owned(),
                message,
            }),
            // The reply was dropped because the process exited.
            Ok(Err(_)) => Err(HarnessError::Exited {
                stderr: self.shared.stderr_tail(),
            }),
            Err(_) => {
                lock(&self.shared.controls).remove(&request_id);
                sent.and(Err(HarnessError::Timeout {
                    request: subtype.to_owned(),
                    stderr: self.shared.stderr_tail(),
                }))
            }
        }
    }
}

impl Shared {
    async fn write(&self, message: &Value) -> Result<(), HarnessError> {
        let mut line = message.to_string();
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        let stdin = stdin.as_mut().ok_or(HarnessError::InputClosed)?;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(HarnessError::Write)?;
        stdin.flush().await.map_err(HarnessError::Write)
    }

    fn stderr_tail(&self) -> String {
        lock(&self.stderr_tail)
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

async fn read_stdout(
    stdout: impl AsyncRead + Unpin,
    shared: Arc<Shared>,
    events: mpsc::UnboundedSender<Event>,
) {
    let mut lines = Lines::new(stdout);
    while let Some(line) = lines.next().await {
        if line.trim().is_empty() {
            continue;
        }
        match protocol::parse_line(&line) {
            Incoming::Events(batch) => {
                for event in batch {
                    let _ = events.send(event);
                }
            }
            Incoming::ControlRequest {
                request_id,
                subtype,
                body,
            } if subtype == "can_use_tool" => {
                let request = protocol::permission_request(request_id, &body);
                lock(&shared.permissions).insert(request.request_id.clone(), request.clone());
                let _ = events.send(Event::PermissionRequest(request));
            }
            Incoming::ControlRequest {
                request_id, subtype, ..
            } => {
                // Hook callbacks, SDK MCP servers and the like. cos registers
                // none, so say so rather than leave Claude waiting.
                let error = format!("cos does not handle `{subtype}` requests");
                let _ = shared.write(&protocol::control_error(&request_id, &error)).await;
            }
            Incoming::ControlResponse { request_id, result } => {
                if let Some(reply) = lock(&shared.controls).remove(&request_id) {
                    let _ = reply.send(result);
                }
            }
            Incoming::ControlCancel { request_id } => {
                if lock(&shared.permissions).remove(&request_id).is_some() {
                    let _ = events.send(Event::PermissionCancelled { request_id });
                }
            }
            Incoming::KeepAlive => {}
        }
    }
}

async fn read_stderr(
    stderr: impl AsyncRead + Unpin,
    shared: Arc<Shared>,
    events: mpsc::UnboundedSender<Event>,
) {
    let mut lines = Lines::new(stderr);
    while let Some(line) = lines.next().await {
        {
            let mut tail = lock(&shared.stderr_tail);
            if tail.len() == STDERR_TAIL {
                tail.pop_front();
            }
            tail.push_back(line.clone());
        }
        let _ = events.send(Event::Stderr(line));
    }
}

/// Waits for the process to exit (or kills it), drains its output, then
/// fails anything still waiting on it and sends [`Event::Exited`].
async fn wait_for_exit(
    mut child: Child,
    killed: oneshot::Receiver<()>,
    readers: [JoinHandle<()>; 2],
    shared: Arc<Shared>,
    events: mpsc::UnboundedSender<Event>,
) {
    let status = tokio::select! {
        status = child.wait() => status,
        // Either `kill()` was called or the session was dropped.
        _ = killed => {
            let _ = child.start_kill();
            child.wait().await
        }
    };

    // A tool's own child process can keep the pipes open after Claude exits,
    // so don't wait for end-of-file forever.
    for reader in readers {
        let abort = reader.abort_handle();
        if tokio::time::timeout(DRAIN_TIMEOUT, reader).await.is_err() {
            abort.abort();
        }
    }

    shared.stdin.lock().await.take();
    lock(&shared.permissions).clear();
    // Dropping the reply senders tells waiting callers the process is gone.
    lock(&shared.controls).clear();

    let code = status.ok().and_then(|status| status.code());
    let _ = events.send(Event::Exited { code });
}

/// Splits a pipe into lines, replacing invalid UTF-8 instead of stopping.
struct Lines<R> {
    reader: BufReader<R>,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Lines<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: Vec::new(),
        }
    }

    async fn next(&mut self) -> Option<String> {
        self.buffer.clear();
        match self.reader.read_until(b'\n', &mut self.buffer).await {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                let line = String::from_utf8_lossy(&self.buffer);
                Some(line.trim_end_matches(['\n', '\r']).to_owned())
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn args_route_every_prompt_to_cos() {
        let options = ClaudeOptions::new("/tmp");
        let args = options.args();

        assert!(has_pair(&args, "--permission-prompt-tool", "stdio"));
        assert!(has_pair(&args, "--setting-sources", "project"));
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
        assert!(has_pair(&args, "--permission-mode", "manual"));
        assert!(has_pair(&args, "--session-id", &options.session_id));
        assert!(!args.contains(&"--no-session-persistence".to_owned()));
    }

    #[test]
    fn args_for_a_resumed_session_with_extras() {
        let mut options = ClaudeOptions::new("/tmp");
        options.resume = true;
        options.model = Some("haiku".into());
        options.allowed_tools = vec!["Bash(cargo test:*)".into(), "Read".into()];
        options.persist = false;
        let args = options.args();

        assert!(has_pair(&args, "--resume", &options.session_id));
        assert!(!args.contains(&"--session-id".to_owned()));
        assert!(has_pair(&args, "--model", "haiku"));
        assert!(has_pair(&args, "--allowedTools", "Bash(cargo test:*),Read"));
        assert!(args.contains(&"--no-session-persistence".to_owned()));
    }

    #[test]
    fn permission_mode_round_trips() {
        for mode in [
            PermissionMode::Manual,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::DontAsk,
        ] {
            assert_eq!(mode.as_str().parse::<PermissionMode>(), Ok(mode));
        }
        assert!("bypassPermissions".parse::<PermissionMode>().is_err());
    }
}
