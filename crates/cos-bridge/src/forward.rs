//! Port forwarding: another machine's `localhost:PORT` on this Mac's
//! `localhost:PORT`. A [`ForwardSet`] belongs to one host. For each forward
//! it listens on this Mac's loopback, and each connection a browser opens
//! there becomes one tunnel ([`Bridge::open_tunnel`]): an HTTP upgrade to
//! the daemon's `/v1/forward/{port}`, then raw bytes both ways. See "Port
//! forwarding" in `docs/desktop.md` and `docs/daemon.md`.

use std::collections::BTreeMap;
use std::io::ErrorKind as IoKind;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode, header};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinSet};

use crate::http::{error_from, read_all};
use crate::messages::{Forward, ForwardList, ForwardMessage, ListenState, RemoteState};
use crate::stream::{self, Sink, StreamHandle};
use crate::{Bridge, BridgeError};

/// The daemon's `Upgrade` token for a tunnel.
pub const UPGRADE: &str = "cos-forward";
/// Busy local ports tried after the one asked for, as Vite does.
const FALLBACKS: u16 = 10;
/// Changes to the list are sent at most this often.
const COALESCE: Duration = Duration::from_millis(250);
/// How long a failed connection waits for the browser's first bytes, to
/// tell HTTP (which gets a 502 page) from anything else (which is closed).
const PEEK: Duration = Duration::from_secs(2);

/// A tunnel to a port on the daemon's loopback: read and write it like the
/// TCP connection it stands for.
pub type Tunnel = TokioIo<hyper::upgrade::Upgraded>;

/// The error for a daemon older than port forwarding.
pub fn too_old(machine: &str) -> BridgeError {
    BridgeError::refused(
        "too_old",
        format!("{machine}'s cos is too old to forward ports. Run `cos upgrade` there."),
    )
}

/// TCP keepalive: probes after 60 s of quiet, every 10 s, 3 times. A
/// forwarded websocket can sit quiet for hours, and only this notices that
/// the other end vanished.
pub(crate) fn keep_alive(stream: &TcpStream) -> std::io::Result<()> {
    let params = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(10))
        .with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&params)
}

fn check_port(port: u16) -> Result<(), BridgeError> {
    if port == 0 {
        return Err(BridgeError::bad_request("a port is a number from 1 to 65535"));
    }
    Ok(())
}

/// Aborts a task when dropped.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Bridge {
    /// Opens one tunnel to `port` on the daemon's loopback. A 404 is a
    /// daemon older than the route: a refusal with code `too_old`.
    pub async fn open_tunnel(&self, port: u16) -> Result<Tunnel, BridgeError> {
        check_port(port)?;
        let stream = self.connect().await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| BridgeError::io(format!("could not talk to the daemon: {error}")))?;
        let driver = AbortOnDrop(tokio::spawn(connection.with_upgrades()).abort_handle());
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("/v1/forward/{port}"))
            .header(header::HOST, self.target.host_header())
            .header(header::CONNECTION, "upgrade")
            .header(header::UPGRADE, UPGRADE)
            .body(Full::new(Bytes::new()))
            .map_err(|error| BridgeError::bad_request(error.to_string()))?;
        let mut response = sender
            .send_request(request)
            .await
            .map_err(|error| BridgeError::io(format!("the daemon didn't answer: {error}")))?;
        let status = response.status();
        if status == StatusCode::SWITCHING_PROTOCOLS {
            let upgraded = hyper::upgrade::on(&mut response)
                .await
                .map_err(|error| BridgeError::io(format!("the tunnel didn't open: {error}")))?;
            // The connection task has handed its connection over by now.
            drop(driver);
            return Ok(TokioIo::new(upgraded));
        }
        let bytes = read_all(response).await?;
        let body = serde_json::from_slice::<Value>(&bytes).ok();
        Err(refusal(status.as_u16(), body.as_ref()))
    }
}

/// A failed answer from the route. A 404 without a code is a daemon that
/// doesn't know it.
fn refusal(status: u16, body: Option<&Value>) -> BridgeError {
    let coded = body.is_some_and(|body| body.get("code").is_some());
    if status == 404 && !coded {
        return too_old("this machine");
    }
    error_from(status, body)
}

/// The forwards to one host. Dropping it stops them all: listeners close and
/// open tunnels drop.
pub struct ForwardSet {
    shared: Arc<Shared>,
}

struct Shared {
    bridge: Bridge,
    host: String,
    forwards: Mutex<BTreeMap<u16, Entry>>,
    /// Bumped on every change, for [`ForwardSet::follow`].
    changed: watch::Sender<u64>,
    next_id: Mutex<u64>,
}

