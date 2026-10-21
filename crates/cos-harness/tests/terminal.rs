//! Drives `ClaudeTerminal` and `HookServer` against `cos-fake-claude`'s
//! interactive mode, which runs hooks the way the real CLI does.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cos_harness::claude::hooks::{Hook, HookEvent, HookServer};
use cos_harness::{
    ClaudeOptions, ClaudeTerminal, Decision, HookTarget, TerminalEvent, TerminalOptions, TerminalUpdate,
};
use tokio::sync::mpsc::UnboundedReceiver;

const WAIT: Duration = Duration::from_secs(10);

const FAKE: &str = env!("CARGO_BIN_EXE_cos-fake-claude");

struct Harness {
    terminal: ClaudeTerminal,
    output: UnboundedReceiver<TerminalEvent>,
    hooks: UnboundedReceiver<HookEvent>,
    _server: HookServer,
    _dir: tempfile::TempDir,
}

/// Starts the fake in a terminal with hooks reporting to a fresh server.
/// `socket` overrides where the hooks report, to test cos being absent.
fn start(prompt: &str, configure: impl FnOnce(&mut TerminalOptions), socket: Option<PathBuf>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let (server, hooks) = HookServer::bind(dir.path().join("hooks.sock")).unwrap();

    let mut claude = ClaudeOptions::new(dir.path());
    claude.binary = FAKE.into();
    let mut options = TerminalOptions::new(claude);
    options.prompt = Some(prompt.to_owned());
    options.hooks = Some(HookTarget {
        command: format!("'{FAKE}' hook"),
        socket: socket.unwrap_or_else(|| server.path().to_owned()),
    });
    configure(&mut options);

    let (terminal, output) = ClaudeTerminal::spawn(options).unwrap();
    Harness {
        terminal,
        output,
        hooks,
        _server: server,
        _dir: dir,
    }
}

impl Harness {
    /// The next hook, skipping withdrawals.
    async fn hook(&mut self) -> Hook {
        loop {
            match self.next_hook_event().await {
                HookEvent::Fired(hook) => return hook,
                HookEvent::Withdrawn { .. } => {}
            }
        }
    }

    async fn next_hook_event(&mut self) -> HookEvent {
        tokio::time::timeout(WAIT, self.hooks.recv())
            .await
            .unwrap_or_else(|_| panic!("no hook; screen:\n{}", self.terminal.screen()))
            .expect("hook server stopped")
    }

    /// Hooks up to and including `last`, as event names.
    async fn hooks_until(&mut self, last: &str) -> Vec<String> {
        let mut seen = Vec::new();
        loop {
            let hook = self.hook().await;
            seen.push(hook.event.clone());
            if hook.event == last {
                return seen;
            }
        }
    }

    async fn screen_shows(&mut self, text: &str) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !self.terminal.screen().contains(text) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "screen never showed {text:?}:\n{}",
                self.terminal.screen()
            );
            let _ = tokio::time::timeout(Duration::from_millis(50), self.output.recv()).await;
        }
    }

    async fn exit(mut self) -> Option<i32> {
        self.terminal.submit("/exit").await.unwrap();
        self.exit_code().await
    }

    async fn exit_code(&mut self) -> Option<i32> {
        loop {
            let event = tokio::time::timeout(WAIT, self.output.recv())
                .await
                .expect("no exit")
                .expect("output ended without Exited");
            if let TerminalEvent::Exited { code } = event {
                return code;
            }
        }
    }
}

#[tokio::test]
async fn follows_a_session_through_hooks() {
    let mut harness = start("hello", |_| {}, None);
    let session_id = harness.terminal.session_id().to_owned();

    let start = harness.hook().await;
    assert_eq!(start.event, "SessionStart");
    assert_eq!(start.session_id.as_deref(), Some(session_id.as_str()));
    assert!(!start.awaits_answer());
    assert_eq!(harness.hooks_until("Stop").await, ["UserPromptSubmit", "Stop"]);
    harness.screen_shows("echo: hello").await;
    harness.screen_shows("TERM=xterm-256color").await;

    harness.terminal.submit("again").await.unwrap();
    assert_eq!(harness.hooks_until("Stop").await, ["UserPromptSubmit", "Stop"]);
    harness.screen_shows("echo: again").await;

    harness.terminal.submit("/exit").await.unwrap();
    assert_eq!(harness.hook().await.event, "SessionEnd");
    assert_eq!(harness.exit_code().await, Some(0));
}

async fn permission_hook(harness: &mut Harness) -> Hook {
    assert_eq!(
        harness.hooks_until("PreToolUse").await,
        ["SessionStart", "UserPromptSubmit", "PreToolUse"]
    );
    let hook = harness.hook().await;
    assert_eq!(hook.event, "PermissionRequest");
    assert!(hook.awaits_answer());
    assert_eq!(hook.tool_name(), Some("Bash"));
    assert_eq!(hook.tool_input().unwrap()["command"], "touch x");
    hook
}

