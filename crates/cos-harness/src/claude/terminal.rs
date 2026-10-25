//! Claude Code's interactive UI running in a pseudo-terminal that cos owns.
//!
//! The terminal is for people: cos keeps the screen and can stream its bytes
//! to an app. What happens in the session (tools, permission requests, turns)
//! reaches cos through hooks instead (see [`super::hooks`]), so nothing here
//! reads the screen to find out what Claude is doing.

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{broadcast, mpsc};

use super::{ClaudeOptions, SESSION_MARKERS, TERMINAL_ENV, hooks, lock};
use crate::HarnessError;

/// How long to keep reading output after the process has exited.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// Lines of scrollback kept above the screen.
const SCROLLBACK: usize = 5_000;
/// Pause between typing a message and pressing Return, so the UI sees typing
/// rather than a paste ending in a newline.
const SUBMIT_PAUSE: Duration = Duration::from_millis(150);
/// The answer that marks Claude's "do you trust this folder" dialog.
const TRUST_ANSWER: &str = "Yes, I trust this folder";
/// How often the trust answer looks at the screen.
const POLL: Duration = Duration::from_millis(50);
/// How long the screen must stay unchanged before the first key.
const QUIET: Duration = Duration::from_millis(400);
/// The most the trust answer waits for the screen to settle.
const QUIET_MAX: Duration = Duration::from_secs(3);
/// How long a down arrow gets to show up as "Yes" selected before it's sent again.
const TRUST_SELECT_WAIT: Duration = Duration::from_millis(1500);
const TRUST_TRIES: usize = 4;
/// Updates buffered per [`ClaudeTerminal::attach`] watcher. One that falls
/// further behind gets `Lagged` and should attach again for a fresh snapshot.
const UPDATE_BUFFER: usize = 1024;

#[derive(Debug, Clone)]
pub struct TerminalOptions {
    pub claude: ClaudeOptions,
    /// Typed into the prompt and submitted as soon as Claude starts.
    pub prompt: Option<String>,
    /// Where hooks report to. Without it cos sees only the screen.
    pub hooks: Option<HookTarget>,
    /// Load the user's own settings: their allow rules, hooks, MCP servers and
    /// theme. Without it only the repo's settings apply, as in headless sessions.
    pub user_settings: bool,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone)]
pub struct HookTarget {
    /// A shell command that runs the relay, such as `'/usr/local/bin/cos' hook`.
    pub command: String,
    pub socket: PathBuf,
}

impl TerminalOptions {
    pub fn new(claude: ClaudeOptions) -> Self {
        Self {
            claude,
            prompt: None,
            hooks: None,
            user_settings: false,
            rows: 40,
            cols: 120,
        }
    }

    /// The command-line arguments for interactive `claude`.
    pub fn args(&self) -> Vec<String> {
        let claude = &self.claude;
        let mut args: Vec<String> = vec![
            if claude.resume { "--resume" } else { "--session-id" }.into(),
            claude.session_id.clone(),
            "--permission-mode".into(),
            claude.permission_mode.as_str().into(),
        ];
        if !self.user_settings {
            args.extend([
                "--setting-sources".into(),
                "project".into(),
                "--strict-mcp-config".into(),
            ]);
        }
        if let Some(hooks) = &self.hooks {
            args.extend(["--settings".into(), hooks::settings(&hooks.command).to_string()]);
        }
        if let Some(model) = &claude.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if !claude.allowed_tools.is_empty() {
            args.extend(["--allowedTools".into(), claude.allowed_tools.join(",")]);
        }
        if let Some(prompt) = &claude.append_system_prompt {
            args.extend(["--append-system-prompt".into(), prompt.clone()]);
        }
        if let Some(config) = &claude.mcp_config {
            args.extend(["--mcp-config".into(), config.clone()]);
        }
        if let Some(prompt) = &self.prompt {
            // `--` so a prompt starting with `-` isn't read as a flag.
            args.extend(["--".into(), prompt.clone()]);
        }
        args
    }
}

/// What a watcher of the terminal sees after its snapshot, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalUpdate {
    /// Bytes Claude wrote, exactly as read from the terminal.
    Output(Arc<[u8]>),
    /// Someone resized the terminal.
    Resize { rows: u16, cols: u16 },
    /// The process exited; nothing follows. `code` is `None` if it was killed.
    Exited { code: Option<i32> },
}

