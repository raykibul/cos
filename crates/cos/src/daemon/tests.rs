use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    Agents, Base, DeviceState, Devices, EventLog, LoggedEvent, NewRun, PlanProposal, Projects, ProposedTask,
    RunState, Runs, SeenDevice, Store, TaskState,
};
use jiff::Timestamp;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use super::agents::{AgentConfig, AgentManager};
use super::chief::{Chief, ChiefConfig};
use super::tailnet::{ListenerState, Tailnet, Timing};
use super::{api, run_until, run_with, serve};
use crate::client::{Client, DaemonError, NotRunning};
use crate::home::CosHome;
use crate::tailscale::{BoxFuture, LocalNode, Peer, Tailscale, TailscaleError};

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"))
}

/// The API served from an in-memory log, so tests can append events.
struct TestServer {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    store: Arc<Store>,
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
        let projects = Arc::new(Projects::new(
            store.clone(),
            log.clone(),
            dir.path().join("worktrees"),
        ));
        let devices = Arc::new(Devices::new(store.clone(), log.clone()));
        let tailnet = Tailnet::load(store.clone(), log.clone());
        let runs = Arc::new(Runs::new(store.clone(), log.clone()));
        let agents = AgentManager::start(
            Agents::new(store.clone(), log.clone()),
            projects.clone(),
            runs.clone(),
            AgentConfig::for_tests(dir.path()),
        )
        .unwrap();
        let chief = Chief::start(
            store.clone(),
            log.clone(),
            runs.clone(),
            agents.clone(),
            dir.path().to_path_buf(),
            ChiefConfig::for_tests(),
        )
        .unwrap();
        let (stop, stopping) = watch::channel(false);
        let state = api::AppState {
            log: log.clone(),
            projects,
            devices,
            tailnet,
            runs,
            agents,
            chief,
            started_at: Timestamp::now(),
            home: dir.path().to_path_buf(),
            stopping: stopping.clone(),
        };
        let server = serve(UnixListener::bind(&socket).unwrap(), state, stopping);
        Self {
            _dir: dir,
            socket,
            store,
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
    assert_eq!(workspace.title.as_deref(), Some("First try"));
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
        .rename_workspace(workspace.id, Some("Login fix".into()))
        .await
        .unwrap();
    assert_eq!(renamed.title.as_deref(), Some("Login fix"));
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

// ---- Remote access ----

#[tokio::test]
async fn devices_are_managed_over_the_socket() {
    let server = TestServer::start();
    let client = server.client();
    assert!(client.devices().await.unwrap().is_empty());
    let devices = Devices::new(server.store.clone(), server.log.clone());
    let id = devices.seen(&seen_phone()).unwrap().id;

    assert_eq!(client.devices().await.unwrap()[0].state, DeviceState::Pending);
    assert_eq!(
        client.approve_device(id).await.unwrap().state,
        DeviceState::Approved
    );
    assert_eq!(client.deny_device(id).await.unwrap().state, DeviceState::Denied);
    client.remove_device(id).await.unwrap();
    assert!(client.devices().await.unwrap().is_empty());

    let (status, message) = daemon_error(client.approve_device(id).await.unwrap_err());
    assert_eq!(status, 404);
    assert!(message.contains("no device"), "{message}");
    let events = client.events(0, 100).await.unwrap();
    assert_eq!(
        kinds(&events),
        [
            kind::DEVICE_REQUESTED,
            kind::DEVICE_APPROVED,
            kind::DEVICE_DENIED,
            kind::DEVICE_REMOVED
        ]
    );
}

#[tokio::test]
async fn remote_access_is_off_by_default_and_rejects_port_zero() {
    let server = TestServer::start();
    let client = server.client();

    let status = client.status().await.unwrap().tailnet.unwrap();
    assert_eq!((status.enabled, status.state), (false, ListenerState::Off));

    let (code, message) = daemon_error(client.configure_tailnet(true, Some(0)).await.unwrap_err());
    assert_eq!(code, 400);
    assert!(message.contains("port"), "{message}");
}

fn seen_phone() -> SeenDevice {
    SeenDevice {
        node_id: "nPHONE".into(),
        name: "phone".into(),
        hostname: "My Phone".into(),
        os: "iOS".into(),
        login_name: "me@example.com".into(),
        ip: "100.100.2.1".into(),
    }
}

/// Tailscale as the tests want it: this machine on 127.0.0.1, and whatever
/// peer the test puts behind every address.
struct FakeTailscale {
    local: Mutex<Result<LocalNode, TailscaleError>>,
    peer: Mutex<Option<Peer>>,
    whois_calls: AtomicUsize,
}

impl FakeTailscale {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            local: Mutex::new(Ok(fake_local())),
            peer: Mutex::new(Some(fake_peer())),
            whois_calls: AtomicUsize::new(0),
        })
    }

    fn set_local(&self, local: Result<LocalNode, TailscaleError>) {
        *self.local.lock().unwrap() = local;
    }

    fn set_peer(&self, peer: Option<Peer>) {
        *self.peer.lock().unwrap() = peer;
    }
}

