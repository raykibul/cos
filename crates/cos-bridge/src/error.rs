use serde::Serialize;

/// Why a call failed. Serializes to `{kind, message}`, which is what a
/// rejected `invoke` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct BridgeError {
    pub kind: ErrorKind,
    /// Meant for the user.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The socket is missing or nothing is listening on it.
    NotRunning,
    /// The daemon was reached but the exchange failed.
    Io,
    /// The call itself was refused, before reaching the daemon.
    BadRequest,
}

impl BridgeError {
    pub fn not_running(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotRunning,
            message: message.into(),
        }
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Io,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::BadRequest,
            message: message.into(),
        }
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
}
