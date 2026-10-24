//! Routes for the Chief of Staff, merged into [`super::api::router`]. The
//! contract is `docs/chief-of-staff.md`:
//!
//! - `GET /v1/projects/{id}/staff` → [`StaffView`], even before it starts
//! - `PUT /v1/projects/{id}/staff` with [`StaffSettings`] → [`StaffView`]
//! - `POST /v1/projects/{id}/staff/start` → [`StaffView`]
//! - `POST /v1/projects/{id}/staff/message` with [`StaffMessage`] → 202
//! - `GET /v1/projects/{id}/staff/inbox` → [`StaffInbox`], newest first
//! - `POST /v1/agents/{id}/tools/{name}`: a Chief of Staff's tool call, from
//!   `cos mcp`. The body is the tool's arguments; the answer is
//!   [`ToolResult`]. Unix socket only ([`is_tool_route`]).

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{ApiError, AppState};
use super::staff::{StaffInbox, StaffMessage, StaffSettings, StaffView};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{id}/staff", get(view).put(configure))
        .route("/v1/projects/{id}/staff/start", post(start))
        .route("/v1/projects/{id}/staff/message", post(message))
        .route("/v1/projects/{id}/staff/inbox", get(inbox))
        .route("/v1/agents/{id}/tools/{name}", post(call_tool))
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ToolResult {
    /// What the chief reads.
    pub text: String,
}

/// Whether `path` is the tool route, which only the local socket serves.
pub(crate) fn is_tool_route(path: &str) -> bool {
    path.strip_prefix("/v1/agents/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, rest)| rest == "tools" || rest.starts_with("tools/"))
}

async fn call_tool(
    State(state): State<AppState>,
    path: Result<Path<(i64, String)>, PathRejection>,
    body: Result<Option<Json<Value>>, JsonRejection>,
) -> Result<Json<ToolResult>, ApiError> {
    let Path((id, name)) = path.map_err(|_| ApiError::not_found("there is no such agent"))?;
    let arguments = body?.map_or(Value::Null, |Json(value)| value);
    let text = state.staff.call_tool(id, &name, arguments).await?;
    Ok(Json(ToolResult { text }))
}

type Id = Result<Path<i64>, PathRejection>;

fn project_id(id: Id) -> Result<i64, ApiError> {
    id.map(|Path(id)| id)
        .map_err(|_| ApiError::not_found("there is no such project"))
}

async fn view(State(state): State<AppState>, id: Id) -> Result<Json<StaffView>, ApiError> {
    Ok(Json(state.staff.view(project_id(id)?).await?))
}

async fn configure(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<StaffSettings>, JsonRejection>,
) -> Result<Json<StaffView>, ApiError> {
    let id = project_id(id)?;
    let Json(body) = body?;
    Ok(Json(state.staff.configure(id, body).await?))
}

async fn start(State(state): State<AppState>, id: Id) -> Result<Json<StaffView>, ApiError> {
    Ok(Json(state.staff.start_staff(project_id(id)?).await?))
}

async fn message(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<StaffMessage>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(id)?;
    let Json(body) = body?;
    state.staff.message(id, &body.text, &body.attachments).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn inbox(State(state): State<AppState>, id: Id) -> Result<Json<StaffInbox>, ApiError> {
    Ok(Json(StaffInbox {
        items: state.staff.inbox(project_id(id)?).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_routes_are_recognized() {
        assert!(is_tool_route("/v1/agents/3/tools/workspaces_list"));
        assert!(is_tool_route("/v1/agents/3/tools"));
        assert!(!is_tool_route("/v1/agents/3/input"));
        assert!(!is_tool_route("/v1/agents"));
        assert!(!is_tool_route("/v1/agents/3/toolsx"));
    }
}
