use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{Base, EventLog, LoggedEvent, Projects, Store};
use jiff::Timestamp;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use super::{api, run_until, serve};
use crate::client::{Client, DaemonError, NotRunning};
use crate::home::CosHome;

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"))
}

/// The API served from an in-memory log, so tests can append events.
struct TestServer {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    log: EventLog,
    stop: watch::Sender<bool>,
    server: JoinHandle<std::io::Result<()>>,
}

impl TestServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cos.sock");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        let projects = Arc::new(Projects::new(store, log.clone(), dir.path().join("worktrees")));
        let (stop, stopping) = watch::channel(false);
        let state = api::AppState {
            log: log.clone(),
            projects,
            started_at: Timestamp::now(),
            home: dir.path().to_path_buf(),
            stopping: stopping.clone(),
        };
        let server = serve(UnixListener::bind(&socket).unwrap(), state, stopping);
        Self {
            _dir: dir,
            socket,
            log,
            stop,
            server,
        }
    }

    fn client(&self) -> Client {
        Client::new(self.socket.clone())
    }

    fn append(&self, kind: &str) -> LoggedEvent {
        self.log.append(kind, None, json!({})).unwrap()
    }
}

fn kinds(events: &[LoggedEvent]) -> Vec<&str> {
    events.iter().map(|event| event.kind.as_str()).collect()
}

#[tokio::test]
async fn status_and_event_pages() {
    let server = TestServer::start();
    for name in ["a", "b", "c"] {
        server.append(name);
    }
    let client = server.client();

    let status = client.status().await.unwrap();
    assert_eq!((status.events, status.last_seq), (3, 3));
    assert_eq!(status.pid, std::process::id());

    assert_eq!(kinds(&client.events(0, 10).await.unwrap()), ["a", "b", "c"]);
    assert_eq!(kinds(&client.events(1, 1).await.unwrap()), ["b"]);
}

#[tokio::test]
async fn follow_sends_the_backlog_then_live_events() {
    let server = TestServer::start();
    server.append("a");
    server.append("b");
    let mut stream = server.client().follow(1).await.unwrap();

    let first = within("backlog", stream.next()).await.unwrap().unwrap();
    assert_eq!((first.seq, first.kind.as_str()), (2, "b"));

    server.append("c");
    let live = within("live event", stream.next()).await.unwrap().unwrap();
    assert_eq!((live.seq, live.kind.as_str()), (3, "c"));
}

#[tokio::test]
async fn last_event_id_header_resumes_a_stream() {
    let server = TestServer::start();
    for name in ["a", "b", "c"] {
        server.append(name);
    }

    // What a browser or URLSession sends when it reconnects.
    let mut socket = UnixStream::connect(&server.socket).await.unwrap();
    socket
        .write_all(b"GET /v1/events/stream?after=0 HTTP/1.1\r\nHost: cos\r\nLast-Event-ID: 2\r\n\r\n")
        .await
        .unwrap();
    let mut received = String::new();
    within("resumed event", async {
        let mut buffer = [0; 4096];
        while !received.contains("\"kind\":\"c\"") {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "stream closed early: {received}");
            received.push_str(&String::from_utf8_lossy(&buffer[..read]));
        }
    })
    .await;

    assert!(received.contains("id: 3"), "{received}");
    assert!(!received.contains("\"kind\":\"a\""), "{received}");
    assert!(!received.contains("\"kind\":\"b\""), "{received}");
}

#[tokio::test]
async fn stopping_ends_open_streams_and_the_server() {
    let server = TestServer::start();
    let mut stream = server.client().follow(0).await.unwrap();

    server.stop.send(true).unwrap();

    assert!(within("stream end", stream.next()).await.unwrap().is_none());
    within("server exit", server.server).await.unwrap().unwrap();
}

#[tokio::test]
async fn client_reports_a_missing_daemon() {
    let dir = tempfile::tempdir().unwrap();

    let error = Client::new(dir.path().join("cos.sock"))
        .status()
        .await
        .unwrap_err();

    assert!(error.is::<NotRunning>(), "{error:#}");
}

/// Starts the real daemon in `home` and waits until it answers.
async fn start_daemon(home: &CosHome) -> (oneshot::Sender<String>, JoinHandle<anyhow::Result<()>>) {
    let (stop, stopped) = oneshot::channel::<String>();
    let task = tokio::spawn({
        let home = home.clone();
        async move { run_until(&home, async { stopped.await.unwrap_or_default() }).await }
    });
    let client = Client::new(home.socket());
    within("daemon start", async {
        while client.status().await.is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    (stop, task)
}

#[tokio::test]
async fn daemon_lifecycle_is_logged_and_cleaned_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = CosHome::new(dir.path().join("home"));

    let (stop, task) = start_daemon(&home).await;
    let events = Client::new(home.socket()).events(0, 10).await.unwrap();
    assert_eq!(kinds(&events), [kind::DAEMON_STARTED]);

    stop.send("test".into()).unwrap();
    within("daemon exit", task).await.unwrap().unwrap();
    assert!(!home.socket().exists(), "socket should be removed");

    // The log is on disk, and a restart continues it.
    let (stop, task) = start_daemon(&home).await;
    let events = Client::new(home.socket()).events(0, 10).await.unwrap();
    assert_eq!(
        kinds(&events),
        [kind::DAEMON_STARTED, kind::DAEMON_STOPPED, kind::DAEMON_STARTED]
    );
    assert_eq!(events[1].data, json!({ "reason": "test" }));
    stop.send("test".into()).unwrap();
    within("daemon exit", task).await.unwrap().unwrap();
}

#[tokio::test]
async fn only_one_daemon_per_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = CosHome::new(dir.path().join("home"));
    let (stop, task) = start_daemon(&home).await;

    let second = run_until(&home, async { String::new() }).await;

    let error = second.expect_err("a second daemon should be refused");
    assert!(error.to_string().contains("already running"), "{error:#}");
    // The first one is unaffected.
    assert!(Client::new(home.socket()).status().await.is_ok());
    stop.send("test".into()).unwrap();
    within("daemon exit", task).await.unwrap().unwrap();
}

