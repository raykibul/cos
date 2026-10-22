//! The daemon's HTTP API. Served on the Unix socket today; the same routes go
//! on the tailnet listener later, so the paths are versioned.
//!
//! - `GET /v1/status`: [`Status`]
//! - `GET /v1/events?after=N&limit=M`: [`EventsPage`], oldest first
//! - `GET /v1/events/stream?after=N`: server-sent events, one per log entry,
//!   with the sequence number as the SSE id. A `Last-Event-ID` header
//!   overrides `after`, so a reconnecting client resumes where it stopped.
//! - Projects and workspaces: see "Projects and workspaces" in `docs/daemon.md`.
//! - Agents and approvals: [`super::agents_api`],
//!   contract in `docs/agents-api.md`.
//! - Devices and remote access: [`super::remote_api`], "Remote access" in
//!   `docs/daemon.md`. The same routes are served on the tailnet listener.
//! - Runs, plans and tasks: see [`super::runs_api`].
//! - Chief answers to approvals and per-run limits: [`super::approvals_api`],
//!   "Approvals policy" in `docs/daemon.md`.
//! - The Chief of Staff: [`super::staff_api`], contract in
//!   `docs/chief-of-staff.md`.
//! - Environment variables for agents: [`super::env_api`], "Environment" in
//!   `docs/daemon.md`.
//! - jev in shadow mode: [`super::judge_api`], `docs/judge.md`.
//!
//! Errors are `{"error": "..."}` with a message meant for the user.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch};
use axum::{Json, Router};
use cos_core::{
    Base, EventLog, LoggedEvent, Project, ProjectError, Projects, RunError, Runs, StoreError, Workspace,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;

const DEFAULT_PAGE: usize = 100;
const MAX_PAGE: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Status {
    pub version: String,
    pub pid: u32,
    pub started_at: Timestamp,
    pub home: PathBuf,
    /// Number of stored events.
    pub events: u64,
    /// Sequence number of the newest event, 0 if there are none.
    pub last_seq: u64,
    /// The tailnet listener. Missing from daemons older than remote access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailnet: Option<super::tailnet::TailnetStatus>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EventsPage {
    pub events: Vec<LoggedEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProjectsPage {
    pub projects: Vec<Project>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AddProject {
    pub path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct NewWorkspace {
    #[serde(default)]
    pub base: Base,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Body of `PATCH /v1/workspaces/{id}`. A null or empty title clears it.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RenameWorkspace {
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub log: EventLog,
    pub projects: Arc<Projects>,
    pub devices: Arc<cos_core::Devices>,
    pub tailnet: super::tailnet::Tailnet,
    pub peers: super::peers::Peers,
    pub runs: Arc<Runs>,
    pub agents: super::agents::AgentManager,
    pub chief: super::chief::Chief,
    pub staff: super::staff::Staff,
    /// jev, in shadow mode.
    pub judge: super::judge::Judge,
    pub started_at: Timestamp,
    pub home: PathBuf,
    /// Flips to true when the daemon is stopping; open streams end then.
    pub stopping: watch::Receiver<bool>,
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/events", get(list_events))
        .route("/v1/events/stream", get(stream_events))
        .route("/v1/projects", get(list_projects).post(add_project))
        .route("/v1/projects/{id}", delete(remove_project))
        .route(
            "/v1/projects/{id}/workspaces",
            axum::routing::post(create_workspace),
        )
        .route(
            "/v1/workspaces/{id}",
            patch(rename_workspace).delete(remove_workspace),
        )
        .merge(super::remote_api::routes())
        .merge(super::runs_api::routes())
        .merge(super::agents_api::routes())
        .merge(super::approvals_api::routes())
        .merge(super::chief_api::routes())
        .merge(super::staff_api::routes())
        .merge(super::env_api::routes())
        .merge(super::judge_api::routes())
        .with_state(state)
}

async fn status(State(state): State<AppState>) -> Result<Json<Status>, ApiError> {
    let (events, last_seq) = state.log.stats()?;
    Ok(Json(Status {
        version: env!("CARGO_PKG_VERSION").into(),
        pid: std::process::id(),
        started_at: state.started_at,
        home: state.home.clone(),
        events,
        last_seq,
        tailnet: Some(state.tailnet.status()),
    }))
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: u64,
    limit: Option<usize>,
}

async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsPage>, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).min(MAX_PAGE);
    let events = state.log.after(query.after, limit)?;
    Ok(Json(EventsPage { events }))
}

async fn stream_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok()?.parse().ok())
        .unwrap_or(query.after);
    let mut subscription = state.log.subscribe(after);
    let mut stopping = state.stopping.clone();
    let (sender, receiver) = mpsc::channel::<Result<SseEvent, Infallible>>(64);

    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                next = subscription.next() => next,
                _ = stopping.changed() => break,
                // The client went away while no events were coming.
                _ = sender.closed() => break,
            };
            let event = match next {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(%error, "event stream stopped");
                    break;
                }
            };
            let sse = SseEvent::default()
                .id(event.seq.to_string())
                .json_data(&event)
                .expect("events serialize to JSON");
            if sender.send(Ok(sse)).await.is_err() {
                break;
            }
        }
    });

    Sse::new(ReceiverStream::new(receiver)).keep_alive(KeepAlive::default())
}

async fn list_projects(State(state): State<AppState>) -> Result<Json<ProjectsPage>, ApiError> {
    let projects = state.projects.list().await?;
    Ok(Json(ProjectsPage { projects }))
}

async fn add_project(
    State(state): State<AppState>,
    body: Result<Json<AddProject>, JsonRejection>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let Json(body) = body?;
    let (project, created) = state.projects.add(&body.path).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(project)))
}

async fn remove_project(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such project"))?;
    // Its Chief of Staff and worktree go with it.
    state.staff.remove_project(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_workspace(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<NewWorkspace>, JsonRejection>,
) -> Result<(StatusCode, Json<Workspace>), ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such project"))?;
    let Json(body) = body?;
    let workspace = state
        .projects
        .create_workspace(id, body.base, body.title.as_deref())
        .await?;
    Ok((StatusCode::CREATED, Json(workspace)))
}

async fn rename_workspace(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<RenameWorkspace>, JsonRejection>,
) -> Result<Json<Workspace>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such workspace"))?;
    let Json(body) = body?;
    Ok(Json(
        state.projects.rename_workspace(id, body.title.as_deref()).await?,
    ))
}

async fn remove_workspace(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such workspace"))?;
    // The Chief of Staff's worktree goes only with its project.
    state.projects.listed_workspace(id)?;
    // Refused while an agent runs there.
    state.agents.remove_workspace(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An error response: `{"error": message}` with a status code.
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn not_found(message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.to_owned(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<ProjectError> for ApiError {
    fn from(error: ProjectError) -> Self {
        let status = match &error {
            ProjectError::Invalid(_) => StatusCode::BAD_REQUEST,
            ProjectError::NotFound(_) => StatusCode::NOT_FOUND,
            ProjectError::Conflict(_) => StatusCode::CONFLICT,
            ProjectError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

impl From<RunError> for ApiError {
    fn from(error: RunError) -> Self {
        let status = match &error {
            RunError::Invalid(_) => StatusCode::BAD_REQUEST,
            RunError::NotFound(_) => StatusCode::NOT_FOUND,
            RunError::Conflict(_) => StatusCode::CONFLICT,
            RunError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: rejection.body_text(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(error = %self.message, "request failed");
        }
        let body = Json(serde_json::json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}
