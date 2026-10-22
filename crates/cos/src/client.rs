//! Talks to the daemon's HTTP API over its Unix socket, or over TCP to a
//! daemon's tailnet listener when `--host` (or `COS_HOST`) is given.

pub(crate) mod agents;
mod approvals;
pub(crate) mod chief;
mod env;
mod judge;
mod staff;

pub(crate) use agents::TerminalMessage;

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Context;
use cos_core::{
    Base, Board, Device, LoggedEvent, NewRun, PlanProposal, Project, Run, RunSummary, TaskState, Workspace,
};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, UnixStream};

use crate::daemon::api::{AddProject, EventsPage, NewWorkspace, ProjectsPage, RenameWorkspace, Status};
use crate::daemon::peers::PeersPage;
use crate::daemon::remote_api::{ConfigureTailnet, DevicesPage};
use crate::daemon::runs_api::{CancelRun, RunsPage, UpdateTask};
use crate::daemon::tailnet::{DEFAULT_PORT, TailnetStatus};

/// Set once at startup from `--host`: every [`Client`] then talks TCP to that
/// daemon instead of the local socket.
static REMOTE_HOST: OnceLock<String> = OnceLock::new();

/// Sends every client in this process to the daemon at `host` (`100.x.y.z`,
/// `name:7700`, `[fd7a::1]:7700`). The port defaults to 7700.
pub(crate) fn use_remote_host(host: &str) {
    let host = if has_port(host) {
        host.to_owned()
    } else if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{DEFAULT_PORT}")
    } else {
        format!("{host}:{DEFAULT_PORT}")
    };
    let _ = REMOTE_HOST.set(host);
}

/// The remote daemon clients talk to, if `--host` was given.
pub(crate) fn remote_host() -> Option<&'static str> {
    REMOTE_HOST.get().map(String::as_str)
}

/// Whether `host` ends in `:port`, allowing for bracketed IPv6.
fn has_port(host: &str) -> bool {
    match host.rsplit_once(':') {
        Some((before, port)) => {
            port.parse::<u16>().is_ok()
                && (!before.contains(':') || (before.starts_with('[') && before.ends_with(']')))
        }
        None => false,
    }
}

/// The daemon isn't running: there is no socket, or nothing is listening on
/// it (or on the remote host).
#[derive(Debug, thiserror::Error)]
#[error("the cos daemon is not running (no answer on {})", .socket.display())]
pub(crate) struct NotRunning {
    /// The socket, or `host:port` for a remote daemon.
    pub socket: PathBuf,
}

/// Where a client sends its requests.
#[derive(Debug, Clone)]
enum Endpoint {
    Unix(PathBuf),
    /// `host:port` of a daemon's tailnet listener.
    Tcp(String),
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// The daemon answered with an error. `message` is meant for the user.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct DaemonError {
    pub status: StatusCode,
    pub message: String,
}

pub(crate) struct Client {
    endpoint: Endpoint,
}

impl Client {
    /// A client for the daemon on `socket`, or for the remote daemon if
    /// `--host` was given.
    pub(crate) fn new(socket: PathBuf) -> Self {
        match remote_host() {
            Some(host) => Self::tcp(host.to_owned()),
            None => Self {
                endpoint: Endpoint::Unix(socket),
            },
        }
    }

    /// A client for the daemon listening on `host:port` over TCP.
    pub(crate) fn tcp(host: String) -> Self {
        Self {
            endpoint: Endpoint::Tcp(host),
        }
    }

    /// Whether this client talks to a daemon on another machine.
    pub(crate) fn is_remote(&self) -> bool {
        matches!(self.endpoint, Endpoint::Tcp(_))
    }

    pub(crate) async fn devices(&self) -> anyhow::Result<Vec<Device>> {
        let page: DevicesPage = self.get_json("/v1/devices").await?;
        Ok(page.devices)
    }

    pub(crate) async fn approve_device(&self, id: i64) -> anyhow::Result<Device> {
        let path = format!("/v1/devices/{id}/approve");
        read_json(self.send::<()>(Method::POST, &path, None).await?).await
    }

    pub(crate) async fn deny_device(&self, id: i64) -> anyhow::Result<Device> {
        let path = format!("/v1/devices/{id}/deny");
        read_json(self.send::<()>(Method::POST, &path, None).await?).await
    }

    pub(crate) async fn remove_device(&self, id: i64) -> anyhow::Result<()> {
        self.send::<()>(Method::DELETE, &format!("/v1/devices/{id}"), None)
            .await?;
        Ok(())
    }

    pub(crate) async fn tailnet(&self) -> anyhow::Result<TailnetStatus> {
        self.get_json("/v1/tailnet").await
    }