#[tokio::test]
async fn stale_socket_from_a_crash_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let home = CosHome::new(dir.path().join("home"));
    home.prepare().unwrap();
    // A socket file nobody listens on, as a crashed daemon leaves behind.
    drop(std::os::unix::net::UnixListener::bind(home.socket()).unwrap());

    let (stop, task) = start_daemon(&home).await;

    stop.send("test".into()).unwrap();
    within("daemon exit", task).await.unwrap().unwrap();
}

/// A git repo with one commit and no remote.
fn git_repo(dir: &std::path::Path) -> PathBuf {
    let repo = dir.join("repo");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=cos test",
                "-c",
                "user.email=test@cos.local",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main", "repo"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&["-C", "repo", "add", "README"]);
    git(&["-C", "repo", "commit", "-q", "-m", "first"]);
    repo.canonicalize().unwrap()
}

/// The status code and message of a failed client call.
fn daemon_error(error: anyhow::Error) -> (u16, String) {
    let error = error.downcast::<DaemonError>().expect("a DaemonError");
    (error.status.as_u16(), error.message)
}

#[tokio::test]
async fn projects_and_workspaces_over_the_socket() {
    let server = TestServer::start();
    let client = server.client();
    let repos = tempfile::tempdir().unwrap();
    let repo = git_repo(repos.path());

    let (project, created) = client.add_project(repo.clone()).await.unwrap();
    let (_, created_again) = client.add_project(repo.join(".")).await.unwrap();
    assert!(created && !created_again);
    assert_eq!(
        (project.root.as_path(), project.base_branch.as_str()),
        (repo.as_path(), "main")
    );
    assert!(!project.has_origin);

    let workspace = client
        .create_workspace(project.id, Base::Head, Some("First try".into()))
        .await
        .unwrap();
    assert_eq!(workspace.title, "First try");
    assert!(workspace.path.join("README").exists());

    // No origin remote: 400 with a message for the user.
    let (status, message) = daemon_error(
        client
            .create_workspace(project.id, Base::Origin, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 400);
    assert!(message.contains("no origin remote"), "{message}");

    let renamed = client
        .rename_workspace(workspace.id, "Login fix".into())
        .await
        .unwrap();
    assert_eq!(renamed.title, "Login fix");
    assert_eq!(client.projects().await.unwrap()[0].workspaces, vec![renamed]);

    // Unsaved work and a project that still has workspaces: 409.
    std::fs::write(workspace.path.join("draft.txt"), "unsaved").unwrap();
    let (status, _) = daemon_error(client.remove_workspace(workspace.id).await.unwrap_err());
    assert_eq!(status, 409);
    let (status, _) = daemon_error(client.remove_project(project.id).await.unwrap_err());
    assert_eq!(status, 409);

    std::fs::remove_file(workspace.path.join("draft.txt")).unwrap();
    client.remove_workspace(workspace.id).await.unwrap();
    client.remove_project(project.id).await.unwrap();
    let (status, _) = daemon_error(client.remove_project(project.id).await.unwrap_err());
    assert_eq!(status, 404);

    let events = client.events(0, 100).await.unwrap();
    assert_eq!(
        kinds(&events),
        [
            kind::PROJECT_ADDED,
            kind::WORKSPACE_CREATED,
            kind::WORKSPACE_RENAMED,
            kind::WORKSPACE_REMOVED,
            kind::PROJECT_REMOVED,
        ]
    );
}

/// Sends a raw HTTP request and returns the whole response.
async fn raw_request(socket: &std::path::Path, request: &str) -> String {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    within("raw response", stream.read_to_string(&mut response))
        .await
        .unwrap();
    response
}

#[tokio::test]
async fn bad_requests_get_json_errors() {
    let server = TestServer::start();

    let bad_json = raw_request(
        &server.socket,
        "POST /v1/projects HTTP/1.1\r\nHost: cos\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{",
    )
    .await;
    let bad_id = raw_request(
        &server.socket,
        "DELETE /v1/workspaces/abc HTTP/1.1\r\nHost: cos\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(bad_json.starts_with("HTTP/1.1 400"), "{bad_json}");
    assert!(bad_json.contains("{\"error\":"), "{bad_json}");
    assert!(bad_id.starts_with("HTTP/1.1 404"), "{bad_id}");
    assert!(bad_id.contains("there is no such workspace"), "{bad_id}");
}
