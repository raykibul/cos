//! HTTP/1.1 to the daemon, over its Unix socket or TCP, one connection per
//! request.

use std::io::ErrorKind as IoKind;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::http::uri::PathAndQuery;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, UnixStream};
use tokio::task::AbortHandle;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::{Bridge, BridgeError, FileResponse, Response, Target};

/// A connection to the daemon, whichever way it was made.
trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

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
        let content_type = body.as_ref().map(|_| "application/json");
        let (response, _connection) = self.send_typed(method, path, content_type, body).await?;
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

    /// Uploads a file: `POST path` with the bytes `data` (base64) as the
    /// body, of `content_type`. Returns the daemon's answer, whatever the
    /// status, as [`Self::request`] does. See "Attachments" in
    /// `docs/agents-api.md`.
    pub async fn upload(&self, path: &str, content_type: &str, data: &str) -> Result<Response, BridgeError> {
        check_path(path)?;
        let bytes = BASE64
            .decode(data.trim())
            .map_err(|error| BridgeError::bad_request(format!("`data` isn't valid base64: {error}")))?;
        let content_type = if content_type.trim().is_empty() {
            "application/octet-stream"
        } else {
            content_type
        };
        let (response, _connection) = self
            .send_typed(Method::POST, path, Some(content_type), Some(bytes))
            .await?;
        let status = response.status().as_u16();
        let bytes = read_all(response).await?;
        Ok(Response {
            status,
            body: parse_body(&bytes),
        })
    }

    /// `GET path` for a file, such as an attachment to show: its bytes as
    /// base64 and its type. A non-2xx answer is an error with the daemon's
    /// message.
    pub async fn fetch_bytes(&self, path: &str) -> Result<FileResponse, BridgeError> {
        check_path(path)?;
        let (response, _connection) = self.send(Method::GET, path, None).await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let bytes = read_all(response).await?;
        if !(200..300).contains(&status) {
            return Err(answer_error(status, &bytes));
        }
        Ok(FileResponse {
            status,
            content_type,
            data: BASE64.encode(&bytes),
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
        let content_type = body.as_ref().map(|_| "application/json");
        self.send_typed(method, path, content_type, body).await
    }

    /// [`Self::send`] with the body's type given.
    async fn send_typed(
        &self,
        method: Method,
        path: &str,
        content_type: Option<&str>,
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
            .header(hyper::header::HOST, self.target.host_header());
        if let Some(content_type) = content_type {
            request = request.header(hyper::header::CONTENT_TYPE, content_type);
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

    async fn connect(&self) -> Result<Box<dyn Stream>, BridgeError> {
        match &self.target {
            Target::Unix(socket) => match UnixStream::connect(socket).await {
                Ok(stream) => Ok(Box::new(stream)),
                Err(error) if matches!(error.kind(), IoKind::NotFound | IoKind::ConnectionRefused) => {
                    Err(BridgeError::not_running(format!(
                        "the cos daemon is not running (no answer on {})",
                        socket.display()
                    )))
                }
                Err(error) => Err(BridgeError::io(format!(
                    "could not connect to {}: {error}",
                    socket.display()
                ))),
            },
            // Any failure to connect means the host can't be reached: it's
            // off, asleep, off the tailnet, or not listening (`cos remote on`).
            Target::Tcp(address) => {
                let connect = TcpStream::connect(address.as_str());
                match tokio::time::timeout(self.connect_timeout, connect).await {
                    Ok(Ok(stream)) => {
                        let _ = stream.set_nodelay(true);
                        Ok(Box::new(stream))
                    }
                    Ok(Err(error)) => Err(BridgeError::not_running(format!(
                        "could not reach {address}: {error}"
                    ))),
                    Err(_) => Err(BridgeError::not_running(format!(
                        "could not reach {address}: no answer in {} s",
                        self.connect_timeout.as_secs()
                    ))),
                }
            }
        }
    }
}

async fn read_all(response: hyper::Response<Incoming>) -> Result<Bytes, BridgeError> {
    Ok(response
        .into_body()
        .collect()
        .await
        .map_err(|error| BridgeError::io(format!("the daemon's answer was cut off: {error}")))?
        .to_bytes())
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
fn error_message(status: u16, body: Option<&Value>) -> String {
    body.and_then(|value| value.get("error")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("the daemon answered {status}"))
}

/// A failed answer as an error: a refusal if the body has a `code` (see
/// "Refusals" in `docs/daemon.md`), else the daemon's message.
pub(crate) fn answer_error(status: u16, bytes: &[u8]) -> BridgeError {
    error_from(status, serde_json::from_slice::<Value>(bytes).ok().as_ref())
}

/// [`answer_error`] for a body that's already parsed.
pub(crate) fn error_from(status: u16, body: Option<&Value>) -> BridgeError {
    let message = error_message(status, body);
    match body.and_then(|value| value.get("code")?.as_str()) {
        Some(code) => BridgeError::refused(code, message),
        None => BridgeError::io(message),
    }
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
        assert_eq!(
            answer_error(404, br#"{"error":"no agent 9"}"#),
            BridgeError::io("no agent 9")
        );
        assert_eq!(
            answer_error(500, b"oops"),
            BridgeError::io("the daemon answered 500")
        );
    }

    #[test]
    fn a_body_with_a_code_is_a_refusal() {
        let error = answer_error(
            403,
            br#"{"error":"this device (laptop) is waiting for approval","code":"device_pending"}"#,
        );
        assert_eq!(
            error,
            BridgeError::refused("device_pending", "this device (laptop) is waiting for approval")
        );
        let error = answer_error(503, br#"{"code":"tailscale_unavailable"}"#);
        assert_eq!(error.code.as_deref(), Some("tailscale_unavailable"));
        assert_eq!(error.message, "the daemon answered 503");
    }
}
