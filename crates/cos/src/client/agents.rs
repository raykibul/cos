//! The agents API from the client side: agents, their terminals, approvals,
//! See `docs/agents-api.md`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use cos_core::{Agent, Approval};
use http_body_util::BodyExt;
use hyper::Method;
use hyper::body::Incoming;
use serde::Deserialize;

use super::{Client, find, read_json, sse_data};
use crate::daemon::agents::{Screen, StartAgent};
use crate::daemon::agents_api::{AgentsPage, Answer, ApprovalsPage, Input, Keys, Resize, Verdict};
use crate::daemon::transcript::Transcript;

impl Client {
    pub(crate) async fn agents(&self, workspace_id: Option<i64>) -> anyhow::Result<Vec<Agent>> {
        let query = workspace_id.map_or_else(String::new, |id| format!("?workspace_id={id}"));
        let page: AgentsPage = self.get_json(&format!("/v1/agents{query}")).await?;
        Ok(page.agents)
    }

    pub(crate) async fn agent(&self, id: i64) -> anyhow::Result<Agent> {
        self.get_json(&format!("/v1/agents/{id}")).await
    }

    pub(crate) async fn start_agent(&self, workspace_id: i64, request: &StartAgent) -> anyhow::Result<Agent> {
        let path = format!("/v1/workspaces/{workspace_id}/agents");
        read_json(self.send(Method::POST, &path, Some(request)).await?).await
    }

    /// Types `text` into the agent and presses Return.
    pub(crate) async fn say(&self, id: i64, text: String) -> anyhow::Result<()> {
        let path = format!("/v1/agents/{id}/input");
        self.send(Method::POST, &path, Some(&Input { text })).await?;
        Ok(())
    }

    /// Sends raw keystrokes.
    pub(crate) async fn keys(&self, id: i64, bytes: &[u8]) -> anyhow::Result<()> {
        let keys = Keys {
            data: BASE64.encode(bytes),
        };
        self.send(Method::POST, &format!("/v1/agents/{id}/keys"), Some(&keys))
            .await?;
        Ok(())
    }

    pub(crate) async fn resize_agent(&self, id: i64, rows: u16, cols: u16) -> anyhow::Result<()> {
        let path = format!("/v1/agents/{id}/resize");
        self.send(Method::POST, &path, Some(&Resize { rows, cols }))
            .await?;
        Ok(())
    }

    pub(crate) async fn trust_agent(&self, id: i64) -> anyhow::Result<()> {
        self.post_empty(&format!("/v1/agents/{id}/trust")).await
    }

    pub(crate) async fn stop_agent(&self, id: i64) -> anyhow::Result<()> {
        self.post_empty(&format!("/v1/agents/{id}/stop")).await
    }

    pub(crate) async fn resume_agent(&self, id: i64) -> anyhow::Result<Agent> {
        let path = format!("/v1/agents/{id}/resume");
        read_json(self.send::<()>(Method::POST, &path, None).await?).await
    }

    pub(crate) async fn remove_agent(&self, id: i64) -> anyhow::Result<()> {
        self.send::<()>(Method::DELETE, &format!("/v1/agents/{id}"), None)
            .await?;
        Ok(())
    }

    pub(crate) async fn screen(&self, id: i64) -> anyhow::Result<Screen> {
        self.get_json(&format!("/v1/agents/{id}/screen")).await
    }

    /// The agent's chat from Claude's transcript, the messages after `after`.
    pub(crate) async fn transcript(&self, id: i64, after: usize) -> anyhow::Result<Transcript> {
        self.get_json(&format!("/v1/agents/{id}/transcript?after={after}"))
            .await
    }

    /// The agent's terminal: a snapshot, then output as it comes.
    pub(crate) async fn terminal(&self, id: i64) -> anyhow::Result<TerminalStream> {
        let response = self.get(&format!("/v1/agents/{id}/terminal")).await?;
        Ok(TerminalStream {
            body: response.into_body(),
            buffer: Vec::new(),
        })
    }

