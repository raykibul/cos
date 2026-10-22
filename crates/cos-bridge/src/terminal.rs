//! An agent's terminal, across drops: each reconnect starts from a fresh
//! snapshot. See "Terminal stream" in `docs/agents-api.md`.

use serde::Deserialize;

use crate::sse::SseEvent;
use crate::stream::{self, Outbox, Retry};
use crate::{Bridge, BridgeError, Sink, StreamHandle, TerminalMessage};

/// How one connection ended.
enum End {
    /// The agent exited; the stream is over.
    Exited,
    /// The sink is gone.
    Stopped,
    /// Dropped; reconnect.
    Lost(BridgeError),
}

impl Bridge {
    /// Streams agent `agent_id`'s terminal until it exits or the handle is
    /// closed. After a drop it sends `disconnected`, reconnects with backoff,
    /// and the next message is a fresh `snapshot`. A refusal is a
    /// `disconnected` with the daemon's code, and waits the longest backoff.
    ///
    /// Must be called inside a Tokio runtime.
    pub fn attach_terminal(&self, agent_id: i64, sink: impl Sink<TerminalMessage>) -> StreamHandle {
        let bridge = self.clone();
        stream::spawn(sink, move |outbox| bridge.attach(agent_id, outbox))
    }

    async fn attach<S: Sink<TerminalMessage>>(self, agent_id: i64, mut outbox: Outbox<S>) {
        let path = format!("/v1/agents/{agent_id}/terminal");
        let mut retry = Retry::new(self.backoff);
        let mut last_drop: Option<String> = None;
        loop {
            let mut connected = false;
            let error = match self.attach_once(&path, &mut outbox, &mut connected).await {
                End::Exited | End::Stopped => return,
                End::Lost(error) => error,
            };
            if connected {
                retry.reset();
                last_drop = None;
            }
            let delay = if error.is_refusal() {
                retry.longest()
            } else {
                retry.delay()
            };
            if last_drop.as_deref() != Some(error.message.as_str()) {
                let message = TerminalMessage::Disconnected {
                    reason: error.message.clone(),
                    code: error.code,
                };
                if !outbox.send(message) {
                    return;
                }
                last_drop = Some(error.message);
            }
            tokio::time::sleep(delay).await;
        }
    }

    async fn attach_once<S: Sink<TerminalMessage>>(
        &self,
        path: &str,
        outbox: &mut Outbox<S>,
        connected: &mut bool,
    ) -> End {
        let mut reader = match self.open_stream(path).await {
            Ok(reader) => reader,
            Err(error) => return End::Lost(error),
        };
        loop {
            let event = match reader.next().await {
                Ok(Some(event)) => event,
                Ok(None) => return End::Lost(BridgeError::io("the daemon closed the terminal stream")),
                Err(error) => return End::Lost(error),
            };
            let Some(message) = parse(&event) else {
                continue;
            };
            *connected = true;
            let exited = matches!(message, TerminalMessage::Exit { .. });
            if !outbox.send(message) {
                return End::Stopped;
            }
            if exited {
                return End::Exited;
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

/// One terminal event. Unknown or malformed events are `None`.
fn parse(event: &SseEvent) -> Option<TerminalMessage> {
    let payload: Payload = serde_json::from_str(&event.data).ok()?;
    Some(match event.event.as_str() {
        "snapshot" => TerminalMessage::Snapshot {
            rows: payload.rows.unwrap_or(0),
            cols: payload.cols.unwrap_or(0),
            data: payload.data.unwrap_or_default(),
        },
        "output" => TerminalMessage::Output {
            data: payload.data.unwrap_or_default(),
        },
        "resize" => TerminalMessage::Resize {
            rows: payload.rows.unwrap_or(0),
            cols: payload.cols.unwrap_or(0),
        },
        "exit" => TerminalMessage::Exit {
            exit_code: payload.exit_code,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, data: &str) -> SseEvent {
        SseEvent {
            event: name.into(),
            data: data.into(),
        }
    }

    #[test]
    fn passes_terminal_events_through() {
        assert_eq!(
            parse(&event("snapshot", r#"{"rows":2,"cols":3,"data":"aGk="}"#)),
            Some(TerminalMessage::Snapshot {
                rows: 2,
                cols: 3,
                data: "aGk=".into()
            })
        );
        assert_eq!(
            parse(&event("output", r#"{"data":"aGk="}"#)),
            Some(TerminalMessage::Output { data: "aGk=".into() })
        );
        assert_eq!(
            parse(&event("resize", r#"{"rows":50,"cols":160}"#)),
            Some(TerminalMessage::Resize { rows: 50, cols: 160 })
        );
        assert_eq!(
            parse(&event("exit", r#"{"exit_code":null}"#)),
            Some(TerminalMessage::Exit { exit_code: None })
        );
        assert_eq!(
            parse(&event("exit", r#"{"exit_code":3}"#)),
            Some(TerminalMessage::Exit { exit_code: Some(3) })
        );
        assert_eq!(parse(&event("later", "{}")), None);
        assert_eq!(parse(&event("output", "not json")), None);
    }
}
