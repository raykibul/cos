//! The desktop app's link to a cos daemon, without any Tauri code.
//!
//! [`Bridge`] sends requests to the daemon's HTTP API, over this Mac's Unix
//! socket or another Mac's tailnet listener ([`Target`]), follows the event
//! log ([`Bridge::follow_events`]) and attaches to agents' terminals
//! ([`Bridge::attach_terminal`]). Both streams reconnect by themselves until
//! their [`StreamHandle`] is closed. The contract with the UI is in
//! `docs/desktop.md`.

mod error;
mod events;
mod http;
mod messages;
mod sse;
mod stream;
mod target;
mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use error::{BridgeError, ErrorKind};
pub use messages::{DaemonInfo, EventMessage, FileResponse, Response, TerminalMessage};
pub use stream::{Sink, StreamHandle};
pub use target::{DEFAULT_PORT, Target, parse_address};

/// How long to wait between reconnects: `initial`, doubling up to `max`.
/// A refusal waits `max` straight away.
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

/// How long connecting to a host over TCP may take. A Mac that's asleep or
/// off the tailnet otherwise hangs for over a minute.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Over TCP, a stream that brings nothing for this long is taken as lost and
/// reopened. The daemon sends a keep-alive every 15 s, so only a host that
/// vanished goes this quiet: the Mac slept, the network changed, a tailnet
/// path broke. Without it such a connection can hang open for hours, and
/// the app shows nothing new while the agent carries on.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// Talks to one daemon. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Bridge {
    target: Target,
    backoff: Backoff,
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl Bridge {
    /// A bridge to the daemon listening on `socket`. Nothing connects until
    /// the first request or stream.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self::to(Target::Unix(socket.into()))
    }

    /// A bridge to the daemon at `target`.
    pub fn to(target: Target) -> Self {
        Self {
            target,
            backoff: Backoff::default(),
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
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

    /// Gives up on a quiet stream over TCP sooner; tests make it short.
    #[must_use]
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// The same bridge, aimed at another daemon.
    #[must_use]
    pub fn retarget(&self, target: Target) -> Self {
        Self {
            target,
            ..self.clone()
        }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The socket, for a bridge to this Mac's daemon.
    pub fn socket(&self) -> Option<&Path> {
        match &self.target {
            Target::Unix(socket) => Some(socket),
            Target::Tcp(_) => None,
        }
    }

    /// Where the bridge looks for this Mac's daemon; `None` for a host over
    /// TCP.
    pub fn info(&self) -> Option<DaemonInfo> {
        self.socket().map(|socket| DaemonInfo {
            socket: socket.to_path_buf(),
            home: socket.parent().map(Path::to_path_buf).unwrap_or_default(),
        })
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
        let info = Bridge::new("/tmp/home/cos.sock").info().unwrap();
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            serde_json::json!({"socket": "/tmp/home/cos.sock", "home": "/tmp/home"})
        );
    }

    #[test]
    fn retargeting_keeps_the_backoff() {
        let backoff = Backoff {
            initial: Duration::from_millis(1),
            max: Duration::from_millis(2),
        };
        let local = Bridge::new("/tmp/cos.sock").with_backoff(backoff);
        let remote = local.retarget(Target::tcp("my-mac").unwrap());
        assert_eq!(remote.backoff, backoff);
        assert_eq!(remote.target(), &Target::Tcp("my-mac:7700".into()));
        assert_eq!(remote.socket(), None);
        assert_eq!(remote.info(), None);
        assert_eq!(local.socket(), Some(Path::new("/tmp/cos.sock")));
    }
}
