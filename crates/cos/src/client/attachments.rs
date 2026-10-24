//! Uploading files to the daemon, for `cos … say --file` and the tests. See
//! "Attachments" in `docs/agents-api.md`.

use std::path::Path;

use anyhow::Context;
use hyper::Method;
use hyper::body::Bytes;

use super::{Client, read_json};
use crate::daemon::attachments::{Attachment, content_type_of};

/// Where an upload goes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Folder {
    /// An agent's folder.
    Agent(i64),
    /// A workspace, for an agent about to start there.
    Workspace(i64),
    /// A project's Chief of Staff.
    Staff(i64),
}

impl Client {
    /// Uploads `bytes` as `name` and returns the saved file; its `name` is
    /// what a message refers to.
    pub(crate) async fn upload(
        &self,
        folder: Folder,
        name: &str,
        bytes: Vec<u8>,
    ) -> anyhow::Result<Attachment> {
        let base = match folder {
            Folder::Agent(id) => format!("/v1/agents/{id}/attachments"),
            Folder::Workspace(id) => format!("/v1/workspaces/{id}/attachments"),
            Folder::Staff(id) => format!("/v1/projects/{id}/staff/attachments"),
        };
        let path = format!("{base}?name={}", encode(name));
        let response = self
            .send_bytes(
                Method::POST,
                &path,
                Some(content_type_of(name)),
                Bytes::from(bytes),
            )
            .await?;
        read_json(response).await
    }

    /// An agent's attachment, as the apps fetch it to show it.
    #[cfg(test)]
    pub(crate) async fn download(&self, agent_id: i64, name: &str) -> anyhow::Result<(String, Vec<u8>)> {
        use http_body_util::BodyExt;
        let path = format!("/v1/agents/{agent_id}/attachments/{}", encode(name));
        let response = self.send_bytes(Method::GET, &path, None, Bytes::new()).await?;
        let content_type = response
            .headers()
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response.into_body().collect().await?.to_bytes().to_vec();
        Ok((content_type, bytes))
    }

    /// Uploads each file and returns the names to put in the message.
    pub(crate) async fn upload_files(
        &self,
        folder: Folder,
        files: &[impl AsRef<Path>],
    ) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::with_capacity(files.len());
        for file in files {
            let file = file.as_ref();
            let bytes = std::fs::read(file).with_context(|| format!("could not read {}", file.display()))?;
            let name = file.file_name().and_then(|name| name.to_str()).unwrap_or("file");
            names.push(self.upload(folder, name, bytes).await?.name);
        }
        Ok(names)
    }
}

/// Percent-encodes everything but unreserved characters, for a query value.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_values_are_encoded() {
        assert_eq!(super::encode("Screen Shot 1.png"), "Screen%20Shot%201.png");
        assert_eq!(super::encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(super::encode("ছ"), "%E0%A6%9B");
    }
}