#[tokio::test]
async fn cos_allows_through_the_hook() {
    let mut harness = start("needs permission", |_| {}, None);
    permission_hook(&mut harness).await.answer(&Decision::Allow);

    assert_eq!(harness.hooks_until("Stop").await, ["PostToolUse", "Stop"]);
    harness.screen_shows("allowed by hook").await;
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn cos_denies_through_the_hook() {
    let mut harness = start("needs permission", |_| {}, None);
    permission_hook(&mut harness).await.answer(&Decision::Deny {
        message: "not now".into(),
    });

    assert_eq!(harness.hooks_until("Stop").await, ["Stop"]);
    harness.screen_shows("denied by hook: not now").await;
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn an_answer_typed_in_the_terminal_withdraws_the_hook() {
    let mut harness = start("needs permission", |_| {}, None);
    let held = permission_hook(&mut harness).await;
    let id = held.id;

    harness.terminal.write(b"1\r").unwrap();

    let mut withdrawn = false;
    let mut events = Vec::new();
    while !(withdrawn && events.last().map(String::as_str) == Some("Stop")) {
        match harness.next_hook_event().await {
            HookEvent::Withdrawn { id: gone } => {
                assert_eq!(gone, id);
                withdrawn = true;
            }
            HookEvent::Fired(hook) => events.push(hook.event.clone()),
        }
    }
    assert_eq!(events, ["PostToolUse", "Stop"]);
    harness.screen_shows("allowed in terminal").await;
    drop(held);
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn an_unanswered_hook_leaves_the_dialog_to_the_terminal() {
    let mut harness = start("needs permission", |_| {}, None);
    // Dropping the hook without an answer: Claude gets no decision from cos.
    drop(permission_hook(&mut harness).await);

    harness.screen_shows("Do you want to run touch x?").await;
    harness.terminal.write(b"3\r").unwrap();
    assert_eq!(harness.hooks_until("Stop").await, ["Stop"]);
    harness.screen_shows("denied in terminal").await;
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn without_cos_the_session_still_works() {
    let mut harness = start(
        "needs permission",
        |_| {},
        Some(Path::new("/nonexistent/hooks.sock").into()),
    );

    harness.screen_shows("Do you want to run touch x?").await;
    harness.terminal.write(b"1\r").unwrap();
    harness.screen_shows("allowed in terminal").await;
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn answers_the_trust_dialog_when_told_to() {
    let mut harness = start(
        "hello",
        |options| {
            options
                .claude
                .env
                .push(("COS_FAKE_CLAUDE_UNTRUSTED".into(), "1".into()))
        },
        None,
    );

    harness.screen_shows("Yes, I trust this folder").await;
    assert!(harness.terminal.asks_for_trust());
    harness.terminal.trust_folder().await.unwrap();

    assert_eq!(
        harness.hooks_until("Stop").await,
        ["SessionStart", "UserPromptSubmit", "Stop"]
    );
    harness.screen_shows("echo: hello").await;
    assert_eq!(harness.exit().await, Some(0));
}

#[tokio::test]
async fn kill_ends_the_session() {
    let mut harness = start("hello", |_| {}, None);
    harness.screen_shows("echo: hello").await;

    harness.terminal.kill();

    assert_eq!(harness.exit_code().await, None);
}

#[tokio::test]
async fn resize_reaches_the_screen() {
    let harness = start("hello", |_| {}, None);
    harness.terminal.resize(10, 50).unwrap();
    let screen = harness.terminal.screen();
    assert!(screen.lines().all(|line| line.chars().count() <= 50), "{screen}");
}

#[tokio::test]
async fn watchers_get_a_snapshot_then_every_update() {
    let mut harness = start("hello", |_| {}, None);
    harness.screen_shows("echo: hello").await;

    let mut attach = harness.terminal.attach();
    assert_eq!((attach.rows, attach.cols), (40, 120));
    assert_eq!(attach.exited, None);
    // Fed to a fresh emulator, the snapshot redraws the same screen.
    let size = |value: u16| std::num::NonZeroU16::new(value).unwrap();
    let mut copy = vt100::Parser::new(size(attach.rows), size(attach.cols), 0);
    copy.process(&attach.snapshot);
    assert_eq!(copy.screen().contents(), harness.terminal.screen());

    harness.terminal.resize(30, 100).unwrap();
    harness.terminal.submit("again").await.unwrap();
    let mut saw_resize = false;
    let mut exiting = false;
    let code = loop {
        let update = tokio::time::timeout(WAIT, attach.updates.recv())
            .await
            .expect("no update")
            .expect("updates ended");
        match update {
            TerminalUpdate::Resize { rows, cols } => {
                assert_eq!((rows, cols), (30, 100));
                copy.screen_mut().set_size(size(rows), size(cols));
                saw_resize = true;
            }
            TerminalUpdate::Output(bytes) => {
                copy.process(&bytes);
                if !exiting && copy.screen().contents().contains("echo: again") {
                    exiting = true;
                    harness.terminal.submit("/exit").await.unwrap();
                }
            }
            TerminalUpdate::Exited { code } => break code,
        }
    };
    assert!(saw_resize);
    assert_eq!(code, Some(0));

    // Watching after the exit: the snapshot, and the exit straight away.
    harness.exit_code().await;
    let late = harness.terminal.attach();
    assert_eq!(late.exited, Some(Some(0)));
    assert_eq!(harness.terminal.exit_status(), Some(Some(0)));
}
