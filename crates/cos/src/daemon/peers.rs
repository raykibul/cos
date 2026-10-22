//! The other devices on this machine's tailnet, for `GET /v1/tailnet/peers`
//! and `cos remote peers`, so an app can offer them when you connect to
//! another Mac.
//!
//! Tailscale says who the peers are. Whether cos runs on one is found by
//! opening a plain TCP connection to its tailnet listener port and closing it
//! without sending anything: an HTTP request would register this machine as a
//! pending device over there. Only online peers of the same Tailscale user
//! are probed, all at once, and each answer is kept for a short while so a
//! sheet that refetches doesn't probe again.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::tailscale::{BoxFuture, PeerNode, Tailscale};

/// How long a probe waits for a peer to accept the connection.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
/// How long a probe's answer is reused.
pub(crate) const PROBE_TTL: Duration = Duration::from_secs(10);

/// `GET /v1/tailnet/peers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PeersPage {
    /// Running cos first, then online, then by name. Never this machine.
    pub peers: Vec<TailnetPeer>,
    /// This machine's MagicDNS name, once Tailscale has answered.
    pub self_name: Option<String>,
    /// This machine, the host answering, once Tailscale has answered, so an
    /// app can say which host the list came from.
    #[serde(rename = "self", default)]
    pub host: Option<SelfNode>,
    /// Why there are no peers: Tailscale isn't usable.
    pub reason: Option<String>,
}

/// The machine answering `GET /v1/tailnet/peers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SelfNode {
    pub name: String,
    /// Without the trailing dot; empty if there is none.
    #[serde(default)]
    pub dns_name: String,
    #[serde(default)]
    pub os: String,
    /// Its first tailnet IPv4 (else first address) and the port asked about.
    /// Empty if Tailscale lists no address.
    #[serde(default)]
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TailnetPeer {
    /// Tailscale's stable node ID.
    pub id: String,
    pub name: String,
    pub hostname: String,
    /// Without the trailing dot; empty if there is none.
    pub dns_name: String,
    pub os: String,
    pub ips: Vec<IpAddr>,
    pub online: bool,
    /// Milliseconds since the epoch, when offline and Tailscale knows.
    pub last_seen_ms: Option<i64>,
    /// Same Tailscale user as this machine and not tagged: only these can
    /// be approved as devices.
    pub mine: bool,
    pub tagged: bool,
    pub cos: CosState,
    /// Where cos would listen on it: the first IPv4 (else the first address)
    /// and the port. Empty if Tailscale lists no address.
    pub address: String,
    /// The device asking, over the tailnet. Always false on the socket.
    #[serde(default)]
    pub is_you: bool,
}

/// Whether cos's tailnet listener answers on a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CosState {
    /// Something accepted a connection on the port.
    Running,
    /// Refused or no answer in time.
    NotRunning,
    /// Not probed: offline, or not this user's device.
    Unknown,
}

/// Tells whether something accepts TCP connections at an address. Tests
/// use a fake.
pub(crate) trait Probe: Send + Sync {
    fn accepts(&self, addr: SocketAddr) -> BoxFuture<'static, bool>;
}

/// Connects and closes at once, sending nothing.
pub(crate) struct TcpProbe {
    pub timeout: Duration,
}

impl Probe for TcpProbe {
    fn accepts(&self, addr: SocketAddr) -> BoxFuture<'static, bool> {
        let timeout = self.timeout;
        Box::pin(async move {
            // Dropping the stream closes it.
            matches!(
                tokio::time::timeout(timeout, TcpStream::connect(addr)).await,
                Ok(Ok(_))
            )
        })
    }
}

/// Lists the peers and probes them. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Peers {
    tailscale: Arc<dyn Tailscale>,
    probe: Arc<dyn Probe>,
    ttl: Duration,
    cache: Arc<Mutex<HashMap<SocketAddr, Probed>>>,
}

struct Probed {
    at: Instant,
    running: bool,
}

impl Peers {
    pub(crate) fn new(tailscale: Arc<dyn Tailscale>) -> Self {
        Self::with_probe(
            tailscale,
            Arc::new(TcpProbe {
                timeout: PROBE_TIMEOUT,
            }),
            PROBE_TTL,
        )
    }

