//! HTTP/1.1 over the daemon's Unix socket, one connection per request.

use std::io::ErrorKind as IoKind;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::http::uri::PathAndQuery;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::UnixStream;
use tokio::task::AbortHandle;

use crate::{Bridge, BridgeError, Response};

/// The methods `daemon_request` accepts. `PATCH` is here because renaming a
/// workspace uses it.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Keeps a request's connection alive; dropping it closes the connection.
pub(crate) struct Connection {
    driver: AbortHandle,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

impl Bridge {
    /// Sends `method path` with `body` as JSON, and returns the status and
    /// body, whatever the status. `path` must be under `/v1/`; anything else
    /// is refused without reaching the daemon.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Response, BridgeError> {
        let method = parse_method(method)?;
        check_path(path)?;
        let body = match body {
            Some(body) => {
                Some(serde_json::to_vec(&body).map_err(|error| BridgeError::bad_request(error.to_string()))?)
            }
            None => None,
        };
        let (response, _connection) = self.send(method, path, body).await?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| BridgeError::io(format!("the daemon's answer was cut off: {error}")))?
            .to_bytes();
        Ok(Response {
            status,
            body: parse_body(&bytes),
        })
    }

    /// Opens a connection and sends one request. The caller keeps the
    /// [`Connection`] for as long as it reads the body.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(hyper::Response<Incoming>, Connection), BridgeError> {
        let stream = self.connect().await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| BridgeError::io(format!("could not talk to the daemon: {error}")))?;
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        let connection = Connection {
            driver: driver.abort_handle(),
        };

        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, "cos");
        if body.is_some() {
            request = request.header(hyper::header::CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|error| BridgeError::bad_request(error.to_string()))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|error| BridgeError::io(format!("the daemon didn't answer: {error}")))?;
        Ok((response, connection))
    }

    async fn connect(&self) -> Result<UnixStream, BridgeError> {
        UnixStream::connect(&self.socket).await.map_err(|error| {
            if matches!(error.kind(), IoKind::NotFound | IoKind::ConnectionRefused) {
                BridgeError::not_running(format!(
                    "the cos daemon is not running (no answer on {})",
                    self.socket.display()
                ))
            } else {
                BridgeError::io(format!("could not connect to {}: {error}", self.socket.display()))
            }
        })
    }
}

fn parse_method(method: &str) -> Result<Method, BridgeError> {
    let upper = method.to_ascii_uppercase();
    if !METHODS.contains(&upper.as_str()) {
        return Err(BridgeError::bad_request(format!(
            "method {method} is not allowed"
        )));
    }
    Method::from_bytes(upper.as_bytes()).map_err(|error| BridgeError::bad_request(error.to_string()))
}

/// Only the daemon's versioned API may be reached: `/v1/…`, with no `..`
/// segments and nothing hyper wouldn't send as is.
pub(crate) fn check_path(path: &str) -> Result<(), BridgeError> {
    let refuse = || {
        BridgeError::bad_request(format!(
            "{path} is not a daemon API path (it must start with /v1/)"
        ))
    };
    if !path.starts_with("/v1/") {
        return Err(refuse());
    }
    let parsed: PathAndQuery = path.parse().map_err(|_| refuse())?;
    if parsed.as_str() != path
        || parsed
            .path()
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
    {
        return Err(refuse());
    }
    Ok(())
}

/// JSON if it parses, else the text, or `None` when empty.
fn parse_body(bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    Some(
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned())),
    )
}

/// The `error` message of a daemon's error body, or a plain description.
pub(crate) fn error_message(status: u16, bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("the daemon answered {status}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_api_paths_only() {
        for path in ["/v1/status", "/v1/events?after=3", "/v1/agents/7/keys"] {
            assert!(check_path(path).is_ok(), "{path}");
        }
        for path in [
            "/status",
            "v1/status",
            "/v1",
            "/v2/status",
            "/v1/../secret",
            "/v1/./status",
            "/v1/a b",
            "/v1/x#frag",
            "http://cos/v1/status",
            "",
        ] {
            let error = check_path(path).unwrap_err();
            assert_eq!(error.kind, crate::ErrorKind::BadRequest, "{path}");
        }
    }

    #[test]
    fn accepts_the_api_methods() {
        assert_eq!(parse_method("get").unwrap(), Method::GET);
        assert_eq!(parse_method("PATCH").unwrap(), Method::PATCH);
        assert!(parse_method("TRACE").is_err());
        assert!(parse_method("").is_err());
    }

    #[test]
    fn bodies_are_json_text_or_nothing() {
        assert_eq!(parse_body(b""), None);
        assert_eq!(parse_body(br#"{"a":1}"#), Some(json!({"a": 1})));
        assert_eq!(parse_body(b"plain text"), Some(json!("plain text")));
    }

    #[test]
    fn error_messages_come_from_the_body() {
        assert_eq!(error_message(404, br#"{"error":"no agent 9"}"#), "no agent 9");
        assert_eq!(error_message(500, b"oops"), "the daemon answered 500");
    }
}