impl Tailscale for FakeTailscale {
    fn local(&self) -> BoxFuture<'_, Result<LocalNode, TailscaleError>> {
        let local = self.local.lock().unwrap().clone();
        Box::pin(async move { local })
    }

    fn whois(&self, _addr: SocketAddr) -> BoxFuture<'_, Result<Option<Peer>, TailscaleError>> {
        self.whois_calls.fetch_add(1, Ordering::SeqCst);
        let peer = self.peer.lock().unwrap().clone();
        Box::pin(async move { Ok(peer) })
    }
}

fn fake_local() -> LocalNode {
    LocalNode {
        node_id: "nHOST".into(),
        name: "my-mac".into(),
        os: "macOS".into(),
        user_id: 1,
        login_name: "me@example.com".into(),
        tagged: false,
        ips: vec!["127.0.0.1".parse().unwrap()],
    }
}

fn fake_peer() -> Peer {
    Peer {
        node_id: "nPHONE".into(),
        name: "phone".into(),
        hostname: "My Phone".into(),
        os: "iOS".into(),
        user_id: 1,
        login_name: "me@example.com".into(),
        tagged: false,
    }
}

/// Fast retries and rechecks. A zero `whois_ttl` asks the fake every time.
fn fast(whois_ttl: Duration) -> Timing {
    Timing {
        retry: Duration::from_millis(50),
        recheck: Duration::from_millis(100),
        whois_ttl,
    }
}

/// A free TCP port on 127.0.0.1.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The real daemon, with a fake Tailscale.
struct RemoteDaemon {
    dir: tempfile::TempDir,
    home: CosHome,
    tailscale: Arc<FakeTailscale>,
    timing: Timing,
    stop: oneshot::Sender<String>,
    task: JoinHandle<anyhow::Result<()>>,
}

impl RemoteDaemon {
    async fn start(tailscale: Arc<FakeTailscale>, timing: Timing) -> Self {
        Self::start_in(tempfile::tempdir().unwrap(), tailscale, timing).await
    }

