//! Scripted turns for the fake's interactive mode, so pipeline tests can have
//! a "worker" that edits files and commits, and a "reviewer" that gives a
//! verdict, without a real Claude.
//!
//! `COS_FAKE_CLAUDE_SCRIPT` names a JSON file:
//!
//! ```json
//! {"a": {"worker":   [{"sh": ["echo hi > a.txt", "git add -A", "git commit -qm a"], "say": "Done.\nDONE"}],
//!        "reviewer": [{"say": "VERDICT: APPROVE"}]}}
//! ```
//!
//! A prompt holding `[fake:a]` plays script `a`. Turns are taken in order,
//! per name and role, across every session; the last one repeats once
//! they run out. A turn runs its `sh` commands in the session's folder (with
//! `sh -c`), then ends with `say` as its last message. `"hang": true` never
//! ends the turn; `"exit": N` exits with that code instead of answering.
//!
//! The role is `reviewer` for a session started with `--permission-mode
//! plan`, else `worker`. Every prompt is appended to `prompts.jsonl` next to
//! the script, as `{"name", "role", "session", "prompt"}`, for tests to read.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

pub(crate) const SCRIPT_ENV: &str = "COS_FAKE_CLAUDE_SCRIPT";

pub(crate) struct Script {
    path: PathBuf,
    session: String,
    role: &'static str,
    /// The script name this session plays, once a prompt named one.
    name: Option<String>,
}

pub(crate) struct Turn {
    sh: Vec<String>,
    pub say: String,
    hang: bool,
    exit: Option<i32>,
}

impl Script {
    pub(crate) fn from_env(session: &str, role: &'static str) -> Option<Self> {
        let path = PathBuf::from(std::env::var_os(SCRIPT_ENV)?);
        let name = std::fs::read_to_string(session_file(&path, session)).ok();
        Some(Self {
            path,
            session: session.to_owned(),
            role,
            name,
        })
    }

    /// The next scripted turn for this prompt, if the session plays a script.
    pub(crate) fn turn(&mut self, prompt: &str) -> Option<Turn> {
        if let Some(name) = marker(prompt) {
            let _ = std::fs::write(session_file(&self.path, &self.session), &name);
            self.name = Some(name);
        }
        let name = self.name.clone()?;
        self.log(&name, prompt);
        let script: Value = serde_json::from_str(&std::fs::read_to_string(&self.path).ok()?).ok()?;
        let turns = script[&name][self.role].as_array()?.clone();
        if turns.is_empty() {
            return None;
        }
        let counter = self.dir().join(format!("{name}.{}.count", self.role));
        let taken: usize = std::fs::read_to_string(&counter)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let _ = std::fs::write(&counter, (taken + 1).to_string());
        let turn = &turns[taken.min(turns.len() - 1)];
        Some(Turn {
            sh: turn["sh"]
                .as_array()
                .map(|commands| {
                    commands
                        .iter()
                        .filter_map(|command| command.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            say: turn["say"].as_str().unwrap_or("DONE").to_owned(),
            hang: turn["hang"].as_bool().unwrap_or(false),
            exit: turn["exit"].as_i64().map(|code| code as i32),
        })
    }

    fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    fn log(&self, name: &str, prompt: &str) {
        let line = json!({ "name": name, "role": self.role, "session": self.session, "prompt": prompt });
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir().join("prompts.jsonl"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

impl Turn {
    /// Runs the commands, then hangs or exits if the turn says so.
    pub(crate) fn play(&self) {
        for command in &self.sh {
            let _ = Command::new("sh").args(["-c", command]).status();
        }
        if let Some(code) = self.exit {
            std::process::exit(code);
        }
        if self.hang {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
    }
}

fn session_file(script: &Path, session: &str) -> PathBuf {
    script
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("session-{session}"))
}

/// `name` from the first `[fake:name]` in the prompt.
fn marker(prompt: &str) -> Option<String> {
    let start = prompt.find("[fake:")? + "[fake:".len();
    let len = prompt[start..].find(']')?;
    let name = prompt[start..start + len].trim();
    (!name.is_empty()).then(|| name.to_owned())
}
