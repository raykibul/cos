//! Talks to the daemon's HTTP API over its Unix socket.

use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use cos_core::{Base, LoggedEvent, Project, Workspace};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::UnixStream;

use crate::daemon::api::{AddProject, EventsPage, NewWorkspace, ProjectsPage, RenameWorkspace, Status};

/// The daemon isn't running: there is no socket, or nothing is listening on it.
#[derive(Debug, thiserror::Error)]
#[error("the cos daemon is not running (no answer on {})", .socket.display())]
pub(crate) struct NotRunning {
    pub socket: PathBuf,
}

/// The daemon answered with an error. `message` is meant for the user.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct DaemonError {
    pub status: StatusCode,
    pub message: String,
}

pub(crate) struct Client {
    socket: PathBuf,
}

impl Client {
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self { socket }
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

    pub(crate) async fn rename_workspace(&self, id: i64, title: String) -> anyhow::Result<Workspace> {
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
        let stream = match UnixStream::connect(&self.socket).await {
            Ok(stream) => stream,
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                return Err(NotRunning {
                    socket: self.socket.clone(),
                }
                .into());
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not connect to {}", self.socket.display()));
            }
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, "cos");
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
}