struct Entry {
    /// Tells this forward from an earlier one on the same port.
    id: u64,
    forward: Forward,
    /// The note that the local port moved, shown again once a tunnel works.
    note: Option<String>,
    listeners: Vec<AbortHandle>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        for listener in &self.listeners {
            listener.abort();
        }
    }
}

impl Drop for ForwardSet {
    fn drop(&mut self) {
        // Aborting the listeners drops their tunnels too.
        let entries = std::mem::take(&mut *self.shared.lock());
        drop(entries);
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<u16, Entry>> {
        self.forwards.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(&self) -> ForwardList {
        ForwardList {
            host: self.host.clone(),
            forwards: self.lock().values().map(|entry| entry.forward.clone()).collect(),
        }
    }

    /// Changes the forward on `remote`, if it is still the one with `id`.
    fn update(&self, remote: u16, id: u64, change: impl FnOnce(&mut Forward, Option<&String>)) {
        let mut forwards = self.lock();
        if let Some(entry) = forwards.get_mut(&remote).filter(|entry| entry.id == id) {
            change(&mut entry.forward, entry.note.as_ref());
            drop(forwards);
            self.changed.send_modify(|version| *version += 1);
        }
    }

    /// What a failed tunnel or probe means for the forward on `remote`, and
    /// the line the 502 page shows.
    fn failure(&self, remote: u16, error: &BridgeError) -> (RemoteState, Option<String>, String) {
        let host = &self.host;
        match error.code.as_deref() {
            Some("nothing_listening") => {
                let line = format!("Nothing is listening on localhost:{remote} on {host}.");
                (RemoteState::NothingListening, None, line)
            }
            Some("connect_timeout") => (RemoteState::NothingListening, None, sentence(&error.message)),
            Some("too_old") => {
                let error = too_old(host);
                (RemoteState::Refused, Some("too_old".into()), error.message)
            }
            Some(code) => (
                RemoteState::Refused,
                Some(code.to_owned()),
                sentence(&error.message),
            ),
            None => (RemoteState::Unreachable, None, sentence(&error.message)),
        }
    }
}

/// `message` with a capital letter and a full stop, for the panel and page.
fn sentence(message: &str) -> String {
    let mut chars = message.trim().chars();
    let mut text: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

impl ForwardSet {
    /// Forwards to the daemon `bridge` talks to, called `host` in messages.
    pub fn new(bridge: Bridge, host: impl Into<String>) -> Self {
        Self {
            shared: Arc::new(Shared {
                bridge,
                host: host.into(),
                forwards: Mutex::default(),
                changed: watch::channel(0).0,
                next_id: Mutex::new(0),
            }),
        }
    }

    pub fn host(&self) -> &str {
        &self.shared.host
    }

    pub fn list(&self) -> ForwardList {
        self.shared.list()
    }

    /// The remote ports forwarded, in order.
    pub fn ports(&self) -> Vec<u16> {
        self.shared.lock().keys().copied().collect()
    }

    /// The forward on `remote`, if any.
    pub fn get(&self, remote: u16) -> Option<Forward> {
        self.shared.lock().get(&remote).map(|entry| entry.forward.clone())
    }

    /// Forwards `remote` on the host to `local` (default: the same port) on
    /// this Mac's loopback. A busy local port moves to the next free one,
    /// up to ten above, then to any. The host is asked whether anything
    /// listens there; a refusal or a daemon too old for forwarding is an
    /// error, and nothing is kept. A host that can't be reached is kept,
    /// marked `unreachable`, since it may come back.
    pub async fn add(&self, remote: u16, local: Option<u16>) -> Result<Forward, BridgeError> {
        self.start(remote, local, false).await
    }

    /// [`Self::add`] for a saved forward, when its host is selected again:
    /// a refusal or a daemon too old is kept too, marked `refused` with its
    /// code, so the port stays in the list and works once the host lets
    /// this Mac in.
    pub async fn restore(&self, remote: u16) -> Result<Forward, BridgeError> {
        self.start(remote, None, true).await
    }

    async fn start(&self, remote: u16, local: Option<u16>, keep: bool) -> Result<Forward, BridgeError> {
        check_port(remote)?;
        let wanted = local.unwrap_or(remote);
        check_port(wanted)?;
        self.refuse_duplicate(remote)?;
        let (listeners, local_port) = bind_local(wanted)
            .await
            .map_err(|error| BridgeError::io(format!("could not listen on localhost:{wanted}: {error}")))?;

        let shared = &self.shared;
        let (remote_state, code, message) = match shared
            .bridge
            .request("GET", &format!("/v1/forward/{remote}"), None)
            .await
        {
            Ok(response) if (200..300).contains(&response.status) => {
                let listening = response
                    .body
                    .as_ref()
                    .and_then(|body| body.get("listening")?.as_bool())
                    .unwrap_or(false);
                if listening {
                    (RemoteState::Listening, None, None)
                } else {
                    let line = format!("Nothing is listening on {remote} on {}.", shared.host);
                    (RemoteState::NothingListening, None, Some(line))
                }
            }
            Ok(response) => {
                let mut error = refusal(response.status, response.body.as_ref());
                if error.code.as_deref() == Some("too_old") {
                    error = too_old(&shared.host);
                }
                if !keep {
                    return Err(error);
                }
                (RemoteState::Refused, error.code, Some(sentence(&error.message)))
            }
            Err(error) if error.is_not_running() || error.code.is_none() => {
                (RemoteState::Unreachable, None, Some(sentence(&error.message)))
            }
            Err(error) if keep => (RemoteState::Refused, error.code, Some(sentence(&error.message))),
            Err(error) => return Err(error),
        };
        let mut moved = None;
        if local_port != wanted {
            moved = Some(format!(
                "localhost:{wanted} was taken on this Mac, so this is on {local_port}. \
                 OAuth redirects and links that name :{wanted} won't reach it."
            ));
        }
        let forward = Forward {
            remote_port: remote,
            local_port,
            state: ListenState::Listening,
            remote: remote_state,
            code,
            message: message.or_else(|| moved.clone()),
            connections: 0,
        };

        let id = {
            let mut next = shared.next_id.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        let mut forwards = shared.lock();
        // Added by someone else while we asked the host.
        if forwards.contains_key(&remote) {
            return Err(duplicate(remote));
        }
        let listeners = listeners
            .into_iter()
            .map(|listener| tokio::spawn(serve(shared.clone(), remote, id, listener)).abort_handle())
            .collect();
        forwards.insert(
            remote,
            Entry {
                id,
                forward: forward.clone(),
                note: moved,
                listeners,
            },
        );
        drop(forwards);
        shared.changed.send_modify(|version| *version += 1);
        Ok(forward)
    }

    fn refuse_duplicate(&self, remote: u16) -> Result<(), BridgeError> {
        if self.shared.lock().contains_key(&remote) {
            return Err(duplicate(remote));
        }
        Ok(())
    }

    /// Stops forwarding `remote`: its listener closes and its tunnels drop.
    /// An unknown port does nothing.
    pub fn remove(&self, remote: u16) -> ForwardList {
        let removed = self.shared.lock().remove(&remote);
        if removed.is_some() {
            drop(removed);
            self.shared.changed.send_modify(|version| *version += 1);
        }
        self.list()
    }

    /// Sends the whole list now and after every change, at most four times
    /// a second, until closed or the set is dropped.
    pub fn follow(&self, sink: impl Sink<ForwardMessage>) -> StreamHandle {
        let weak: Weak<Shared> = Arc::downgrade(&self.shared);
        let mut changed = self.shared.changed.subscribe();
        stream::spawn(sink, move |mut outbox| async move {
            loop {
                let Some(shared) = weak.upgrade() else { return };
                changed.borrow_and_update();
                let ForwardList { host, forwards } = shared.list();
                drop(shared);
                if !outbox.send(ForwardMessage::Forwards { host, forwards }) {
                    return;
                }
                if changed.changed().await.is_err() {
                    return;
                }
                tokio::time::sleep(COALESCE).await;
            }
        })
    }
}

fn duplicate(remote: u16) -> BridgeError {
    BridgeError::bad_request(format!(
        ":{remote} is already forwarded. Remove it first to change its local port."
    ))
}

/// Listens on `127.0.0.1:port` and `[::1]:port` (the browser may take
/// `localhost` as either), or the next port where both are free. Without
/// IPv6, v4 alone. Sockets are made without `SO_REUSEADDR`, so a server
/// already on the Mac's port, even on `0.0.0.0`, counts as busy.
async fn bind_local(port: u16) -> std::io::Result<(Vec<TcpListener>, u16)> {
    let last = port.saturating_add(FALLBACKS);
    for candidate in port..=last {
        match bind_pair(candidate) {
            Ok(listeners) => return Ok((listeners, candidate)),
            Err(error) if error.kind() == IoKind::AddrInUse => {}
            Err(error) => return Err(error),
        }
    }
    // Let the OS pick a v4 port, and hope its v6 twin is free too.
    let mut last_error = None;
    for _ in 0..8 {
        let v4 = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let picked = v4.local_addr()?.port();
        match listen(SocketAddr::from((Ipv6Addr::LOCALHOST, picked))) {
            Ok(v6) => return Ok((vec![v4, v6], picked)),
            Err(error) if error.kind() == IoKind::AddrInUse => last_error = Some(error),
            Err(_) => return Ok((vec![v4], picked)),
        }
    }
    Err(last_error.unwrap_or_else(|| IoKind::AddrInUse.into()))
}

fn bind_pair(port: u16) -> std::io::Result<Vec<TcpListener>> {
    let v4 = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    match listen(SocketAddr::from((Ipv6Addr::LOCALHOST, port))) {
        Ok(v6) => Ok(vec![v4, v6]),
        Err(error) if error.kind() == IoKind::AddrInUse => Err(error),
        // No IPv6 loopback on this Mac: v4 is enough.
        Err(_) => Ok(vec![v4]),
    }
}

fn listen(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.bind(addr)?;
    socket.listen(1024)
}

/// Accepts connections on one of a forward's listeners, one tunnel each.
/// Aborting it drops every tunnel it started.
async fn serve(shared: Arc<Shared>, remote: u16, id: u64, listener: TcpListener) {
    let mut tunnels = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tunnels.spawn(tunnel(shared.clone(), remote, id, stream));
                }
                // Out of file descriptors, mostly: wait a little.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = tunnels.join_next(), if !tunnels.is_empty() => {}
        }
    }
}

/// Counts an open tunnel until dropped.
struct Counted {
    shared: Arc<Shared>,
    remote: u16,
    id: u64,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.shared.update(self.remote, self.id, |forward, _| {
            forward.connections = forward.connections.saturating_sub(1);
        });
    }
}