    pub(crate) fn with_probe(tailscale: Arc<dyn Tailscale>, probe: Arc<dyn Probe>, ttl: Duration) -> Self {
        Self {
            tailscale,
            probe,
            ttl,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Without Tailscale: every list says it isn't installed. For tests
    /// that don't look at peers.
    #[cfg(test)]
    pub(crate) fn without_tailscale() -> Self {
        Self::new(Arc::new(NoTailscale))
    }

    /// The peers, with cos probed on `port`.
    pub(crate) async fn list(&self, port: u16) -> PeersPage {
        let nodes = match self.tailscale.peers().await {
            Ok(nodes) => nodes,
            Err(error) => {
                return PeersPage {
                    peers: Vec::new(),
                    self_name: None,
                    host: None,
                    reason: Some(error.to_string()),
                };
            }
        };
        let probed: Vec<SocketAddr> = nodes
            .peers
            .iter()
            .filter(|peer| peer.online && peer.mine)
            .filter_map(|peer| address(peer, port))
            .collect();
        let running = self.probe_all(&probed).await;
        let mut peers: Vec<TailnetPeer> = nodes
            .peers
            .into_iter()
            .map(|peer| {
                let address = address(&peer, port);
                let cos = match address.and_then(|address| running.get(&address)) {
                    Some(true) if peer.online && peer.mine => CosState::Running,
                    Some(false) if peer.online && peer.mine => CosState::NotRunning,
                    _ => CosState::Unknown,
                };
                TailnetPeer {
                    id: peer.node_id,
                    name: peer.name,
                    hostname: peer.hostname,
                    dns_name: peer.dns_name,
                    os: peer.os,
                    ips: peer.ips,
                    online: peer.online,
                    last_seen_ms: peer.last_seen.map(|at| at.as_millisecond()),
                    mine: peer.mine,
                    tagged: peer.tagged,
                    cos,
                    address: address.map(|address| address.to_string()).unwrap_or_default(),
                    is_you: false,
                }
            })
            .collect();
        sort(&mut peers);
        let local = nodes.local;
        let host = SelfNode {
            address: first_address(&local.ips, port)
                .map(|address| address.to_string())
                .unwrap_or_default(),
            name: local.name.clone(),
            dns_name: local.dns_name,
            os: local.os,
        };
        PeersPage {
            peers,
            self_name: Some(local.name),
            host: Some(host),
            reason: None,
        }
    }

    /// Probes each address not answered in the last `ttl`, all at once.
    async fn probe_all(&self, addresses: &[SocketAddr]) -> HashMap<SocketAddr, bool> {
        let mut answers = HashMap::new();
        let mut probing = HashSet::new();
        let mut probes = JoinSet::new();
        {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            for &address in addresses {
                match cache
                    .get(&address)
                    .filter(|probed| probed.at.elapsed() < self.ttl)
                {
                    Some(probed) => {
                        answers.insert(address, probed.running);
                    }
                    None if probing.insert(address) => {
                        let probe = self.probe.accepts(address);
                        probes.spawn(async move { (address, probe.await) });
                    }
                    None => {}
                }
            }
        }
        while let Some(probed) = probes.join_next().await {
            let Ok((address, running)) = probed else {
                continue;
            };
            answers.insert(address, running);
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|_, probed| probed.at.elapsed() < self.ttl);
            cache.insert(
                address,
                Probed {
                    at: Instant::now(),
                    running,
                },
            );
        }
        answers
    }
}

/// Where cos would listen on `peer`: its first IPv4 address, else its first
/// address, on `port`.
fn address(peer: &PeerNode, port: u16) -> Option<SocketAddr> {
    first_address(&peer.ips, port)
}

fn first_address(ips: &[IpAddr], port: u16) -> Option<SocketAddr> {
    let ip = ips.iter().find(|ip| ip.is_ipv4()).or_else(|| ips.first())?;
    Some(SocketAddr::new(*ip, port))
}

impl PeersPage {
    /// Marks the device asking, by its Tailscale node ID or else the address
    /// it connected from, so an app can leave itself out.
    pub(crate) fn mark_requester(&mut self, node_id: &str, ip: IpAddr) {
        let by_id = self
            .peers
            .iter()
            .position(|peer| !node_id.is_empty() && peer.id == node_id);
        let found = by_id.or_else(|| self.peers.iter().position(|peer| peer.ips.contains(&ip)));
        if let Some(index) = found {
            self.peers[index].is_you = true;
        }
    }
}

/// Running cos first, then online, then by name.
fn sort(peers: &mut [TailnetPeer]) {
    peers.sort_by_cached_key(|peer| {
        (
            Reverse(peer.cos == CosState::Running),
            Reverse(peer.online),
            peer.name.to_lowercase(),
            peer.name.clone(),
        )
    });
}

#[cfg(test)]
struct NoTailscale;

#[cfg(test)]
impl Tailscale for NoTailscale {
    fn local(&self) -> BoxFuture<'_, Result<crate::tailscale::LocalNode, crate::tailscale::TailscaleError>> {
        Box::pin(async { Err(crate::tailscale::TailscaleError::NotInstalled) })
    }

