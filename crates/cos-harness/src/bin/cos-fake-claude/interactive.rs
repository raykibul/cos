//! The fake's interactive mode: a stand-in for `claude` running in a terminal.
//!
//! It runs the hooks passed in `--settings` the way Claude does (a shell
//! command, the hook input as JSON on stdin, the answer on stdout), prints a
//! plain-text "UI", and reads lines typed into the terminal.
//!
//! What a prompt does:
//! - starts with `ask:<Tool> <arg>`: like `permission`, but asks for that
//!   tool. `arg` is Bash's command, a file tool's `file_path` (Write also gets
//!   a `content`), or the whole input as JSON when it starts with `{`.
//! - contains `permission`: Bash `touch x` that needs approval. The
//!   `PermissionRequest` hook runs while the terminal dialog is up, like the
//!   real CLI: a hook answer settles it, and so does typing `1` (allow) or `3`
//!   (deny), which kills the hook.
//! - contains `hang forever`: never finishes and ignores whatever is typed, so only
//!   a kill ends the session.
//! - `mcp tools`: lists the tools of the server in `--mcp-config` (see
//!   [`crate::mcp`]) and replies with their names, comma-separated.
//! - `mcp <tool> <json arguments>`: calls that tool, between `PreToolUse` and
//!   `PostToolUse` hooks for `mcp__<server>__<tool>`, and replies with what it
//!   returned (`error: …` for a tool error).
//! - anything else: replies `echo: <prompt>`.
//!
//! **Transcript:** like Claude, it keeps one JSONL file per session, at
//! `$COS_FAKE_CLAUDE_TRANSCRIPTS/<id>.jsonl` (else under the temp folder),
//! and passes it to every hook as `transcript_path`: each prompt (a
//! multi-line one wrapped in `<pasted_content>` as a paste), each tool call
//! with its result, and each reply.
//!
//! `--resume <id>` continues session `<id>`; its `SessionStart` says
//! `resume`. `/exit` ends the session. With `COS_FAKE_CLAUDE_UNTRUSTED` set it first
//! shows the trust dialog, reading keys one at a time like the real CLI: the
//! arrows move the selection, which is redrawn, and Return confirms it.
//! `COS_FAKE_CLAUDE_TRUST_DELAY_MS` drops keys typed that soon after the dialog
//! appears, as the real CLI does while it's still starting.
//!
//! **Environment:** with `COS_FAKE_CLAUDE_ENV` naming a folder, it writes
//! its whole environment there as `<session id>.json` (a JSON object) when
//! it starts, replacing the file on every launch, so tests can see what an
//! agent was started with, a resume included.
//!
//! **Logged out:** with `COS_FAKE_CLAUDE_LOGGED_OUT` set it first shows
//! Claude's "Select login method" screen, before any trust dialog or hook,
//! until a line `login` is typed.
//!
//! **Scripted turns**, for pipeline tests: with `COS_FAKE_CLAUDE_SCRIPT`
//! naming a JSON file, a session whose prompt holds a marker `[fake:<name>]`
//! plays the script's turns for that name instead (see [`script`]). The
//! marker is remembered for the session's later turns, and across a
//! `--resume`. A bracketed paste arrives as one message, as in the real CLI.

use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::mcp::Server;
use crate::script::Script;

pub(crate) fn run(args: &[String]) {
    let flag = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
    };
    let settings: Value = flag("--settings")
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let prompt = args
        .iter()
        .position(|arg| arg == "--")
        .and_then(|at| args.get(at + 1).cloned());
    let id = flag("--session-id")
        .or_else(|| flag("--resume"))
        .unwrap_or_else(|| "fake-session".into());
    // The reviewer is the one started read-only.
    let role = if flag("--permission-mode").as_deref() == Some("plan") {
        "reviewer"
    } else {
        "worker"
    };
    dump_env(&id);
    if std::env::var_os("COS_FAKE_CLAUDE_LOGGED_OUT").is_some() && !login() {
        std::process::exit(1);
    }
    if std::env::var_os("COS_FAKE_CLAUDE_UNTRUSTED").is_some() && !trust() {
        std::process::exit(1);
    }

    let mut session = Session {
        script: Script::from_env(&id, role),
        transcript: transcript_path(&id),
        id,
        cwd: std::env::current_dir().unwrap_or_default().display().to_string(),
        settings,
        input: read_lines(),
        mcp: flag("--mcp-config").and_then(|config| Server::from_config(&config)),
        tool_uses: 0,
    };

    show(&format!(
        "fake claude · TERM={}",
        std::env::var("TERM").unwrap_or_default()
    ));
    let source = if flag("--resume").is_some() {
        "resume"
    } else {
        "startup"
    };
    show(&format!("session {} ({source})", session.id));
    session.hook("SessionStart", json!({ "source": source }));
    if let Some(prompt) = prompt {
        session.turn(&prompt);
    }
    while let Some(line) = session.next_line(None) {
        match line.trim() {
            "/exit" => {
                session.hook("SessionEnd", json!({ "reason": "prompt_input_exit" }));
                return;
            }
            // Esc with nothing to interrupt.
            "" | ESC => {}
            text => session.turn(text),
        }
    }
}

