//! The chief's routes from the client side: tool calls (for `cos mcp`),
//! questions, messages and the report. See `docs/orchestration.md`.

use cos_core::{Approval, Question};
use hyper::Method;
use serde_json::Value;

use super::{Client, read_json};
use crate::daemon::chief_api::{AnswerQuestion, QuestionsPage, Report, RunMessage, ToolResult};

impl Client {
    /// Runs one of the chief's tools as agent `agent_id`.
    pub(crate) async fn chief_tool(
        &self,
        agent_id: i64,
        name: &str,
        arguments: &Value,
    ) -> anyhow::Result<String> {
        let path = format!("/v1/agents/{agent_id}/tools/{name}");
        let result: ToolResult = read_json(self.send(Method::POST, &path, Some(arguments)).await?).await?;
        Ok(result.text)
    }

    pub(crate) async fn approval(&self, id: i64) -> anyhow::Result<Approval> {
        self.get_json(&format!("/v1/approvals/{id}")).await
    }

    pub(crate) async fn questions(&self, state: Option<&str>) -> anyhow::Result<Vec<Question>> {
        let query = state.map_or_else(String::new, |state| format!("?state={state}"));
        let page: QuestionsPage = self.get_json(&format!("/v1/questions{query}")).await?;
        Ok(page.questions)
    }

    pub(crate) async fn answer_question(&self, id: i64, answer: String) -> anyhow::Result<Question> {
        let path = format!("/v1/questions/{id}/answer");
        read_json(
            self.send(Method::POST, &path, Some(&AnswerQuestion { answer }))
                .await?,
        )
        .await
    }

    /// A message to the run's chief.
    pub(crate) async fn message_chief(&self, run_id: i64, text: String) -> anyhow::Result<()> {
        let path = format!("/v1/runs/{run_id}/message");
        self.send(Method::POST, &path, Some(&RunMessage { text })).await?;
        Ok(())
    }

    pub(crate) async fn report(&self, run_id: i64) -> anyhow::Result<Report> {
        self.get_json(&format!("/v1/runs/{run_id}/report")).await
    }
}