/// One browser connection: a tunnel to the host, or, if that fails, a 502
/// page for HTTP and a close for anything else.
async fn tunnel(shared: Arc<Shared>, remote: u16, id: u64, mut local: TcpStream) {
    let _ = local.set_nodelay(true);
    match shared.bridge.open_tunnel(remote).await {
        Ok(mut tunnel) => {
            shared.update(remote, id, |forward, note| {
                forward.remote = RemoteState::Listening;
                forward.code = None;
                forward.message = note.cloned();
                forward.connections += 1;
            });
            let _counted = Counted {
                shared: shared.clone(),
                remote,
                id,
            };
            let _ = tokio::io::copy_bidirectional(&mut local, &mut tunnel).await;
        }
        Err(error) => {
            let (state, code, line) = shared.failure(remote, &error);
            shared.update(remote, id, |forward, _| {
                forward.remote = state;
                forward.code = code;
                forward.message = Some(line.clone());
            });
            reply_failure(local, &line).await;
        }
    }
}

/// Answers an HTTP request with a one-line 502 page; closes anything else
/// (a TLS hello gets no HTTP written into it).
async fn reply_failure(mut local: TcpStream, line: &str) {
    let mut first = [0u8; 8];
    let read = match tokio::time::timeout(PEEK, local.peek(&mut first)).await {
        Ok(Ok(read)) => read,
        _ => 0,
    };
    if !looks_like_http(&first[..read]) {
        return;
    }
    let body = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>502 Bad Gateway</title>\n<p>{}</p>\n",
        escape_html(line)
    );
    let response = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if local.write_all(response.as_bytes()).await.is_err() {
        return;
    }
    let _ = local.shutdown().await;
    // Read what's left of the request, so closing doesn't reset the
    // connection before the browser has the page.
    let mut sink = [0u8; 4096];
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while matches!(local.read(&mut sink).await, Ok(read) if read > 0) {}
    })
    .await;
}