struct Session {
    id: String,
    script: Option<Script>,
    cwd: String,
    settings: Value,
    input: Receiver<String>,
    mcp: Option<Server>,
    tool_uses: u32,
    transcript: PathBuf,
}

impl Session {
    fn turn(&mut self, prompt: &str) {
        let typed = if prompt.contains('\n') {
            format!("<pasted_content id=\"fake\">\n{prompt}\n</pasted_content id=\"fake\">")
        } else {
            prompt.to_owned()
        };
        self.record("user", json!(typed));
        self.hook("UserPromptSubmit", json!({ "prompt": prompt }));
        if prompt.contains("hang forever") {
            show("working forever…");
            // Ignores everything typed, `/exit` included; only a kill ends it.
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        let scripted = self.script.as_mut().and_then(|script| script.turn(prompt));
        let reply = if let Some(turn) = scripted {
            show("working…");
            turn.play();
            turn.say
        } else if let Some(command) = prompt.strip_prefix("mcp ") {
            self.mcp_turn(command.trim())
        } else if let Some((tool, input)) = asked_tool(prompt) {
            self.permission_turn(&tool, input)
        } else if prompt.contains("permission") {
            self.permission_turn("Bash", json!({ "command": "touch x" }))
        } else {
            format!("echo: {prompt}")
        };
        show(&format!("⏺ {reply}"));
        self.record("assistant", json!([{ "type": "text", "text": reply }]));
        self.hook(
            "Stop",
            json!({ "last_assistant_message": reply, "stop_hook_active": false }),
        );
    }

    /// `tools`, or `<tool> <json arguments>`.
    fn mcp_turn(&mut self, command: &str) -> String {
        self.tool_uses += 1;
        let Some(server) = &self.mcp else {
            return "error: no --mcp-config".into();
        };
        if command == "tools" {
            return server.list().unwrap_or_else(|error| format!("error: {error}"));
        }
        let (tool, arguments) = command.split_once(' ').unwrap_or((command, "{}"));
        let arguments: Value = match serde_json::from_str(arguments.trim()) {
            Ok(arguments) => arguments,
            Err(error) => return format!("error: bad arguments: {error}"),
        };
        let tool_use = json!({
            "tool_name": format!("mcp__{}__{tool}", server.name),
            "tool_input": arguments,
            "tool_use_id": format!("toolu_mcp_{}", self.tool_uses),
        });
        self.record_tool_use(&tool_use);
        self.hook("PreToolUse", tool_use.clone());
        let reply = server
            .call(tool, arguments)
            .unwrap_or_else(|error| format!("error: {error}"));
        self.record_tool_result(&tool_use, &reply, reply.starts_with("error:"));
        let mut post = tool_use;
        post["tool_response"] = json!(reply);
        self.hook("PostToolUse", post);
        reply
    }

    fn permission_turn(&mut self, tool_name: &str, tool_input: Value) -> String {
        let tool = json!({ "tool_name": tool_name, "tool_input": tool_input });
        let mut pre = tool.clone();
        pre["tool_use_id"] = json!("toolu_1");
        self.record_tool_use(&pre);
        self.hook("PreToolUse", pre.clone());

        let what = match tool_input.get("command").and_then(Value::as_str) {
            Some(command) => format!("run {command}"),
            None => format!("use {tool_name} {tool_input}"),
        };
        show(&format!("Do you want to {what}?\r\n ❯ 1. Yes\r\n   3. No"));
        let outcome = match self.start_hook("PermissionRequest", tool.clone()) {
            Some(hook) => self.race(hook),
            None => self.answer_in_terminal(),
        };
        let allowed = outcome.starts_with("allowed");
        self.record_tool_result(&pre, &outcome, !allowed);
        if allowed {
            let mut post = tool;
            post["tool_use_id"] = json!("toolu_1");
            post["tool_response"] = json!({ "stdout": "" });
            self.hook("PostToolUse", post);
        }
        outcome
    }

    /// The hook and the terminal dialog race, as in the real CLI.
    fn race(&mut self, mut hook: Child) -> String {
        loop {
            if let Ok(Some(_)) = hook.try_wait() {
                let mut output = String::new();
                if let Some(mut stdout) = hook.stdout.take() {
                    let _ = stdout.read_to_string(&mut output);
                }
                let decision = serde_json::from_str::<Value>(output.trim())
                    .map(|value| value["hookSpecificOutput"]["decision"].clone())
                    .unwrap_or(Value::Null);
                return match decision["behavior"].as_str() {
                    Some("allow") => "allowed by hook".into(),
                    Some("deny") => format!("denied by hook: {}", decision["message"].as_str().unwrap_or("")),
                    // No answer: the dialog stays up for the person at the terminal.
                    _ => self.answer_in_terminal(),
                };
            }
            if let Some(line) = self.next_line(Some(Duration::from_millis(20))) {
                kill_group(&mut hook);
                return terminal_answer(&line);
            }
        }
    }

    fn answer_in_terminal(&mut self) -> String {
        self.next_line(None)
            .map_or_else(|| "no answer".into(), |line| terminal_answer(&line))
    }

    /// The next line typed, waiting at most `timeout`. Empty lines (from a
    /// lone Return) are skipped. `None` on timeout or when input ends.
    fn next_line(&mut self, timeout: Option<Duration>) -> Option<String> {
        loop {
            let line = match timeout {
                Some(timeout) => match self.input.recv_timeout(timeout) {
                    Ok(line) => line,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
                },
                None => self.input.recv().ok()?,
            };
            if !line.trim().is_empty() {
                return Some(line);
            }
        }
    }

    /// Appends an entry to the transcript.
    fn record(&self, kind: &str, content: Value) {
        let entry = json!({
            "type": kind,
            "isSidechain": false,
            "sessionId": self.id,
            "message": { "role": kind, "content": content },
        });
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.transcript);
        if let Ok(mut file) = file {
            let _ = writeln!(file, "{entry}");
        }
    }

