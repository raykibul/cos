//! The agents API, merged into [`api::router`]. The contract is
//! `docs/agents-api.md`:
//!
//! - `GET /v1/agents?workspace_id=&state=`, `GET /v1/agents/{id}`
//! - `POST /v1/workspaces/{id}/agents` with [`StartAgent`]
//! - `POST /v1/agents/{id}/input|keys|resize|trust|stop|resume`
//! - `DELETE /v1/agents/{id}`
//! - `GET /v1/agents/{id}/screen` and `GET /v1/agents/{id}/terminal` (SSE)
//! - `GET /v1/approvals?state=`, `GET /v1/approvals/{id}`, `POST /v1/approvals/{id}`

use std::convert::Infallible;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use cos_core::{Agent, AgentState, Approval, ApprovalState, Role};
use cos_harness::TerminalUpdate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::agents::{AgentError, Screen, StartAgent, TerminalView};
use super::api::{ApiError, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AgentsPage {
    pub agents: Vec<Agent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApprovalsPage {
    pub approvals: Vec<Approval>,
}

/// Body of `POST /v1/agents/{id}/input`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Input {
    pub text: String,
}

/// Body of `POST /v1/agents/{id}/keys`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Keys {
    /// Base64 of the raw bytes.
    pub data: String,
}

/// Body of `POST /v1/agents/{id}/resize`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Resize {
    pub rows: u16,
    pub cols: u16,
}

/// Body of `POST /v1/approvals/{id}`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Answer {
    pub decision: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Verdict {
    Allow,
    Deny,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/{id}", get(get_agent).delete(remove_agent))
        .route("/v1/workspaces/{id}/agents", post(start_agent))
        .route("/v1/agents/{id}/input", post(input))
        .route("/v1/agents/{id}/keys", post(keys))
        .route("/v1/agents/{id}/resize", post(resize))
        .route("/v1/agents/{id}/trust", post(trust))
        .route("/v1/agents/{id}/stop", post(stop))
        .route("/v1/agents/{id}/resume", post(resume))
        .route("/v1/agents/{id}/screen", get(screen))
        .route("/v1/agents/{id}/terminal", get(terminal))
        .route("/v1/approvals", get(list_approvals))
        .route("/v1/approvals/{id}", get(get_approval).post(answer))
}

#[derive(Deserialize)]
struct AgentsQuery {
    workspace_id: Option<i64>,
    state: Option<String>,
}

#[derive(Deserialize)]
struct ApprovalsQuery {
    state: Option<String>,
}

type Id = Result<Path<i64>, PathRejection>;

fn agent_id(id: Id) -> Result<i64, ApiError> {
    id.map(|Path(id)| id)
        .map_err(|_| ApiError::not_found("there is no such agent"))
}

async fn list_agents(
    State(state): State<AppState>,
    query: Result<Query<AgentsQuery>, QueryRejection>,
) -> Result<Json<AgentsPage>, ApiError> {
    let Query(query) = query.map_err(bad_query)?;
    let filter = query
        .state
        .filter(|state| !state.is_empty())
        .map(|state| state.parse::<AgentState>())
        .transpose()
        .map_err(AgentError::Invalid)?;
    let agents = state.agents.list(query.workspace_id, filter)?;
    Ok(Json(AgentsPage { agents }))
}

async fn get_agent(State(state): State<AppState>, id: Id) -> Result<Json<Agent>, ApiError> {
    Ok(Json(state.agents.get(agent_id(id)?)?))
}

async fn start_agent(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<StartAgent>, JsonRejection>,
) -> Result<(StatusCode, Json<Agent>), ApiError> {
    let Path(workspace_id) = id.map_err(|_| ApiError::not_found("there is no such workspace"))?;
    let Json(body) = body?;
    let agent = state.agents.start_agent(workspace_id, Role::User, None, &body)?;
    Ok((StatusCode::CREATED, Json(agent)))
}

