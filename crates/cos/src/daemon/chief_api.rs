//! Routes for the chief, merged into [`super::api::router`]. The contract is
//! `docs/orchestration.md`:
//!
//! - `POST /v1/agents/{id}/tools/{name}`: a chief's tool call, from
//!   `cos mcp`. The body is the tool's arguments; the answer is
//!   [`ToolResult`]. Unix socket only.
//! - `GET /v1/questions?state=&run_id=` → [`QuestionsPage`], oldest first
//! - `GET /v1/questions/{id}` → [`Question`]
//! - `POST /v1/questions/{id}/answer` with [`AnswerQuestion`] → [`Question`]
//! - `POST /v1/runs/{id}/message` with [`RunMessage`] → 202
//! - `GET /v1/runs/{id}/report` → [`Report`], 404 before it exists

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use cos_core::{Question, QuestionState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{ApiError, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ToolResult {
    /// What the chief reads.
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct QuestionsPage {
    pub questions: Vec<Question>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AnswerQuestion {
    pub answer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunMessage {
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Report {
    pub markdown: String,
    pub path: std::path::PathBuf,
}

#[derive(Deserialize)]
struct QuestionsQuery {
    state: Option<String>,
    run_id: Option<i64>,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/agents/{id}/tools/{name}", post(call_tool))
        .route("/v1/questions", get(list_questions))
        .route("/v1/questions/{id}", get(get_question))
        .route("/v1/questions/{id}/answer", post(answer))
        .route("/v1/runs/{id}/message", post(message))
        .route("/v1/runs/{id}/report", get(report))
}

/// Whether `path` is the chief's tool route, which only the local socket serves.
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
    let text = state.chief.call_tool(id, &name, arguments).await?;
    Ok(Json(ToolResult { text }))
}

async fn list_questions(
    State(state): State<AppState>,
    query: Result<Query<QuestionsQuery>, QueryRejection>,
) -> Result<Json<QuestionsPage>, ApiError> {
    let Query(query) = query.map_err(|rejection| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: rejection.body_text(),
    })?;
    let filter = query
        .state
        .filter(|state| !state.is_empty())
        .map(|state| state.parse::<QuestionState>())
        .transpose()
        .map_err(|message| ApiError {
            status: StatusCode::BAD_REQUEST,
            message,
        })?;
    Ok(Json(QuestionsPage {
        questions: state.chief.questions(filter, query.run_id)?,
    }))
}

async fn get_question(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Question>, ApiError> {
    Ok(Json(state.chief.question(question_id(id)?)?))
}

async fn answer(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<AnswerQuestion>, JsonRejection>,
) -> Result<Json<Question>, ApiError> {
    let id = question_id(id)?;
    let Json(body) = body?;
    Ok(Json(state.chief.answer(id, &body.answer)?))
}

async fn message(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<RunMessage>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = run_id(id)?;
    let Json(body) = body?;
    state.chief.message(id, &body.text)?;
    Ok(StatusCode::ACCEPTED)
}

async fn report(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Report>, ApiError> {
    let (markdown, path) = state.chief.report(run_id(id)?)?;
    Ok(Json(Report { markdown, path }))
}

fn question_id(id: Result<Path<i64>, PathRejection>) -> Result<i64, ApiError> {
    id.map(|Path(id)| id)
        .map_err(|_| ApiError::not_found("there is no such question"))
}

fn run_id(id: Result<Path<i64>, PathRejection>) -> Result<i64, ApiError> {
    id.map(|Path(id)| id)
        .map_err(|_| ApiError::not_found("there is no such run"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_routes_are_recognised() {
        assert!(is_tool_route("/v1/agents/3/tools/board_get"));
        assert!(is_tool_route("/v1/agents/3/tools"));
        assert!(!is_tool_route("/v1/agents/3/input"));
        assert!(!is_tool_route("/v1/agents"));
        assert!(!is_tool_route("/v1/agents/3/toolsx"));
    }
}
