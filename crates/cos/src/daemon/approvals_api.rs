//! Routes for the approvals policy, merged into [`super::api::router`]. See
//! "Approvals policy" in `docs/daemon.md` and `docs/approvals.md`.
//!
//! - `POST /v1/approvals/{id}/chief`: [`ChiefAnswer`] → `Approval`. 403 when
//!   the run's limits refuse it; the approval then waits for the user.
//! - `GET /v1/runs/{id}/limits` → [`RunLimits`]
//! - `PUT /v1/runs/{id}/limits`: [`LimitsChange`] → [`RunLimits`]

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use cos_core::{Approval, LimitsChange, RunLimits};
use serde::{Deserialize, Serialize};

use super::agents_api::Verdict;
use super::api::{ApiError, AppState};

/// Body of `POST /v1/approvals/{id}/chief`: the chief's `approval_decide`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ChiefAnswer {
    pub decision: Verdict,
    /// Required to allow; it goes in the report.
    #[serde(default)]
    pub reason: String,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/approvals/{id}/chief", post(chief_answer))
        .route("/v1/runs/{id}/limits", get(limits).put(set_limits))
}

async fn chief_answer(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<ChiefAnswer>, JsonRejection>,
) -> Result<Json<Approval>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such approval"))?;
    let Json(body) = body?;
    let approval = state
        .agents
        .chief_decide(id, body.decision == Verdict::Allow, &body.reason)?;
    Ok(Json(approval))
}

async fn limits(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<RunLimits>, ApiError> {
    Ok(Json(state.runs.limits(run_id(id)?)?))
}

async fn set_limits(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<LimitsChange>, JsonRejection>,
) -> Result<Json<RunLimits>, ApiError> {
    let id = run_id(id)?;
    let Json(change) = body?;
    Ok(Json(state.runs.set_limits(id, &change)?))
}

fn run_id(id: Result<Path<i64>, PathRejection>) -> Result<i64, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such run"))?;
    Ok(id)
}