    /// Switches remote access on or off. `port` defaults to the current one.
    pub(crate) async fn configure_tailnet(
        &self,
        enabled: bool,
        port: Option<u16>,
    ) -> anyhow::Result<TailnetStatus> {
        let body = ConfigureTailnet { enabled, port };
        read_json(self.send(Method::PUT, "/v1/tailnet", Some(&body)).await?).await
    }

    /// The other devices on the tailnet, with cos probed on `port` (default:
    /// the listener's port).
    pub(crate) async fn tailnet_peers(&self, port: Option<u16>) -> anyhow::Result<PeersPage> {
        match port {
            Some(port) => self.get_json(&format!("/v1/tailnet/peers?port={port}")).await,
            None => self.get_json("/v1/tailnet/peers").await,
        }
    }

    async fn connect(&self) -> anyhow::Result<Box<dyn Stream>> {
        let result = match &self.endpoint {
            Endpoint::Unix(socket) => UnixStream::connect(socket)
                .await
                .map(|stream| Box::new(stream) as Box<dyn Stream>),
            Endpoint::Tcp(host) => TcpStream::connect(host.as_str())
                .await
                .map(|stream| Box::new(stream) as Box<dyn Stream>),
        };
        let place = match &self.endpoint {
            Endpoint::Unix(socket) => socket.clone(),
            Endpoint::Tcp(host) => PathBuf::from(host),
        };
        match result {
            Ok(stream) => Ok(stream),
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                Err(NotRunning { socket: place }.into())
            }
            Err(error) => Err(error).with_context(|| format!("could not connect to {}", place.display())),
        }
    }

    /// The `Host` header: the remote host, or a placeholder on the socket.
    fn host_header(&self) -> &str {
        match &self.endpoint {
            Endpoint::Unix(_) => "cos",
            Endpoint::Tcp(host) => host,
        }
    }

    pub(crate) async fn status(&self) -> anyhow::Result<Status> {
        self.get_json("/v1/status").await
    }

    /// Up to `limit` events after sequence number `after`, oldest first.
    pub(crate) async fn events(&self, after: u64, limit: usize) -> anyhow::Result<Vec<LoggedEvent>> {
        let page: EventsPage = self
            .get_json(&format!("/v1/events?after={after}&limit={limit}"))
            .await?;
        Ok(page.events)
    }

    /// Every event after `after`, then new ones as they happen.
    pub(crate) async fn follow(&self, after: u64) -> anyhow::Result<EventStream> {
        let response = self.get(&format!("/v1/events/stream?after={after}")).await?;
        Ok(EventStream {
            body: response.into_body(),
            buffer: Vec::new(),
        })
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let body = self.get(path).await?.into_body().collect().await?.to_bytes();
        serde_json::from_slice(&body).with_context(|| format!("unexpected answer from the daemon for {path}"))
    }

    pub(crate) async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        let page: ProjectsPage = self.get_json("/v1/projects").await?;
        Ok(page.projects)
    }

    /// Registers the repo containing `path`. Returns the project and whether it is new.
    pub(crate) async fn add_project(&self, path: PathBuf) -> anyhow::Result<(Project, bool)> {
        let response = self
            .send(Method::POST, "/v1/projects", Some(&AddProject { path }))
            .await?;
        let created = response.status() == StatusCode::CREATED;
        Ok((read_json(response).await?, created))
    }

    pub(crate) async fn remove_project(&self, id: i64) -> anyhow::Result<()> {
        self.send::<()>(Method::DELETE, &format!("/v1/projects/{id}"), None)
            .await?;
        Ok(())
    }

    pub(crate) async fn create_workspace(
        &self,
        project_id: i64,
        base: Base,
        title: Option<String>,
    ) -> anyhow::Result<Workspace> {
        let body = NewWorkspace { base, title };
        let path = format!("/v1/projects/{project_id}/workspaces");
        read_json(self.send(Method::POST, &path, Some(&body)).await?).await
    }

    pub(crate) async fn rename_workspace(&self, id: i64, title: Option<String>) -> anyhow::Result<Workspace> {
        let path = format!("/v1/workspaces/{id}");
        read_json(
            self.send(Method::PATCH, &path, Some(&RenameWorkspace { title }))
                .await?,
        )
        .await
    }

    pub(crate) async fn remove_workspace(&self, id: i64) -> anyhow::Result<()> {
        self.send::<()>(Method::DELETE, &format!("/v1/workspaces/{id}"), None)
            .await?;
        Ok(())
    }

    /// Every run, newest first.
    pub(crate) async fn runs(&self) -> anyhow::Result<Vec<RunSummary>> {
        let page: RunsPage = self.get_json("/v1/runs").await?;
        Ok(page.runs)
    }

    pub(crate) async fn create_run(&self, new: &NewRun) -> anyhow::Result<Run> {
        read_json(self.send(Method::POST, "/v1/runs", Some(new)).await?).await
    }

    pub(crate) async fn board(&self, run_id: i64) -> anyhow::Result<Board> {
        self.get_json(&format!("/v1/runs/{run_id}")).await
    }

    /// What the chief's `board_propose` tool will call.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the chief's bridge arrives in milestone 5")
    )]
    pub(crate) async fn propose_plan(&self, run_id: i64, plan: &PlanProposal) -> anyhow::Result<Board> {
        let path = format!("/v1/runs/{run_id}/plan");
        read_json(self.send(Method::POST, &path, Some(plan)).await?).await
    }

    pub(crate) async fn approve_plan(&self, run_id: i64) -> anyhow::Result<Board> {
        let path = format!("/v1/runs/{run_id}/approve-plan");
        read_json(self.send::<()>(Method::POST, &path, None).await?).await
    }

    pub(crate) async fn cancel_run(&self, run_id: i64, reason: Option<String>) -> anyhow::Result<Board> {
        let path = format!("/v1/runs/{run_id}/cancel");
        read_json(
            self.send(Method::POST, &path, Some(&CancelRun { reason }))
                .await?,
        )
        .await
    }

    /// Moves a task; the orchestration does this inside the daemon.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "workers report through the daemon, not the CLI")
    )]
    pub(crate) async fn update_task(
        &self,
        task_id: i64,
        state: TaskState,
        reason: Option<String>,
    ) -> anyhow::Result<Board> {
        let path = format!("/v1/tasks/{task_id}");
        read_json(
            self.send(Method::PATCH, &path, Some(&UpdateTask { state, reason }))
                .await?,
        )
        .await
    }

    async fn get(&self, path: &str) -> anyhow::Result<Response<Incoming>> {
        self.send::<()>(Method::GET, path, None).await
    }

    /// Sends a request, with `body` as JSON if given. A non-2xx answer becomes
    /// a [`DaemonError`].
    async fn send<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> anyhow::Result<Response<Incoming>> {
        let stream = self.connect().await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, self.host_header());
        let body = match body {
            Some(body) => {
                request = request.header(hyper::header::CONTENT_TYPE, "application/json");
                Bytes::from(serde_json::to_vec(body)?)
            }
            None => Bytes::new(),
        };
        let response = sender.send_request(request.body(Full::new(body))?).await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.into_body().collect().await?.to_bytes();
            let message = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| {
                    format!(
                        "daemon answered {status} for {path}: {}",
                        String::from_utf8_lossy(&body)
                    )
                });
            return Err(DaemonError { status, message }.into());
        }
        Ok(response)
    }
}

