//! jev's routes, merged into [`super::api::router`] and served on the
//! tailnet listener too. See "jev" in `docs/daemon.md` and `docs/judge.md`:
//!
//! - `GET /v1/judge` → [`JudgeOverview`]: where the key was found, the
//!   per-project switches, and each decision point's numbers
//! - `POST /v1/judge/reload` → [`JudgeOverview`]: looks for the key again,
//!   the login shell included, and answers once that's done
//! - `PUT /v1/projects/{id}/judge` with [`SetJudge`] → [`ProjectSwitch`]
//!
//! The key's value is never in a response, an event or a log line.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use cos_core::ProjectSwitch;
use serde::{Deserialize, Serialize};

use super::api::{ApiError, AppState};
use super::judge::JudgeOverview;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/judge", get(overview))
        .route("/v1/judge/reload", post(reload))
        .route("/v1/projects/{id}/judge", put(set_project))
}

/// Body of `PUT /v1/projects/{id}/judge`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SetJudge {
    pub enabled: bool,
}

async fn overview(State(state): State<AppState>) -> Result<Json<JudgeOverview>, ApiError> {
    Ok(Json(state.judge.overview()?))
}

async fn reload(State(state): State<AppState>) -> Result<Json<JudgeOverview>, ApiError> {
    Ok(Json(state.judge.reload().await?))
}

async fn set_project(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<SetJudge>, JsonRejection>,
) -> Result<Json<ProjectSwitch>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such project"))?;
    let Json(body) = body.map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "the body must be JSON like {\"enabled\": true}".into(),
    })?;
    state
        .judge
        .set_enabled(id, body.enabled)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(&format!("there is no project {id}")))
}
