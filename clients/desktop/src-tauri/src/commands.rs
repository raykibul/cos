//! The commands the UI calls with `invoke`. See "The bridge" and "Shell" in
//! `docs/desktop.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cos_bridge::{
    Bridge, BridgeError, DaemonInfo, EventMessage, FileResponse, Forward, ForwardList, ForwardMessage,
    ForwardSet, Response, StreamHandle, TerminalMessage,
};
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
    /// The ports forwarded from the host in use. Replaced, and so stopped,
    /// when another host is selected.
    forwards: Mutex<Forwards>,
}

/// A host's forwards, and which host they belong to.
struct Forwards {
    host_id: String,
    set: Arc<ForwardSet>,
}

impl Forwards {
    /// Forwards for `host`, with its saved ports started again in the
    /// background.
    fn start(bridge: &Bridge, host: &Host) -> Self {
        let set = Arc::new(ForwardSet::new(bridge.clone(), host.name.clone()));
        if host.address.is_some() && !host.forwards.is_empty() {
            let ports = host.forwards.clone();
            let restoring = set.clone();
            tauri::async_runtime::spawn(async move {
                for port in ports {
                    let _ = restoring.restore(port).await;
                }
            });
        }
        Self {
            host_id: host.id.clone(),
            set,
        }
    }
}

