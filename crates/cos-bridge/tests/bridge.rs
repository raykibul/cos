//! The bridge against a fake daemon: a small axum server on a temporary Unix
//! socket, or on 127.0.0.1 over TCP like a tailnet listener, that serves the
//! routes the bridge uses, and can drop its streams, stop, restart, lose its
//! log and refuse clients on demand.

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use cos_bridge::redirect::Table;
use cos_bridge::{
    Backoff, Bridge, ErrorKind, EventMessage, ForwardMessage, ForwardSet, Redirector, Redirects, RemoteState,
    StreamHandle, Target, TerminalMessage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"))
}

async fn eventually(what: &str, condition: impl Fn() -> bool) {
    within(what, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
}

/// The next message a stream delivered.
async fn next<M>(messages: &mut UnboundedReceiver<M>) -> M {
    within("the next message", messages.recv())
        .await
        .expect("the stream ended")
}

/// Starts a stream whose messages arrive on the returned receiver.
fn collect<M: Send + 'static>(
    start: impl FnOnce(Box<dyn FnMut(M) -> bool + Send>) -> StreamHandle,
) -> (StreamHandle, UnboundedReceiver<M>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let handle = start(Box::new(move |message| sender.send(message).is_ok()));
    (handle, receiver)
}

fn fast(bridge: Bridge) -> Bridge {
    bridge.with_backoff(Backoff {
        initial: Duration::from_millis(20),
        max: Duration::from_millis(100),
    })
}

#[derive(Default)]
struct Log {
    events: Vec<Value>,
    /// Reported as `last_seq` instead of the last event's seq.
    last_seq: Option<u64>,
}

struct FakeState {
    log: Mutex<Log>,
    live: broadcast::Sender<Value>,
    /// The `after` of each event stream opened.
    afters: Mutex<Vec<u64>>,
    /// Bumped to end every open stream.
    drop_streams: watch::Sender<u64>,
    /// Live terminal events: (event name, data).
    terminal: broadcast::Sender<(String, Value)>,
    snapshots: AtomicUsize,
    open_streams: AtomicUsize,
    /// Every request that reached the daemon, as `METHOD /path`.
    hits: Mutex<Vec<String>>,
    /// While set, every request gets this refusal: (status, code, message),
    /// as the tailnet guard answers a device that isn't approved.
    refusal: Mutex<Option<(StatusCode, String, String)>>,
    /// Requests the guard saw, refused or not.
    guarded: AtomicUsize,
    /// While set, an event stream opens and then sends nothing, not even a
    /// keep-alive, like a host that vanished without closing the connection.
    silent: AtomicBool,
    /// How `/v1/forward/{port}` answers: "echo" (an echo server listens
    /// there), "nothing" (502 nothing_listening) or "old" (a daemon without
    /// the route: 404).
    forward: Mutex<&'static str>,
    /// Tunnels the fake has opened, in all.
    tunnels: AtomicUsize,
}

impl FakeState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            log: Mutex::default(),
            live: broadcast::channel(64).0,
            afters: Mutex::default(),
            drop_streams: watch::channel(0).0,
            terminal: broadcast::channel(64).0,
            snapshots: AtomicUsize::new(0),
            open_streams: AtomicUsize::new(0),
            hits: Mutex::default(),
            refusal: Mutex::default(),
            guarded: AtomicUsize::new(0),
            silent: AtomicBool::new(false),
            forward: Mutex::new("echo"),
            tunnels: AtomicUsize::new(0),
        })
    }
}

/// Like the daemon's tailnet guard: runs before routing.
async fn guard(State(state): State<Arc<FakeState>>, request: Request, next: Next) -> Response {
    state.guarded.fetch_add(1, Ordering::SeqCst);
    let refusal = state.refusal.lock().unwrap().clone();
    match refusal {
        Some((status, code, message)) => {
            (status, axum::Json(json!({"error": message, "code": code}))).into_response()
        }
        None => next.run(request).await,
    }
}

/// How the fake daemon listens.
enum Listen {
    Unix {
        _dir: tempfile::TempDir,
        socket: PathBuf,
    },
    /// 127.0.0.1 on a port picked when the daemon was made.
    Tcp { port: u16 },
}

/// Counts a stream as open until dropped.
struct Open(Arc<FakeState>);

impl Open {
    fn new(state: &Arc<FakeState>) -> Self {
        state.open_streams.fetch_add(1, Ordering::SeqCst);
        Self(state.clone())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open_streams.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FakeDaemon {
    listen: Listen,
    state: Arc<FakeState>,
    server: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    /// A daemon that isn't running yet: no socket.
    fn stopped() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cos.sock");
        Self {
            listen: Listen::Unix { _dir: dir, socket },
            state: FakeState::new(),
            server: None,
        }
    }

    fn start() -> Self {
        let mut daemon = Self::stopped();
        daemon.run();
        daemon
    }

    /// A tailnet listener on 127.0.0.1 that isn't running yet: its port is
    /// free, so connecting is refused.
    fn stopped_tcp() -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Self {
            listen: Listen::Tcp { port },
            state: FakeState::new(),
            server: None,
        }
    }

    fn start_tcp() -> Self {
        let mut daemon = Self::stopped_tcp();
        daemon.run();
        daemon
    }

    fn socket(&self) -> &PathBuf {
        match &self.listen {
            Listen::Unix { socket, .. } => socket,
            Listen::Tcp { .. } => panic!("a TCP daemon has no socket"),
        }
    }

    fn address(&self) -> String {
        match &self.listen {
            Listen::Unix { socket, .. } => socket.display().to_string(),
            Listen::Tcp { port } => format!("127.0.0.1:{port}"),
        }
    }

    fn bridge(&self) -> Bridge {
        match &self.listen {
            Listen::Unix { socket, .. } => fast(Bridge::new(socket)),
            Listen::Tcp { .. } => fast(Bridge::to(Target::tcp(&self.address()).unwrap())),
        }
    }

