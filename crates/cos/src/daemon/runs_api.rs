//! Routes for runs, their plans and tasks. See "Runs, tasks and the board"
//! in `docs/daemon.md` for the contract.
//!
//! - `GET /v1/runs`: [`RunsPage`], newest first
//! - `POST /v1/runs`: [`NewRun`] → 201 [`Run`]
//! - `GET /v1/runs/{id}`: [`Board`]
//! - `POST /v1/runs/{id}/plan`: [`PlanProposal`] → [`Board`]
//! - `POST /v1/runs/{id}/approve-plan` → [`Board`]
//! - `POST /v1/runs/{id}/cancel`: optional [`CancelRun`] → [`Board`]
//! - `PATCH /v1/tasks/{id}`: [`UpdateTask`] → [`Board`]

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use cos_core::{Board, NewRun, PlanProposal, Run, RunSummary, TaskState};
use serde::{Deserialize, Serialize};

use super::api::{ApiError, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunsPage {
    pub runs: Vec<RunSummary>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct CancelRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UpdateTask {
    pub state: TaskState,
    /// Required when blocking, failing or retrying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/{id}", get(board))
        .route("/v1/runs/{id}/plan", post(propose_plan))
        .route("/v1/runs/{id}/approve-plan", post(approve_plan))
        .route("/v1/runs/{id}/cancel", post(cancel_run))
        .route("/v1/tasks/{id}", patch(update_task))
}

async fn list_runs(State(state): State<AppState>) -> Result<Json<RunsPage>, ApiError> {
    Ok(Json(RunsPage {
        runs: state.runs.list()?,
    }))
}

async fn create_run(
    State(state): State<AppState>,
    body: Result<Json<NewRun>, JsonRejection>,
) -> Result<(StatusCode, Json<Run>), ApiError> {
    let Json(body) = body?;
    Ok((StatusCode::CREATED, Json(state.runs.create(body)?)))
}

async fn board(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Board>, ApiError> {
    Ok(Json(state.runs.board(run_id(id)?)?))
}

async fn propose_plan(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<PlanProposal>, JsonRejection>,
) -> Result<Json<Board>, ApiError> {
    let id = run_id(id)?;
    let Json(plan) = body?;
    Ok(Json(state.runs.propose_plan(id, &plan)?))
}

async fn approve_plan(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Board>, ApiError> {
    Ok(Json(state.runs.approve_plan(run_id(id)?)?))
}

async fn cancel_run(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Option<Json<CancelRun>>, JsonRejection>,
) -> Result<Json<Board>, ApiError> {
    let id = run_id(id)?;
    let body = body?.map(|Json(body)| body).unwrap_or_default();
    Ok(Json(state.runs.cancel(id, body.reason.as_deref())?))
}

async fn update_task(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<UpdateTask>, JsonRejection>,
) -> Result<Json<Board>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such task"))?;
    let Json(body) = body?;
    Ok(Json(state.runs.update_task(
        id,
        body.state,
        body.reason.as_deref(),
    )?))
}

fn run_id(id: Result<Path<i64>, PathRejection>) -> Result<i64, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such run"))?;
    Ok(id)
}
