//! The `tailscale` CLI: who this machine is on the tailnet
//! (`tailscale status --json`) and who is behind a tailnet address
//! (`tailscale whois --json`). The daemon uses it to bind its tailnet
//! listener and to check every remote client.
//!
//! The JSON isn't a documented contract, so parsing is lenient: unknown fields
//! are ignored and only the few fields cos needs are required. The fixtures in
//! `crates/cos/tests/fixtures` are real output from Tailscale 1.102.3 on
//! macOS, with personal data replaced.

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use serde::Deserialize;

/// The Mac app's binary doubles as the CLI when it isn't on `PATH`.
const MAC_APP_CLI: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
/// A `tailscale` call that takes longer than this has hung.
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the daemon asks Tailscale. [`TailscaleCli`] runs the real CLI; tests
/// use a fake.
pub(crate) trait Tailscale: Send + Sync {
    /// This machine's node and its owner.
    fn local(&self) -> BoxFuture<'_, Result<LocalNode, TailscaleError>>;
    /// The node behind a tailnet address, or `None` if Tailscale doesn't
    /// know it (not a tailnet address, or not a peer of this machine).
    fn whois(&self, addr: SocketAddr) -> BoxFuture<'_, Result<Option<Peer>, TailscaleError>>;
}

/// This machine on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalNode {
    /// Stable node ID, such as `nAbC123CNTRL`.
    pub node_id: String,
    /// MagicDNS name, such as `my-mac`.
    pub name: String,
    pub os: String,
    /// The owner: the Tailscale user this machine is logged in as.
    pub user_id: i64,
    pub login_name: String,
    /// A tagged node is owned by its tags, not by a user.
    pub tagged: bool,
    /// The machine's tailnet addresses (100.64.0.0/10 and fd7a:115c:a1e0::/48).
    pub ips: Vec<IpAddr>,
}

/// A node on the other end of a connection, from `tailscale whois`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Peer {
    pub node_id: String,
    pub name: String,
    pub hostname: String,
    /// Empty when Tailscale doesn't report it (it omits it for this machine).
    pub os: String,
    pub user_id: i64,
    pub login_name: String,
    pub tagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TailscaleError {
    #[error("Tailscale is not installed (no `tailscale` on PATH or at {MAC_APP_CLI})")]
    NotInstalled,
    #[error("Tailscale is not running ({0})")]
    NotRunning(String),
    #[error("Tailscale is not logged in")]
    NeedsLogin,
    #[error("`tailscale {command}` failed: {message}")]
    Failed { command: String, message: String },
    #[error("unexpected output from `tailscale {command}`: {reason}")]
    Unexpected { command: String, reason: String },
}

/// Whether `ip` is in Tailscale's address ranges: 100.64.0.0/10 (CGNAT) or
/// fd7a:115c:a1e0::/48.
pub(crate) fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            a == 100 && b & 0xc0 == 64
        }
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

// ---- Parsing ----

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StatusJson {
    #[serde(default)]
    backend_state: String,
    #[serde(rename = "Self")]
    self_node: Option<StatusNode>,
    #[serde(default)]
    user: Option<HashMap<String, UserJson>>,
}

#[derive(Deserialize)]
struct StatusNode {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "UserID", default)]
    user_id: i64,
    #[serde(rename = "TailscaleIPs", default)]
    tailscale_ips: Option<Vec<String>>,
    #[serde(rename = "Tags", default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct UserJson {
    #[serde(rename = "ID", default)]
    id: i64,
    #[serde(rename = "LoginName", default)]
    login_name: String,
}

#[derive(Deserialize)]
struct WhoisJson {
    #[serde(rename = "Node")]
    node: WhoisNode,
    #[serde(rename = "UserProfile")]
    user_profile: Option<UserJson>,
}