    fn run(&mut self) {
        assert!(self.server.is_none(), "already running");
        let app = Router::new()
            .route("/v1/status", get(status))
            .route("/v1/events/stream", get(events_stream))
            .route("/v1/agents/{id}/terminal", get(terminal))
            .route("/v1/shells/{id}/terminal", get(terminal))
            .route("/v1/host", get(host))
            .route("/v1/forward/{port}", get(forward))
            .fallback(echo)
            .layer(middleware::from_fn_with_state(self.state.clone(), guard))
            .with_state(self.state.clone());
        self.server = Some(match &self.listen {
            Listen::Unix { socket, .. } => {
                let _ = std::fs::remove_file(socket);
                let listener = UnixListener::bind(socket).unwrap();
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                })
            }
            Listen::Tcp { port } => {
                let listener = std::net::TcpListener::bind(("127.0.0.1", *port)).unwrap();
                listener.set_nonblocking(true).unwrap();
                let listener = TcpListener::from_std(listener).unwrap();
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                })
            }
        });
    }

    /// Ends every open stream, as a dropped connection would.
    fn drop_streams(&self) {
        self.state.drop_streams.send_modify(|generation| *generation += 1);
    }

    /// Stops listening and removes the socket, like a daemon that quit.
    fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
        self.drop_streams();
        if let Listen::Unix { socket, .. } = &self.listen {
            std::fs::remove_file(socket).unwrap();
        }
    }

    /// Refuses every request from now on, as the tailnet guard does.
    fn refuse(&self, status: StatusCode, code: &str, message: &str) {
        *self.state.refusal.lock().unwrap() = Some((status, code.to_owned(), message.to_owned()));
    }

    /// Lets requests in again, as approving the device does.
    fn admit(&self) {
        *self.state.refusal.lock().unwrap() = None;
    }

    fn guarded(&self) -> usize {
        self.state.guarded.load(Ordering::SeqCst)
    }

    /// Appends an event with the next seq.
    fn log(&self, kind: &str) -> u64 {
        let mut log = self.state.log.lock().unwrap();
        let seq = log.events.len() as u64 + 1;
        let event = json!({"seq": seq, "ts": "2026-10-22T09:00:00Z", "kind": kind, "data": {}});
        log.events.push(event.clone());
        let _ = self.state.live.send(event);
        seq
    }

    /// Wipes the log, as a daemon with a new database would have.
    fn forget_log(&self) {
        *self.state.log.lock().unwrap() = Log::default();
    }

    fn terminal(&self, event: &str, data: Value) {
        self.state.terminal.send((event.to_owned(), data)).unwrap();
    }

    fn afters(&self) -> Vec<u64> {
        self.state.afters.lock().unwrap().clone()
    }

    fn open_streams(&self) -> usize {
        self.state.open_streams.load(Ordering::SeqCst)
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

async fn status(State(state): State<Arc<FakeState>>) -> impl IntoResponse {
    let log = state.log.lock().unwrap();
    let last_seq = log.last_seq.unwrap_or(log.events.len() as u64);
    axum::Json(json!({"version": "test", "pid": 1, "events": log.events.len(), "last_seq": last_seq}))
}

#[derive(Deserialize)]
struct After {
    #[serde(default)]
    after: u64,
}

/// Keep-alives every 20 ms, so the fake notices a client that went away.
fn sse(receiver: mpsc::Receiver<Result<SseEvent, Infallible>>) -> Response {
    Sse::new(ReceiverStream::new(receiver))
        .keep_alive(KeepAlive::new().interval(Duration::from_millis(20)))
        .into_response()
}

async fn events_stream(State(state): State<Arc<FakeState>>, Query(query): Query<After>) -> Response {
    let after = query.after;
    state.afters.lock().unwrap().push(after);
    if state.silent.load(Ordering::SeqCst) {
        let body = axum::body::Body::from_stream(tokio_stream::pending::<Result<Bytes, Infallible>>());
        return ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }
    let (stored, mut live) = {
        let log = state.log.lock().unwrap();
        let stored: Vec<Value> = log
            .events
            .iter()
            .filter(|event| event["seq"].as_u64().unwrap() > after)
            .cloned()
            .collect();
        (stored, state.live.subscribe())
    };
    let mut dropped = state.drop_streams.subscribe();
    let open = Open::new(&state);
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let _open = open;
        let to_sse = |event: &Value| {
            Ok(SseEvent::default()
                .id(event["seq"].to_string())
                .data(event.to_string()))
        };
        for event in &stored {
            if sender.send(to_sse(event)).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                event = live.recv() => {
                    let Ok(event) = event else { return };
                    if event["seq"].as_u64().unwrap() > after && sender.send(to_sse(&event)).await.is_err() {
                        return;
                    }
                }
                _ = dropped.changed() => return,
                _ = sender.closed() => return,
            }
        }
    });
    sse(receiver)
}

async fn terminal(State(state): State<Arc<FakeState>>, Path(id): Path<i64>) -> Response {
    if id == 404 {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"error": "no agent 404"})),
        )
            .into_response();
    }
    let number = state.snapshots.fetch_add(1, Ordering::SeqCst) + 1;
    let mut live = state.terminal.subscribe();
    let mut dropped = state.drop_streams.subscribe();
    let open = Open::new(&state);
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let _open = open;
        let snapshot = json!({"rows": 24, "cols": 80, "data": BASE64.encode(format!("screen {number}"))});
        let event = |name: &str, data: &Value| Ok(SseEvent::default().event(name).data(data.to_string()));
        if sender.send(event("snapshot", &snapshot)).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                update = live.recv() => {
                    let Ok((name, data)) = update else { return };
                    if sender.send(event(&name, &data)).await.is_err() || name == "exit" {
                        return;
                    }
                }
                _ = dropped.changed() => return,
                _ = sender.closed() => return,
            }
        }
    });
    sse(receiver)
}