    fn whois(
        &self,
        _addr: SocketAddr,
    ) -> BoxFuture<'_, Result<Option<crate::tailscale::Peer>, crate::tailscale::TailscaleError>> {
        Box::pin(async { Err(crate::tailscale::TailscaleError::NotInstalled) })
    }

    fn peers(
        &self,
    ) -> BoxFuture<'_, Result<crate::tailscale::TailnetNodes, crate::tailscale::TailscaleError>> {
        Box::pin(async { Err(crate::tailscale::TailscaleError::NotInstalled) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::tailscale::{LocalNode, TailnetNodes, TailscaleError};

    fn node(name: &str, ip: &str, online: bool, mine: bool) -> PeerNode {
        PeerNode {
            node_id: format!("n{name}"),
            name: name.into(),
            hostname: name.into(),
            dns_name: format!("{name}.tail0000.ts.net"),
            os: "macOS".into(),
            ips: vec![ip.parse().unwrap()],
            online,
            last_seen: None,
            user_id: if mine { 1 } else { 2 },
            tagged: false,
            mine,
        }
    }

    struct Fixed(Result<TailnetNodes, TailscaleError>);

    impl Tailscale for Fixed {
        fn local(&self) -> BoxFuture<'_, Result<LocalNode, TailscaleError>> {
            let local = self.0.clone().map(|nodes| nodes.local);
            Box::pin(async move { local })
        }

        fn whois(
            &self,
            _addr: SocketAddr,
        ) -> BoxFuture<'_, Result<Option<crate::tailscale::Peer>, TailscaleError>> {
            Box::pin(async { Ok(None) })
        }

        fn peers(&self) -> BoxFuture<'_, Result<TailnetNodes, TailscaleError>> {
            let nodes = self.0.clone();
            Box::pin(async move { nodes })
        }
    }

    fn local() -> LocalNode {
        LocalNode {
            node_id: "nHOST".into(),
            name: "my-mac".into(),
            dns_name: "my-mac.tail0000.ts.net".into(),
            os: "macOS".into(),
            user_id: 1,
            login_name: "me@example.com".into(),
            tagged: false,
            ips: vec!["100.100.1.1".parse().unwrap()],
        }
    }

    /// Says cos runs at the addresses it was given, and counts its calls.
    struct FakeProbe {
        running: Vec<SocketAddr>,
        calls: AtomicUsize,
    }

    impl Probe for FakeProbe {
        fn accepts(&self, addr: SocketAddr) -> BoxFuture<'static, bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let running = self.running.contains(&addr);
            Box::pin(async move { running })
        }
    }

    fn peers_with(nodes: Vec<PeerNode>, running: &[&str], ttl: Duration) -> (Peers, Arc<FakeProbe>) {
        let probe = Arc::new(FakeProbe {
            running: running.iter().map(|addr| addr.parse().unwrap()).collect(),
            calls: AtomicUsize::new(0),
        });
        let tailscale = Arc::new(Fixed(Ok(TailnetNodes {
            local: local(),
            peers: nodes,
        })));
        (Peers::with_probe(tailscale, probe.clone(), ttl), probe)
    }

    #[tokio::test]
    async fn only_online_peers_of_mine_are_probed_and_running_ones_come_first() {
        let (peers, probe) = peers_with(
            vec![
                node("zebra", "100.100.2.9", true, true),
                node("alpha", "100.100.2.1", false, true),
                node("Mango", "100.100.2.5", true, true),
                node("stranger", "100.100.2.7", true, false),
                node("beta", "100.100.2.2", true, true),
            ],
            &["100.100.2.9:7700", "100.100.2.7:7700"],
            PROBE_TTL,
        );

        let page = peers.list(7700).await;

        let order: Vec<(&str, CosState)> = page
            .peers
            .iter()
            .map(|peer| (peer.name.as_str(), peer.cos))
            .collect();
        assert_eq!(
            order,
            [
                ("zebra", CosState::Running),
                ("beta", CosState::NotRunning),
                ("Mango", CosState::NotRunning),
                ("stranger", CosState::Unknown),
                ("alpha", CosState::Unknown),
            ]
        );
        // zebra, Mango and beta; never the stranger or the offline one.
        assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
        assert_eq!(page.self_name.as_deref(), Some("my-mac"));
        assert_eq!(
            page.host,
            Some(SelfNode {
                name: "my-mac".into(),
                dns_name: "my-mac.tail0000.ts.net".into(),
                os: "macOS".into(),
                address: "100.100.1.1:7700".into(),
            })
        );
        assert_eq!(page.reason, None);
        assert_eq!(page.peers[0].address, "100.100.2.9:7700");
        assert!(page.peers.iter().all(|peer| !peer.is_you));
    }

    #[tokio::test]
    async fn answers_are_reused_per_address_for_a_while() {
        let (peers, probe) = peers_with(
            vec![node("box", "100.100.2.2", true, true)],
            &[],
            Duration::from_secs(60),
        );

        peers.list(7700).await;
        peers.list(7700).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        // Another port is another address.
        peers.list(7701).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);

        let (peers, probe) = peers_with(vec![node("box", "100.100.2.2", true, true)], &[], Duration::ZERO);
        peers.list(7700).await;
        peers.list(7700).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn without_tailscale_there_are_no_peers_and_a_reason() {
        let peers = Peers::with_probe(
            Arc::new(Fixed(Err(TailscaleError::NeedsLogin))),
            Arc::new(FakeProbe {
                running: Vec::new(),
                calls: AtomicUsize::new(0),
            }),
            PROBE_TTL,
        );

        let page = peers.list(7700).await;

        assert_eq!(
            page,
            PeersPage {
                peers: Vec::new(),
                self_name: None,
                host: None,
                reason: Some("Tailscale is not logged in".into()),
            }
        );
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"peers": [], "self_name": null, "self": null, "reason": "Tailscale is not logged in"})
        );
    }

    #[tokio::test]
    async fn the_requester_is_marked_by_node_id_else_by_address() {
        let (peers, _) = peers_with(
            vec![
                node("phone", "100.100.2.1", true, true),
                node("tablet", "100.100.2.2", true, true),
            ],
            &[],
            PROBE_TTL,
        );
        let page = peers.list(7700).await;
        let yours = |page: &PeersPage| -> Vec<String> {
            page.peers
                .iter()
                .filter(|peer| peer.is_you)
                .map(|peer| peer.name.clone())
                .collect()
        };

        let mut by_id = page.clone();
        by_id.mark_requester("nphone", "100.100.9.9".parse().unwrap());
        assert_eq!(yours(&by_id), ["phone"]);

        let mut by_ip = page.clone();
        by_ip.mark_requester("nGONE", "100.100.2.2".parse().unwrap());
        assert_eq!(yours(&by_ip), ["tablet"]);

        let mut neither = page;
        neither.mark_requester("", "100.100.9.9".parse().unwrap());
        assert!(yours(&neither).is_empty());
    }

    #[test]
    fn an_older_page_without_self_or_is_you_still_parses() {
        let page: PeersPage = serde_json::from_value(serde_json::json!({
            "peers": [{"id": "n1", "name": "box", "hostname": "box", "dns_name": "", "os": "linux",
                       "ips": [], "online": true, "last_seen_ms": null, "mine": true, "tagged": false,
                       "cos": "running", "address": ""}],
            "self_name": "my-mac", "reason": null
        }))
        .unwrap();
        assert_eq!(page.host, None);
        assert!(!page.peers[0].is_you);
    }

    #[test]
    fn the_address_prefers_ipv4_and_brackets_ipv6() {
        let mut peer = node("box", "100.100.2.2", true, true);
        peer.ips = vec![
            "fd7a:115c:a1e0::2:2".parse().unwrap(),
            "100.100.2.2".parse().unwrap(),
        ];
        assert_eq!(address(&peer, 7700).unwrap().to_string(), "100.100.2.2:7700");
        peer.ips.truncate(1);
        assert_eq!(
            address(&peer, 7700).unwrap().to_string(),
            "[fd7a:115c:a1e0::2:2]:7700"
        );
        peer.ips.clear();
        assert_eq!(address(&peer, 7700), None);
    }

    #[tokio::test]
    async fn the_tcp_probe_tells_a_listener_from_a_closed_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = listener.local_addr().unwrap();
        let closed = {
            let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            spare.local_addr().unwrap()
        };
        let probe = TcpProbe {
            timeout: PROBE_TIMEOUT,
        };

        assert!(probe.accepts(open).await);
        assert!(!probe.accepts(closed).await);

        // The probe sends nothing and hangs up.
        let (mut accepted, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut accepted, &mut received)
            .await
            .unwrap();
        assert!(received.is_empty());
    }

    #[tokio::test]
    async fn the_tcp_probe_gives_up_after_its_timeout() {
        // A non-routable address: the connection neither opens nor fails.
        let probe = TcpProbe {
            timeout: Duration::from_millis(100),
        };
        let started = Instant::now();
        assert!(!probe.accepts("10.255.255.1:7700".parse().unwrap()).await);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
