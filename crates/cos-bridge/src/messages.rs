//! What the bridge hands the UI. The shapes are fixed by `docs/desktop.md`:
//! a `type` tag, camelCase for the bridge's own fields, and the daemon's JSON
//! passed through untouched.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where the bridge looks for the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub socket: PathBuf,
    pub home: PathBuf,
}

/// The daemon's answer to [`crate::Bridge::request`]. Any status is a normal
/// answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    /// The parsed JSON, a string if it wasn't JSON, or `None` if empty.
    pub body: Option<Value>,
}

/// A file from [`crate::Bridge::fetch_bytes`], such as an attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResponse {
    pub status: u16,
    pub content_type: String,
    /// The bytes, as base64.
    pub data: String,
}

/// One message of [`crate::Bridge::follow_events`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventMessage {
    /// `GET /v1/status`, sent on each (re)connect.
    Connected { status: Value },
    /// One logged event, in order.
    Event { event: Value },
    /// The daemon's log restarted below the last event delivered. The UI
    /// drops its events; the stream carries on from 0.
    LogReset,
    Disconnected {
        reason: String,
        /// Nothing answers: the socket is missing, or the host can't be
        /// reached.
        #[serde(rename = "notRunning")]
        not_running: bool,
        /// The daemon's refusal code (`device_pending`, …), or null when it
        /// didn't refuse.
        #[serde(default)]
        code: Option<String>,
    },
}

/// One message of [`crate::Bridge::attach_terminal`] or
/// [`crate::Bridge::attach_shell`]. `data` is base64, as the daemon sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalMessage {
    Snapshot {
        rows: u16,
        cols: u16,
        data: String,
    },
    Output {
        data: String,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    Exit {
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
    },
    Disconnected {
        reason: String,
        /// The daemon's refusal code, or null.
        #[serde(default)]
        code: Option<String>,
    },
}

/// One forwarded port of a [`crate::ForwardSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forward {
    /// The port on the host's loopback.
    pub remote_port: u16,
    /// The port on this Mac's loopback; another one when that was busy.
    pub local_port: u16,
    /// The port really listened on, when `local_port` is below 1024 and
    /// redirected there by pf (macOS lets only root listen below 1024).
    #[serde(default)]
    pub via: Option<u16>,
    /// This Mac's listener.
    pub state: ListenState,
    /// What the host said last: from the probe, then from each tunnel.
    pub remote: RemoteState,
    /// The daemon's code when it refused (`device_pending`, `too_old`, …).
    pub code: Option<String>,
    /// The latest problem, in words, or a note that the local port moved.
    pub message: Option<String>,
    /// Tunnels open now.
    pub connections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenState {
    Listening,
    /// No local port could be bound.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteState {
    Unknown,
    /// Something answered there.
    Listening,
    /// Nothing runs on that port on the host.
    NothingListening,
    /// The host refused this Mac (see `code`).
    Refused,
    /// The host couldn't be reached.
    Unreachable,
}

/// `forwards_list`'s answer: the forwards of the host in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardList {
    pub host: String,
    pub forwards: Vec<Forward>,
}

/// One message of [`crate::ForwardSet::follow`]: the whole list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ForwardMessage {
    Forwards { host: String, forwards: Vec<Forward> },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn to_json(value: impl Serialize) -> Value {
        serde_json::to_value(value).unwrap()
    }

    #[test]
    fn event_messages_match_the_contract() {
        let status = json!({"last_seq": 4, "started_at": "2026-10-22T09:00:00Z"});
        assert_eq!(
            to_json(EventMessage::Connected {
                status: status.clone()
            }),
            json!({"type": "connected", "status": status})
        );
        let event = json!({"seq": 3, "kind": "project.added", "data": {"project_id": 1}});
        assert_eq!(
            to_json(EventMessage::Event { event: event.clone() }),
            json!({"type": "event", "event": event})
        );
        assert_eq!(to_json(EventMessage::LogReset), json!({"type": "log_reset"}));
        assert_eq!(
            to_json(EventMessage::Disconnected {
                reason: "gone".into(),
                not_running: true,
                code: None,
            }),
            json!({"type": "disconnected", "reason": "gone", "notRunning": true, "code": null})
        );
        assert_eq!(
            to_json(EventMessage::Disconnected {
                reason: "wait".into(),
                not_running: false,
                code: Some("device_pending".into()),
            }),
            json!({"type": "disconnected", "reason": "wait", "notRunning": false, "code": "device_pending"})
        );
    }

    #[test]
    fn terminal_messages_match_the_contract() {
        assert_eq!(
            to_json(TerminalMessage::Snapshot {
                rows: 40,
                cols: 120,
                data: "aGk=".into()
            }),
            json!({"type": "snapshot", "rows": 40, "cols": 120, "data": "aGk="})
        );
        assert_eq!(
            to_json(TerminalMessage::Output { data: "aGk=".into() }),
            json!({"type": "output", "data": "aGk="})
        );
        assert_eq!(
            to_json(TerminalMessage::Resize { rows: 50, cols: 160 }),
            json!({"type": "resize", "rows": 50, "cols": 160})
        );
        assert_eq!(
            to_json(TerminalMessage::Exit { exit_code: None }),
            json!({"type": "exit", "exitCode": null})
        );
        assert_eq!(
            to_json(TerminalMessage::Exit { exit_code: Some(2) }),
            json!({"type": "exit", "exitCode": 2})
        );
        assert_eq!(
            to_json(TerminalMessage::Disconnected {
                reason: "gone".into(),
                code: None,
            }),
            json!({"type": "disconnected", "reason": "gone", "code": null})
        );
    }

    #[test]
    fn forward_messages_match_the_contract() {
        let forward = Forward {
            remote_port: 3343,
            local_port: 3344,
            via: None,
            state: ListenState::Listening,
            remote: RemoteState::NothingListening,
            code: None,
            message: Some("Nothing is listening on 3343 on ec2-box.".into()),
            connections: 0,
        };
        assert_eq!(
            to_json(ForwardMessage::Forwards {
                host: "ec2-box".into(),
                forwards: vec![forward]
            }),
            json!({"type": "forwards", "host": "ec2-box", "forwards": [{
                "remotePort": 3343, "localPort": 3344, "via": null, "state": "listening",
                "remote": "nothing_listening", "code": null,
                "message": "Nothing is listening on 3343 on ec2-box.", "connections": 0
            }]})
        );
    }

    #[test]
    fn responses_keep_the_body_as_is() {
        assert_eq!(
            to_json(Response {
                status: 204,
                body: None
            }),
            json!({"status": 204, "body": null})
        );
        assert_eq!(
            to_json(Response {
                status: 200,
                body: Some(json!({"base_branch": "main"}))
            }),
            json!({"status": 200, "body": {"base_branch": "main"}})
        );
    }
}