#[derive(Deserialize)]
struct WhoisNode {
    #[serde(rename = "StableID", default)]
    stable_id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "ComputedName", default)]
    computed_name: String,
    #[serde(rename = "User", default)]
    user: i64,
    #[serde(rename = "Hostinfo", default)]
    hostinfo: Option<Hostinfo>,
    #[serde(rename = "Tags", default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Hostinfo {
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "Hostname", default)]
    hostname: String,
}

/// Parses `tailscale status --json` into this machine's node.
pub(crate) fn parse_status(json: &[u8]) -> Result<LocalNode, TailscaleError> {
    let unexpected = |reason: String| TailscaleError::Unexpected {
        command: "status".into(),
        reason,
    };
    let status: StatusJson = serde_json::from_slice(json).map_err(|error| unexpected(error.to_string()))?;
    match status.backend_state.as_str() {
        "Running" => {}
        "NeedsLogin" => return Err(TailscaleError::NeedsLogin),
        "NeedsMachineAuth" => {
            return Err(TailscaleError::NotRunning(
                "this machine is waiting for a tailnet admin to approve it".into(),
            ));
        }
        "Stopped" => return Err(TailscaleError::NotRunning("it is switched off".into())),
        other => return Err(TailscaleError::NotRunning(format!("its state is {other:?}"))),
    }
    let node = status
        .self_node
        .filter(|node| !node.id.is_empty())
        .ok_or_else(|| unexpected("no Self node".into()))?;
    let login_name = status
        .user
        .unwrap_or_default()
        .into_values()
        .find(|user| user.id == node.user_id)
        .map(|user| user.login_name)
        .unwrap_or_default();
    let ips = node
        .tailscale_ips
        .unwrap_or_default()
        .iter()
        .filter_map(|ip| ip.parse().ok())
        .filter(|ip| is_tailnet_ip(*ip))
        .collect();
    Ok(LocalNode {
        name: short_name(&node.dns_name).unwrap_or(node.host_name),
        node_id: node.id,
        os: node.os,
        user_id: node.user_id,
        login_name,
        tagged: node.tags.is_some_and(|tags| !tags.is_empty()),
        ips,
    })
}

/// Parses `tailscale whois --json` into the peer it describes.
pub(crate) fn parse_whois(json: &[u8]) -> Result<Peer, TailscaleError> {
    let whois: WhoisJson = serde_json::from_slice(json).map_err(|error| TailscaleError::Unexpected {
        command: "whois".into(),
        reason: error.to_string(),
    })?;
    let node = whois.node;
    if node.stable_id.is_empty() {
        return Err(TailscaleError::Unexpected {
            command: "whois".into(),
            reason: "no Node.StableID".into(),
        });
    }
    let hostinfo = node.hostinfo.unwrap_or(Hostinfo {
        os: String::new(),
        hostname: String::new(),
    });
    let user = whois.user_profile.unwrap_or(UserJson {
        id: node.user,
        login_name: String::new(),
    });
    let name = if node.computed_name.is_empty() {
        short_name(&node.name).unwrap_or_else(|| hostinfo.hostname.clone())
    } else {
        node.computed_name
    };
    Ok(Peer {
        node_id: node.stable_id,
        name,
        hostname: hostinfo.hostname,
        os: hostinfo.os,
        user_id: user.id,
        login_name: user.login_name,
        tagged: node.tags.is_some_and(|tags| !tags.is_empty()),
    })
}

/// `my-mac` from `my-mac.tail1234.ts.net.`.
fn short_name(dns_name: &str) -> Option<String> {
    let label = dns_name.split('.').next()?;
    (!label.is_empty()).then(|| label.to_owned())
}

/// The useful part of the CLI's stderr: its last line, without the log
/// timestamp it starts with (`2026/10/22 01:52:25 peer not found`).
fn cli_message(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let mut parts = line.splitn(3, ' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(date), Some(time), Some(rest))
            if date.len() == 10 && date.contains('/') && time.len() == 8 && time.contains(':') =>
        {
            rest.trim().to_owned()
        }
        _ => line.trim().to_owned(),
    }
}

