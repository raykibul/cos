//! Port forwarding: a TCP tunnel from an app to a port on this machine's
//! loopback, so another Mac can open `http://localhost:3343` and get the dev
//! server running here. One route, merged into [`super::api::router`] (see
//! "Port forwarding" in `docs/daemon.md`):
//!
//! - `GET /v1/forward/{port}`: a probe, `{port, listening, address}`
//! - the same with `Connection: upgrade` and `Upgrade: cos-forward`: the
//!   daemon connects to `127.0.0.1:{port}` (else `[::1]:{port}`), answers
//!   101 and copies bytes both ways until each side closes.
//!
//! The address is built from the two loopback IPs and the port, never from
//! anything else in the request, so the route can't reach the network. Over
//! the tailnet the guard runs first, as for every route: an approved device
//! already has a shell here, so this gives it nothing new. Tunnels are not
//! logged as events; there are far too many (a page load opens several).

use std::collections::HashMap;
use std::io::ErrorKind as IoKind;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::extract::rejection::PathRejection;
use axum::extract::{Extension, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::watch;

use super::api::AppState;
use super::remote_api::RemoteClient;
use super::tailnet::Refusal;

/// The `Upgrade` token of a tunnel.
pub(crate) const UPGRADE: &str = "cos-forward";
/// How long connecting to each loopback address may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Open tunnels per device, at most.
const PER_DEVICE: usize = 256;
/// Open tunnels in all, at most.
const TOTAL: usize = 1024;

/// The probe's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Probe {
    pub port: u16,
    pub listening: bool,
    /// Where something answered, or null.
    pub address: Option<String>,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/v1/forward/{port}", get(forward))
}

/// The two places a port is tried, in order: Node 17+ and Vite often listen
/// on `::1` only.
pub(crate) fn loopback(port: u16) -> [SocketAddr; 2] {
    [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
}

#[derive(Debug, PartialEq, Eq)]
enum ConnectError {
    /// Both addresses refused: nothing runs on the port.
    Refused,
    /// No answer in time on at least one of them.
    Timeout,
}

/// Connects to `port` on this machine's loopback, v4 first.
async fn connect_loopback(port: u16) -> Result<(TcpStream, SocketAddr), ConnectError> {
    let mut timed_out = false;
    for addr in loopback(port) {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok((stream, addr));
            }
            // Refused, or no IPv6 loopback at all: try the next one.
            Ok(Err(error)) => {
                if !matches!(
                    error.kind(),
                    IoKind::ConnectionRefused | IoKind::AddrNotAvailable | IoKind::Unsupported
                ) {
                    tracing::debug!(%addr, %error, "forward: could not connect");
                }
            }
            Err(_) => timed_out = true,
        }
    }
    Err(if timed_out {
        ConnectError::Timeout
    } else {
        ConnectError::Refused
    })
}

/// Whether the request asks for a tunnel: `Ok(true)` for `Upgrade:
/// cos-forward`, `Ok(false)` for a plain probe, an error for any other
/// upgrade.
fn wants_tunnel(headers: &HeaderMap) -> Result<bool, Refusal> {
    let Some(upgrade) = headers.get(header::UPGRADE) else {
        return Ok(false);
    };
    let upgrade_ok = upgrade
        .to_str()
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case(UPGRADE));
    let connection_ok = headers.get_all(header::CONNECTION).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
    });
    if upgrade_ok && connection_ok {
        Ok(true)
    } else {
        Err(Refusal {
            status: StatusCode::BAD_REQUEST,
            code: "bad_upgrade",
            message: format!("this route only upgrades to `{UPGRADE}`, with `Connection: upgrade`"),
        })
    }
}

fn parse_port(port: Result<Path<String>, PathRejection>) -> Result<u16, Refusal> {
    port.ok()
        .and_then(|Path(port)| port.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| Refusal {
            status: StatusCode::BAD_REQUEST,
            code: "bad_port",
            message: "the port must be a number from 1 to 65535".into(),
        })
}

/// This machine's name for messages: its tailnet name, else "this machine".
fn machine(state: &AppState) -> String {
    state
        .tailnet
        .status()
        .host
        .unwrap_or_else(|| "this machine".to_owned())
}

