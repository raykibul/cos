//! The Chief of Staff's routes from the client side. See
//! `docs/chief-of-staff.md`.

use cos_core::StaffItem;
use hyper::Method;

use super::{Client, read_json};
use crate::daemon::staff::{StaffInbox, StaffMessage, StaffSettings, StaffView};

impl Client {
    pub(crate) async fn staff(&self, project_id: i64) -> anyhow::Result<StaffView> {
        self.get_json(&format!("/v1/projects/{project_id}/staff")).await
    }

    /// Sets the project's model; `None` goes back to the default.
    pub(crate) async fn set_staff_model(
        &self,
        project_id: i64,
        model: Option<String>,
    ) -> anyhow::Result<StaffView> {
        self.configure_staff(
            project_id,
            &StaffSettings {
                model: Some(model),
                ..StaffSettings::default()
            },
        )
        .await
    }

    /// Pauses or resumes its coordination.
    pub(crate) async fn set_staff_paused(&self, project_id: i64, paused: bool) -> anyhow::Result<StaffView> {
        self.configure_staff(
            project_id,
            &StaffSettings {
                paused: Some(paused),
                ..StaffSettings::default()
            },
        )
        .await
    }

    async fn configure_staff(&self, project_id: i64, settings: &StaffSettings) -> anyhow::Result<StaffView> {
        let path = format!("/v1/projects/{project_id}/staff");
        read_json(self.send(Method::PUT, &path, Some(settings)).await?).await
    }

    pub(crate) async fn start_staff(&self, project_id: i64) -> anyhow::Result<StaffView> {
        let path = format!("/v1/projects/{project_id}/staff/start");
        read_json(self.send::<()>(Method::POST, &path, None).await?).await
    }

    pub(crate) async fn message_staff(&self, project_id: i64, text: String) -> anyhow::Result<()> {
        let path = format!("/v1/projects/{project_id}/staff/message");
        self.send(Method::POST, &path, Some(&StaffMessage { text }))
            .await?;
        Ok(())
    }

    pub(crate) async fn staff_inbox(&self, project_id: i64) -> anyhow::Result<Vec<StaffItem>> {
        let inbox: StaffInbox = self
            .get_json(&format!("/v1/projects/{project_id}/staff/inbox"))
            .await?;
        Ok(inbox.items)
    }
}
