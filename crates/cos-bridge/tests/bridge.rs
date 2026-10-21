//! The bridge against a fake daemon: a small axum server on a temporary Unix
//! socket that serves the routes the bridge uses, and can drop its streams,
//! stop, restart and lose its log on demand.

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{Method, StatusCode, Uri};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use cos_bridge::{Backoff, Bridge, ErrorKind, EventMessage, StreamHandle, TerminalMessage};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::UnixListener;
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
        })
    }
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
    _dir: tempfile::TempDir,
    socket: PathBuf,
    state: Arc<FakeState>,
    server: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    /// A daemon that isn't running yet: no socket.
    fn stopped() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cos.sock");
        Self {
            _dir: dir,
            socket,
            state: FakeState::new(),
            server: None,
        }
    }

    fn start() -> Self {
        let mut daemon = Self::stopped();
        daemon.run();
        daemon
    }

    fn bridge(&self) -> Bridge {
        fast(Bridge::new(&self.socket))
    }

    fn run(&mut self) {
        assert!(self.server.is_none(), "already running");
        let _ = std::fs::remove_file(&self.socket);
        let listener = UnixListener::bind(&self.socket).unwrap();
        let app = Router::new()
            .route("/v1/status", get(status))
            .route("/v1/events/stream", get(events_stream))
            .route("/v1/agents/{id}/terminal", get(terminal))
            .fallback(echo)
            .with_state(self.state.clone());
        self.server = Some(tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        }));
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
        std::fs::remove_file(&self.socket).unwrap();
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

/// Everything else: a few fixed answers, and an echo of the request.
async fn echo(State(state): State<Arc<FakeState>>, method: Method, uri: Uri, body: Bytes) -> Response {
    state.hits.lock().unwrap().push(format!("{method} {uri}"));
    match uri.path() {
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
async fn requests_pass_bodies_and_statuses_through() {
    let daemon = FakeDaemon::start();
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
    drop(std::os::unix::net::UnixListener::bind(&daemon.socket).unwrap());
    assert!(daemon.socket.exists());
    let error = daemon
        .bridge()
        .request("GET", "/v1/status", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotRunning);
}

// Events

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
    let TerminalMessage::Disconnected { reason } = next(&mut messages).await else {
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
            reason: "no agent 404".into()
        }
    );
    handle.close();
    assert_eq!(within("the stream to end", messages.recv()).await, None);
}
