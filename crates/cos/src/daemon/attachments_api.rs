//! Routes for attachments, merged into [`super::api::router`]. The contract
//! is "Attachments" in `docs/agents-api.md`:
//!
//! - `POST /v1/agents/{id}/attachments?name=` → 201 [`Attachment`], into the
//!   agent's folder
//! - `POST /v1/workspaces/{id}/attachments?name=` → 201, into a workspace,
//!   for an agent you're about to start there
//! - `POST /v1/projects/{id}/staff/attachments?name=` → 201, into the Chief
//!   of Staff's worktree, made if needed (the chief isn't started)
//! - `GET /v1/agents/{id}/attachments/{name}`: the file itself
//!
//! An upload's body is the file's bytes, as they are.

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cos_core::events::kind;
use serde::Deserialize;
use serde_json::json;

use super::api::{ApiError, AppState};
use super::attachments::{self, Attachment};

pub(super) fn routes() -> Router<AppState> {
    // Room for the largest file plus the request around it.
    let limit = DefaultBodyLimit::max(attachments::MAX_BYTES + 64 * 1024);
    Router::new()
        .route("/v1/agents/{id}/attachments", post(to_agent).layer(limit))
        .route("/v1/workspaces/{id}/attachments", post(to_workspace).layer(limit))
        .route("/v1/projects/{id}/staff/attachments", post(to_staff).layer(limit))
        .route("/v1/agents/{id}/attachments/{name}", get(download))
}

#[derive(Deserialize)]
struct Upload {
    #[serde(default)]
    name: String,
}

type Id = Result<Path<i64>, PathRejection>;
type Name = Result<Query<Upload>, QueryRejection>;
type Body = Result<Bytes, BytesRejection>;

async fn to_agent(
    State(state): State<AppState>,
    id: Id,
    query: Name,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such agent"))?;
    let folder = state.agents.folder(id)?;
    let agent = state.agents.get(id)?;
    upload(&state, &folder, agent.workspace_id, query, &headers, &body)
}

async fn to_workspace(
    State(state): State<AppState>,
    id: Id,
    query: Name,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such workspace"))?;
    // The Chief of Staff's worktree has its own route.
    let workspace = state.projects.listed_workspace(id)?;
    upload(&state, &workspace.path, workspace.id, query, &headers, &body)
}

async fn to_staff(
    State(state): State<AppState>,
    id: Id,
    query: Name,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such project"))?;
    let worktree = state.staff.attachments_folder(id).await?;
    upload(&state, &worktree.path, worktree.id, query, &headers, &body)
}

fn upload(
    state: &AppState,
    folder: &std::path::Path,
    workspace_id: i64,
    query: Name,
    headers: &HeaderMap,
    body: &Body,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let body = body.as_ref().map_err(|rejection| ApiError {
        status: rejection.status(),
        message: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            format!(
                "a file can be at most {} MB",
                attachments::MAX_BYTES / 1024 / 1024
            )
        } else {
            rejection.body_text()
        },
    })?;
    let Query(query) = query.map_err(|rejection| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: rejection.body_text(),
    })?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let attachment = attachments::save(folder, &query.name, content_type, body)?;
    state.log.append(
        kind::ATTACHMENT_ADDED,
        None,
        json!({
            "workspace_id": workspace_id,
            "name": attachment.name,
            "size": attachment.size,
            "content_type": attachment.content_type,
        }),
    )?;
    Ok((StatusCode::CREATED, Json(attachment)))
}

/// The file, with its type from its extension. Names are unique and files
/// never change, so clients may cache it.
async fn download(
    State(state): State<AppState>,
    path: Result<Path<(i64, String)>, PathRejection>,
) -> Result<Response, ApiError> {
    let Path((id, name)) = path.map_err(|_| ApiError::not_found("there is no such attachment"))?;
    let folder = state.agents.folder(id)?;
    let attachment = attachments::find(&folder, &name)?;
    let path = attachment.path.clone();
    let read = tokio::task::spawn_blocking(move || std::fs::read(path)).await;
    let bytes = read
        .unwrap_or_else(|error| Err(std::io::Error::other(error)))
        .map_err(|error| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("could not read {}: {error}", attachment.name),
        })?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&attachment.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
