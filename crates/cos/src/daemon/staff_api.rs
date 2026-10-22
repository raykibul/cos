//! Routes for the Chief of Staff, merged into [`super::api::router`]. The
//! contract is `docs/chief-of-staff.md`:
//!
//! - `GET /v1/projects/{id}/staff` → [`StaffView`], even before it starts
//! - `PUT /v1/projects/{id}/staff` with [`StaffSettings`] → [`StaffView`]
//! - `POST /v1/projects/{id}/staff/start` → [`StaffView`]
//! - `POST /v1/projects/{id}/staff/message` with [`StaffMessage`] → 202
//! - `GET /v1/projects/{id}/staff/inbox` → [`StaffInbox`], newest first
//!
//! Its tool calls come through `POST /v1/agents/{id}/tools/{name}`
//! ([`super::chief_api`]).

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use super::api::{ApiError, AppState};
use super::staff::{StaffInbox, StaffMessage, StaffSettings, StaffView};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{id}/staff", get(view).put(configure))
        .route("/v1/projects/{id}/staff/start", post(start))
        .route("/v1/projects/{id}/staff/message", post(message))
        .route("/v1/projects/{id}/staff/inbox", get(inbox))
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
    state.staff.message(id, &body.text).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn inbox(State(state): State<AppState>, id: Id) -> Result<Json<StaffInbox>, ApiError> {
    Ok(Json(StaffInbox {
        items: state.staff.inbox(project_id(id)?).await?,
    }))
}
