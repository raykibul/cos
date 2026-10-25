//! Shell terminals from the client side, for tests: see "Shells" in
//! `docs/agents-api.md`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hyper::Method;

use super::agents::TerminalStream;
use super::{Client, read_json};
use crate::daemon::agents_api::{Keys, Resize};
use crate::daemon::shells::{Shell, ShellsPage, StartShell};

impl Client {
    pub(crate) async fn shells(&self) -> anyhow::Result<Vec<Shell>> {
        let page: ShellsPage = self.get_json("/v1/shells").await?;
        Ok(page.shells)
    }

    pub(crate) async fn start_shell(&self, request: &StartShell) -> anyhow::Result<Shell> {
        read_json(self.send(Method::POST, "/v1/shells", Some(request)).await?).await
    }

    pub(crate) async fn shell_keys(&self, id: i64, bytes: &[u8]) -> anyhow::Result<()> {
        let keys = Keys {
            data: BASE64.encode(bytes),
        };
        self.send(Method::POST, &format!("/v1/shells/{id}/keys"), Some(&keys))
            .await?;
        Ok(())
    }

    pub(crate) async fn resize_shell(&self, id: i64, rows: u16, cols: u16) -> anyhow::Result<()> {
        let path = format!("/v1/shells/{id}/resize");
        self.send(Method::POST, &path, Some(&Resize { rows, cols }))
            .await?;
        Ok(())
    }

    pub(crate) async fn close_shell(&self, id: i64) -> anyhow::Result<()> {
        self.send::<()>(Method::DELETE, &format!("/v1/shells/{id}"), None)
            .await?;
        Ok(())
    }

    pub(crate) async fn shell_terminal(&self, id: i64) -> anyhow::Result<TerminalStream> {
        TerminalStream::open(self, &format!("/v1/shells/{id}/terminal")).await
    }
}
