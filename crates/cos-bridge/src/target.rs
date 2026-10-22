//! Where a bridge finds its daemon: the Unix socket on this Mac, or another
//! Mac's tailnet listener over TCP (see "Remote access" in `docs/daemon.md`).

use std::fmt;
use std::path::PathBuf;

use crate::BridgeError;

/// The port a daemon's tailnet listener uses unless told otherwise.
pub const DEFAULT_PORT: u16 = 7700;

/// A daemon to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// This Mac's daemon, through its socket.
    Unix(PathBuf),
    /// A daemon's tailnet listener, as `host:port` (`my-mac:7700`,
    /// `100.101.102.103:7700`, `[fd7a:115c:a1e0::1]:7700`). Plain HTTP:
    /// WireGuard already encrypts the traffic.
    Tcp(String),
}

impl Target {
    /// A TCP target from what a person typed, as the CLI's `--host` reads
    /// it: the port defaults to 7700, and a bare IPv6 address gets brackets.
    pub fn tcp(address: &str) -> Result<Self, BridgeError> {
        parse_address(address).map(Self::Tcp)
    }

    /// The `Host` header: the address over TCP, a placeholder on the socket.
    pub(crate) fn host_header(&self) -> &str {
        match self {
            Self::Unix(_) => "cos",
            Self::Tcp(address) => address,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Tcp(_))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(socket) => write!(f, "{}", socket.display()),
            Self::Tcp(address) => f.write_str(address),
        }
    }
}

/// `host[:port]` → `host:port`. Refuses anything that isn't a host name or
/// an IP address, such as a URL, a path or an empty string.
pub fn parse_address(address: &str) -> Result<String, BridgeError> {
    let address = address.trim();
    let refuse = |why: &str| {
        BridgeError::bad_request(format!(
            "{address:?} is not a host address ({why}). Use a name or IP, like my-mac, my-mac:7700 or 100.101.102.103"
        ))
    };
    if address.is_empty() {
        return Err(refuse("it's empty"));
    }
    if address.contains("://") || address.contains('/') {
        return Err(refuse("leave out http:// and any path"));
    }
    if address.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refuse("it has spaces"));
    }
    let full = if has_port(address) {
        address.to_owned()
    } else if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{DEFAULT_PORT}")
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    let (host, port) = full.rsplit_once(':').ok_or_else(|| refuse("no host"))?;
    let port: u16 = port.parse().map_err(|_| refuse("bad port"))?;
    if port == 0 {
        return Err(refuse("port 0"));
    }
    let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));
    let valid = match bare {
        Some(ip) => ip.parse::<std::net::Ipv6Addr>().is_ok(),
        None => {
            !host.is_empty()
                && !host.contains(['[', ']', ':'])
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
        }
    };
    if !valid {
        return Err(refuse("not a name or IP address"));
    }
    Ok(full)
}

/// Whether `host` ends in `:port`, allowing for bracketed IPv6.
fn has_port(host: &str) -> bool {
    match host.rsplit_once(':') {
        Some((before, port)) => {
            !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit())
                && (!before.contains(':') || (before.starts_with('[') && before.ends_with(']')))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_defaults_to_7700() {
        for (typed, expected) in [
            ("my-mac", "my-mac:7700"),
            ("my-mac:7701", "my-mac:7701"),
            ("  my-mac.tail1234.ts.net ", "my-mac.tail1234.ts.net:7700"),
            ("100.101.102.103", "100.101.102.103:7700"),
            ("100.101.102.103:7700", "100.101.102.103:7700"),
            ("fd7a:115c:a1e0::1", "[fd7a:115c:a1e0::1]:7700"),
            ("[fd7a:115c:a1e0::1]", "[fd7a:115c:a1e0::1]:7700"),
            ("[fd7a:115c:a1e0::1]:7701", "[fd7a:115c:a1e0::1]:7701"),
        ] {
            assert_eq!(parse_address(typed).unwrap(), expected, "{typed}");
        }
    }

    #[test]
    fn refuses_what_isnt_an_address() {
        for typed in [
            "",
            "   ",
            "http://my-mac:7700",
            "my-mac/v1",
            "my mac",
            "my-mac:0",
            "my-mac:99999",
            "my-mac:port",
            "[not-ip]:7700",
            "[fd7a::1",
            "a:b:c:7700x",
        ] {
            let error = parse_address(typed).unwrap_err();
            assert_eq!(error.kind, crate::ErrorKind::BadRequest, "{typed}");
        }
    }

    #[test]
    fn targets_name_themselves() {
        let tcp = Target::tcp("my-mac").unwrap();
        assert_eq!(tcp, Target::Tcp("my-mac:7700".into()));
        assert!(tcp.is_remote());
        assert_eq!(tcp.host_header(), "my-mac:7700");
        assert_eq!(tcp.to_string(), "my-mac:7700");
        let unix = Target::Unix("/tmp/cos.sock".into());
        assert!(!unix.is_remote());
        assert_eq!(unix.host_header(), "cos");
        assert_eq!(unix.to_string(), "/tmp/cos.sock");
    }
}