    /// `tool` is a hook's `{tool_name, tool_input, tool_use_id}`.
    fn record_tool_use(&self, tool: &Value) {
        self.record(
            "assistant",
            json!([{
                "type": "tool_use",
                "id": tool["tool_use_id"],
                "name": tool["tool_name"],
                "input": tool["tool_input"],
            }]),
        );
    }

    fn record_tool_result(&self, tool: &Value, result: &str, is_error: bool) {
        self.record(
            "user",
            json!([{
                "type": "tool_result",
                "tool_use_id": tool["tool_use_id"],
                "content": result,
                "is_error": is_error,
            }]),
        );
    }

    /// Runs a hook to completion and returns what it printed.
    fn hook(&self, event: &str, extra: Value) -> String {
        let Some(mut child) = self.start_hook(event, extra) else {
            return String::new();
        };
        let mut output = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_string(&mut output);
        }
        let _ = child.wait();
        output
    }

    fn start_hook(&self, event: &str, extra: Value) -> Option<Child> {
        let command = self.settings["hooks"][event][0]["hooks"][0]["command"].as_str()?;
        let mut input = json!({
            "hook_event_name": event,
            "session_id": self.id,
            "cwd": self.cwd,
            "transcript_path": self.transcript.display().to_string(),
        });
        if let (Some(input), Value::Object(extra)) = (input.as_object_mut(), extra) {
            input.extend(extra);
        }
        let mut child = Command::new("sh")
            .args(["-c", command])
            // Its own process group, so killing the hook kills the relay too.
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = writeln!(stdin, "{input}");
        }
        Some(child)
    }
}

/// Where the session's transcript goes; the folder is made if needed.
fn transcript_path(id: &str) -> PathBuf {
    let dir = std::env::var_os("COS_FAKE_CLAUDE_TRANSCRIPTS")
        .map_or_else(|| std::env::temp_dir().join("cos-fake-claude"), PathBuf::from);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{id}.jsonl"))
}

/// Kills a hook and everything it started, as Claude does when it gives up on one.
fn kill_group(hook: &mut Child) {
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{}", hook.id())])
        .status();
    let _ = hook.wait();
}

/// The tool and input an `ask:<Tool> <arg>` prompt asks for.
fn asked_tool(prompt: &str) -> Option<(String, Value)> {
    let rest = prompt.trim().strip_prefix("ask:")?;
    let (tool, arg) = rest.split_once(' ').unwrap_or((rest, ""));
    let arg = arg.trim();
    let input = if arg.starts_with('{') {
        serde_json::from_str(arg).ok()?
    } else {
        match tool {
            "Bash" => json!({ "command": arg }),
            "Write" => json!({ "file_path": arg, "content": "x" }),
            _ => json!({ "file_path": arg }),
        }
    };
    Some((tool.to_owned(), input))
}