async fn forward(
    State(state): State<AppState>,
    port: Result<Path<String>, PathRejection>,
    client: Option<Extension<RemoteClient>>,
    request: Request,
) -> Response {
    match open(state, port, client.map(|Extension(client)| client), request).await {
        Ok(response) => response,
        Err(refusal) => {
            tracing::warn!(code = refusal.code, error = %refusal.message, "forward refused");
            refusal.into_response()
        }
    }
}

async fn open(
    state: AppState,
    port: Result<Path<String>, PathRejection>,
    client: Option<RemoteClient>,
    mut request: Request,
) -> Result<Response, Refusal> {
    let port = parse_port(port)?;
    if !wants_tunnel(request.headers())? {
        let probe = match connect_loopback(port).await {
            Ok((stream, addr)) => {
                drop(stream);
                Probe {
                    port,
                    listening: true,
                    address: Some(addr.to_string()),
                }
            }
            Err(_) => Probe {
                port,
                listening: false,
                address: None,
            },
        };
        return Ok(Json(probe).into_response());
    }

    let device = client.map(|client| client.node_id);
    let slot = state.forwards.reserve(device.as_deref())?;
    let (stream, addr) = connect_loopback(port).await.map_err(|error| match error {
        ConnectError::Refused => Refusal {
            status: StatusCode::BAD_GATEWAY,
            code: "nothing_listening",
            message: format!("nothing is listening on port {port} on {}", machine(&state)),
        },
        ConnectError::Timeout => Refusal {
            status: StatusCode::GATEWAY_TIMEOUT,
            code: "connect_timeout",
            message: format!(
                "port {port} on {} didn't answer within {} s",
                machine(&state),
                CONNECT_TIMEOUT.as_secs()
            ),
        },
    })?;
    let on_upgrade = hyper::upgrade::on(&mut request);
    let mut stopping = state.stopping.clone();
    tokio::spawn(async move {
        let mut closed = slot.closed.clone();
        let upgraded = match on_upgrade.await {
            Ok(upgraded) => upgraded,
            Err(error) => {
                tracing::debug!(%error, "forward: upgrade failed");
                return;
            }
        };
        tracing::debug!(%addr, device = ?slot.device, "forward: tunnel open");
        let mut remote = TokioIo::new(upgraded);
        let mut local = stream;
        tokio::select! {
            result = tokio::io::copy_bidirectional(&mut remote, &mut local) => {
                if let Err(error) = result {
                    tracing::debug!(%addr, %error, "forward: tunnel ended");
                }
            }
            _ = closed.wait_for(|closed| *closed) => {}
            _ = stopping.wait_for(|stopping| *stopping) => {}
        }
        drop(slot);
    });

    let mut response = StatusCode::SWITCHING_PROTOCOLS.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    headers.insert(header::UPGRADE, HeaderValue::from_static(UPGRADE));
    Ok(response)
}

/// The open tunnels, so caps hold and a device that's denied or removed
/// loses its tunnels at once. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Forwards {
    inner: Arc<Mutex<Inner>>,
    per_device: usize,
    total: usize,
}

#[derive(Default)]
struct Inner {
    next: u64,
    tunnels: HashMap<u64, Open>,
}

struct Open {
    /// The device's node id; none on the Unix socket.
    device: Option<String>,
    close: watch::Sender<bool>,
}

/// One tunnel's place in [`Forwards`], given back when dropped.
pub(crate) struct Slot {
    forwards: Forwards,
    id: u64,
    device: Option<String>,
    /// Flips to true when the tunnel must close.
    closed: watch::Receiver<bool>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.forwards.lock().tunnels.remove(&self.id);
    }
}

impl Default for Forwards {
    fn default() -> Self {
        Self::with_caps(PER_DEVICE, TOTAL)
    }
}