    async fn start_in(dir: tempfile::TempDir, tailscale: Arc<FakeTailscale>, timing: Timing) -> Self {
        let home = CosHome::new(dir.path().join("home"));
        let (stop, stopped) = oneshot::channel::<String>();
        let task = tokio::spawn({
            let home = home.clone();
            let tailscale: Arc<dyn Tailscale> = tailscale.clone();
            async move {
                run_with(&home, tailscale, timing, async {
                    stopped.await.unwrap_or_default()
                })
                .await
            }
        });
        let client = Client::new(home.socket());
        within("daemon start", async {
            while client.status().await.is_err() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        Self {
            dir,
            home,
            tailscale,
            timing,
            stop,
            task,
        }
    }

    fn local(&self) -> Client {
        Client::new(self.home.socket())
    }

    /// Turns remote access on and returns the address it listens on.
    async fn listen(&self) -> SocketAddr {
        let status = self
            .local()
            .configure_tailnet(true, Some(free_port()))
            .await
            .unwrap();
        assert_eq!(status.state, ListenerState::Listening, "{status:?}");
        status.addresses[0]
    }

    async fn wait_for_state(&self, state: ListenerState) -> api::Status {
        within("tailnet state", async {
            loop {
                let status = self.local().status().await.unwrap();
                if status.tailnet.as_ref().unwrap().state == state {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
    }

    /// Stops the daemon and starts it again on the same home.
    async fn restart(self) -> Self {
        let Self {
            dir,
            tailscale,
            timing,
            stop,
            task,
            ..
        } = self;
        stop.send("test".into()).unwrap();
        within("daemon exit", task).await.unwrap().unwrap();
        Self::start_in(dir, tailscale, timing).await
    }

    async fn stop(self) {
        self.stop.send("test".into()).unwrap();
        within("daemon exit", self.task).await.unwrap().unwrap();
    }
}

/// Sends a raw HTTP request over TCP. Returns the status code and the JSON body.
async fn tcp_request(addr: SocketAddr, method: &str, path: &str) -> (u16, serde_json::Value) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request =
        format!("{method} {path} HTTP/1.1\r\nHost: cos\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    within("tcp response", stream.read_to_string(&mut response))
        .await
        .unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(json!(null)))
}

#[tokio::test]
async fn a_new_device_waits_for_approval_on_the_host() {
    let daemon = RemoteDaemon::start(FakeTailscale::new(), fast(Duration::ZERO)).await;
    let addr = daemon.listen().await;
    assert!(addr.ip().is_loopback());

    // First contact: recorded as pending, and refused.
    let (status, body) = tcp_request(addr, "GET", "/v1/status").await;
    assert_eq!(status, 403);
    assert_eq!(body["code"], "device_pending");
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("On my-mac, run `cos devices approve 1`"),
        "{message}"
    );
    // Unknown paths are guarded too: no probing for 404s.
    let (status, body) = tcp_request(addr, "GET", "/v1/no-such-route").await;
    assert_eq!((status, body["code"].as_str()), (403, Some("device_pending")));
    let devices = daemon.local().devices().await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        (devices[0].node_id.as_str(), devices[0].state),
        ("nPHONE", DeviceState::Pending)
    );
    assert_eq!(devices[0].last_ip.as_deref(), Some("127.0.0.1"));

    // Approved on the host: in, through the same client code the CLI uses.
    daemon.local().approve_device(devices[0].id).await.unwrap();
    let remote = Client::tcp(addr.to_string());
    let status = remote.status().await.unwrap();
    assert_eq!(status.tailnet.unwrap().addresses, vec![addr]);
    assert!(remote.projects().await.unwrap().is_empty());
    let (status, _) = tcp_request(addr, "GET", "/v1/no-such-route").await;
    assert_eq!(status, 404);

    // Device and listener management stay on the host, even for approved devices.
    for (method, path) in [
        ("GET", "/v1/devices"),
        ("POST", "/v1/devices/1/approve"),
        ("DELETE", "/v1/devices/1"),
        ("GET", "/v1/tailnet"),
        ("PUT", "/v1/tailnet"),
    ] {
        let (status, body) = tcp_request(addr, method, path).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (403, Some("host_only")),
            "{method} {path}"
        );
    }

    // Denied: out again at once.
    daemon.local().deny_device(devices[0].id).await.unwrap();
    let (status, body) = tcp_request(addr, "GET", "/v1/status").await;
    assert_eq!((status, body["code"].as_str()), (403, Some("device_denied")));

    // Removed: it starts over as a new pending device.
    daemon.local().remove_device(devices[0].id).await.unwrap();
    let (_, body) = tcp_request(addr, "GET", "/v1/status").await;
    assert_eq!(body["code"], "device_pending");
    assert_eq!(
        daemon.local().devices().await.unwrap()[0].state,
        DeviceState::Pending
    );

    let events = daemon.local().events(0, 100).await.unwrap();
    let device_events: Vec<&str> = kinds(&events)
        .into_iter()
        .filter(|kind| kind.starts_with("device."))
        .collect();
    assert_eq!(
        device_events,
        [
            kind::DEVICE_REQUESTED,
            kind::DEVICE_APPROVED,
            kind::DEVICE_DENIED,
            kind::DEVICE_REMOVED,
            kind::DEVICE_REQUESTED
        ]
    );
    assert!(kinds(&events).contains(&kind::TAILNET_CHANGED));
    daemon.stop().await;
}

#[tokio::test]
async fn other_users_unknown_peers_and_tagged_devices_never_become_devices() {
    let daemon = RemoteDaemon::start(FakeTailscale::new(), fast(Duration::ZERO)).await;
    let addr = daemon.listen().await;

    daemon.tailscale.set_peer(Some(Peer {
        user_id: 2,
        login_name: "someone@example.com".into(),
        ..fake_peer()
    }));
    let (status, body) = tcp_request(addr, "GET", "/v1/status").await;
    assert_eq!((status, body["code"].as_str()), (403, Some("wrong_user")));

    daemon.tailscale.set_peer(Some(Peer {
        tagged: true,
        ..fake_peer()
    }));
    let (status, body) = tcp_request(addr, "GET", "/v1/projects").await;
    assert_eq!((status, body["code"].as_str()), (403, Some("tagged_device")));

    daemon.tailscale.set_peer(None);
    let (status, body) = tcp_request(addr, "GET", "/v1/status").await;
    assert_eq!((status, body["code"].as_str()), (403, Some("unknown_peer")));

    assert!(daemon.local().devices().await.unwrap().is_empty());
    daemon.stop().await;
}

#[tokio::test]
async fn whois_answers_are_cached_per_address() {
    let daemon = RemoteDaemon::start(FakeTailscale::new(), fast(Duration::from_secs(60))).await;
    let addr = daemon.listen().await;

    for _ in 0..3 {
        tcp_request(addr, "GET", "/v1/status").await;
    }
    // Approval is read fresh, so it works without waiting for the cache.
    daemon.local().approve_device(1).await.unwrap();
    let (status, _) = tcp_request(addr, "GET", "/v1/status").await;

    assert_eq!(status, 200);
    assert_eq!(daemon.tailscale.whois_calls.load(Ordering::SeqCst), 1);
    daemon.stop().await;
}

#[tokio::test]
async fn without_tailscale_the_daemon_keeps_its_socket_and_retries() {
    let tailscale = FakeTailscale::new();
    tailscale.set_local(Err(TailscaleError::NeedsLogin));
    let daemon = RemoteDaemon::start(tailscale, fast(Duration::ZERO)).await;

    let status = daemon
        .local()
        .configure_tailnet(true, Some(free_port()))
        .await
        .unwrap();
    assert_eq!(status.state, ListenerState::Unavailable);
    assert_eq!(status.reason.as_deref(), Some("Tailscale is not logged in"));
    assert!(status.addresses.is_empty());

    // Tailscale comes up: the next retry binds.
    daemon.tailscale.set_local(Ok(fake_local()));
    let status = daemon.wait_for_state(ListenerState::Listening).await;
    assert_eq!(status.tailnet.unwrap().owner.as_deref(), Some("me@example.com"));

    // Tailscale goes away again: the listener closes.
    daemon
        .tailscale
        .set_local(Err(TailscaleError::NotRunning("it is switched off".into())));
    let status = daemon.wait_for_state(ListenerState::Unavailable).await;
    let reason = status.tailnet.unwrap().reason.unwrap();
    assert!(reason.contains("switched off"), "{reason}");
    daemon.stop().await;
}

#[tokio::test]
async fn turning_remote_access_off_closes_the_port_and_on_survives_restarts() {
    let daemon = RemoteDaemon::start(FakeTailscale::new(), fast(Duration::ZERO)).await;
    let addr = daemon.listen().await;

    let status = daemon.local().configure_tailnet(false, None).await.unwrap();
    assert_eq!((status.enabled, status.state), (false, ListenerState::Off));
    assert_eq!(status.port, addr.port());
    within("listener closed", async {
        while tokio::net::TcpStream::connect(addr).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;

    // On again (same port), then a restart: it comes back on by itself.
    let status = daemon.local().configure_tailnet(true, None).await.unwrap();
    assert_eq!(status.addresses, vec![addr]);
    let daemon = daemon.restart().await;

    let status = daemon.wait_for_state(ListenerState::Listening).await;
    assert_eq!(status.tailnet.unwrap().addresses, vec![addr]);
    daemon.stop().await;
}

fn proposed(title: &str, files: &[&str], depends_on: &[&str]) -> ProposedTask {
    ProposedTask {
        title: title.into(),
        instructions: format!("do {title}"),
        files: files.iter().map(|f| (*f).to_owned()).collect(),
        test_command: None,
        depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
    }
}

#[tokio::test]
async fn runs_plans_and_tasks_over_the_socket() {
    let server = TestServer::start();
    let client = server.client();
    let repos = tempfile::tempdir().unwrap();
    let (project, _) = client.add_project(git_repo(repos.path())).await.unwrap();
    let workspace = client
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();
    let new_run = NewRun {
        workspace_id: workspace.id,
        goal: "add rate limiting".into(),
        preset: Default::default(),
        approval_cap: None,
    };

    let run = client.create_run(&new_run).await.unwrap();
    assert_eq!((run.state, run.approval_cap), (RunState::Planning, 20));
    let (status, message) = daemon_error(client.create_run(&new_run).await.unwrap_err());
    assert_eq!(status, 409);
    assert!(message.contains("already has run"), "{message}");

    // A plan with a cycle: 400, with the reason for the chief.
    let cyclic = PlanProposal {
        tasks: vec![proposed("a", &["a"], &["b"]), proposed("b", &["b"], &["a"])],
    };
    let (status, message) = daemon_error(client.propose_plan(run.id, &cyclic).await.unwrap_err());
    assert_eq!(status, 400);
    assert!(message.contains("circle"), "{message}");

    let plan = PlanProposal {
        tasks: vec![
            proposed("api", &["src/api.rs"], &[]),
            proposed("ui", &["src/ui/**"], &["api"]),
        ],
    };
    let board = client.propose_plan(run.id, &plan).await.unwrap();
    assert_eq!(board.run.state, RunState::AwaitingPlanApproval);
    assert_eq!(board.workspace.as_ref().map(|w| w.id), Some(workspace.id));
    let (api, ui) = (board.tasks[0].id, board.tasks[1].id);

    let board = client.approve_plan(run.id).await.unwrap();
    assert_eq!((board.counts.ready, board.counts.proposed), (1, 1));
    let (status, _) = daemon_error(client.approve_plan(run.id).await.unwrap_err());
    assert_eq!(status, 409);

    // Illegal move, missing reason, gated dependency, unknown task.
    let (status, _) = daemon_error(
        client
            .update_task(api, TaskState::Merged, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 409);
    let (status, _) = daemon_error(
        client
            .update_task(api, TaskState::Blocked, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 400);
    let (status, _) = daemon_error(client.update_task(ui, TaskState::Ready, None).await.unwrap_err());
    assert_eq!(status, 409);
    let (status, _) = daemon_error(client.update_task(999, TaskState::Ready, None).await.unwrap_err());
    assert_eq!(status, 404);

    for state in [
        TaskState::Running,
        TaskState::Checking,
        TaskState::Review,
        TaskState::Merged,
    ] {
        client.update_task(api, state, None).await.unwrap();
    }
    let board = client.board(run.id).await.unwrap();
    assert_eq!(board.tasks[1].state, TaskState::Ready);
    let runs = client.runs().await.unwrap();
    assert_eq!((runs[0].run.id, runs[0].tasks.merged), (run.id, 1));

    // Cancel without a body, as `curl -X POST` sends it.
    let response = raw_request(
        &server.socket,
        &format!(
            "POST /v1/runs/{}/cancel HTTP/1.1\r\nHost: cos\r\nConnection: close\r\n\r\n",
            run.id
        ),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let board = client.board(run.id).await.unwrap();
    assert_eq!(board.run.state, RunState::Cancelled);
    assert_eq!(board.counts.cancelled, 1);
    let (status, _) = daemon_error(client.cancel_run(run.id, Some("again".into())).await.unwrap_err());
    assert_eq!(status, 409);
    let (status, _) = daemon_error(client.board(run.id + 1).await.unwrap_err());
    assert_eq!(status, 404);

    let run_events: Vec<LoggedEvent> = client
        .events(0, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.run_id.is_some())
        .collect();
    assert!(
        run_events
            .iter()
            .all(|event| event.run_id == Some(run.id.to_string()))
    );
    assert_eq!(
        kinds(&run_events[..4]),
        [
            kind::RUN_CREATED,
            kind::PLAN_PROPOSED,
            kind::RUN_STATE_CHANGED,
            kind::PLAN_APPROVED
        ]
    );
    assert_eq!(kinds(&run_events).last(), Some(&kind::RUN_STATE_CHANGED));
}

#[tokio::test]
async fn bad_run_and_task_ids_get_404() {
    let server = TestServer::start();

    let bad_run = raw_request(
        &server.socket,
        "GET /v1/runs/abc HTTP/1.1\r\nHost: cos\r\nConnection: close\r\n\r\n",
    )
    .await;
    let bad_task = raw_request(
        &server.socket,
        "PATCH /v1/tasks/abc HTTP/1.1\r\nHost: cos\r\nContent-Type: application/json\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{\"state\":\"running\"}",
    )
    .await;

    assert!(bad_run.starts_with("HTTP/1.1 404"), "{bad_run}");
    assert!(bad_run.contains("there is no such run"), "{bad_run}");
    assert!(bad_task.starts_with("HTTP/1.1 404"), "{bad_task}");
    assert!(bad_task.contains("there is no such task"), "{bad_task}");
}
