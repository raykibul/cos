//! The fake's interactive mode: a stand-in for `claude` running in a terminal.
//!
//! It runs the hooks passed in `--settings` the way Claude does (a shell
//! command, the hook input as JSON on stdin, the answer on stdout), prints a
//! plain-text "UI", and reads lines typed into the terminal.
//!
//! What a prompt does:
//! - contains `permission`: Bash `touch x` that needs approval. The
//!   `PermissionRequest` hook runs while the terminal dialog is up, like the
//!   real CLI: a hook answer settles it, and so does typing `1` (allow) or `3`
//!   (deny), which kills the hook.
//! - contains `hang forever`: never finishes and ignores whatever is typed, so only
//!   a kill ends the session.
//! - anything else: replies `echo: <prompt>`.
//!
//! `--resume <id>` continues session `<id>`; its `SessionStart` says
//! `resume`. `/exit` ends the session. With `COS_FAKE_CLAUDE_UNTRUSTED` set it first
//! shows the trust dialog and only carries on after a down arrow and Return.

use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde_json::{Value, json};

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
    let mut session = Session {
        id: flag("--session-id")
            .or_else(|| flag("--resume"))
            .unwrap_or_else(|| "fake-session".into()),
        cwd: std::env::current_dir().unwrap_or_default().display().to_string(),
        settings,
        input: read_lines(),
    };

    if std::env::var_os("COS_FAKE_CLAUDE_UNTRUSTED").is_some() && !session.trust() {
        std::process::exit(1);
    }

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
    cwd: String,
    settings: Value,
    input: Receiver<String>,
}

impl Session {
    fn trust(&mut self) -> bool {
        show("Accessing workspace:\r\n\r\n ❯ No, exit\r\n   Yes, I trust this folder");
        // The terminal is in line mode, so the arrow key arrives with the Return.
        let trusted = self.next_line(None).is_some_and(|line| line.contains("\x1b[B"));
        show(if trusted { "trusted" } else { "not trusted" });
        trusted
    }

    fn turn(&mut self, prompt: &str) {
        self.hook("UserPromptSubmit", json!({ "prompt": prompt }));
        if prompt.contains("hang forever") {
            show("working forever…");
            // Ignores everything typed, `/exit` included; only a kill ends it.
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        let reply = if prompt.contains("permission") {
            self.permission_turn()
        } else {
            format!("echo: {prompt}")
        };
        show(&format!("⏺ {reply}"));
        self.hook(
            "Stop",
            json!({ "last_assistant_message": reply, "stop_hook_active": false }),
        );
    }

    fn permission_turn(&mut self) -> String {
        let tool = json!({ "tool_name": "Bash", "tool_input": { "command": "touch x" } });
        let mut pre = tool.clone();
        pre["tool_use_id"] = json!("toolu_1");
        self.hook("PreToolUse", pre);

        show("Do you want to run touch x?\r\n ❯ 1. Yes\r\n   3. No");
        let outcome = match self.start_hook("PermissionRequest", tool) {
            Some(hook) => self.race(hook),
            None => self.answer_in_terminal(),
        };
        let allowed = outcome.starts_with("allowed");
        if allowed {
            let mut post = json!({ "tool_name": "Bash", "tool_input": { "command": "touch x" } });
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
            "transcript_path": format!("/fake/{}.jsonl", self.id),
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

/// Kills a hook and everything it started, as Claude does when it gives up on one.
fn kill_group(hook: &mut Child) {
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{}", hook.id())])
        .status();
    let _ = hook.wait();
}

fn terminal_answer(line: &str) -> String {
    match line.trim() {
        "1" => "allowed in terminal".into(),
        other => format!("denied in terminal ({other})"),
    }
}

/// Lines typed into the terminal, read on a thread so a hook can run meanwhile.
fn read_lines() -> Receiver<String> {
    let (lines, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            for part in split_escape(line) {
                if lines.send(part).is_err() {
                    return;
                }
            }
        }
    });
    receiver
}

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