/// A consistent view of the terminal for someone who starts watching it:
/// the screen as it is now, then every update after it, with no gap or
/// repeat in between.
#[derive(Debug)]
pub struct TerminalAttach {
    pub rows: u16,
    pub cols: u16,
    /// Escape sequences that redraw the current screen on an empty terminal
    /// of the same size: see [`snapshot`].
    pub snapshot: Vec<u8>,
    /// Set if the process has already exited; then no updates follow.
    pub exited: Option<Option<i32>>,
    pub updates: broadcast::Receiver<TerminalUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Bytes Claude wrote to the terminal, escape sequences and all.
    Output(Vec<u8>),
    /// The process exited. Always the last event. `code` is `None` if it was
    /// killed by a signal.
    Exited { code: Option<i32> },
}

/// A running interactive Claude in a pseudo-terminal.
///
/// Dropping it kills the process.
pub struct ClaudeTerminal {
    session_id: String,
    pid: Option<u32>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    screen: Arc<Mutex<Screen>>,
}

/// The emulated screen, and the channel that tells watchers what changed.
/// Updates are sent while the screen is locked, so [`ClaudeTerminal::attach`]
/// sees each one either in its snapshot or on its channel, never both.
struct Screen {
    parser: vt100::Parser<Replies>,
    updates: broadcast::Sender<TerminalUpdate>,
    exited: Option<Option<i32>>,
}

impl ClaudeTerminal {
    pub fn spawn(
        options: TerminalOptions,
    ) -> Result<(Self, mpsc::UnboundedReceiver<TerminalEvent>), HarnessError> {
        let mut command = CommandBuilder::new(&options.claude.binary);
        command.args(options.args());
        command.cwd(&options.claude.cwd);
        // The daemon's environment, then the extra variables, then cos's
        // own, so nothing in `env` can replace what cos relies on (see
        // `reserved_env_names`).
        for (key, value) in &options.claude.env {
            command.env(key, value);
        }
        for name in SESSION_MARKERS {
            command.env_remove(name);
        }
        for (name, value) in TERMINAL_ENV {
            command.env(name, value);
        }
        if let Some(hooks) = &options.hooks {
            command.env(hooks::SOCKET_ENV, &hooks.socket);
        }
        Self::start(command, options.claude.session_id, options.rows, options.cols)
    }

    /// Runs any program in a terminal of its own, such as a login shell, in
    /// `cwd`. It gets the daemon's environment with `env` on top, the
    /// terminal's variables, and none of Claude's session markers. Its
    /// session id is empty.
    pub fn spawn_program(
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        rows: u16,
        cols: u16,
    ) -> Result<(Self, mpsc::UnboundedReceiver<TerminalEvent>), HarnessError> {
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.cwd(cwd);
        for (key, value) in env {
            command.env(key, value);
        }
        for name in SESSION_MARKERS {
            command.env_remove(name);
        }
        for (name, value) in TERMINAL_ENV {
            command.env(name, value);
        }
        Self::start(command, String::new(), rows, cols)
    }