/// Like the daemon's forward route, with an echo server behind every port.
async fn forward(
    State(state): State<Arc<FakeState>>,
    Path(port): Path<u16>,
    mut request: Request,
) -> Response {
    let mode = *state.forward.lock().unwrap();
    if mode == "old" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let upgrade = request.headers().get("upgrade").cloned();
    let Some(upgrade) = upgrade else {
        let listening = mode == "echo";
        let address = listening.then(|| format!("127.0.0.1:{port}"));
        return axum::Json(json!({"port": port, "listening": listening, "address": address})).into_response();
    };
    assert_eq!(upgrade, "cos-forward");
    if mode == "nothing" {
        return (
            StatusCode::BAD_GATEWAY,
            axum::Json(json!({"error": format!("nothing is listening on port {port} on studio"), "code": "nothing_listening"})),
        )
            .into_response();
    }
    state.tunnels.fetch_add(1, Ordering::SeqCst);
    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        let Ok(upgraded) = on_upgrade.await else { return };
        let io = hyper_util::rt::TokioIo::new(upgraded);
        let (mut read, mut write) = tokio::io::split(io);
        let _ = tokio::io::copy(&mut read, &mut write).await;
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut write).await;
    });
    (
        StatusCode::SWITCHING_PROTOCOLS,
        [("connection", "upgrade"), ("upgrade", "cos-forward")],
    )
        .into_response()
}

/// The `Host` header the bridge sent.
async fn host(headers: HeaderMap) -> String {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// Everything else: a few fixed answers, and an echo of the request.
async fn echo(
    State(state): State<Arc<FakeState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.hits.lock().unwrap().push(format!("{method} {uri}"));
    match uri.path() {
        "/v1/upload" => {
            let content_type = headers.get("content-type").and_then(|v| v.to_str().ok());
            (
                StatusCode::CREATED,
                axum::Json(json!({"content_type": content_type, "bytes": BASE64.encode(&body)})),
            )
                .into_response()
        }
        "/v1/file.png" => ([("content-type", "image/png")], b"\x89PNG".to_vec()).into_response(),
        "/v1/empty" => StatusCode::NO_CONTENT.into_response(),
        "/v1/text" => "plain text".into_response(),
        "/v1/missing" => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"error": "no such thing"})),
        )
            .into_response(),
        _ => {
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            axum::Json(json!({"method": method.as_str(), "uri": uri.to_string(), "body": body}))
                .into_response()
        }
    }
}

fn snapshot(number: usize) -> TerminalMessage {
    TerminalMessage::Snapshot {
        rows: 24,
        cols: 80,
        data: BASE64.encode(format!("screen {number}")),
    }
}

fn seq(message: &EventMessage) -> u64 {
    match message {
        EventMessage::Event { event } => event["seq"].as_u64().unwrap(),
        other => panic!("expected an event, got {other:?}"),
    }
}

fn assert_connected(message: &EventMessage, last_seq: u64) {
    match message {
        EventMessage::Connected { status } => assert_eq!(status["last_seq"], last_seq, "{status}"),
        other => panic!("expected connected, got {other:?}"),
    }
}

// Requests

#[tokio::test]
async fn files_go_up_and_come_down_as_base64() {
    let daemon = FakeDaemon::start();
    let bridge = daemon.bridge();
    let up = bridge
        .upload("/v1/upload", "image/png", &BASE64.encode(b"\x00\x01raw"))
        .await
        .unwrap();
    assert_eq!(up.status, 201);
    let body = up.body.unwrap();
    assert_eq!(body["content_type"], "image/png");
    assert_eq!(body["bytes"], BASE64.encode(b"\x00\x01raw"));
    let typeless = bridge.upload("/v1/upload", "", "").await.unwrap().body.unwrap();
    assert_eq!(typeless["content_type"], "application/octet-stream");
    assert_eq!(
        bridge
            .upload("/v1/upload", "text/plain", "not base64!")
            .await
            .unwrap_err()
            .kind,
        cos_bridge::ErrorKind::BadRequest
    );
    assert!(bridge.upload("/elsewhere", "text/plain", "").await.is_err());

    let file = bridge.fetch_bytes("/v1/file.png").await.unwrap();
    assert_eq!(file.content_type, "image/png");
    assert_eq!(file.data, BASE64.encode(b"\x89PNG"));
    let missing = bridge.fetch_bytes("/v1/missing").await.unwrap_err();
    assert_eq!(missing.message, "no such thing");
}

#[tokio::test]
async fn requests_pass_bodies_and_statuses_through() {
    check_requests(&FakeDaemon::start()).await;
}

#[tokio::test]
async fn requests_work_over_tcp() {
    let daemon = FakeDaemon::start_tcp();
    check_requests(&daemon).await;
    // The Host header names the host, as the CLI's `--host` sends it.
    let response = daemon.bridge().request("GET", "/v1/host", None).await.unwrap();
    assert_eq!(response.body, Some(json!(daemon.address())));
    let response = FakeDaemon::start()
        .bridge()
        .request("GET", "/v1/host", None)
        .await
        .unwrap();
    assert_eq!(response.body, Some(json!("cos")));
}

async fn check_requests(daemon: &FakeDaemon) {
    let bridge = daemon.bridge();

    let response = bridge
        .request("POST", "/v1/projects?x=1", Some(json!({"path": "/repo"})))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body,
        Some(json!({"method": "POST", "uri": "/v1/projects?x=1", "body": {"path": "/repo"}}))
    );

    let response = bridge
        .request("patch", "/v1/workspaces/3", Some(json!({"title": "t"})))
        .await
        .unwrap();
    assert_eq!(response.body.unwrap()["method"], "PATCH");

    let response = bridge.request("DELETE", "/v1/empty", None).await.unwrap();
    assert_eq!((response.status, response.body), (204, None));

    let response = bridge.request("GET", "/v1/text", None).await.unwrap();
    assert_eq!(response.body, Some(json!("plain text")));

    // An error status is an answer, not an error.
    let response = bridge.request("GET", "/v1/missing", None).await.unwrap();
    assert_eq!(response.status, 404);
    assert_eq!(response.body, Some(json!({"error": "no such thing"})));
}

