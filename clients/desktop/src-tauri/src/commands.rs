//! The commands the UI calls with `invoke`. See "The bridge" and "Shell" in
//! `docs/desktop.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cos_bridge::{Bridge, BridgeError, DaemonInfo, EventMessage, Response, StreamHandle, TerminalMessage};
use serde::Serialize;
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::hosts::{Host, HostList, Hosts};
use crate::tray::Tray;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bridge in use, numbered so a stream started just before a switch of
/// host isn't kept afterwards.
struct Current {
    generation: u64,
    bridge: Bridge,
}

/// The hosts, the bridge to the one in use, and the streams the UI has open,
/// by id.
pub(crate) struct AppState {
    /// This Mac's daemon, through its socket.
    local: Bridge,
    current: Mutex<Current>,
    hosts: Mutex<Hosts>,
    next_id: AtomicU32,
    streams: Mutex<HashMap<u32, StreamHandle>>,
}

impl AppState {
    pub(crate) fn new(local: Bridge, hosts: Hosts) -> Self {
        let bridge = bridge_for(&local, &hosts.current());
        Self {
            local,
            current: Mutex::new(Current {
                generation: 0,
                bridge,
            }),
            hosts: Mutex::new(hosts),
            next_id: AtomicU32::new(1),
            streams: Mutex::default(),
        }
    }

    /// The bridge to the host in use, and its generation for [`Self::keep`].
    fn bridge(&self) -> (u64, Bridge) {
        let current = lock(&self.current);
        (current.generation, current.bridge.clone())
    }

    /// Keeps a stream until `close_stream`. One started on a host that has
    /// since been switched away from is closed at once; its id is still
    /// valid to close.
    fn keep(&self, generation: u64, handle: StreamHandle) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let current = lock(&self.current);
        if current.generation == generation {
            self.streams().insert(id, handle);
        }
        drop(current);
        id
    }

    fn streams(&self) -> MutexGuard<'_, HashMap<u32, StreamHandle>> {
        lock(&self.streams)
    }

    /// Closes every stream; dropping a handle closes it.
    pub(crate) fn close_all_streams(&self) {
        self.streams().clear();
    }

    /// Points the bridge at `host` and closes every stream, which belonged
    /// to the host before.
    fn switch_to(&self, host: &Host) {
        let mut current = lock(&self.current);
        current.generation += 1;
        current.bridge = bridge_for(&self.local, host);
        self.close_all_streams();
    }

    fn list(&self) -> HostList {
        lock(&self.hosts).list()
    }

    fn add_host(&self, name: &str, address: &str) -> CommandResult<Host> {
        lock(&self.hosts).add(name, address)
    }

    fn remove_host(&self, id: &str) -> CommandResult<HostList> {
        let mut hosts = lock(&self.hosts);
        if hosts.remove(id)? {
            self.switch_to(&hosts.current());
        }
        Ok(hosts.list())
    }

    fn select_host(&self, id: &str) -> CommandResult<HostList> {
        let mut hosts = lock(&self.hosts);
        let host = hosts.select(id)?;
        self.switch_to(&host);
        Ok(hosts.list())
    }

    /// The devices on this Mac's tailnet. Always asked of this Mac's daemon,
    /// whichever host is in use: the route answers on the socket only.
    async fn tailnet_peers(&self, port: Option<u16>) -> CommandResult<Value> {
        let path = match port {
            Some(port) => format!("/v1/tailnet/peers?port={port}"),
            None => "/v1/tailnet/peers".to_owned(),
        };
        let response = self.local.request("GET", &path, None).await.map_err(|error| {
            if error.is_not_running() {
                let socket = self.local.socket().map(Path::display);
                BridgeError::not_running(match socket {
                    Some(socket) => format!(
                        "This Mac's cos daemon isn't running (no answer on {socket}), so it can't list your tailnet"
                    ),
                    None => "This Mac's cos daemon isn't running, so it can't list your tailnet".to_owned(),
                })
            } else {
                error
            }
        })?;
        peers_from(response)
    }

    fn info(&self) -> Info {
        Info {
            daemon: self.local.info().unwrap_or(DaemonInfo {
                socket: PathBuf::new(),
                home: PathBuf::new(),
            }),
            host: lock(&self.hosts).current(),
        }
    }
}

/// The bridge for `host`: this Mac's socket, or its address over TCP.
fn bridge_for(local: &Bridge, host: &Host) -> Bridge {
    match host.target() {
        Some(target) => local.retarget(target),
        None => local.clone(),
    }
}

type CommandResult<T> = Result<T, BridgeError>;

