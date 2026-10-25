//! Following the daemon's event log, across drops and restarts.

use http_body_util::BodyExt;
use hyper::Method;
use serde_json::Value;

use crate::http::{answer_error, error_from};
use crate::sse::SseReader;
use crate::stream::{self, Outbox, Retry};
use crate::{Bridge, BridgeError, EventMessage, Sink, StreamHandle};

impl Bridge {
    /// Follows the event log after `after` (from the start if `None`) until
    /// the handle is closed. Each (re)connect sends `connected` with the
    /// daemon's status, then the events after the last one delivered. A drop
    /// sends `disconnected` and retries with backoff. If the daemon's log is
    /// now shorter than what was delivered, it sends `log_reset` and starts
    /// again from 0. A refusal (a remote daemon answering with a `code`,
    /// such as `device_pending`) is a `disconnected` with that code, and
    /// waits the longest backoff before asking again.
    ///
    /// Must be called inside a Tokio runtime.
    pub fn follow_events(&self, after: Option<u64>, sink: impl Sink<EventMessage>) -> StreamHandle {
        let bridge = self.clone();
        stream::spawn(sink, move |outbox| bridge.follow(after.unwrap_or(0), outbox))
    }

    async fn follow<S: Sink<EventMessage>>(self, mut cursor: u64, mut outbox: Outbox<S>) {
        let mut retry = Retry::new(self.backoff);
        // The last `disconnected` sent since the last connect, so retries
        // that fail the same way don't repeat it.
        let mut last_drop: Option<EventMessage> = None;
        loop {
            let mut connected = false;
            let Some(error) = self.follow_once(&mut cursor, &mut outbox, &mut connected).await else {
                return;
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
            let message = EventMessage::Disconnected {
                not_running: error.is_not_running(),
                reason: error.message,
                code: error.code,
            };
            if last_drop.as_ref() != Some(&message) {
                if !outbox.send(message.clone()) {
                    return;
                }
                last_drop = Some(message);
            }
            tokio::time::sleep(delay).await;
        }
    }

    /// One connection. Returns why it ended, or `None` if the sink is gone.
    async fn follow_once<S: Sink<EventMessage>>(
        &self,
        cursor: &mut u64,
        outbox: &mut Outbox<S>,
        connected: &mut bool,
    ) -> Option<BridgeError> {
        let status = match self.status().await {
            Ok(status) => status,
            Err(error) => return Some(error),
        };
        let last_seq = status.get("last_seq").and_then(Value::as_u64);
        *connected = true;
        if !outbox.send(EventMessage::Connected { status }) {
            return None;
        }
        if last_seq.is_some_and(|last| last < *cursor) {
            *cursor = 0;
            if !outbox.send(EventMessage::LogReset) {
                return None;
            }
        }

        let path = format!("/v1/events/stream?after={cursor}");
        let mut reader = match self.open_stream(&path).await {
            Ok(reader) => reader,
            Err(error) => return Some(error),
        };
        loop {
            let event = match reader.next().await {
                Ok(Some(event)) => event,
                Ok(None) => return Some(BridgeError::io("the daemon closed the event stream")),
                Err(error) => return Some(error),
            };
            let Ok(event) = serde_json::from_str::<Value>(&event.data) else {
                continue;
            };
            // Skip anything without a sequence number or already delivered.
            let Some(seq) = event.get("seq").and_then(Value::as_u64) else {
                continue;
            };
            if seq <= *cursor {
                continue;
            }
            *cursor = seq;
            if !outbox.send(EventMessage::Event { event }) {
                return None;
            }
        }
    }

    async fn status(&self) -> Result<Value, BridgeError> {
        let response = self.request("GET", "/v1/status", None).await?;
        match response.body {
            Some(status @ Value::Object(_)) if response.status == 200 => Ok(status),
            body if !(200..300).contains(&response.status) => Err(error_from(response.status, body.as_ref())),
            _ => Err(BridgeError::io(format!(
                "unexpected answer to /v1/status ({})",
                response.status
            ))),
        }
    }

    /// Opens a server-sent events stream, turning an error status into an
    /// error with the daemon's message (and code, for a refusal).
    pub(crate) async fn open_stream(&self, path: &str) -> Result<SseReader, BridgeError> {
        let (response, connection) = self.send(Method::GET, path, None).await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = response
                .into_body()
                .collect()
                .await
                .map(|body| body.to_bytes())
                .unwrap_or_default();
            return Err(answer_error(status.as_u16(), &bytes));
        }
        // This Mac's socket can't be left half open, so only TCP needs a limit.
        let idle = self.target().is_remote().then_some(self.read_timeout);
        Ok(SseReader::new(response.into_body(), connection, idle))
    }
}