#[tokio::test]
async fn paths_outside_the_api_are_refused_before_sending() {
    let daemon = FakeDaemon::start();
    let bridge = daemon.bridge();
    for path in ["/secret", "/v1/../secret", "v1/status", "/v2/status"] {
        let error = bridge.request("GET", path, None).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::BadRequest, "{path}");
    }
    let error = bridge.request("TRACE", "/v1/status", None).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadRequest);
    assert!(daemon.state.hits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_socket_means_not_running() {
    let daemon = FakeDaemon::stopped();
    let error = daemon
        .bridge()
        .request("GET", "/v1/status", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotRunning);
    assert!(error.message.contains("not running"), "{}", error.message);

    // A socket left behind with nobody listening is the same.
    drop(std::os::unix::net::UnixListener::bind(daemon.socket()).unwrap());
    assert!(daemon.socket().exists());
    let error = daemon
        .bridge()
        .request("GET", "/v1/status", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotRunning);
}

#[tokio::test]
async fn a_host_that_cant_be_reached_is_not_running() {
    let daemon = FakeDaemon::stopped_tcp();
    let error = daemon
        .bridge()
        .request("GET", "/v1/status", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotRunning);
    let expected = format!("could not reach {}", daemon.address());
    assert!(error.message.starts_with(&expected), "{}", error.message);
}

#[tokio::test]
async fn a_refusal_is_a_normal_answer() {
    let daemon = FakeDaemon::start_tcp();
    let message = "this device (laptop) is waiting for approval. On studio, run `cos devices approve 2`";
    daemon.refuse(StatusCode::FORBIDDEN, "device_pending", message);
    let response = daemon
        .bridge()
        .request("GET", "/v1/projects", None)
        .await
        .unwrap();
    assert_eq!(response.status, 403);
    assert_eq!(
        response.body,
        Some(json!({"error": message, "code": "device_pending"}))
    );
    // The guard runs before routing, so nothing reached the routes.
    assert!(daemon.state.hits.lock().unwrap().is_empty());
}

// Events

#[tokio::test]
async fn follow_works_over_tcp() {
    let daemon = FakeDaemon::start_tcp();
    daemon.log("daemon.started");
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 1);
    assert_eq!(seq(&next(&mut messages).await), 1);
    daemon.log("project.added");
    assert_eq!(seq(&next(&mut messages).await), 2);

    daemon.drop_streams();
    assert!(matches!(
        next(&mut messages).await,
        EventMessage::Disconnected {
            not_running: false,
            code: None,
            ..
        }
    ));
    assert_connected(&next(&mut messages).await, 2);
    daemon.log("workspace.created");
    assert_eq!(seq(&next(&mut messages).await), 3);
    assert_eq!(daemon.afters(), [0, 2]);
}

#[tokio::test]
async fn a_silent_stream_over_tcp_is_reopened() {
    let daemon = FakeDaemon::start_tcp();
    daemon.log("daemon.started");
    daemon.state.silent.store(true, Ordering::SeqCst);
    // Slow enough to reconnect that the test speaks up again first.
    let bridge = daemon
        .bridge()
        .with_read_timeout(Duration::from_millis(200))
        .with_backoff(Backoff {
            initial: Duration::from_millis(300),
            max: Duration::from_millis(300),
        });
    let (_handle, mut messages) = collect(|sink| bridge.follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 1);
    // Nothing comes, not even a keep-alive: the connection is given up.
    match next(&mut messages).await {
        EventMessage::Disconnected {
            not_running, reason, ..
        } => {
            assert!(!not_running);
            assert!(reason.contains("no word from the daemon"), "{reason}");
        }
        other => panic!("expected a disconnect, got {other:?}"),
    }
    daemon.state.silent.store(false, Ordering::SeqCst);
    let connected = next(&mut messages).await;
    assert_connected(&connected, 1);
    assert_eq!(seq(&next(&mut messages).await), 1);
}

#[tokio::test]
async fn keep_alives_hold_a_quiet_stream_open() {
    let daemon = FakeDaemon::start_tcp();
    daemon.log("daemon.started");
    // The fake's keep-alives come every 20 ms.
    let bridge = daemon.bridge().with_read_timeout(Duration::from_millis(200));
    let (_handle, mut messages) = collect(|sink| bridge.follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 1);
    assert_eq!(seq(&next(&mut messages).await), 1);
    tokio::time::sleep(Duration::from_millis(600)).await;
    daemon.log("project.added");
    assert_eq!(seq(&next(&mut messages).await), 2);
    assert_eq!(daemon.afters(), [0]);
}

#[tokio::test]
async fn a_refused_follow_says_why_and_waits_before_asking_again() {
    let daemon = FakeDaemon::start_tcp();
    daemon.log("daemon.started");
    let message = "this device (laptop) is waiting for approval. On studio, run `cos devices approve 2`";
    daemon.refuse(StatusCode::FORBIDDEN, "device_pending", message);
    let bridge = daemon.bridge().with_backoff(Backoff {
        initial: Duration::from_millis(10),
        max: Duration::from_millis(300),
    });
    let (_handle, mut messages) = collect(|sink| bridge.follow_events(None, sink));
    assert_eq!(
        next(&mut messages).await,
        EventMessage::Disconnected {
            reason: message.into(),
            not_running: false,
            code: Some("device_pending".into()),
        }
    );

    // Doubling from 10 ms would ask about six times in 450 ms; a refusal
    // waits the longest backoff (300 ms) every time instead.
    tokio::time::sleep(Duration::from_millis(450)).await;
    let asked = daemon.guarded();
    assert!((2..=3).contains(&asked), "asked {asked} times");
    // The same refusal isn't repeated.
    assert!(messages.try_recv().is_err());

    // Approved: the next try connects and catches up.
    daemon.admit();
    assert_connected(&next(&mut messages).await, 1);
    assert_eq!(seq(&next(&mut messages).await), 1);
}

