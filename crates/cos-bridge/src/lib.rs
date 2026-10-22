//! The desktop app's link to the cos daemon, without any Tauri code.
//!
//! [`Bridge`] sends requests to the daemon's HTTP API over its Unix socket,
//! follows the event log ([`Bridge::follow_events`]) and attaches to agents'
//! terminals ([`Bridge::attach_terminal`]). Both streams reconnect by
//! themselves until their [`StreamHandle`] is closed. The contract with the
//! UI is in `docs/desktop.md`.

mod error;
mod events;
mod http;
mod messages;
mod sse;
mod stream;
mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use error::{BridgeError, ErrorKind};
pub use messages::{DaemonInfo, EventMessage, Response, TerminalMessage};
pub use stream::{Sink, StreamHandle};

/// How long to wait between reconnects: `initial`, doubling up to `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    /// 0.5 s doubling to 5 s, as `docs/desktop.md` says.
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(500),
            max: Duration::from_secs(5),
        }
    }
}

/// Talks to one daemon through its socket. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Bridge {
    socket: PathBuf,
    backoff: Backoff,
}

impl Bridge {
    /// A bridge to the daemon listening on `socket`. Nothing connects until
    /// the first request or stream.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            backoff: Backoff::default(),
        }
    }

    /// A bridge to the daemon of [`cos_home`].
    pub fn from_env() -> Option<Self> {
        cos_home().map(|home| Self::new(home.join("cos.sock")))
    }

    /// Uses another reconnect delay; tests make it short.
    #[must_use]
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Where the bridge looks for the daemon.
    pub fn info(&self) -> DaemonInfo {
        DaemonInfo {
            socket: self.socket.clone(),
            home: self.socket.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }
}

/// cos's home folder: `$COS_HOME` if set, else `~/.cos`. `None` if neither
/// `COS_HOME` nor `HOME` is set.
pub fn cos_home() -> Option<PathBuf> {
    home_from(std::env::var_os("COS_HOME"), std::env::var_os("HOME"))
}

fn home_from(cos_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    match cos_home.filter(|value| !value.is_empty()) {
        Some(root) => Some(PathBuf::from(root)),
        None => home
            .filter(|value| !value.is_empty())
            .map(|home| Path::new(&home).join(".cos")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_home_prefers_the_variable() {
        assert_eq!(
            home_from(Some("/tmp/c".into()), Some("/Users/me".into())),
            Some(PathBuf::from("/tmp/c"))
        );
        assert_eq!(
            home_from(None, Some("/Users/me".into())),
            Some(PathBuf::from("/Users/me/.cos"))
        );
        assert_eq!(
            home_from(Some("".into()), Some("/Users/me".into())),
            Some(PathBuf::from("/Users/me/.cos"))
        );
        assert_eq!(home_from(None, None), None);
    }

    #[test]
    fn info_names_the_socket_and_its_folder() {
        let info = Bridge::new("/tmp/home/cos.sock").info();
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            serde_json::json!({"socket": "/tmp/home/cos.sock", "home": "/tmp/home"})
        );
    }
}
