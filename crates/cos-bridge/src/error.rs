use serde::Serialize;

/// Why a call failed. Serializes to `{kind, message}`, which is what a
/// rejected `invoke` carries, plus `code` for a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct BridgeError {
    pub kind: ErrorKind,
    /// Meant for the user.
    pub message: String,
    /// The daemon's `code` when it refused this client, such as
    /// `device_pending` (see "Refusals" in `docs/daemon.md`). Only streams
    /// end with one: a request hands the refusal back as a response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The socket is missing or nothing is listening on it. For a host over
    /// TCP: it couldn't be reached at all.
    NotRunning,
    /// The daemon was reached but the exchange failed.
    Io,
    /// The call itself was refused, before reaching the daemon.
    BadRequest,
}

impl BridgeError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: None,
        }
    }

    pub fn not_running(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotRunning, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadRequest, message)
    }

    /// The daemon answered, but refused this client with `code`.
    pub fn refused(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.into()),
            ..Self::io(message)
        }
    }

    /// Whether the daemon refused this client, rather than being away.
    pub fn is_refusal(&self) -> bool {
        self.code.is_some()
    }

    pub fn is_not_running(&self) -> bool {
        self.kind == ErrorKind::NotRunning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let error = BridgeError::not_running("the cos daemon is not running");
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"kind": "not_running", "message": "the cos daemon is not running"})
        );
        assert_eq!(
            serde_json::to_value(BridgeError::bad_request("x")).unwrap()["kind"],
            "bad_request"
        );
        assert_eq!(serde_json::to_value(BridgeError::io("x")).unwrap()["kind"], "io");
    }

    #[test]
    fn a_refusal_carries_its_code() {
        let error = BridgeError::refused("device_pending", "wait for approval");
        assert!(error.is_refusal());
        assert!(!BridgeError::io("x").is_refusal());
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"kind": "io", "message": "wait for approval", "code": "device_pending"})
        );
    }
}