// ---- The real CLI ----

/// Runs the `tailscale` CLI. It is looked up on every call, so installing or
/// starting Tailscale later works without restarting the daemon.
#[derive(Debug, Default, Clone)]
pub(crate) struct TailscaleCli;

impl TailscaleCli {
    /// `COS_TAILSCALE` if set, else `tailscale` on `PATH`, else the Mac app's binary.
    fn program() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("COS_TAILSCALE") {
            return Some(PathBuf::from(path));
        }
        let on_path = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("tailscale"))
                .find(|candidate| candidate.is_file())
        });
        on_path.or_else(|| Path::new(MAC_APP_CLI).is_file().then(|| MAC_APP_CLI.into()))
    }

    /// Runs `tailscale <args>`. Returns stdout on success, or the exit's
    /// stderr message on failure.
    async fn run(args: &[&str]) -> Result<Result<Vec<u8>, String>, TailscaleError> {
        let program = Self::program().ok_or(TailscaleError::NotInstalled)?;
        let command = args.join(" ");
        let output = tokio::process::Command::new(&program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = match tokio::time::timeout(TIMEOUT, output).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(TailscaleError::NotInstalled);
            }
            Ok(Err(error)) => {
                return Err(TailscaleError::Failed {
                    command,
                    message: error.to_string(),
                });
            }
            Err(_) => {
                return Err(TailscaleError::Failed {
                    command,
                    message: format!("no answer after {TIMEOUT:?}"),
                });
            }
        };
        if output.status.success() {
            Ok(Ok(output.stdout))
        } else {
            Ok(Err(cli_message(&output.stderr)))
        }
    }
}

