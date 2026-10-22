//! Environment variables for agents, from the client side. See
//! "Environment" in `docs/daemon.md`.

use cos_core::{GlobalEnv, ProjectEnv};
use hyper::Method;

use super::{Client, read_json};
use crate::daemon::env_api::SetVariable;

impl Client {
    pub(crate) async fn global_env(&self) -> anyhow::Result<GlobalEnv> {
        self.get_json("/v1/env").await
    }

    pub(crate) async fn project_env(&self, project_id: i64) -> anyhow::Result<ProjectEnv> {
        self.get_json(&format!("/v1/projects/{project_id}/env")).await
    }

    pub(crate) async fn set_global_env(&self, name: &str, value: String) -> anyhow::Result<GlobalEnv> {
        let path = format!("/v1/env/{}", checked(name)?);
        read_json(
            self.send(Method::PUT, &path, Some(&SetVariable { value }))
                .await?,
        )
        .await
    }

    pub(crate) async fn set_project_env(
        &self,
        project_id: i64,
        name: &str,
        value: String,
    ) -> anyhow::Result<ProjectEnv> {
        let path = format!("/v1/projects/{project_id}/env/{}", checked(name)?);
        read_json(
            self.send(Method::PUT, &path, Some(&SetVariable { value }))
                .await?,
        )
        .await
    }

    pub(crate) async fn unset_global_env(&self, name: &str) -> anyhow::Result<GlobalEnv> {
        let path = format!("/v1/env/{}", checked(name)?);
        read_json(self.send::<()>(Method::DELETE, &path, None).await?).await
    }

    pub(crate) async fn unset_project_env(&self, project_id: i64, name: &str) -> anyhow::Result<ProjectEnv> {
        let path = format!("/v1/projects/{project_id}/env/{}", checked(name)?);
        read_json(self.send::<()>(Method::DELETE, &path, None).await?).await
    }
}

/// `name` if it can be a variable name, which also makes it safe in a path.
fn checked(name: &str) -> anyhow::Result<&str> {
    cos_core::env::check_name(name).map_err(anyhow::Error::msg)?;
    Ok(name)
}
