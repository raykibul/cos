//! The chief's tools, as the daemon runs them. `cos mcp` forwards each call
//! here with the calling agent's id; the tool list and its schemas live in
//! [`crate::mcp`]. `approval_decide` goes to the approvals component instead.

use axum::http::StatusCode;
use cos_core::events::kind;
use cos_core::runs::plan_problems;
use cos_core::{Board, PlanProposal, Role, RunState, TaskState, render_board};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Chief, error, quote, report_path};
use crate::daemon::api::ApiError;

/// The most bytes of markdown `report_write` takes.
const MAX_REPORT_BYTES: usize = 256 * 1024;
const MAX_SEND_CHARS: usize = 10_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskUpdate {
    task_id: i64,
    action: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskSend {
    task_id: i64,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskUser {
    question: String,
    #[serde(default)]
    choices: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportWrite {
    markdown: String,
}

impl Chief {
    /// Runs one tool for the chief agent `agent_id` and returns the text the
    /// chief reads. Errors are meant for the chief too.
    pub(crate) async fn call_tool(
        &self,
        agent_id: i64,
        name: &str,
        arguments: Value,
    ) -> Result<String, ApiError> {
        let inner = &self.inner;
        let agent = inner.agents.get(agent_id)?;
        let run_id = match (agent.role, agent.run_id) {
            (Role::Chief, Some(run_id)) => run_id,
            _ => {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    format!("agent {agent_id} is not a run's chief"),
                ));
            }
        };
        let run = inner.runs.run(run_id)?;
        if run.chief_agent_id != Some(agent_id) {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("agent {agent_id} is no longer the chief of run {run_id}"),
            ));
        }
        match name {
            "board_get" => Ok(board_text(&inner.runs.board(run_id)?)),
            "board_propose" => self.board_propose(run_id, arguments),
            "task_update" => self.task_update(run_id, parse(name, arguments)?),
            "task_send" => self.task_send(run_id, parse(name, arguments)?).await,
            "ask_user" => self.ask_user(run_id, agent_id, parse(name, arguments)?),
            "report_write" => self.report_write(run_id, parse(name, arguments)?),
            "approval_decide" => Err(error(
                StatusCode::BAD_REQUEST,
                "approval_decide is answered by POST /v1/approvals/{id}/chief".into(),
            )),
            _ => Err(ApiError::not_found(&format!("there is no tool {name}"))),
        }
    }

    fn board_propose(&self, run_id: i64, arguments: Value) -> Result<String, ApiError> {
        let plan: PlanProposal = parse("board_propose", arguments)?;
        let problems = plan_problems(&plan);
        if !problems.is_empty() {
            let list: Vec<String> = problems.iter().map(|problem| format!("- {problem}")).collect();
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!(
                    "The plan was not accepted. Fix every problem below and call board_propose again with the whole plan:\n{}",
                    list.join("\n")
                ),
            ));
        }
        let board = self.inner.runs.propose_plan(run_id, &plan)?;
        Ok(format!(
            "Plan proposed: {} tasks. Run {run_id} now waits for the user to approve it; cos then starts the \
             workers and wakes you when tasks merge, block or fail. Stop here and end your turn.\n\n{}",
            board.tasks.len(),
            render_board(&board)
        ))
    }

    fn task_update(&self, run_id: i64, update: TaskUpdate) -> Result<String, ApiError> {
        let runs = &self.inner.runs;
        let task = runs.task(update.task_id)?;
        if task.run_id != run_id {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("task {} belongs to another run", task.id),
            ));
        }
        let to = match update.action.as_str() {
            "retry" => TaskState::Ready,
            "fail" => TaskState::Failed,
            "cancel" => TaskState::Cancelled,
            other => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    format!("unknown action {other:?}; use retry, fail or cancel"),
                ));
            }
        };
        let reason = update.reason.trim();
        if reason.is_empty() {
            return Err(error(StatusCode::BAD_REQUEST, "say why in `reason`".into()));
        }
        let board = runs.update_task(task.id, to, Some(reason))?;
        let now = board
            .tasks
            .iter()
            .find(|candidate| candidate.id == task.id)
            .map_or(to, |candidate| candidate.state);
        Ok(format!(
            "Task {} {} is now {now}. Board: {}.",
            task.id,
            quote(&task.title),
            board.counts.summary()
        ))
    }

    async fn task_send(&self, run_id: i64, send: TaskSend) -> Result<String, ApiError> {
        let inner = &self.inner;
        let task = inner.runs.task(send.task_id)?;
        if task.run_id != run_id {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("task {} belongs to another run", task.id),
            ));
        }
        let message = send.message.trim();
        if message.is_empty() || message.chars().count() > MAX_SEND_CHARS {
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!("the message must be 1 to {MAX_SEND_CHARS} characters"),
            ));
        }
        let Some(worker) = task.agent_id.filter(|_| task.state == TaskState::Running) else {
            return Err(error(
                StatusCode::CONFLICT,
                format!("task {} has no running worker (it is {})", task.id, task.state),
            ));
        };
        inner
            .agents
            .send_message(worker, &format!("Message from the chief: {message}"))
            .await?;
        Ok(format!(
            "Sent to the worker of task {} (agent {worker}).",
            task.id
        ))
    }

    fn ask_user(&self, run_id: i64, agent_id: i64, ask: AskUser) -> Result<String, ApiError> {
        let run = self.inner.runs.run(run_id)?;
        if run.state.is_finished() {
            return Err(error(
                StatusCode::CONFLICT,
                format!("run {run_id} is {}", run.state),
            ));
        }
        let question = self
            .inner
            .questions
            .ask(run_id, Some(agent_id), &ask.question, &ask.choices)?;
        Ok(format!(
            "Asked the user (question {}). Their answer comes in a later wakeup. Carry on with anything that \
             doesn't depend on it, or end your turn.",
            question.id
        ))
    }

    fn report_write(&self, run_id: i64, write: ReportWrite) -> Result<String, ApiError> {
        let inner = &self.inner;
        let run = inner.runs.run(run_id)?;
        if run.state != RunState::Reporting {
            return Err(error(
                StatusCode::CONFLICT,
                format!(
                    "run {run_id} is {}; report_write is only for when every task is merged, failed or cancelled",
                    run.state
                ),
            ));
        }
        let markdown = write.markdown.trim();
        if markdown.is_empty() || markdown.len() > MAX_REPORT_BYTES {
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!("the report must be 1 to {MAX_REPORT_BYTES} bytes of markdown"),
            ));
        }
        // The approvals component's "Approved on your behalf (N)" section.
        let section = inner.runs.approvals_report_section(run_id)?;
        let mut text = format!("{markdown}\n");
        if !section.trim().is_empty() {
            text.push('\n');
            text.push_str(section.trim_end());
            text.push('\n');
        }
        let path = report_path(&inner.home, run_id);
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &text));
        if let Err(failure) = saved {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not save {}: {failure}", path.display()),
            ));
        }
        inner.runs.finish(run_id, &path)?;
        inner.log.append(
            kind::RUN_REPORT_WRITTEN,
            Some(&run_id.to_string()),
            json!({ "run_id": run_id, "path": path }),
        )?;
        Ok(format!(
            "Report saved to {}. Run {run_id} is done; end your turn.",
            path.display()
        ))
    }
}

/// The board for the chief: markdown, then the full JSON.
fn board_text(board: &Board) -> String {
    format!(
        "{}\nThe board as JSON:\n```json\n{}\n```",
        render_board(board),
        serde_json::to_string_pretty(board).unwrap_or_default()
    )
}

fn parse<T: DeserializeOwned>(tool: &str, arguments: Value) -> Result<T, ApiError> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    serde_json::from_value(arguments).map_err(|failure| {
        error(
            StatusCode::BAD_REQUEST,
            format!("bad arguments for {tool}: {failure}"),
        )
    })
}