    fn start(
        command: CommandBuilder,
        session_id: String,
        rows: u16,
        cols: u16,
    ) -> Result<(Self, mpsc::UnboundedReceiver<TerminalEvent>), HarnessError> {
        let program = command
            .get_argv()
            .first()
            .map(|arg| arg.to_string_lossy().into_owned())
            .unwrap_or_default();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pty_error)?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| HarnessError::Terminal(format!("could not start {program}: {error:#}")))?;
        // Only the child should hold the terminal's other end, so reads end
        // once it exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().map_err(pty_error)?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(pty_error)?));
        let killer = child.clone_killer();
        let pid = child.process_id();
        let screen = Arc::new(Mutex::new(Screen {
            parser: vt100::Parser::new_with_callbacks(size(rows), size(cols), SCROLLBACK, Replies::default()),
            updates: broadcast::channel(UPDATE_BUFFER).0,
            exited: None,
        }));

        let (events, receiver) = mpsc::unbounded_channel();
        let (reader_done, reader_finished) = std::sync::mpsc::channel::<()>();
        {
            let screen = screen.clone();
            let writer = writer.clone();
            let events = events.clone();
            std::thread::Builder::new()
                .name("claude-pty-read".into())
                .spawn(move || {
                    read_output(reader, &screen, &writer, &events);
                    drop(reader_done);
                })
                .map_err(|error| HarnessError::Terminal(error.to_string()))?;
        }
        let exit_screen = screen.clone();
        std::thread::Builder::new()
            .name("claude-pty-wait".into())
            .spawn(move || {
                let status = child.wait();
                // A tool's own child process can keep the terminal open after
                // Claude exits, so don't wait for end-of-file forever.
                let _ = reader_finished.recv_timeout(DRAIN_TIMEOUT);
                let code = status.ok().and_then(|status| {
                    status
                        .signal()
                        .is_none()
                        .then(|| i32::try_from(status.exit_code()).unwrap_or(i32::MAX))
                });
                {
                    let mut screen = lock(&exit_screen);
                    screen.exited = Some(code);
                    let _ = screen.updates.send(TerminalUpdate::Exited { code });
                }
                let _ = events.send(TerminalEvent::Exited { code });
            })
            .map_err(|error| HarnessError::Terminal(error.to_string()))?;

        let terminal = Self {
            session_id,
            pid,
            writer,
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            screen,
        };
        Ok((terminal, receiver))
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Sends keystrokes, exactly as if typed.
    pub fn write(&self, bytes: &[u8]) -> Result<(), HarnessError> {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes).map_err(HarnessError::Write)?;
        writer.flush().map_err(HarnessError::Write)
    }

    /// Types `text` into Claude's prompt and presses Return.
    pub async fn submit(&self, text: &str) -> Result<(), HarnessError> {
        self.write(text.as_bytes())?;
        tokio::time::sleep(SUBMIT_PAUSE).await;
        self.write(b"\r")
    }

    /// Pastes `text` into Claude's prompt as one bracketed paste and presses
    /// Return, so a message with several lines is sent as one message rather
    /// than one per line.
    pub async fn submit_pasted(&self, text: &str) -> Result<(), HarnessError> {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        self.write(&bytes)?;
        tokio::time::sleep(SUBMIT_PAUSE).await;
        self.write(b"\r")
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), HarnessError> {
        lock(&self.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pty_error)?;
        let mut screen = lock(&self.screen);
        screen.parser.screen_mut().set_size(size(rows), size(cols));
        let _ = screen.updates.send(TerminalUpdate::Resize { rows, cols });
        Ok(())
    }

    /// The text currently on screen, one line per row.
    pub fn screen(&self) -> String {
        lock(&self.screen).parser.screen().contents()
    }

    /// Escape sequences that redraw the current screen, colours and all.
    pub fn screen_formatted(&self) -> Vec<u8> {
        snapshot(lock(&self.screen).parser.screen())
    }

    /// The terminal's size as `(rows, cols)`.
    pub fn size(&self) -> (u16, u16) {
        let screen = lock(&self.screen);
        let (rows, cols) = screen.parser.screen().size();
        (rows.get(), cols.get())
    }

    /// The cursor as `(row, col)`, counted from 0.
    pub fn cursor(&self) -> (u16, u16) {
        lock(&self.screen).parser.screen().cursor_position()
    }

    /// `Some(code)` once the process has exited.
    pub fn exit_status(&self) -> Option<Option<i32>> {
        lock(&self.screen).exited
    }

    /// Starts watching the terminal: the screen now, then every update.
    /// Watching never slows the terminal down: a watcher that falls more
    /// than [`UPDATE_BUFFER`] updates behind gets `Lagged` instead.
    pub fn attach(&self) -> TerminalAttach {
        let screen = lock(&self.screen);
        let (rows, cols) = screen.parser.screen().size();
        TerminalAttach {
            rows: rows.get(),
            cols: cols.get(),
            snapshot: snapshot(screen.parser.screen()),
            exited: screen.exited,
            updates: screen.updates.subscribe(),
        }
    }

    /// True while Claude asks whether to trust the folder. It asks before
    /// anything else, hooks included, in a folder nobody has trusted yet. A
    /// worktree of a trusted repo counts as trusted.
    ///
    /// This is the one place cos reads the screen; if Claude rewords the
    /// dialog, the person at the terminal can still answer it.
    pub fn asks_for_trust(&self) -> bool {
        self.screen().contains(TRUST_ANSWER)
    }

    /// True while Claude shows its login screen, or says it isn't logged
    /// in, which is what a session in a Claude config dir (`CLAUDE_CONFIG_DIR`)
    /// nobody has logged into shows first. Read like [`Self::asks_for_trust`]:
    /// this and the trust dialog are the only screen reads.
    pub fn asks_for_login(&self) -> bool {
        is_login_screen(&self.screen())
    }

    /// Answers the trust dialog with yes. Only for folders the user has told
    /// cos to trust: Claude remembers the answer for every later session.
    ///
    /// Claude draws the dialog a moment before it reads keys, and drops keys
    /// typed before that. So this waits for the screen to settle, moves to
    /// "Yes" (one row down from "No, exit"), and presses Return only once
    /// "Yes" shows as selected. Return on "No, exit" would end the session.
    pub async fn trust_folder(&self) -> Result<(), HarnessError> {
        self.wait_until_quiet().await;
        for _ in 0..TRUST_TRIES {
            self.write(b"\x1b[B")?;
            let deadline = tokio::time::Instant::now() + TRUST_SELECT_WAIT;
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(POLL).await;
                if self.trust_answer_selected() {
                    tokio::time::sleep(SUBMIT_PAUSE).await;
                    return self.write(b"\r");
                }
            }
        }
        Err(HarnessError::Terminal(
            "the trust dialog never selected \"Yes, I trust this folder\"".into(),
        ))
    }

    /// Whether the trust dialog's selection marker is on "Yes".
    fn trust_answer_selected(&self) -> bool {
        self.screen()
            .lines()
            .any(|line| line.contains(TRUST_ANSWER) && line.contains('❯'))
    }

    /// Waits until the screen has stopped changing for a moment, up to a few
    /// seconds.
    async fn wait_until_quiet(&self) {
        let deadline = tokio::time::Instant::now() + QUIET_MAX;
        let mut last = self.screen();
        let mut quiet = Duration::ZERO;
        while quiet < QUIET && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
            let now = self.screen();
            if now == last {
                quiet += POLL;
            } else {
                quiet = Duration::ZERO;
                last = now;
            }
        }
    }

    /// Kills the process right away.
    pub fn kill(&self) {
        let _ = lock(&self.killer).kill();
    }
}