impl Forwards {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_caps(per_device: usize, total: usize) -> Self {
        Self {
            inner: Arc::default(),
            per_device,
            total,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a place for one more tunnel from `device`, if the caps allow.
    pub(crate) fn reserve(&self, device: Option<&str>) -> Result<Slot, Refusal> {
        let mut inner = self.lock();
        let mine = inner
            .tunnels
            .values()
            .filter(|open| open.device.as_deref() == device)
            .count();
        if inner.tunnels.len() >= self.total || (device.is_some() && mine >= self.per_device) {
            return Err(Refusal {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "too_many_forwards",
                message: format!(
                    "too many forwarded connections are open (at most {} per device, {} in all)",
                    self.per_device, self.total
                ),
            });
        }
        inner.next += 1;
        let id = inner.next;
        let (close, closed) = watch::channel(false);
        inner.tunnels.insert(
            id,
            Open {
                device: device.map(str::to_owned),
                close,
            },
        );
        drop(inner);
        Ok(Slot {
            forwards: self.clone(),
            id,
            device: device.map(str::to_owned),
            closed,
        })
    }

    /// Closes every tunnel of the device with this node id.
    pub(crate) fn close_device(&self, node_id: &str) {
        let inner = self.lock();
        let mut closed = 0;
        for open in inner.tunnels.values() {
            if open.device.as_deref() == Some(node_id) {
                let _ = open.close.send(true);
                closed += 1;
            }
        }
        if closed > 0 {
            tracing::info!(
                device = node_id,
                tunnels = closed,
                "forward: closed a device's tunnels"
            );
        }
    }

    /// How many tunnels are open.
    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.lock().tunnels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_is_ever_dialled() {
        for port in [1, 22, 3343, 7700, 65535] {
            let [v4, v6] = loopback(port);
            assert!(v4.ip().is_loopback() && v4.is_ipv4(), "{v4}");
            assert!(v6.ip().is_loopback() && v6.is_ipv6(), "{v6}");
            assert_eq!((v4.port(), v6.port()), (port, port));
        }
    }

    #[test]
    fn ports_are_1_to_65535() {
        let ok = |port: &str| parse_port(Ok(Path(port.to_owned())));
        assert_eq!(ok("3343").unwrap(), 3343);
        assert_eq!(ok("65535").unwrap(), 65535);
        for bad in ["0", "65536", "abc", "-1", ""] {
            assert_eq!(ok(bad).unwrap_err().code, "bad_port", "{bad}");
        }
    }

    #[test]
    fn upgrades_must_name_cos_forward() {
        let headers = |pairs: &[(&'static str, &'static str)]| {
            let mut map = HeaderMap::new();
            for (name, value) in pairs {
                map.append(*name, HeaderValue::from_static(value));
            }
            map
        };
        assert_eq!(wants_tunnel(&headers(&[])), Ok(false));
        assert_eq!(
            wants_tunnel(&headers(&[("connection", "Upgrade"), ("upgrade", "cos-forward")])),
            Ok(true)
        );
        assert_eq!(
            wants_tunnel(&headers(&[
                ("connection", "keep-alive, upgrade"),
                ("upgrade", "COS-FORWARD")
            ])),
            Ok(true)
        );
        for bad in [
            headers(&[("connection", "upgrade"), ("upgrade", "websocket")]),
            headers(&[("upgrade", "cos-forward")]),
        ] {
            assert_eq!(wants_tunnel(&bad).unwrap_err().code, "bad_upgrade");
        }
    }

    #[test]
    fn caps_hold_per_device_and_in_all() {
        let forwards = Forwards::with_caps(2, 3);
        let a1 = forwards.reserve(Some("a")).unwrap();
        let _a2 = forwards.reserve(Some("a")).unwrap();
        assert_eq!(
            forwards.reserve(Some("a")).err().unwrap().code,
            "too_many_forwards"
        );
        let _b1 = forwards.reserve(Some("b")).unwrap();
        // Three in all: nobody gets a fourth.
        assert_eq!(
            forwards.reserve(Some("c")).err().unwrap().code,
            "too_many_forwards"
        );
        drop(a1);
        let _a3 = forwards.reserve(Some("a")).unwrap();
        assert_eq!(forwards.open_count(), 3);
    }

    #[test]
    fn closing_a_device_signals_only_its_tunnels() {
        let forwards = Forwards::new();
        let a = forwards.reserve(Some("a")).unwrap();
        let b = forwards.reserve(Some("b")).unwrap();
        let socket = forwards.reserve(None).unwrap();
        forwards.close_device("a");
        assert!(*a.closed.borrow());
        assert!(!*b.closed.borrow());
        assert!(!*socket.closed.borrow());
    }
}