    /// Approvals, all of them or only those in `state` (such as `pending`).
    pub(crate) async fn approvals(&self, state: Option<&str>) -> anyhow::Result<Vec<Approval>> {
        let query = state.map_or_else(String::new, |state| format!("?state={state}"));
        let page: ApprovalsPage = self.get_json(&format!("/v1/approvals{query}")).await?;
        Ok(page.approvals)
    }

    pub(crate) async fn answer_approval(
        &self,
        id: i64,
        allow: bool,
        message: Option<String>,
    ) -> anyhow::Result<Approval> {
        let answer = Answer {
            decision: if allow { Verdict::Allow } else { Verdict::Deny },
            message,
        };
        let path = format!("/v1/approvals/{id}");
        read_json(self.send(Method::POST, &path, Some(&answer)).await?).await
    }

    async fn post_empty(&self, path: &str) -> anyhow::Result<()> {
        self.send::<()>(Method::POST, path, None).await?;
        Ok(())
    }
}

/// One event of an agent's terminal stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalMessage {
    /// Escape sequences that redraw the screen; always first.
    Snapshot {
        rows: u16,
        cols: u16,
        data: Vec<u8>,
    },
    Output(Vec<u8>),
    Resize {
        rows: u16,
        cols: u16,
    },
    /// The agent exited; the stream ends.
    Exit {
        exit_code: Option<i32>,
    },
}

/// `GET /v1/agents/{id}/terminal`.
pub(crate) struct TerminalStream {
    body: Incoming,
    buffer: Vec<u8>,
}

impl TerminalStream {
    /// The next message, or `None` when the daemon ends the stream.
    pub(crate) async fn next(&mut self) -> anyhow::Result<Option<TerminalMessage>> {
        loop {
            while let Some(end) = find(&self.buffer, b"\n\n") {
                let message: Vec<u8> = self.buffer.drain(..end + 2).collect();
                if let Some(parsed) = parse_terminal_event(&message)? {
                    return Ok(Some(parsed));
                }
            }
            match self.body.frame().await {
                None => return Ok(None),
                Some(frame) => {
                    if let Ok(data) = frame?.into_data() {
                        self.buffer.extend_from_slice(&data);
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Payload {
    rows: Option<u16>,
    cols: Option<u16>,
    data: Option<String>,
    exit_code: Option<i32>,
}

/// One server-sent event of a terminal stream. Unknown events and
/// keep-alives are `None`.
fn parse_terminal_event(message: &[u8]) -> anyhow::Result<Option<TerminalMessage>> {
    let text = String::from_utf8_lossy(message);
    let event = text
        .lines()
        .find_map(|line| line.strip_prefix("event:"))
        .map(str::trim)
        .unwrap_or("message");
    let Some(data) = sse_data(message) else {
        return Ok(None);
    };
    let payload: Payload = serde_json::from_str(&data)?;
    let bytes = || -> anyhow::Result<Vec<u8>> { Ok(BASE64.decode(payload.data.as_deref().unwrap_or(""))?) };
    Ok(match event {
        "snapshot" => Some(TerminalMessage::Snapshot {
            rows: payload.rows.unwrap_or(0),
            cols: payload.cols.unwrap_or(0),
            data: bytes()?,
        }),
        "output" => Some(TerminalMessage::Output(bytes()?)),
        "resize" => Some(TerminalMessage::Resize {
            rows: payload.rows.unwrap_or(0),
            cols: payload.cols.unwrap_or(0),
        }),
        "exit" => Some(TerminalMessage::Exit {
            exit_code: payload.exit_code,
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terminal_events() {
        let parse = |text: &str| parse_terminal_event(text.as_bytes()).unwrap();

        assert_eq!(
            parse("event: snapshot\ndata: {\"rows\":2,\"cols\":3,\"data\":\"aGk=\"}\n\n"),
            Some(TerminalMessage::Snapshot {
                rows: 2,
                cols: 3,
                data: b"hi".to_vec()
            })
        );
        assert_eq!(
            parse("event: exit\ndata: {\"exit_code\":null}\n\n"),
            Some(TerminalMessage::Exit { exit_code: None })
        );
        assert_eq!(parse(":\n\n"), None);
        assert_eq!(parse("event: later\ndata: {}\n\n"), None);
    }
}