impl Drop for ClaudeTerminal {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Text on Claude's first-run and login screens: the theme picker a new
/// config dir starts with, the login method picker, and the notice a
/// logged-out session shows.
const LOGIN_MARKERS: [&str; 3] = [
    "Select login method",
    "Choose the text style",
    "Not logged in · Please run /login",
];

fn is_login_screen(screen: &str) -> bool {
    LOGIN_MARKERS.iter().any(|marker| screen.contains(marker))
}

fn size(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap_or(NonZeroU16::MIN)
}

/// Escape sequences that bring an empty terminal of the same size to this
/// screen's state: the alternate screen if it is in use, then vt100's
/// `state_formatted` (contents, cursor position and visibility, then the
/// input modes: application keypad and cursor keys, bracketed paste, mouse
/// reporting, cursor style). Without the input modes, pastes and arrow keys
/// typed after a reconnect would reach Claude the wrong way.
fn snapshot(screen: &vt100::Screen) -> Vec<u8> {
    let mut bytes = Vec::new();
    if screen.alternate_screen() {
        bytes.extend_from_slice(b"\x1b[?1049h");
    }
    bytes.extend_from_slice(screen.state_formatted().as_bytes());
    bytes
}

/// Feeds the screen and forwards output until the terminal closes.
fn read_output(
    mut reader: Box<dyn Read + Send>,
    screen: &Mutex<Screen>,
    writer: &Mutex<Box<dyn Write + Send>>,
    events: &mpsc::UnboundedSender<TerminalEvent>,
) {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let bytes = &buffer[..read];
        let replies = {
            let mut screen = lock(screen);
            screen.parser.process(bytes);
            // Nobody watching is fine.
            let _ = screen.updates.send(TerminalUpdate::Output(bytes.into()));
            std::mem::take(&mut screen.parser.callbacks_mut().0)
        };
        if !replies.is_empty() {
            let mut writer = lock(writer);
            let _ = writer.write_all(&replies).and_then(|()| writer.flush());
        }
        let _ = events.send(TerminalEvent::Output(bytes.to_vec()));
    }
}