/// The peers from the daemon's answer. Anything but a 2xx is an error; a
/// 404 means a daemon older than the route.
fn peers_from(response: Response) -> CommandResult<Value> {
    let Response { status, body } = response;
    if (200..300).contains(&status) {
        return Ok(body.unwrap_or(Value::Null));
    }
    if status == 404 {
        return Err(BridgeError::io(
            "This Mac's cos daemon is too old to list your tailnet. Update cos and restart the daemon.",
        ));
    }
    let message = body
        .as_ref()
        .and_then(|body| body.get("error")?.as_str())
        .map_or_else(|| format!("the cos daemon answered {status}"), str::to_owned);
    match body.as_ref().and_then(|body| body.get("code")?.as_str()) {
        Some(code) => Err(BridgeError::refused(code, message)),
        None => Err(BridgeError::io(message)),
    }
}

/// `daemon_info`: this Mac's socket and home, and the host in use.
#[derive(Debug, Serialize)]
pub(crate) struct Info {
    #[serde(flatten)]
    daemon: DaemonInfo,
    host: Host,
}

#[tauri::command]
pub(crate) fn daemon_info(state: State<'_, AppState>) -> Info {
    state.info()
}

#[tauri::command]
pub(crate) async fn daemon_request(
    state: State<'_, AppState>,
    method: String,
    path: String,
    body: Option<Value>,
) -> CommandResult<Response> {
    let (_, bridge) = state.bridge();
    bridge.request(&method, &path, body).await
}

// The stream commands are async so they run on Tauri's Tokio runtime, which
// the bridge spawns its streams on.

#[tauri::command]
pub(crate) async fn follow_events(
    state: State<'_, AppState>,
    after: Option<u64>,
    on_message: Channel<EventMessage>,
) -> CommandResult<u32> {
    let (generation, bridge) = state.bridge();
    let handle = bridge.follow_events(after, move |message| on_message.send(message).is_ok());
    Ok(state.keep(generation, handle))
}

#[tauri::command]
pub(crate) async fn attach_terminal(
    state: State<'_, AppState>,
    agent_id: i64,
    on_message: Channel<TerminalMessage>,
) -> CommandResult<u32> {
    let (generation, bridge) = state.bridge();
    let handle = bridge.attach_terminal(agent_id, move |message| on_message.send(message).is_ok());
    Ok(state.keep(generation, handle))
}

#[tauri::command]
pub(crate) fn hosts_list(state: State<'_, AppState>) -> HostList {
    state.list()
}

/// Saves a host; it doesn't switch to it.
#[tauri::command]
pub(crate) fn host_add(state: State<'_, AppState>, name: String, address: String) -> CommandResult<Host> {
    state.add_host(&name, &address)
}

/// Forgets a host. Removing the one in use switches to this Mac and closes
/// every stream.
#[tauri::command]
pub(crate) fn host_remove(state: State<'_, AppState>, id: String) -> CommandResult<HostList> {
    state.remove_host(&id)
}

/// Switches to a host: every stream closes, and requests and new streams go
/// to it.
#[tauri::command]
pub(crate) fn host_select(state: State<'_, AppState>, id: String) -> CommandResult<HostList> {
    state.select_host(&id)
}