#[tokio::test]
async fn a_refusal_from_the_stream_itself_carries_its_code() {
    // Tailscale's whois can fail on one request and not the next.
    let daemon = FakeDaemon::start_tcp();
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 0);
    daemon.refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        "tailscale_unavailable",
        "tailscale whois failed",
    );
    daemon.drop_streams();
    let message = loop {
        match next(&mut messages).await {
            EventMessage::Disconnected { code: None, .. } => {}
            other => break other,
        }
    };
    assert_eq!(
        message,
        EventMessage::Disconnected {
            reason: "tailscale whois failed".into(),
            not_running: false,
            code: Some("tailscale_unavailable".into()),
        }
    );
}

#[tokio::test]
async fn follow_sends_connected_then_events_in_order() {
    let daemon = FakeDaemon::start();
    daemon.log("daemon.started");
    daemon.log("project.added");
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));

    assert_connected(&next(&mut messages).await, 2);
    assert_eq!(seq(&next(&mut messages).await), 1);
    assert_eq!(seq(&next(&mut messages).await), 2);
    daemon.log("workspace.created");
    let message = next(&mut messages).await;
    assert_eq!(seq(&message), 3);
    let EventMessage::Event { event } = message else {
        unreachable!()
    };
    assert_eq!(event["kind"], "workspace.created");
    assert_eq!(event["ts"], "2026-10-22T09:00:00Z");
}

#[tokio::test]
async fn follow_starts_after_the_given_seq() {
    let daemon = FakeDaemon::start();
    daemon.log("a");
    daemon.log("b");
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(Some(1), sink));
    assert_connected(&next(&mut messages).await, 2);
    assert_eq!(seq(&next(&mut messages).await), 2);
    assert_eq!(daemon.afters(), [1]);
}

#[tokio::test]
async fn follow_reconnects_and_resumes_after_a_drop() {
    let daemon = FakeDaemon::start();
    daemon.log("a");
    daemon.log("b");
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 2);
    assert_eq!(seq(&next(&mut messages).await), 1);
    assert_eq!(seq(&next(&mut messages).await), 2);

    daemon.drop_streams();
    assert!(matches!(
        next(&mut messages).await,
        EventMessage::Disconnected {
            not_running: false,
            ..
        }
    ));
    assert_connected(&next(&mut messages).await, 2);
    daemon.log("c");
    // Nothing is delivered twice: the next message is the new event.
    assert_eq!(seq(&next(&mut messages).await), 3);
    assert_eq!(daemon.afters(), [0, 2]);
}

#[tokio::test]
async fn follow_waits_for_the_daemon_and_survives_restarts() {
    let mut daemon = FakeDaemon::stopped();
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert!(matches!(
        next(&mut messages).await,
        EventMessage::Disconnected {
            not_running: true,
            ..
        }
    ));

    daemon.run();
    daemon.log("daemon.started");
    // Retries that fail the same way don't repeat `disconnected`, so the
    // next message is `connected`. The event may land before or after the
    // status was read.
    match next(&mut messages).await {
        EventMessage::Connected { status } => assert!(status["last_seq"].as_u64().unwrap() <= 1),
        other => panic!("expected connected, got {other:?}"),
    }
    assert_eq!(seq(&next(&mut messages).await), 1);

    daemon.stop();
    let mut saw_not_running = false;
    loop {
        match next(&mut messages).await {
            EventMessage::Disconnected { not_running, .. } => saw_not_running |= not_running,
            other => panic!("expected disconnected, got {other:?}"),
        }
        if saw_not_running {
            break;
        }
    }

    daemon.run();
    assert_connected(&next(&mut messages).await, 1);
    daemon.log("daemon.started");
    assert_eq!(seq(&next(&mut messages).await), 2);
    assert_eq!(daemon.afters().last(), Some(&1));
}

#[tokio::test]
async fn a_shorter_log_after_a_restart_resets() {
    let mut daemon = FakeDaemon::start();
    for kind in ["a", "b", "c"] {
        daemon.log(kind);
    }
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 3);
    for expected in 1..=3 {
        assert_eq!(seq(&next(&mut messages).await), expected);
    }

    daemon.stop();
    daemon.forget_log();
    daemon.log("daemon.started");
    daemon.run();
    let message = loop {
        match next(&mut messages).await {
            EventMessage::Disconnected { .. } => {}
            other => break other,
        }
    };
    assert_connected(&message, 1);
    assert_eq!(next(&mut messages).await, EventMessage::LogReset);
    assert_eq!(seq(&next(&mut messages).await), 1);
    assert_eq!(daemon.afters().last(), Some(&0));
}

#[tokio::test]
async fn a_cursor_past_the_end_resets_too() {
    let daemon = FakeDaemon::start();
    daemon.log("a");
    daemon.state.log.lock().unwrap().last_seq = Some(1);
    let (_handle, mut messages) = collect(|sink| daemon.bridge().follow_events(Some(9), sink));
    assert_connected(&next(&mut messages).await, 1);
    assert_eq!(next(&mut messages).await, EventMessage::LogReset);
    assert_eq!(seq(&next(&mut messages).await), 1);
}

#[tokio::test]
async fn closing_a_stream_drops_its_connection() {
    let daemon = FakeDaemon::start();
    let (handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 0);
    eventually("the stream to open", || daemon.open_streams() == 1).await;

    handle.close();
    eventually("the daemon to see the stream close", || {
        daemon.open_streams() == 0
    })
    .await;
    daemon.log("a");
    // The sink was dropped with the stream, and nothing came after close.
    assert_eq!(within("the stream to end", messages.recv()).await, None);
    assert!(handle.is_finished());
}

#[tokio::test]
async fn dropping_the_handle_closes_the_stream() {
    let daemon = FakeDaemon::start();
    let (handle, mut messages) = collect(|sink| daemon.bridge().follow_events(None, sink));
    assert_connected(&next(&mut messages).await, 0);
    eventually("the stream to open", || daemon.open_streams() == 1).await;
    drop(handle);
    eventually("the daemon to see the stream close", || {
        daemon.open_streams() == 0
    })
    .await;
}

