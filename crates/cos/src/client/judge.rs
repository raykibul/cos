//! jev's routes from the client side. See `docs/judge.md`.

use cos_core::ProjectSwitch;
use hyper::Method;

use super::{Client, read_json};
use crate::daemon::judge::JudgeOverview;
use crate::daemon::judge_api::SetJudge;

impl Client {
    pub(crate) async fn judge(&self) -> anyhow::Result<JudgeOverview> {
        self.get_json("/v1/judge").await
    }

    /// Looks for the key again, the login shell included.
    pub(crate) async fn reload_judge(&self) -> anyhow::Result<JudgeOverview> {
        read_json(self.send::<()>(Method::POST, "/v1/judge/reload", None).await?).await
    }

    pub(crate) async fn set_judge(&self, project_id: i64, enabled: bool) -> anyhow::Result<ProjectSwitch> {
        let path = format!("/v1/projects/{project_id}/judge");
        read_json(self.send(Method::PUT, &path, Some(&SetJudge { enabled })).await?).await
    }
}