async fn input(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Input>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = agent_id(id)?;
    let Json(body) = body?;
    state.agents.input(id, &body.text).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn keys(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Keys>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = agent_id(id)?;
    let Json(body) = body?;
    let bytes = BASE64
        .decode(body.data.trim())
        .map_err(|error| AgentError::Invalid(format!("`data` isn't valid base64: {error}")))?;
    state.agents.keys(id, &bytes)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resize(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Resize>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = agent_id(id)?;
    let Json(body) = body?;
    state.agents.resize(id, body.rows, body.cols)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn trust(State(state): State<AppState>, id: Id) -> Result<StatusCode, ApiError> {
    state.agents.trust(agent_id(id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop(State(state): State<AppState>, id: Id) -> Result<StatusCode, ApiError> {
    state.agents.stop(agent_id(id)?)?;
    Ok(StatusCode::ACCEPTED)
}

async fn resume(State(state): State<AppState>, id: Id) -> Result<(StatusCode, Json<Agent>), ApiError> {
    let agent = state.agents.resume(agent_id(id)?)?;
    Ok((StatusCode::CREATED, Json(agent)))
}

async fn remove_agent(State(state): State<AppState>, id: Id) -> Result<StatusCode, ApiError> {
    state.agents.remove(agent_id(id)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn screen(State(state): State<AppState>, id: Id) -> Result<Json<Screen>, ApiError> {
    Ok(Json(state.agents.screen(agent_id(id)?)?))
}

/// The terminal as server-sent events: a `snapshot`, then `output` and
/// `resize` in order, and `exit` last. A watcher that falls too far behind
/// is dropped; it reconnects for a fresh snapshot.
async fn terminal(State(state): State<AppState>, id: Id) -> Result<impl IntoResponse, ApiError> {
    let view = state.agents.attach(agent_id(id)?)?;
    let mut stopping = state.stopping.clone();
    let (sender, receiver) = mpsc::channel::<Result<SseEvent, Infallible>>(64);

    tokio::spawn(async move {
        let send = |event: SseEvent| {
            let sender = sender.clone();
            async move { sender.send(Ok(event)).await.is_ok() }
        };
        let mut attach = match view {
            TerminalView::Gone {
                rows,
                cols,
                exit_code,
            } => {
                let _ = send(snapshot(rows, cols, &[])).await && send(exit(exit_code)).await;
                return;
            }
            TerminalView::Live(attach) => attach,
        };
        if !send(snapshot(attach.rows, attach.cols, &attach.snapshot)).await {
            return;
        }
        if let Some(code) = attach.exited {
            let _ = send(exit(code)).await;
            return;
        }
        loop {
            let update = tokio::select! {
                update = attach.updates.recv() => update,
                _ = stopping.changed() => return,
                _ = sender.closed() => return,
            };
            let (event, last) = match update {
                Ok(TerminalUpdate::Output(bytes)) => (output(&bytes), false),
                Ok(TerminalUpdate::Resize { rows, cols }) => (
                    SseEvent::default()
                        .event("resize")
                        .data(json!({ "rows": rows, "cols": cols }).to_string()),
                    false,
                ),
                Ok(TerminalUpdate::Exited { code }) => (exit(code), true),
                // Too slow to keep up: end the stream so the client
                // reconnects and starts again from a snapshot.
                Err(RecvError::Lagged(_) | RecvError::Closed) => return,
            };
            if !send(event).await || last {
                return;
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(KeepAlive::default()))
}

fn snapshot(rows: u16, cols: u16, data: &[u8]) -> SseEvent {
    SseEvent::default()
        .event("snapshot")
        .data(json!({ "rows": rows, "cols": cols, "data": BASE64.encode(data) }).to_string())
}

fn output(bytes: &[u8]) -> SseEvent {
    SseEvent::default()
        .event("output")
        .data(json!({ "data": BASE64.encode(bytes) }).to_string())
}

fn exit(code: Option<i32>) -> SseEvent {
    SseEvent::default()
        .event("exit")
        .data(json!({ "exit_code": code }).to_string())
}

async fn list_approvals(
    State(state): State<AppState>,
    query: Result<Query<ApprovalsQuery>, QueryRejection>,
) -> Result<Json<ApprovalsPage>, ApiError> {
    let Query(query) = query.map_err(bad_query)?;
    let filter = query
        .state
        .filter(|state| !state.is_empty())
        .map(|state| state.parse::<ApprovalState>())
        .transpose()
        .map_err(AgentError::Invalid)?;
    Ok(Json(ApprovalsPage {
        approvals: state.agents.approvals(filter)?,
    }))
}

async fn get_approval(State(state): State<AppState>, id: Id) -> Result<Json<Approval>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such approval"))?;
    Ok(Json(state.agents.approval(id)?))
}

async fn answer(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Answer>, JsonRejection>,
) -> Result<Json<Approval>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such approval"))?;
    let Json(body) = body?;
    let message = body.message.filter(|message| !message.trim().is_empty());
    let approval = state
        .agents
        .decide(id, body.decision == Verdict::Allow, message)?;
    Ok(Json(approval))
}

fn bad_query(rejection: QueryRejection) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: rejection.body_text(),
    }
}

impl From<AgentError> for ApiError {
    fn from(error: AgentError) -> Self {
        let status = match &error {
            AgentError::Invalid(_) => StatusCode::BAD_REQUEST,
            AgentError::NotFound(_) => StatusCode::NOT_FOUND,
            AgentError::Conflict(_) => StatusCode::CONFLICT,
            AgentError::Forbidden(_) => StatusCode::FORBIDDEN,
            AgentError::Failed(_) | AgentError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}