#[tokio::test]
async fn a_gone_sink_ends_the_stream() {
    let daemon = FakeDaemon::start();
    let handle = daemon.bridge().follow_events(None, |_: EventMessage| false);
    eventually("the stream to end", || handle.is_finished()).await;
}

// Terminals

#[tokio::test]
async fn terminal_messages_pass_through() {
    let daemon = FakeDaemon::start();
    let (handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(7, sink));
    assert_eq!(next(&mut messages).await, snapshot(1));

    daemon.terminal("output", json!({"data": "aGk="}));
    daemon.terminal("resize", json!({"rows": 50, "cols": 160}));
    daemon.terminal("exit", json!({"exit_code": 0}));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Output { data: "aGk=".into() }
    );
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Resize { rows: 50, cols: 160 }
    );
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Exit { exit_code: Some(0) }
    );

    // After `exit` the stream is over and doesn't reconnect.
    assert_eq!(within("the stream to end", messages.recv()).await, None);
    assert!(handle.is_finished());
    assert_eq!(daemon.state.snapshots.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_shell_terminal_streams_from_the_shells_route() {
    let daemon = FakeDaemon::start();
    let (handle, mut messages) = collect(|sink| daemon.bridge().attach_shell(3, sink));
    assert_eq!(next(&mut messages).await, snapshot(1));
    daemon.terminal("exit", json!({"exit_code": 0}));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Exit { exit_code: Some(0) }
    );
    assert_eq!(within("the stream to end", messages.recv()).await, None);
    assert!(handle.is_finished());
}

#[tokio::test]
async fn a_terminal_reconnect_starts_from_a_fresh_snapshot() {
    let daemon = FakeDaemon::start();
    let (_handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(7, sink));
    assert_eq!(next(&mut messages).await, snapshot(1));
    eventually("the stream to open", || daemon.open_streams() == 1).await;

    daemon.drop_streams();
    assert!(matches!(
        next(&mut messages).await,
        TerminalMessage::Disconnected { .. }
    ));
    assert_eq!(next(&mut messages).await, snapshot(2));
    eventually("the stream to reopen", || daemon.open_streams() == 1).await;
    daemon.terminal("exit", json!({"exit_code": null}));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Exit { exit_code: None }
    );
}

#[tokio::test]
async fn a_terminal_waits_for_the_daemon() {
    let mut daemon = FakeDaemon::stopped();
    let (_handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(7, sink));
    let TerminalMessage::Disconnected { reason, code: None } = next(&mut messages).await else {
        panic!("expected disconnected");
    };
    assert!(reason.contains("not running"), "{reason}");
    daemon.run();
    assert_eq!(next(&mut messages).await, snapshot(1));
}

#[tokio::test]
async fn an_unknown_agent_reports_the_daemons_error() {
    let daemon = FakeDaemon::start();
    let (handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(404, sink));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Disconnected {
            reason: "no agent 404".into(),
            code: None,
        }
    );
    handle.close();
    assert_eq!(within("the stream to end", messages.recv()).await, None);
}

#[tokio::test]
async fn terminals_work_over_tcp() {
    let mut daemon = FakeDaemon::stopped_tcp();
    let (handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(7, sink));
    let TerminalMessage::Disconnected { reason, code: None } = next(&mut messages).await else {
        panic!("expected disconnected");
    };
    assert!(reason.contains("could not reach"), "{reason}");

    daemon.run();
    assert_eq!(next(&mut messages).await, snapshot(1));
    eventually("the stream to open", || daemon.open_streams() == 1).await;
    daemon.terminal("output", json!({"data": "aGk="}));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Output { data: "aGk=".into() }
    );
    daemon.drop_streams();
    assert!(matches!(
        next(&mut messages).await,
        TerminalMessage::Disconnected { code: None, .. }
    ));
    assert_eq!(next(&mut messages).await, snapshot(2));
    eventually("the stream to reopen", || daemon.open_streams() == 1).await;
    daemon.terminal("exit", json!({"exit_code": 0}));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Exit { exit_code: Some(0) }
    );
    assert_eq!(within("the stream to end", messages.recv()).await, None);
    assert!(handle.is_finished());
}

#[tokio::test]
async fn a_refused_terminal_carries_the_code_until_approved() {
    let daemon = FakeDaemon::start_tcp();
    let message = "this device (laptop) was denied. On studio, run `cos devices approve 2` to let it in";
    daemon.refuse(StatusCode::FORBIDDEN, "device_denied", message);
    let (_handle, mut messages) = collect(|sink| daemon.bridge().attach_terminal(7, sink));
    assert_eq!(
        next(&mut messages).await,
        TerminalMessage::Disconnected {
            reason: message.into(),
            code: Some("device_denied".into()),
        }
    );
    daemon.admit();
    assert_eq!(next(&mut messages).await, snapshot(1));
}

// Port forwarding

impl FakeDaemon {
    fn forward_mode(&self, mode: &'static str) {
        *self.state.forward.lock().unwrap() = mode;
    }
}

#[tokio::test]
async fn a_tunnel_echoes_over_the_socket_and_tcp() {
    for daemon in [FakeDaemon::start(), FakeDaemon::start_tcp()] {
        let mut tunnel = daemon.bridge().open_tunnel(5173).await.unwrap();
        tunnel.write_all(b"hello").await.unwrap();
        let mut back = [0u8; 5];
        within("echo", tunnel.read_exact(&mut back)).await.unwrap();
        assert_eq!(&back, b"hello");
        tunnel.shutdown().await.unwrap();
        let mut rest = Vec::new();
        within("eof", tunnel.read_to_end(&mut rest)).await.unwrap();
        assert!(rest.is_empty());
    }
}

#[tokio::test]
async fn tunnel_errors_keep_their_codes() {
    let daemon = FakeDaemon::start_tcp();
    daemon.forward_mode("old");
    let error = daemon.bridge().open_tunnel(5173).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("too_old"));

    daemon.forward_mode("nothing");
    let error = daemon.bridge().open_tunnel(5173).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("nothing_listening"));

    daemon.refuse(StatusCode::FORBIDDEN, "device_pending", "wait for approval");
    let error = daemon.bridge().open_tunnel(5173).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("device_pending"));

    let error = FakeDaemon::stopped_tcp()
        .bridge()
        .open_tunnel(5173)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotRunning);
    let error = daemon.bridge().open_tunnel(0).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadRequest);
}

