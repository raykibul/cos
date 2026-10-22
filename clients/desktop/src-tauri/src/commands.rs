//! The commands the UI calls with `invoke`. See "The bridge" and "Shell" in
//! `docs/desktop.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use cos_bridge::{Bridge, BridgeError, DaemonInfo, EventMessage, Response, StreamHandle, TerminalMessage};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::tray::Tray;

/// The bridge and the streams the UI has open, by id.
pub(crate) struct AppState {
    bridge: Bridge,
    next_id: AtomicU32,
    streams: Mutex<HashMap<u32, StreamHandle>>,
}

impl AppState {
    pub(crate) fn new(bridge: Bridge) -> Self {
        Self {
            bridge,
            next_id: AtomicU32::new(1),
            streams: Mutex::default(),
        }
    }

    fn keep(&self, handle: StreamHandle) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.streams().insert(id, handle);
        id
    }

    fn streams(&self) -> std::sync::MutexGuard<'_, HashMap<u32, StreamHandle>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes every stream; dropping a handle closes it.
    pub(crate) fn close_all_streams(&self) {
        self.streams().clear();
    }
}

type CommandResult<T> = Result<T, BridgeError>;

#[tauri::command]
pub(crate) fn daemon_info(state: State<'_, AppState>) -> DaemonInfo {
    state.bridge.info()
}

#[tauri::command]
pub(crate) async fn daemon_request(
    state: State<'_, AppState>,
    method: String,
    path: String,
    body: Option<Value>,
) -> CommandResult<Response> {
    state.bridge.request(&method, &path, body).await
}

// The stream commands are async so they run on Tauri's Tokio runtime, which
// the bridge spawns its streams on.

#[tauri::command]
pub(crate) async fn follow_events(
    state: State<'_, AppState>,
    after: Option<u64>,
    on_message: Channel<EventMessage>,
) -> CommandResult<u32> {
    let handle = state
        .bridge
        .follow_events(after, move |message| on_message.send(message).is_ok());
    Ok(state.keep(handle))
}

#[tauri::command]
pub(crate) async fn attach_terminal(
    state: State<'_, AppState>,
    agent_id: i64,
    on_message: Channel<TerminalMessage>,
) -> CommandResult<u32> {
    let handle = state
        .bridge
        .attach_terminal(agent_id, move |message| on_message.send(message).is_ok());
    Ok(state.keep(handle))
}

/// Closing an unknown or already closed stream does nothing.
#[tauri::command]
pub(crate) fn close_stream(state: State<'_, AppState>, id: u32) {
    state.streams().remove(&id);
}

#[tauri::command]
pub(crate) fn set_waiting_count(tray: State<'_, Tray>, count: u32) -> CommandResult<()> {
    tray.set_waiting(count).map_err(io)
}

#[tauri::command]
pub(crate) fn notify(app: AppHandle, title: String, body: String) -> CommandResult<()> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(io)
}

/// Shows `path` in Finder.
#[tauri::command]
pub(crate) fn reveal_path(path: String) -> CommandResult<()> {
    let path = existing(&path)?;
    run(Command::new("/usr/bin/open").arg("-R").arg(&path))
}

/// Opens a terminal window in the folder `path`: Ghostty if it's installed,
/// else Terminal.
#[tauri::command]
pub(crate) fn open_in_terminal(path: String) -> CommandResult<()> {
    let path = existing(&path)?;
    if !path.is_dir() {
        return Err(BridgeError::bad_request(format!(
            "{} is not a folder",
            path.display()
        )));
    }
    let app = ghostty().unwrap_or_else(|| PathBuf::from("Terminal"));
    run(Command::new("/usr/bin/open").arg("-a").arg(app).arg(&path))
}

/// Ghostty's app bundle, if installed in either Applications folder.
fn ghostty() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(|home| Path::new(&home).join("Applications/Ghostty.app"));
    [Some(PathBuf::from("/Applications/Ghostty.app")), home]
        .into_iter()
        .flatten()
        .find(|app| app.is_dir())
}

/// An absolute path that exists.
fn existing(path: &str) -> CommandResult<PathBuf> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(BridgeError::bad_request(format!(
            "{} is not an absolute path",
            path.display()
        )));
    }
    if !path.exists() {
        return Err(BridgeError::bad_request(format!(
            "{} does not exist",
            path.display()
        )));
    }
    Ok(path)
}

fn run(command: &mut Command) -> CommandResult<()> {
    let output = command.output().map_err(io)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BridgeError::io(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn io(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::io(error.to_string())
}

#[cfg(test)]
mod tests {
    use cos_bridge::ErrorKind;

    use super::*;

    #[test]
    fn paths_must_be_absolute_and_exist() {
        assert_eq!(existing("relative").unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(
            existing("/no/such/place/for/cos").unwrap_err().kind,
            ErrorKind::BadRequest
        );
        assert_eq!(existing("/").unwrap(), PathBuf::from("/"));
    }

    #[test]
    fn open_in_terminal_wants_a_folder() {
        let error = open_in_terminal("/etc/hosts".into()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::BadRequest);
    }
}