impl Tailscale for TailscaleCli {
    fn local(&self) -> BoxFuture<'_, Result<LocalNode, TailscaleError>> {
        Box::pin(async {
            match Self::run(&["status", "--json"]).await? {
                Ok(stdout) => parse_status(&stdout),
                // The CLI is there but the app or tailscaled isn't.
                Err(message)
                    if message.contains("failed to connect") || message.contains("is Tailscale running") =>
                {
                    Err(TailscaleError::NotRunning(message))
                }
                Err(message) => Err(TailscaleError::Failed {
                    command: "status --json".into(),
                    message,
                }),
            }
        })
    }

    fn whois(&self, addr: SocketAddr) -> BoxFuture<'_, Result<Option<Peer>, TailscaleError>> {
        Box::pin(async move {
            let addr = addr.to_string();
            match Self::run(&["whois", "--json", &addr]).await? {
                Ok(stdout) => parse_whois(&stdout).map(Some),
                Err(message) if message.contains("peer not found") => Ok(None),
                Err(message) => Err(TailscaleError::Failed {
                    command: format!("whois --json {addr}"),
                    message,
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const STATUS: &str = include_str!("../tests/fixtures/tailscale-status.json");
    const WHOIS_PEER: &str = include_str!("../tests/fixtures/tailscale-whois-peer.json");
    const WHOIS_SELF: &str = include_str!("../tests/fixtures/tailscale-whois-self.json");

    fn edited(json: &str, edit: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut value: Value = serde_json::from_str(json).unwrap();
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn status_gives_this_machine_and_its_owner() {
        let local = parse_status(STATUS.as_bytes()).unwrap();

        assert_eq!(
            local,
            LocalNode {
                node_id: "nSELFstableID1CNTRL".into(),
                name: "my-mac".into(),
                os: "macOS".into(),
                user_id: 1111111111111111,
                login_name: "me@example.com".into(),
                tagged: false,
                ips: vec![
                    "100.100.1.1".parse().unwrap(),
                    "fd7a:115c:a1e0::1:101".parse().unwrap()
                ],
            }
        );
    }

    #[test]
    fn status_says_why_tailscale_is_unusable() {
        let with_state =
            |state: &str| parse_status(&edited(STATUS, |v| v["BackendState"] = json!(state))).unwrap_err();

        assert_eq!(with_state("NeedsLogin"), TailscaleError::NeedsLogin);
        assert!(matches!(with_state("Stopped"), TailscaleError::NotRunning(_)));
        assert!(matches!(with_state("Starting"), TailscaleError::NotRunning(_)));
        assert!(matches!(
            parse_status(b"not json"),
            Err(TailscaleError::Unexpected { .. })
        ));
        assert!(matches!(
            parse_status(br#"{"BackendState":"Running"}"#),
            Err(TailscaleError::Unexpected { .. })
        ));
    }

    #[test]
    fn status_keeps_only_tailnet_addresses_and_notices_tags() {
        let json = edited(STATUS, |v| {
            v["Self"]["TailscaleIPs"] = json!(["100.100.1.1", "192.168.1.5", "fd00::1", "nonsense"]);
            v["Self"]["Tags"] = json!(["tag:server"]);
        });

        let local = parse_status(&json).unwrap();

        assert_eq!(local.ips, vec!["100.100.1.1".parse::<IpAddr>().unwrap()]);
        assert!(local.tagged);
    }

    #[test]
    fn whois_gives_the_peer_and_its_user() {
        let peer = parse_whois(WHOIS_PEER.as_bytes()).unwrap();

        assert_eq!(
            peer,
            Peer {
                node_id: "nPEER1stableIDCNTRL".into(),
                name: "build-box".into(),
                hostname: "build-box".into(),
                os: "linux".into(),
                user_id: 1111111111111111,
                login_name: "me@example.com".into(),
                tagged: false,
            }
        );
    }

    #[test]
    fn whois_for_this_machine_has_no_os() {
        let peer = parse_whois(WHOIS_SELF.as_bytes()).unwrap();

        assert_eq!(peer.node_id, "nSELFstableID1CNTRL");
        assert_eq!((peer.name.as_str(), peer.hostname.as_str()), ("my-mac", "My Mac"));
        assert_eq!(peer.os, "");
    }

    #[test]
    fn whois_is_lenient_about_missing_fields() {
        let tagged = edited(WHOIS_PEER, |v| {
            v["Node"]["Tags"] = json!(["tag:ci"]);
            v["Node"].as_object_mut().unwrap().remove("ComputedName");
            v["Node"].as_object_mut().unwrap().remove("Hostinfo");
            v.as_object_mut().unwrap().remove("UserProfile");
        });

        let peer = parse_whois(&tagged).unwrap();

        assert!(peer.tagged);
        assert_eq!(peer.name, "build-box");
        assert_eq!(peer.login_name, "");
        assert_eq!(peer.user_id, 1111111111111111);
        assert!(matches!(
            parse_whois(br#"{"Node":{}}"#),
            Err(TailscaleError::Unexpected { .. })
        ));
    }

    #[test]
    fn tailnet_ranges() {
        for ip in [
            "100.64.0.1",
            "100.127.255.254",
            "100.100.1.1",
            "fd7a:115c:a1e0::1",
        ] {
            assert!(is_tailnet_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "100.63.0.1",
            "100.128.0.1",
            "10.0.0.1",
            "127.0.0.1",
            "fd7a:115c:a1e1::1",
            "::1",
        ] {
            assert!(!is_tailnet_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn cli_messages_lose_their_timestamp() {
        assert_eq!(
            cli_message(b"2026/10/22 01:52:25 peer not found\n"),
            "peer not found"
        );
        assert_eq!(
            cli_message(b"some detail\n2026/10/22 01:58:34 400 Bad Request: invalid 'addr' parameter\n"),
            "400 Bad Request: invalid 'addr' parameter"
        );
        assert_eq!(cli_message(b"plain error\n"), "plain error");
    }
}