/// A free port on 127.0.0.1 whose next ten are free too, so a forward to it
/// lands on it.
fn free_port() -> u16 {
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if port < 65000 {
            return port;
        }
    }
}

async fn connect(port: u16) -> tokio::net::TcpStream {
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap()
}

#[tokio::test]
async fn a_forward_serves_a_real_local_port_end_to_end() {
    let daemon = FakeDaemon::start_tcp();
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let port = free_port();
    let forward = set.add(port, None).await.unwrap();
    assert_eq!((forward.remote_port, forward.local_port), (port, port));
    assert_eq!(forward.remote, RemoteState::Listening);
    assert_eq!(forward.message, None);

    let mut stream = connect(port).await;
    stream.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    let mut back = [0u8; 18];
    within("echo", stream.read_exact(&mut back)).await.unwrap();
    assert_eq!(&back, b"GET / HTTP/1.1\r\n\r\n");
    eventually("one connection", || set.get(port).unwrap().connections == 1).await;
    drop(stream);
    eventually("no connections", || set.get(port).unwrap().connections == 0).await;

    // Twice the same port is refused.
    let error = set.add(port, None).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadRequest);
    assert_eq!(set.ports(), vec![port]);
}

#[tokio::test]
async fn a_busy_local_port_moves_up() {
    let daemon = FakeDaemon::start();
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let port = free_port();
    let _taken = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let forward = set.add(port, None).await.unwrap();
    assert_ne!(forward.local_port, port);
    assert!(
        forward.local_port > port && forward.local_port <= port + 10,
        "{forward:?}"
    );
    let message = forward.message.unwrap();
    assert!(
        message.contains(&format!("localhost:{port} was taken")),
        "{message}"
    );

    let mut stream = connect(forward.local_port).await;
    stream.write_all(b"moved").await.unwrap();
    let mut back = [0u8; 5];
    within("echo", stream.read_exact(&mut back)).await.unwrap();
    assert_eq!(&back, b"moved");

    // A failure replaces the note, and a tunnel that works brings it back.
    daemon.forward_mode("nothing");
    let mut failed = connect(forward.local_port).await;
    failed.write_all(b"\x16").await.unwrap();
    let mut rest = Vec::new();
    // Closed, or reset when the byte is left unread.
    let _ = within("close", failed.read_to_end(&mut rest)).await;
    let local = forward.local_port;
    eventually("nothing listening", || {
        set.get(port).unwrap().remote == RemoteState::NothingListening
    })
    .await;
    assert!(!set.get(port).unwrap().message.unwrap().contains("was taken"));
    daemon.forward_mode("echo");
    let mut again = connect(local).await;
    again.write_all(b"ok").await.unwrap();
    let mut back = [0u8; 2];
    within("echo", again.read_exact(&mut back)).await.unwrap();
    eventually("listening", || {
        set.get(port).unwrap().remote == RemoteState::Listening
    })
    .await;
    assert_eq!(set.get(port).unwrap().message.as_deref(), Some(message.as_str()));
}

/// Records the redirect tables, or refuses them like a cancelled password
/// prompt.
#[derive(Clone, Default)]
struct FakeRedirector {
    tables: Arc<Mutex<Vec<Table>>>,
    refuse: Arc<AtomicBool>,
}

impl Redirector for FakeRedirector {
    fn apply(&self, table: &Table) -> Result<(), String> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err("you cancelled the password prompt".into());
        }
        self.tables.lock().unwrap().push(table.clone());
        Ok(())
    }
}

#[tokio::test]
async fn a_port_below_1024_listens_higher_and_is_redirected_while_forwarded() {
    let daemon = FakeDaemon::start();
    let redirector = FakeRedirector::default();
    let redirects = Redirects::new(redirector.clone());
    let set = ForwardSet::with_redirects(daemon.bridge(), "Studio", Some(redirects.clone()));
    let forward = set.add(443, None).await.unwrap();
    assert_eq!(forward.local_port, 443);
    let via = forward.via.unwrap();
    assert!((8443..=8453).contains(&via), "{forward:?}");
    assert_eq!(redirects.table(), Table::from([(443, via)]));

    // The browser's localhost:443 lands on `via`, which pf stands in for here.
    let mut stream = connect(via).await;
    stream.write_all(b"tls").await.unwrap();
    let mut back = [0u8; 3];
    within("echo", stream.read_exact(&mut back)).await.unwrap();
    assert_eq!(&back, b"tls");
    drop(stream);

    // A local port of 1024 or more is never redirected.
    let port = free_port();
    let plain = set.add(port, None).await.unwrap();
    assert_eq!((plain.local_port, plain.via), (port, None));
    assert_eq!(redirects.table(), Table::from([(443, via)]));

    // Removing the forward removes its redirect and closes `via`.
    set.remove(443);
    assert!(redirects.table().is_empty());
    assert_eq!(redirector.tables.lock().unwrap().last(), Some(&Table::new()));
    eventually("via closed", || {
        std::net::TcpStream::connect(("127.0.0.1", via)).is_err()
    })
    .await;

    // A refused redirect is an error, and nothing is kept or left listening.
    redirector.refuse.store(true, Ordering::SeqCst);
    let error = set.add(80, None).await.unwrap_err();
    assert_eq!(
        error.message,
        "Couldn't redirect localhost:80 to 8080: You cancelled the password prompt."
    );
    assert_eq!(set.ports(), vec![port]);
    assert!(std::net::TcpStream::connect(("127.0.0.1", 8080)).is_err());

    // Dropping the set (another host selected) removes its redirects too.
    redirector.refuse.store(false, Ordering::SeqCst);
    set.add(80, None).await.unwrap();
    assert_eq!(redirects.table().keys().copied().collect::<Vec<_>>(), [80]);
    drop(set);
    assert!(redirects.table().is_empty());
}

