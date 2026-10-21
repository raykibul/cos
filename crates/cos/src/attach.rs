//! `cos agent attach <id>`: your terminal becomes the agent's terminal. Keys
//! go to the agent as typed, its output comes back, and Ctrl-] detaches.
//! The agent is resized to your terminal at the start and whenever yours
//! changes size.

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

use anyhow::{Context, bail};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::client::{Client, TerminalMessage};

/// Ctrl-], as telnet and ssh use it.
const DETACH: u8 = 0x1d;

pub(crate) async fn run(client: &Client, id: i64) -> anyhow::Result<ExitCode> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("attach needs a terminal; use `cos agent screen {id}` to print the screen");
    }
    let agent = client.agent(id).await?;
    if agent.state.is_running() {
        let (cols, rows) = terminal::size().context("could not read the terminal size")?;
        client.resize_agent(id, rows, cols).await?;
    }
    let mut stream = client.terminal(id).await?;

    let raw = RawMode::enter()?;
    let mut keys = read_keys();
    let mut resized = signal(SignalKind::window_change())?;
    let mut stdout = std::io::stdout();
    let mut exit = None;

    loop {
        tokio::select! {
            message = stream.next() => match message? {
                Some(TerminalMessage::Snapshot { data, .. } | TerminalMessage::Output(data)) => {
                    stdout.write_all(&data)?;
                    stdout.flush()?;
                }
                Some(TerminalMessage::Resize { .. }) => {}
                Some(TerminalMessage::Exit { exit_code }) => {
                    exit = Some(exit_code);
                    break;
                }
                None => break,
            },
            key = keys.recv() => match key {
                Some(Key::Bytes(bytes)) => client.keys(id, &bytes).await?,
                Some(Key::Detach) | None => break,
            },
            _ = resized.recv() => {
                if let Ok((cols, rows)) = terminal::size() {
                    // It may have exited meanwhile; the stream will say so.
                    let _ = client.resize_agent(id, rows, cols).await;
                }
            }
        }
    }
    drop(raw);
    match exit {
        Some(code) => println!(
            "Agent #{id} exited ({}).",
            code.map_or_else(|| "killed".into(), |code| format!("code {code}"))
        ),
        None => println!("Detached from agent #{id}; it keeps running."),
    }
    Ok(ExitCode::SUCCESS)
}

enum Key {
    Bytes(Vec<u8>),
    Detach,
}

/// Reads stdin on a thread, since reading a terminal blocks. Everything up
/// to a Ctrl-] is sent on; the Ctrl-] itself detaches.
fn read_keys() -> mpsc::UnboundedReceiver<Key> {
    let (keys, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; 1024];
        loop {
            let read = match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = keys.send(Key::Detach);
                    return;
                }
                Ok(read) => read,
            };
            let bytes = &buffer[..read];
            let (before, detach) = match bytes.iter().position(|&byte| byte == DETACH) {
                Some(at) => (&bytes[..at], true),
                None => (bytes, false),
            };
            if !before.is_empty() && keys.send(Key::Bytes(before.to_vec())).is_err() {
                return;
            }
            if detach {
                let _ = keys.send(Key::Detach);
                return;
            }
        }
    });
    receiver
}

/// Raw mode on a screen of its own, restored on drop even after an error.
struct RawMode;

impl RawMode {
    fn enter() -> anyhow::Result<Self> {
        terminal::enable_raw_mode().context("could not switch the terminal to raw mode")?;
        let guard = Self;
        crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
        Ok(guard)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
