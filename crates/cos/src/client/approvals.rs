//! The approvals policy from the client side: the chief's answers and
//! per-run limits. See "Approvals policy" in `docs/daemon.md`.

use cos_core::{Approval, LimitsChange, RunLimits};
use hyper::Method;

use super::{Client, read_json};
use crate::daemon::agents_api::Verdict;
use crate::daemon::approvals_api::ChiefAnswer;

impl Client {
    /// Answers an approval routed to the chief, as the chief.
    #[allow(dead_code, reason = "the chief's MCP bridge and the tests call it")]
    pub(crate) async fn chief_answer(&self, id: i64, allow: bool, reason: &str) -> anyhow::Result<Approval> {
        let answer = ChiefAnswer {
            decision: if allow { Verdict::Allow } else { Verdict::Deny },
            reason: reason.to_owned(),
        };
        let path = format!("/v1/approvals/{id}/chief");
        read_json(self.send(Method::POST, &path, Some(&answer)).await?).await
    }

    pub(crate) async fn run_limits(&self, run_id: i64) -> anyhow::Result<RunLimits> {
        self.get_json(&format!("/v1/runs/{run_id}/limits")).await
    }

    pub(crate) async fn set_run_limits(
        &self,
        run_id: i64,
        change: &LimitsChange,
    ) -> anyhow::Result<RunLimits> {
        let path = format!("/v1/runs/{run_id}/limits");
        read_json(self.send(Method::PUT, &path, Some(change)).await?).await
    }
}