/// Whether `bytes` start an HTTP/1 request: a method in capitals, then a
/// space.
fn looks_like_http(bytes: &[u8]) -> bool {
    let Some(space) = bytes.iter().position(|byte| *byte == b' ') else {
        return false;
    };
    (3..=7).contains(&space) && bytes[..space].iter().all(u8::is_ascii_uppercase)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn http_is_told_from_tls_and_noise() {
        assert!(looks_like_http(b"GET / HT"));
        assert!(looks_like_http(b"OPTIONS "));
        assert!(!looks_like_http(&[
            0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01
        ]));
        assert!(!looks_like_http(b"get / HT"));
        assert!(!looks_like_http(b"GE"));
        assert!(!looks_like_http(b""));
    }

    #[test]
    fn a_404_without_a_code_is_a_daemon_too_old() {
        let error = refusal(404, None);
        assert_eq!(error.code.as_deref(), Some("too_old"));
        let error = refusal(403, Some(&json!({"error": "wait", "code": "device_pending"})));
        assert_eq!(error.code.as_deref(), Some("device_pending"));
        let error = refusal(
            502,
            Some(&json!({"error": "nothing is listening on port 3", "code": "nothing_listening"})),
        );
        assert_eq!(error.code.as_deref(), Some("nothing_listening"));
    }

    #[test]
    fn sentences_get_a_capital_and_a_stop() {
        assert_eq!(sentence("could not reach box:7700"), "Could not reach box:7700.");
        assert_eq!(sentence("Done."), "Done.");
        assert_eq!(sentence(""), ".");
    }

    #[test]
    fn the_page_escapes_its_line() {
        assert_eq!(escape_html("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
    }
}