/// Answers the questions a program asks its terminal (where is the cursor,
/// what kind of terminal is this, what colour is the background), the way a
/// real terminal would. Without answers some programs pause or guess.
#[derive(Default)]
struct Replies(Vec<u8>);

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params
            .first()
            .and_then(|param| param.first())
            .copied()
            .unwrap_or(0);
        match (i1, c, first) {
            // Cursor position report.
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.0.extend(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            // Status report: all good.
            (None, 'n', 5) => self.0.extend(b"\x1b[0n"),
            // Primary and secondary device attributes, as xterm answers them.
            (None, 'c', 0) => self.0.extend(b"\x1b[?62;22c"),
            (Some(b'>'), 'c', 0) => self.0.extend(b"\x1b[>0;276;0c"),
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        // Foreground (10) and background (11) colour queries. Claude picks
        // its light or dark theme from the background.
        if let [code @ (b"10" | b"11"), b"?"] = params {
            let colour = if *code == b"10" {
                "ffff/ffff/ffff"
            } else {
                "0000/0000/0000"
            };
            let code = std::str::from_utf8(code).unwrap_or_default();
            self.0
                .extend(format!("\x1b]{code};rgb:{colour}\x1b\\").as_bytes());
        }
    }
}

/// `portable-pty` reports errors as `anyhow::Error`; `{:#}` keeps their causes.
fn pty_error(error: impl std::fmt::Display) -> HarnessError {
    HarnessError::Terminal(format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn args_for_a_session_cos_follows_through_hooks() {
        let mut options = TerminalOptions::new(ClaudeOptions::new("/tmp"));
        options.prompt = Some("-start with a dash".into());
        options.hooks = Some(HookTarget {
            command: "'/bin/cos' hook".into(),
            socket: "/tmp/hooks.sock".into(),
        });
        let args = options.args();

        assert!(has_pair(&args, "--session-id", &options.claude.session_id));
        assert!(has_pair(&args, "--setting-sources", "project"));
        assert!(!args.contains(&"-p".to_owned()));
        assert!(!args.contains(&"--permission-prompt-tool".to_owned()));
        let settings = args.windows(2).find(|pair| pair[0] == "--settings").unwrap();
        let settings: serde_json::Value = serde_json::from_str(&settings[1]).unwrap();
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/bin/cos' hook"
        );
        assert_eq!(&args[args.len() - 2..], ["--", "-start with a dash"]);
    }

    #[test]
    fn user_settings_drop_the_project_only_flags() {
        let mut options = TerminalOptions::new(ClaudeOptions::new("/tmp"));
        options.user_settings = true;
        let args = options.args();

        assert!(!args.contains(&"--setting-sources".to_owned()));
        assert!(!args.contains(&"--strict-mcp-config".to_owned()));
        assert!(!args.contains(&"--".to_owned()));
    }

    #[test]
    fn spots_the_login_screens() {
        assert!(is_login_screen(
            " Select login method:\n > 1. Claude account with subscription"
        ));
        assert!(is_login_screen(
            "Let's get started.\n Choose the text style that looks best"
        ));
        assert!(is_login_screen("  Not logged in · Please run /login"));
        assert!(!is_login_screen("> how do I log in to the app?\n"));
    }

    #[test]
    fn answers_terminal_queries() {
        let mut parser = vt100::Parser::new_with_callbacks(size(24), size(80), 0, Replies::default());
        parser.process(b"hi\x1b[6n\x1b[c\x1b]11;?\x07");

        let replies = String::from_utf8(parser.callbacks().0.clone()).unwrap();
        assert_eq!(replies, "\x1b[1;3R\x1b[?62;22c\x1b]11;rgb:0000/0000/0000\x1b\\");
    }

    #[test]
    fn a_snapshot_restores_the_screen_cursor_and_input_modes() {
        let mut claude = vt100::Parser::new(size(24), size(80), 0);
        // Bracketed paste, application cursor keys, the alternate screen,
        // a hidden cursor, and some text with the cursor left mid-line.
        claude.process(b"\x1b[?2004h\x1b[?1h\x1b[?1049h\x1b[?25l\x1b[3;5Hhello\x1b[3;7H");

        let mut viewer = vt100::Parser::new(size(24), size(80), 0);
        viewer.process(&snapshot(claude.screen()));

        let (claude, viewer) = (claude.screen(), viewer.screen());
        assert_eq!(viewer.contents(), claude.contents());
        assert_eq!(viewer.cursor_position(), (2, 6));
        assert!(viewer.hide_cursor());
        assert!(viewer.bracketed_paste());
        assert!(viewer.application_cursor());
        assert!(viewer.alternate_screen());
    }
}