/// The devices on this Mac's tailnet (`GET /v1/tailnet/peers`), from this
/// Mac's daemon even while another host is in use.
#[tauri::command]
pub(crate) async fn tailnet_peers(state: State<'_, AppState>, port: Option<u16>) -> CommandResult<Value> {
    state.tailnet_peers(port).await
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

    fn state() -> AppState {
        AppState::new(Bridge::new("/nowhere/cos.sock"), Hosts::load(None))
    }

    /// Opens an event stream on the current host, as `follow_events` does.
    fn follow(state: &AppState) -> u32 {
        let (generation, bridge) = state.bridge();
        let handle =
            tauri::async_runtime::block_on(async { bridge.follow_events(None, |_: EventMessage| true) });
        state.keep(generation, handle)
    }

    #[test]
    fn selecting_a_host_closes_every_stream_and_retargets() {
        let state = state();
        let studio = state.add_host("Studio", "studio").unwrap();
        // Adding doesn't switch.
        assert_eq!(state.bridge().1.socket(), Some(Path::new("/nowhere/cos.sock")));
        follow(&state);
        follow(&state);
        assert_eq!(state.streams().len(), 2);

        let list = state.select_host(&studio.id).unwrap();
        assert_eq!(list.current, studio.id);
        assert!(state.streams().is_empty());
        assert_eq!(
            state.bridge().1.target(),
            &cos_bridge::Target::Tcp("studio:7700".into())
        );
        let info = serde_json::to_value(state.info()).unwrap();
        assert_eq!(
            info,
            serde_json::json!({
                "socket": "/nowhere/cos.sock",
                "home": "/nowhere",
                "host": {"id": "h1", "name": "Studio", "address": "studio:7700"}
            })
        );

        // Removing the host in use goes back to this Mac, closing streams.
        follow(&state);
        let list = state.remove_host(&studio.id).unwrap();
        assert_eq!(list.current, crate::hosts::THIS_MAC);
        assert!(state.streams().is_empty());
        assert_eq!(state.bridge().1.socket(), Some(Path::new("/nowhere/cos.sock")));
        assert_eq!(state.info().host.name, "This Mac");
    }

    #[test]
    fn a_stream_started_before_a_switch_is_not_kept() {
        let state = state();
        let studio = state.add_host("Studio", "studio").unwrap();
        let (generation, bridge) = state.bridge();
        state.select_host(&studio.id).unwrap();
        let handle =
            tauri::async_runtime::block_on(async { bridge.follow_events(None, |_: EventMessage| true) });
        let id = state.keep(generation, handle);
        assert!(id > 0);
        assert!(state.streams().is_empty());
    }

    /// A daemon on a Unix socket that answers one request with `status` and
    /// `body`, and hands back the request line it got.
    fn one_shot_daemon(socket: &Path, status: &str, body: &str) -> std::thread::JoinHandle<String> {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        let answer = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            // Read the headers, then answer.
            let mut header = String::new();
            while reader.read_line(&mut header).unwrap() > 2 {
                header.clear();
            }
            reader.get_mut().write_all(answer.as_bytes()).unwrap();
            line.trim_end().to_owned()
        })
    }

    #[test]
    fn tailnet_peers_asks_this_macs_socket_even_on_another_host() {
        let scratch = crate::hosts::tests::Scratch::new();
        std::fs::create_dir_all(&scratch.0).unwrap();
        let socket = scratch.0.join("cos.sock");
        let peers = r#"{"peers":[{"id":"n1","name":"build-box","cos":"running"}],"self_name":"my-mac","reason":null}"#;
        let daemon = one_shot_daemon(&socket, "200 OK", peers);

        let state = AppState::new(Bridge::new(&socket), Hosts::load(None));
        let studio = state.add_host("Studio", "studio").unwrap();
        state.select_host(&studio.id).unwrap();
        assert!(state.bridge().1.socket().is_none(), "requests now go to Studio");

        let answer = tauri::async_runtime::block_on(state.tailnet_peers(Some(7701))).unwrap();
        assert_eq!(answer, serde_json::from_str::<Value>(peers).unwrap());
        assert_eq!(daemon.join().unwrap(), "GET /v1/tailnet/peers?port=7701 HTTP/1.1");
    }

    #[test]
    fn tailnet_peers_says_when_this_macs_daemon_is_down() {
        let state = state();
        let error = tauri::async_runtime::block_on(state.tailnet_peers(None)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotRunning);
        assert!(
            error.message.starts_with("This Mac's cos daemon isn't running"),
            "{}",
            error.message
        );
    }

    #[test]
    fn tailnet_peers_turns_refusals_into_errors() {
        let ok = peers_from(Response {
            status: 200,
            body: Some(serde_json::json!({"peers": []})),
        });
        assert_eq!(ok.unwrap(), serde_json::json!({"peers": []}));

        let old = peers_from(Response {
            status: 404,
            body: Some(serde_json::json!({"error": "no route"})),
        })
        .unwrap_err();
        assert_eq!(old.kind, ErrorKind::Io);
        assert!(old.message.contains("too old"), "{}", old.message);

        let refused = peers_from(Response {
            status: 403,
            body: Some(serde_json::json!({"error": "only on the host", "code": "host_only"})),
        })
        .unwrap_err();
        assert_eq!(refused.message, "only on the host");
        assert_eq!(refused.code.as_deref(), Some("host_only"));

        let plain = peers_from(Response {
            status: 500,
            body: None,
        })
        .unwrap_err();
        assert_eq!(plain.message, "the cos daemon answered 500");
    }

    #[test]
    fn unknown_hosts_are_refused() {
        let state = state();
        assert_eq!(state.select_host("h7").unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(
            state.remove_host("local").unwrap_err().kind,
            ErrorKind::BadRequest
        );
        assert_eq!(state.list().hosts.len(), 1);
    }
}