#[tokio::test]
async fn nothing_listening_gets_a_502_page_for_http_and_a_close_for_tls() {
    let daemon = FakeDaemon::start_tcp();
    daemon.forward_mode("nothing");
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let port = free_port();
    let forward = set.add(port, None).await.unwrap();
    assert_eq!(forward.remote, RemoteState::NothingListening);

    let mut stream = connect(port).await;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut page = String::new();
    within("502 page", stream.read_to_string(&mut page))
        .await
        .unwrap();
    assert!(page.starts_with("HTTP/1.1 502 Bad Gateway"), "{page}");
    assert!(
        page.contains(&format!("Nothing is listening on localhost:{port} on Studio.")),
        "{page}"
    );

    let mut tls = connect(port).await;
    tls.write_all(&[0x16, 0x03, 0x01, 0x00, 0x05, 0x01, 0x00, 0x00, 0x01, 0x00])
        .await
        .unwrap();
    let mut answer = Vec::new();
    let _ = within("tls close", tls.read_to_end(&mut answer)).await;
    assert!(answer.is_empty(), "{answer:?}");

    // Once something runs there, the next connection goes through.
    daemon.forward_mode("echo");
    let mut stream = connect(port).await;
    stream.write_all(b"up").await.unwrap();
    let mut back = [0u8; 2];
    within("echo", stream.read_exact(&mut back)).await.unwrap();
    eventually("listening", || {
        set.get(port).unwrap().remote == RemoteState::Listening
    })
    .await;
}

#[tokio::test]
async fn fifty_parallel_connections_each_get_their_own_bytes() {
    let daemon = FakeDaemon::start();
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let port = free_port();
    set.add(port, None).await.unwrap();

    let mut streams = Vec::new();
    for i in 0..50u32 {
        let mut stream = connect(port).await;
        stream.write_all(&i.to_be_bytes()).await.unwrap();
        streams.push(stream);
    }
    for (i, stream) in streams.iter_mut().enumerate() {
        let mut back = [0u8; 4];
        within("echo", stream.read_exact(&mut back)).await.unwrap();
        assert_eq!(u32::from_be_bytes(back), i as u32);
    }
    eventually("fifty", || set.get(port).unwrap().connections == 50).await;
    drop(streams);
    eventually("none", || set.get(port).unwrap().connections == 0).await;
}

#[tokio::test]
async fn removing_or_dropping_stops_the_listener_and_its_tunnels() {
    let daemon = FakeDaemon::start();
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let port = free_port();
    set.add(port, None).await.unwrap();
    let mut open = connect(port).await;
    open.write_all(b"x").await.unwrap();
    let mut back = [0u8; 1];
    within("echo", open.read_exact(&mut back)).await.unwrap();

    assert!(set.remove(port).forwards.is_empty());
    let mut rest = Vec::new();
    let _ = within("tunnel dropped", open.read_to_end(&mut rest)).await;
    eventually("listener closed", || {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
    })
    .await;

    let other = free_port();
    set.add(other, None).await.unwrap();
    let mut open = connect(other).await;
    open.write_all(b"y").await.unwrap();
    within("echo", open.read_exact(&mut back)).await.unwrap();
    drop(set);
    let _ = within("tunnel dropped", open.read_to_end(&mut rest)).await;
    eventually("listener closed", || {
        std::net::TcpStream::connect(("127.0.0.1", other)).is_err()
    })
    .await;
}

#[tokio::test]
async fn refusals_and_old_daemons_are_errors_but_an_unreachable_host_is_kept() {
    let daemon = FakeDaemon::start_tcp();
    let set = ForwardSet::new(daemon.bridge(), "Studio");

    daemon.forward_mode("old");
    let error = set.add(free_port(), None).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("too_old"));
    assert_eq!(
        error.message,
        "Studio's cos is too old to forward ports. Run `cos upgrade` there."
    );

    daemon.forward_mode("echo");
    daemon.refuse(StatusCode::FORBIDDEN, "device_pending", "wait for approval");
    let error = set.add(free_port(), None).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("device_pending"));
    assert!(set.ports().is_empty());

    // A saved forward restored on select is kept, refused, until let in.
    let port = free_port();
    let forward = set.restore(port).await.unwrap();
    assert_eq!(forward.remote, RemoteState::Refused);
    assert_eq!(forward.code.as_deref(), Some("device_pending"));
    assert_eq!(set.ports(), vec![port]);
    daemon.admit();
    let mut stream = connect(port).await;
    stream.write_all(b"in").await.unwrap();
    let mut back = [0u8; 2];
    within("echo", stream.read_exact(&mut back)).await.unwrap();
    eventually("listening", || {
        set.get(port).unwrap().remote == RemoteState::Listening
    })
    .await;
    set.remove(port);

    let away = ForwardSet::new(FakeDaemon::stopped_tcp().bridge(), "Laptop");
    let port = free_port();
    let forward = away.add(port, None).await.unwrap();
    assert_eq!(forward.remote, RemoteState::Unreachable);
    assert_eq!(away.ports(), vec![port]);
}

#[tokio::test]
async fn follow_sends_the_list_then_each_change() {
    let daemon = FakeDaemon::start();
    let set = ForwardSet::new(daemon.bridge(), "Studio");
    let (_handle, mut messages) = collect(|sink| set.follow(sink));
    let ForwardMessage::Forwards { host, forwards } = next(&mut messages).await;
    assert_eq!((host.as_str(), forwards.len()), ("Studio", 0));

    let port = free_port();
    set.add(port, None).await.unwrap();
    let ForwardMessage::Forwards { forwards, .. } = next(&mut messages).await;
    assert_eq!(forwards[0].remote_port, port);

    set.remove(port);
    let ForwardMessage::Forwards { forwards, .. } = next(&mut messages).await;
    assert!(forwards.is_empty());

    // Dropping the set ends the stream.
    drop(set);
    assert!(within("end", messages.recv()).await.is_none());
}