fn terminal_answer(line: &str) -> String {
    match line.trim() {
        "1" => "allowed in terminal".into(),
        other => format!("denied in terminal ({other})"),
    }
}

/// Lines typed into the terminal, read on a thread so a hook can run meanwhile.
/// A bracketed paste (`ESC[200~` … `ESC[201~`) comes out as one line.
/// The trust dialog. Runs before [`read_lines`] starts, with the terminal in
/// character mode so each key arrives on its own.
fn trust() -> bool {
    let stty = |args: &[&str]| {
        Command::new("stty")
            .args(args)
            .stdin(Stdio::inherit())
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let saved = stty(&["-g"]);
    stty(&["-icanon", "-echo", "min", "1"]);
    let delay = std::env::var("COS_FAKE_CLAUDE_TRUST_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(Duration::ZERO, Duration::from_millis);

    let draw = |yes: bool| {
        let (no_mark, yes_mark) = if yes { (" ", "❯") } else { ("❯", " ") };
        show(&format!(
            "\x1b[2J\x1b[HAccessing workspace:\r\n\r\n {no_mark} No, exit\r\n {yes_mark} Yes, I trust this folder"
        ));
    };
    draw(false);
    let ignore_until = Instant::now() + delay;
    let mut yes = false;
    let mut stdin = std::io::stdin().lock();
    let mut buffer = [0u8; 64];
    let confirmed = 'keys: loop {
        let Ok(read) = stdin.read(&mut buffer) else {
            break false;
        };
        if read == 0 {
            break false;
        }
        if Instant::now() < ignore_until {
            continue;
        }
        let keys = &buffer[..read];
        let mut at = 0;
        while at < keys.len() {
            match &keys[at..] {
                [b'\x1b', b'[', b'B', ..] => {
                    yes = true;
                    draw(yes);
                    at += 3;
                }
                [b'\x1b', b'[', b'A', ..] => {
                    yes = false;
                    draw(yes);
                    at += 3;
                }
                [b'\r' | b'\n', ..] => break 'keys true,
                _ => at += 1,
            }
        }
    };
    if let Some(saved) = saved.filter(|saved| !saved.is_empty()) {
        stty(&[saved.as_str()]);
    }
    let trusted = confirmed && yes;
    show(if trusted { "trusted" } else { "not trusted" });
    trusted
}

/// Writes the environment to `$COS_FAKE_CLAUDE_ENV/<id>.json`, whole, so a
/// reader never sees half a file.
fn dump_env(id: &str) {
    let Some(dir) = std::env::var_os("COS_FAKE_CLAUDE_ENV").map(PathBuf::from) else {
        return;
    };
    let env: serde_json::Map<String, Value> = std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                Value::String(value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    let _ = std::fs::create_dir_all(&dir);
    let partial = dir.join(format!("{id}.json.partial"));
    if std::fs::write(&partial, Value::Object(env).to_string()).is_ok() {
        let _ = std::fs::rename(&partial, dir.join(format!("{id}.json")));
    }
}

/// Claude's login screen, until a line `login` is typed. False if the
/// terminal closed first.
fn login() -> bool {
    show(
        "\x1b[2J\x1b[HSelect login method:\r\n\r\n \u{276f} 1. Claude account with subscription\r\n   2. Anthropic Console account",
    );
    let mut stdin = std::io::stdin().lock();
    loop {
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) if line.trim() == "login" => break,
            Ok(_) => {}
        }
    }
    show("\x1b[2J\x1b[HLogin successful.");
    true
}

fn read_lines() -> Receiver<String> {
    let (lines, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut pasting: Option<String> = None;
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let line = match pasting.take() {
                Some(mut pasted) => {
                    pasted.push('\n');
                    pasted.push_str(&line);
                    pasted
                }
                None => line,
            };
            if line.contains(PASTE_START) && !line.contains(PASTE_END) {
                pasting = Some(line);
                continue;
            }
            let line = line.replace(PASTE_START, "").replace(PASTE_END, "");
            for part in split_escape(line) {
                if lines.send(part).is_err() {
                    return;
                }
            }
        }
    });
    receiver
}

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// The terminal is in line mode, so an Esc pressed on its own arrives with
/// whatever is typed after it. Split it off so it reads as a key of its own
/// (it denies an open permission dialog). Escape sequences such as the down
/// arrow stay whole.
fn split_escape(line: String) -> Vec<String> {
    match line.strip_prefix(ESC) {
        Some(rest) if !rest.starts_with('[') => vec![ESC.to_owned(), rest.to_owned()],
        _ => vec![line],
    }
}

const ESC: &str = "\x1b";

fn show(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = write!(stdout, "{text}\r\n");
    let _ = stdout.flush();
}