async fn read_json<T: DeserializeOwned>(response: Response<Incoming>) -> anyhow::Result<T> {
    let body = response.into_body().collect().await?.to_bytes();
    serde_json::from_slice(&body).context("unexpected answer from the daemon")
}

/// A live stream of events from `GET /v1/events/stream`.
pub(crate) struct EventStream {
    body: Incoming,
    buffer: Vec<u8>,
}

impl EventStream {
    /// The next event, or `None` when the daemon closes the stream.
    pub(crate) async fn next(&mut self) -> anyhow::Result<Option<LoggedEvent>> {
        loop {
            while let Some(end) = find(&self.buffer, b"\n\n") {
                let message: Vec<u8> = self.buffer.drain(..end + 2).collect();
                if let Some(data) = sse_data(&message) {
                    return Ok(Some(serde_json::from_str(&data)?));
                }
            }
            match self.body.frame().await {
                None => return Ok(None),
                Some(frame) => {
                    if let Ok(data) = frame?.into_data() {
                        self.buffer.extend_from_slice(&data);
                    }
                }
            }
        }
    }
}

/// The `data:` payload of one server-sent event, or `None` for a message
/// without data, such as a keep-alive comment.
fn sse_data(message: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(message);
    let lines: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.strip_prefix(' ').unwrap_or(data))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_data_joins_data_lines_and_skips_comments() {
        assert_eq!(
            sse_data(b"id: 3\ndata: {\"a\":1}\n\n").as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(sse_data(b"data:x\ndata: y\n\n").as_deref(), Some("x\ny"));
        assert_eq!(sse_data(b":\n\n"), None);
    }

    #[test]
    fn hosts_with_and_without_ports() {
        assert!(has_port("100.100.1.1:7700"));
        assert!(has_port("my-mac:7701"));
        assert!(has_port("[fd7a:115c:a1e0::1]:7700"));
        assert!(!has_port("100.100.1.1"));
        assert!(!has_port("my-mac"));
        assert!(!has_port("fd7a:115c:a1e0::1"));
        assert!(!has_port("[fd7a:115c:a1e0::1]"));
    }
}
