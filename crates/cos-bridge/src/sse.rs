//! Reads server-sent events from a response body.

use std::time::Duration;

use http_body_util::BodyExt;
use hyper::body::Incoming;

use crate::BridgeError;
use crate::http::Connection;

/// One server-sent event with data. Comments and keep-alives are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// The `event:` name, or `message` when there is none.
    pub event: String,
    pub data: String,
}

pub(crate) struct SseReader {
    body: Incoming,
    buffer: Vec<u8>,
    /// How long to wait for the next bytes, keep-alives included, before
    /// taking the stream as lost. `None` waits for ever.
    idle: Option<Duration>,
    _connection: Connection,
}

impl SseReader {
    pub(crate) fn new(body: Incoming, connection: Connection, idle: Option<Duration>) -> Self {
        Self {
            body,
            buffer: Vec::new(),
            idle,
            _connection: connection,
        }
    }

    /// The next event, or `None` when the daemon ends the stream. An error
    /// when nothing at all arrives for the `idle` time.
    pub(crate) async fn next(&mut self) -> Result<Option<SseEvent>, BridgeError> {
        loop {
            while let Some(end) = find(&self.buffer, b"\n\n") {
                let message: Vec<u8> = self.buffer.drain(..end + 2).collect();
                if let Some(event) = parse(&message) {
                    return Ok(Some(event));
                }
            }
            let frame = match self.idle {
                Some(idle) => tokio::time::timeout(idle, self.body.frame()).await.map_err(|_| {
                    BridgeError::io(format!(
                        "no word from the daemon in {} s; reconnecting",
                        idle.as_secs()
                    ))
                })?,
                None => self.body.frame().await,
            };
            match frame {
                None => return Ok(None),
                Some(Err(error)) => return Err(BridgeError::io(format!("the stream broke: {error}"))),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.buffer.extend_from_slice(&data);
                    }
                }
            }
        }
    }
}

/// One message (the text up to a blank line), or `None` if it has no data.
pub(crate) fn parse(message: &[u8]) -> Option<SseEvent> {
    let text = String::from_utf8_lossy(message);
    let mut event = None;
    let mut data = Vec::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.to_owned()),
            "data" => data.push(value),
            _ => {}
        }
    }
    (!data.is_empty()).then(|| SseEvent {
        event: event.unwrap_or_else(|| "message".to_owned()),
        data: data.join("\n"),
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_and_skips_comments() {
        assert_eq!(
            parse(b"id: 3\ndata: {\"a\":1}\n\n"),
            Some(SseEvent {
                event: "message".into(),
                data: "{\"a\":1}".into()
            })
        );
        assert_eq!(
            parse(b"event: output\ndata:x\ndata: y\n\n"),
            Some(SseEvent {
                event: "output".into(),
                data: "x\ny".into()
            })
        );
        assert_eq!(parse(b":\n\n"), None);
        assert_eq!(parse(b"event: ping\n\n"), None);
    }
}