impl AppState {
    pub(crate) fn new(local: Bridge, hosts: Hosts) -> Self {
        let host = hosts.current();
        let bridge = bridge_for(&local, &host);
        let forwards = Forwards::start(&bridge, &host);
        Self {
            local,
            current: Mutex::new(Current {
                generation: 0,
                bridge,
            }),
            hosts: Mutex::new(hosts),
            next_id: AtomicU32::new(1),
            streams: Mutex::default(),
            forwards: Mutex::new(forwards),
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

    /// Points the bridge at `host` and closes every stream and forward,
    /// which belonged to the host before. The new host's saved forwards
    /// start again.
    fn switch_to(&self, host: &Host) {
        let mut current = lock(&self.current);
        current.generation += 1;
        current.bridge = bridge_for(&self.local, host);
        self.close_all_streams();
        *lock(&self.forwards) = Forwards::start(&current.bridge, host);
    }

    /// The forwards of the host in use, and that host's id.
    fn forward_set(&self) -> (String, Arc<ForwardSet>) {
        let forwards = lock(&self.forwards);
        (forwards.host_id.clone(), forwards.set.clone())
    }

    fn forwards_list(&self) -> ForwardList {
        self.forward_set().1.list()
    }

    /// Forwards `remote_port` from the host in use, and saves its ports.
    async fn add_forward(&self, remote_port: u16, local_port: Option<u16>) -> CommandResult<Forward> {
        let (host_id, set) = self.forward_set();
        if host_id == crate::hosts::THIS_MAC {
            return Err(BridgeError::bad_request("This Mac's ports are already here"));
        }
        let forward = set.add(remote_port, local_port).await?;
        self.save_forwards(&host_id, &set);
        Ok(forward)
    }

    fn remove_forward(&self, remote_port: u16) -> ForwardList {
        let (host_id, set) = self.forward_set();
        let list = set.remove(remote_port);
        self.save_forwards(&host_id, &set);
        list
    }

    /// Remembers the ports for the next time `host_id` is selected. Failing
    /// to save doesn't stop a forward that works.
    fn save_forwards(&self, host_id: &str, set: &ForwardSet) {
        let _ = lock(&self.hosts).set_forwards(host_id, set.ports());
    }

    /// `http://localhost:{local}/` for the forward of `remote_port`.
    fn forward_url(&self, remote_port: u16) -> CommandResult<String> {
        let (_, set) = self.forward_set();
        let forward = set
            .get(remote_port)
            .ok_or_else(|| BridgeError::bad_request(format!(":{remote_port} isn't forwarded")))?;
        Ok(format!("http://localhost:{}/", forward.local_port))
    }

    fn list(&self) -> HostList {
        lock(&self.hosts).list()
    }

    fn add_host(&self, name: &str, address: &str) -> CommandResult<Host> {
        lock(&self.hosts).add(name, address)
    }

    fn rename_host(&self, id: &str, name: &str) -> CommandResult<HostList> {
        let mut hosts = lock(&self.hosts);
        hosts.rename(id, name)?;
        Ok(hosts.list())
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

/// Uploads a file (an attachment): `POST path` with `data`, base64, as the
/// body. Any status is a normal result, as for `daemon_request`.
#[tauri::command]
pub(crate) async fn daemon_upload(
    state: State<'_, AppState>,
    path: String,
    content_type: String,
    data: String,
) -> CommandResult<Response> {
    let (_, bridge) = state.bridge();
    bridge.upload(&path, &content_type, &data).await
}

/// Fetches a file, such as an attachment to show: `{status, contentType,
/// data}` with the bytes as base64. A non-2xx answer is an error.
#[tauri::command]
pub(crate) async fn daemon_fetch_bytes(
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<FileResponse> {
    let (_, bridge) = state.bridge();
    bridge.fetch_bytes(&path).await
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

/// A shell's terminal (`/v1/shells/{id}/terminal`), like `attach_terminal`.
#[tauri::command]
pub(crate) async fn attach_shell(
    state: State<'_, AppState>,
    shell_id: i64,
    on_message: Channel<TerminalMessage>,
) -> CommandResult<u32> {
    let (generation, bridge) = state.bridge();
    let handle = bridge.attach_shell(shell_id, move |message| on_message.send(message).is_ok());
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

/// Renames a host, this Mac included. The name is trimmed, at most 60
/// characters and not another host's; the id stays.
#[tauri::command]
pub(crate) fn host_rename(state: State<'_, AppState>, id: String, name: String) -> CommandResult<HostList> {
    state.rename_host(&id, &name)
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

/// The ports forwarded from the host in use.
#[tauri::command]
pub(crate) fn forwards_list(state: State<'_, AppState>) -> ForwardList {
    state.forwards_list()
}

/// Forwards a port of the host in use to this Mac's loopback: the same
/// port, or `local_port`, or the next free one.
#[tauri::command]
pub(crate) async fn forward_add(
    state: State<'_, AppState>,
    remote_port: u16,
    local_port: Option<u16>,
) -> CommandResult<Forward> {
    state.add_forward(remote_port, local_port).await
}

/// Stops forwarding a port: its listener closes and its connections drop.
#[tauri::command]
pub(crate) fn forward_remove(state: State<'_, AppState>, remote_port: u16) -> ForwardList {
    state.remove_forward(remote_port)
}

/// The forwards of the host in use: the whole list now and on each change.
#[tauri::command]
pub(crate) async fn follow_forwards(
    state: State<'_, AppState>,
    on_message: Channel<ForwardMessage>,
) -> CommandResult<u32> {
    let (generation, _) = state.bridge();
    let (_, set) = state.forward_set();
    let handle = set.follow(move |message| on_message.send(message).is_ok());
    Ok(state.keep(generation, handle))
}

/// Opens a forward in the default browser. The URL is made here from a
/// forward that exists, so the page can't open anything else this way.
#[tauri::command]
pub(crate) fn forward_open(state: State<'_, AppState>, remote_port: u16) -> CommandResult<()> {
    let url = state.forward_url(remote_port)?;
    run(Command::new("/usr/bin/open").arg(url))
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

/// Opens a web or mail link in the default browser (or mail app).
#[tauri::command]
pub(crate) fn open_url(url: String) -> CommandResult<()> {
    let url = crate::links::external(&url)
        .ok_or_else(|| BridgeError::bad_request(format!("{url} is not a web or mail link")))?;
    run(Command::new("/usr/bin/open").arg(url.as_str()))
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

    #[test]
    fn open_url_wants_a_web_or_mail_link() {
        let error = open_url("file:///etc/hosts".into()).unwrap_err();
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
    fn renaming_the_host_in_use_keeps_its_streams_and_shows_in_info() {
        let state = state();
        follow(&state);
        let list = state.rename_host("local", "Work Mac").unwrap();
        assert_eq!(list.current, "local");
        assert_eq!(list.hosts[0].name, "Work Mac");
        assert_eq!(state.info().host.name, "Work Mac");
        assert_eq!(state.streams().len(), 1, "a rename doesn't switch");
        assert_eq!(
            state.rename_host("h4", "Nope").unwrap_err().kind,
            ErrorKind::BadRequest
        );
    }

    #[test]
    fn forwards_are_refused_on_this_mac_and_dropped_on_a_switch() {
        let state = state();
        let error = tauri::async_runtime::block_on(state.add_forward(5173, None)).unwrap_err();
        assert_eq!(error.message, "This Mac's ports are already here");
        assert!(state.forwards_list().forwards.is_empty());
        assert_eq!(state.forwards_list().host, "This Mac");
        assert_eq!(state.forward_url(5173).unwrap_err().kind, ErrorKind::BadRequest);

        // Studio can't be reached, so its forward is kept as unreachable.
        let studio = state.add_host("Studio", "127.0.0.1:1").unwrap();
        state.select_host(&studio.id).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let forward = tauri::async_runtime::block_on(state.add_forward(port, None)).unwrap();
        assert_eq!(forward.remote, cos_bridge::RemoteState::Unreachable);
        assert_eq!(state.forwards_list().host, "Studio");
        assert_eq!(
            state.forward_url(port).unwrap(),
            format!("http://localhost:{}/", forward.local_port)
        );
        assert_eq!(state.list().hosts[1].forwards, [port]);

        // Selecting This Mac stops it: the port is free again.
        state.select_host(crate::hosts::THIS_MAC).unwrap();
        assert!(state.forwards_list().forwards.is_empty());
        let mut free = false;
        for _ in 0..100 {
            if std::net::TcpListener::bind(("127.0.0.1", forward.local_port)).is_ok() {
                free = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(free, "the forward's listener is still open");
        // The saved port stays with Studio.
        assert_eq!(state.list().hosts[1].forwards, [port]);
        assert!(state.remove_forward(port).forwards.is_empty());
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
