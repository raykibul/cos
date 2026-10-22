//! Environment variables for agents, merged into [`super::api::router`] and
//! served on the tailnet listener too. The rules are in "Environment" in
//! `docs/daemon.md`:
//!
//! - `GET /v1/env` → [`GlobalEnv`]
//! - `PUT /v1/env/{name}` with [`SetVariable`] → [`GlobalEnv`]
//! - `DELETE /v1/env/{name}` → [`GlobalEnv`]
//! - `GET /v1/projects/{id}/env` → [`ProjectEnv`]
//! - `PUT /v1/projects/{id}/env/{name}` with [`SetVariable`] → [`ProjectEnv`]
//! - `DELETE /v1/projects/{id}/env/{name}` → [`ProjectEnv`]
//!
//! Values are returned as they are; the apps mask them. They never go in
//! an error message, an event or a log line.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use cos_core::{EnvError, GlobalEnv, ProjectEnv};
use cos_harness::claude::{RESERVED_ENV_PREFIX, is_reserved_env};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{ApiError, AppState};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/env", get(global))
        .route("/v1/env/{name}", put(set_global).delete(unset_global))
        .route("/v1/projects/{id}/env", get(project))
        .route(
            "/v1/projects/{id}/env/{name}",
            put(set_project).delete(unset_project),
        )
}

/// Body of `PUT …/env/{name}`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SetVariable {
    pub value: String,
}

type ProjectPath = Result<Path<(i64, String)>, PathRejection>;

async fn global(State(state): State<AppState>) -> Result<Json<GlobalEnv>, ApiError> {
    Ok(Json(GlobalEnv {
        variables: state.agents.env().global()?,
    }))
}

async fn set_global(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<GlobalEnv>, ApiError> {
    let value = value(body)?;
    check_settable(&name)?;
    state.agents.env().set(None, &name, &value)?;
    global(State(state)).await
}

async fn unset_global(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<GlobalEnv>, ApiError> {
    state.agents.env().unset(None, &name)?;
    global(State(state)).await
}

async fn project(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<ProjectEnv>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such project"))?;
    Ok(Json(state.agents.env().project(id)?))
}

async fn set_project(
    State(state): State<AppState>,
    path: ProjectPath,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<ProjectEnv>, ApiError> {
    let Path((id, name)) = path.map_err(|_| ApiError::not_found("there is no such project"))?;
    let value = value(body)?;
    check_settable(&name)?;
    let env = state.agents.env();
    env.set(Some(id), &name, &value)?;
    Ok(Json(env.project(id)?))
}

async fn unset_project(
    State(state): State<AppState>,
    path: ProjectPath,
) -> Result<Json<ProjectEnv>, ApiError> {
    let Path((id, name)) = path.map_err(|_| ApiError::not_found("there is no such project"))?;
    let env = state.agents.env();
    env.unset(Some(id), &name)?;
    Ok(Json(env.project(id)?))
}

/// The body's `value`. Read as plain JSON first, so a malformed body's
/// error, which can quote the body, never reaches the response.
fn value(body: Result<Json<Value>, JsonRejection>) -> Result<String, ApiError> {
    let invalid = || ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "the body must be JSON like {\"value\": \"…\"}, with a string value".into(),
    };
    let Json(body) = body.map_err(|_| invalid())?;
    match body.get("value") {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(invalid()),
    }
}

/// Refuses the names cos sets itself for an agent (see
/// [`cos_harness::claude::reserved_env_names`]). The shape is checked by
/// [`cos_core::Environment::set`].
fn check_settable(name: &str) -> Result<(), ApiError> {
    if !is_reserved_env(name) {
        return Ok(());
    }
    let message = if name.starts_with(RESERVED_ENV_PREFIX) {
        format!("names starting with {RESERVED_ENV_PREFIX} are kept for cos itself, so {name} can't be set")
    } else {
        format!("cos sets {name} itself for every agent, so it can't be set here")
    };
    Err(ApiError {
        status: StatusCode::BAD_REQUEST,
        message,
    })
}

impl From<EnvError> for ApiError {
    fn from(error: EnvError) -> Self {
        let status = match &error {
            EnvError::Invalid(_) => StatusCode::BAD_REQUEST,
            EnvError::NotFound(_) => StatusCode::NOT_FOUND,
            EnvError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}
