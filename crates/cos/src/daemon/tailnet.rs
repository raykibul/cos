//! The tailnet listener: the daemon's API on TCP port 7700 of this machine's
//! tailnet addresses, for your other devices. It is off until you turn it on
//! (`cos remote on`), and the setting survives restarts. It never binds
//! `0.0.0.0`, only the addresses Tailscale gives this machine.
//!
//! Every request over it is checked by a [`Guard`]: `tailscale whois` must
//! say the client is logged in as the same Tailscale user as this machine,
//! and the device must have been approved on the host (`cos devices
//! approve`). Device and listener management stay on the Unix socket.
//!
//! If Tailscale isn't installed, running or logged in, the daemon carries on
//! with the Unix socket alone, reports why in `/v1/status`, and tries again
//! every 30 seconds.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use cos_core::events::kind;
use cos_core::{Device, DeviceState, Devices, EventLog, SeenDevice, Store, StoreError};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::api::AppState;
use super::remote_api;
use crate::tailscale::{LocalNode, Peer, Tailscale, TailscaleError};

pub(crate) const DEFAULT_PORT: u16 = 7700;
/// The key the settings are stored under.
const SETTING: &str = "tailnet";
/// How long open connections get once a listener is being replaced.
const LISTENER_GRACE: Duration = Duration::from_secs(5);

/// The persisted switch. Off by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TailnetSettings {
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for TailnetSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
        }
    }
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

/// What the listener is doing, as `/v1/status` and `/v1/tailnet` report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TailnetStatus {
    /// Whether remote access is switched on.
    pub enabled: bool,
    pub port: u16,
    pub state: ListenerState,
    /// Where it listens. Empty unless `state` is `listening`.
    #[serde(default)]
    pub addresses: Vec<SocketAddr>,
    /// Why it isn't listening, or why some addresses failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// This machine's MagicDNS name, once Tailscale has been reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The Tailscale user devices must be logged in as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ListenerState {
    /// Remote access is switched off.
    Off,
    /// Switched on, not yet tried.
    Starting,
    Listening,
    /// Switched on, but Tailscale can't be used right now; see `reason`.
    Unavailable,
}

impl TailnetStatus {
    fn new(settings: TailnetSettings, state: ListenerState) -> Self {
        Self {
            enabled: settings.enabled,
            port: settings.port,
            state,
            addresses: Vec::new(),
            reason: None,
            host: None,
            owner: None,
        }
    }

    fn initial(settings: TailnetSettings) -> Self {
        let state = if settings.enabled {
            ListenerState::Starting
        } else {
            ListenerState::Off
        };
        Self::new(settings, state)
    }

    fn unavailable(settings: TailnetSettings, reason: String, local: Option<&LocalNode>) -> Self {
        Self {
            reason: Some(reason),
            host: local.map(|local| local.name.clone()),
            owner: local.map(|local| local.login_name.clone()),
            ..Self::new(settings, ListenerState::Unavailable)
        }
    }
}

/// How often the supervisor retries and rechecks, and how long a `whois`
/// answer is trusted. Shorter in tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// Wait before trying again when Tailscale can't be used.
    pub retry: Duration,
    /// How often a running listener checks that this machine's tailnet
    /// addresses and owner haven't changed.
    pub recheck: Duration,
    pub whois_ttl: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            retry: Duration::from_secs(30),
            recheck: Duration::from_secs(30),
            whois_ttl: Duration::from_secs(60),
        }
    }
}

/// The listener's settings and status, shared between the API and the
/// supervisor task. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Tailnet {
    store: Arc<Store>,
    log: EventLog,
    settings: watch::Sender<TailnetSettings>,
    status: watch::Sender<TailnetStatus>,
}

impl Tailnet {
    /// Loads the saved settings. An unreadable setting counts as off.
    pub(crate) fn load(store: Arc<Store>, log: EventLog) -> Self {
        let settings = match store.setting::<TailnetSettings>(SETTING) {
            Ok(settings) => settings.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "ignoring the saved tailnet setting");
                TailnetSettings::default()
            }
        };
        Self {
            store,
            log,
            settings: watch::Sender::new(settings),
            status: watch::Sender::new(TailnetStatus::initial(settings)),
        }
    }

    pub(crate) fn status(&self) -> TailnetStatus {
        self.status.borrow().clone()
    }

    pub(crate) fn settings(&self) -> TailnetSettings {
        *self.settings.borrow()
    }

    /// Saves new settings; the supervisor applies them.
    pub(crate) fn configure(&self, settings: TailnetSettings) -> Result<(), StoreError> {
        self.store.set_setting(SETTING, &settings)?;
        if self.settings.send_replace(settings) != settings {
            self.set_status(TailnetStatus::initial(settings));
        }
        Ok(())
    }

    /// Waits until the supervisor has acted on the current settings, or
    /// `timeout` passes, and returns the status then.
    pub(crate) async fn settled(&self, timeout: Duration) -> TailnetStatus {
        let settings = self.settings();
        let mut status = self.status.subscribe();
        let _ = tokio::time::timeout(
            timeout,
            status.wait_for(|status| {
                status.enabled == settings.enabled
                    && status.port == settings.port
                    && status.state != ListenerState::Starting
            }),
        )
        .await;
        self.status()
    }

    /// Publishes a new status, logging `tailnet.changed` if it differs.
    fn set_status(&self, status: TailnetStatus) {
        let changed = self.status.send_if_modified(|current| {
            if *current == status {
                return false;
            }
            *current = status.clone();
            true
        });
        if !changed {
            return;
        }
        match status.state {
            ListenerState::Listening => {
                tracing::info!(addresses = ?status.addresses, "tailnet listener up");
            }
            ListenerState::Unavailable => {
                tracing::warn!(
                    reason = status.reason.as_deref().unwrap_or(""),
                    "tailnet listener unavailable"
                );
            }
            ListenerState::Off | ListenerState::Starting => {
                tracing::info!(state = ?status.state, "tailnet listener");
            }
        }
        let data = serde_json::to_value(&status).expect("status serializes to JSON");
        if let Err(error) = self.log.append(kind::TAILNET_CHANGED, None, data) {
            tracing::error!(%error, "could not log the tailnet change");
        }
    }
}

/// Runs the listener until the daemon stops: binds when switched on and
/// Tailscale is usable, retries when it isn't, and rebinds when the settings
/// or this machine's tailnet identity change.
pub(crate) fn spawn(state: AppState, tailscale: Arc<dyn Tailscale>, timing: Timing) -> JoinHandle<()> {
    tokio::spawn(supervise(state, tailscale, timing))
}

async fn supervise(state: AppState, tailscale: Arc<dyn Tailscale>, timing: Timing) {
    let tailnet = state.tailnet.clone();
    let mut settings_changed = tailnet.settings.subscribe();
    let mut stopping = state.stopping.clone();

    loop {
        if *stopping.borrow() {
            return;
        }
        let settings = *settings_changed.borrow_and_update();
        if !settings.enabled {
            tailnet.set_status(TailnetStatus::initial(settings));
            tokio::select! {
                _ = settings_changed.changed() => continue,
                _ = stopped(&mut stopping) => return,
            }
        }

        let (local, listeners, reason) = match bind(tailscale.as_ref(), settings.port).await {
            Bound::Listening {
                local,
                listeners,
                failures,
            } => (local, listeners, failures),
            Bound::Unavailable { reason, local } => {
                tailnet.set_status(TailnetStatus::unavailable(settings, reason, local.as_ref()));
                tokio::select! {
                    _ = tokio::time::sleep(timing.retry) => {}
                    _ = settings_changed.changed() => {}
                    _ = stopped(&mut stopping) => return,
                }
                continue;
            }
        };

        // Streams over these listeners end when this flips, whether the
        // daemon is stopping or the listener is being replaced.
        let (close, closing) = watch::channel(false);
        let guard = Arc::new(Guard::new(
            local.clone(),
            tailscale.clone(),
            state.devices.clone(),
            timing.whois_ttl,
        ));
        let remote_state = AppState {
            stopping: closing.clone(),
            ..state.clone()
        };
        let addresses = listeners
            .iter()
            .filter_map(|listener| listener.local_addr().ok())
            .collect();
        let servers: Vec<_> = listeners
            .into_iter()
            .map(|listener| serve(listener, remote_state.clone(), guard.clone(), closing.clone()))
            .collect();
        let warnings: Vec<String> = reason.into_iter().chain(firewall_warning().await).collect();
        tailnet.set_status(TailnetStatus {
            addresses,
            reason: (!warnings.is_empty()).then(|| warnings.join("; ")),
            host: Some(local.name.clone()),
            owner: Some(local.login_name.clone()),
            ..TailnetStatus::new(settings, ListenerState::Listening)
        });

        let why = loop {
            tokio::select! {
                _ = settings_changed.changed() => break None,
                _ = stopped(&mut stopping) => break None,
                _ = tokio::time::sleep(timing.recheck) => {
                    match tailscale.local().await {
                        Ok(now) if same_identity(&now, &local) => {}
                        Ok(_) => break Some("this machine's tailnet addresses or owner changed".to_owned()),
                        Err(error) => break Some(error.to_string()),
                    }
                }
            }
        };
        let _ = close.send(true);
        for server in servers {
            match tokio::time::timeout(LISTENER_GRACE, server).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => tracing::error!(%error, "tailnet listener failed"),
                Ok(Err(error)) => tracing::error!(%error, "tailnet listener task failed"),
                Err(_) => {
                    tracing::warn!("tailnet connections still open after {LISTENER_GRACE:?}; closing anyway")
                }
            }
        }
        if let Some(reason) = why {
            tailnet.set_status(TailnetStatus::unavailable(settings, reason, Some(&local)));
            tokio::select! {
                _ = tokio::time::sleep(timing.retry) => {}
                _ = settings_changed.changed() => {}
                _ = stopped(&mut stopping) => return,
            }
        }
    }
}

/// The outcome of one attempt to start the listener.
enum Bound {
    /// At least one address is bound. `failures` says which weren't.
    Listening {
        local: LocalNode,
        listeners: Vec<TcpListener>,
        failures: Option<String>,
    },
    /// Nothing is bound: why, and this machine's node if Tailscale answered.
    Unavailable {
        reason: String,
        local: Option<LocalNode>,
    },
}

/// Asks Tailscale who this machine is and binds `port` on each of its
/// tailnet addresses.
async fn bind(tailscale: &dyn Tailscale, port: u16) -> Bound {
    let unavailable = |reason: String, local| Bound::Unavailable { reason, local };
    let local = match tailscale.local().await {
        Ok(local) => local,
        Err(error) => return unavailable(error.to_string(), None),
    };
    if local.tagged {
        return unavailable(
            "this machine is a tagged Tailscale device, so it has no owner to match other devices against"
                .into(),
            Some(local),
        );
    }
    if local.ips.is_empty() {
        return unavailable(
            "Tailscale reports no tailnet address for this machine".into(),
            Some(local),
        );
    }
    let mut listeners = Vec::new();
    let mut failures = Vec::new();
    for ip in &local.ips {
        let addr = SocketAddr::new(*ip, port);
        match TcpListener::bind(addr).await {
            Ok(listener) => listeners.push(listener),
            Err(error) => failures.push(format!("could not listen on {addr}: {error}")),
        }
    }
    let failures = (!failures.is_empty()).then(|| failures.join("; "));
    if listeners.is_empty() {
        return unavailable(failures.unwrap_or_default(), Some(local));
    }
    Bound::Listening {
        local,
        listeners,
        failures,
    }
}

/// Resolves once `stopping` is true. (`wait_for` alone holds a guard that
/// isn't `Send`.)
async fn stopped(stopping: &mut watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stopping| *stopping).await;
}

/// On macOS, says so if the application firewall is on and blocks incoming
/// connections to this binary. The listener then binds fine, but every
/// connection is dropped before cos sees it.
async fn firewall_warning() -> Option<String> {
    const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";
    if !cfg!(target_os = "macos") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let ask = |args: Vec<std::ffi::OsString>| async move {
        let output = tokio::process::Command::new(SOCKETFILTERFW)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(Duration::from_secs(5), output)
            .await
            .ok()?
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let global = ask(vec!["--getglobalstate".into()]).await?;
    if !global.contains("enabled") {
        return None;
    }
    let app = ask(vec!["--getappblocked".into(), exe.clone().into()]).await?;
    firewall_blocks(&app).then(|| {
        format!(
            "the macOS firewall blocks incoming connections to {exe}, so other devices can't connect. \
             Allow it in System Settings > Network > Firewall > Options, or run \
             `sudo {SOCKETFILTERFW} --unblockapp {exe}`",
            exe = exe.display()
        )
    })
}

/// Reads `socketfilterfw --getappblocked`: "Incoming connection to <path> is blocked."
fn firewall_blocks(answer: &str) -> bool {
    answer.trim_end().ends_with("is blocked.")
}

fn same_identity(now: &LocalNode, then: &LocalNode) -> bool {
    now.ips == then.ips && now.user_id == then.user_id && now.login_name == then.login_name && !now.tagged
}

fn serve(
    listener: TcpListener,
    state: AppState,
    guard: Arc<Guard>,
    mut closing: watch::Receiver<bool>,
) -> JoinHandle<std::io::Result<()>> {
    let app = remote_api::remote_router(state, guard).into_make_service_with_connect_info::<SocketAddr>();
    let shutdown = async move {
        let _ = closing.wait_for(|closing| *closing).await;
    };
    tokio::spawn(
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .into_future(),
    )
}

// ---- Who may connect ----

/// Why a remote request was refused: an HTTP status, a stable code for apps
/// and a message for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn forbidden(code: &'static str, message: String) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message,
        }
    }
}

/// Lets a peer through only if it is logged in to Tailscale as the same user
/// as this machine. Tagged devices have no user and are refused.
pub(crate) fn check_owner(local: &LocalNode, addr: SocketAddr, peer: Option<&Peer>) -> Result<(), Refusal> {
    let Some(peer) = peer else {
        return Err(Refusal::forbidden(
            "unknown_peer",
            format!(
                "Tailscale doesn't know who {} is, so cos can't let it in",
                addr.ip()
            ),
        ));
    };
    if peer.tagged {
        return Err(Refusal::forbidden(
            "tagged_device",
            format!(
                "{} is a tagged Tailscale device; cos only lets in devices logged in as the user who owns {}",
                peer.name, local.name
            ),
        ));
    }
    let same_login = local.login_name.is_empty()
        || peer.login_name.is_empty()
        || peer.login_name.eq_ignore_ascii_case(&local.login_name);
    if peer.user_id != local.user_id || !same_login {
        return Err(Refusal::forbidden(
            "wrong_user",
            format!(
                "{} is logged in to Tailscale as a different user; cos only lets in devices logged in as the user who owns {}",
                peer.name, local.name
            ),
        ));
    }
    Ok(())
}

/// Lets an approved device through; a pending or denied one is told how to
/// get approved.
pub(crate) fn check_device(device: &Device, host: &str) -> Result<(), Refusal> {
    match device.state {
        DeviceState::Approved => Ok(()),
        DeviceState::Pending => Err(Refusal::forbidden(
            "device_pending",
            format!(
                "this device ({}) is waiting for approval. On {host}, run `cos devices approve {}`",
                device.name, device.id
            ),
        )),
        DeviceState::Denied => Err(Refusal::forbidden(
            "device_denied",
            format!(
                "this device ({}) was denied. On {host}, run `cos devices approve {}` to let it in",
                device.name, device.id
            ),
        )),
    }
}

/// Checks each remote request. The `whois` answer and owner check are cached
/// per address for a short while; the device's approval is read from the
/// database every time, so approving or denying takes effect at once.
pub(crate) struct Guard {
    local: LocalNode,
    tailscale: Arc<dyn Tailscale>,
    devices: Arc<Devices>,
    ttl: Duration,
    cache: Mutex<HashMap<IpAddr, Cached>>,
}

struct Cached {
    at: Instant,
    /// The peer and its device id, or why it was refused.
    who: Result<(Peer, i64), Refusal>,
}

impl Guard {
    pub(crate) fn new(
        local: LocalNode,
        tailscale: Arc<dyn Tailscale>,
        devices: Arc<Devices>,
        ttl: Duration,
    ) -> Self {
        Self {
            local,
            tailscale,
            devices,
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The approved device behind `addr`, or why it may not come in.
    pub(crate) async fn admit(&self, addr: SocketAddr) -> Result<Device, Refusal> {
        let (peer, device_id) = match self.cached(addr.ip()) {
            Some(who) => who?,
            None => {
                let who = self.identify(addr).await?;
                self.remember(addr.ip(), who.clone());
                who?
            }
        };
        let device = match self.devices.get(device_id).map_err(internal)? {
            Some(device) => device,
            // Removed on the host since we cached it: it starts over as pending.
            None => {
                let device = self.record(&peer, addr)?;
                self.remember(addr.ip(), Ok((peer, device.id)));
                device
            }
        };
        check_device(&device, &self.local.name)?;
        Ok(device)
    }

    fn cached(&self, ip: IpAddr) -> Option<Result<(Peer, i64), Refusal>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .get(&ip)
            .filter(|cached| cached.at.elapsed() < self.ttl)
            .map(|cached| cached.who.clone())
    }

    fn remember(&self, ip: IpAddr, who: Result<(Peer, i64), Refusal>) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|_, cached| cached.at.elapsed() < self.ttl);
        cache.insert(
            ip,
            Cached {
                at: Instant::now(),
                who,
            },
        );
    }

    /// Asks Tailscale who `addr` is and checks the owner. The outer error is
    /// for failures that shouldn't be cached (Tailscale didn't answer); the
    /// inner result is the verdict.
    async fn identify(&self, addr: SocketAddr) -> Result<Result<(Peer, i64), Refusal>, Refusal> {
        let peer = self.tailscale.whois(addr).await.map_err(unavailable)?;
        if let Err(refusal) = check_owner(&self.local, addr, peer.as_ref()) {
            tracing::warn!(%addr, peer = ?peer.as_ref().map(|peer| &peer.name), code = refusal.code, "refused a tailnet client");
            return Ok(Err(refusal));
        }
        let mut peer = peer.expect("check_owner refuses unknown peers");
        // Tailscale leaves out the OS when the peer is this machine.
        if peer.os.is_empty() && peer.node_id == self.local.node_id {
            peer.os = self.local.os.clone();
        }
        let device = self.record(&peer, addr)?;
        Ok(Ok((peer, device.id)))
    }

    fn record(&self, peer: &Peer, addr: SocketAddr) -> Result<Device, Refusal> {
        self.devices
            .seen(&SeenDevice {
                node_id: peer.node_id.clone(),
                name: peer.name.clone(),
                hostname: peer.hostname.clone(),
                os: peer.os.clone(),
                login_name: peer.login_name.clone(),
                ip: addr.ip().to_string(),
            })
            .map_err(internal)
    }
}

fn unavailable(error: TailscaleError) -> Refusal {
    Refusal {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "tailscale_unavailable",
        message: format!("cos couldn't check who you are with Tailscale: {error}"),
    }
}

fn internal(error: impl std::fmt::Display) -> Refusal {
    Refusal {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "internal",
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> LocalNode {
        LocalNode {
            node_id: "nHOST".into(),
            name: "my-mac".into(),
            os: "macOS".into(),
            user_id: 1,
            login_name: "me@example.com".into(),
            tagged: false,
            ips: vec!["100.100.1.1".parse().unwrap()],
        }
    }

    fn peer() -> Peer {
        Peer {
            node_id: "nPHONE".into(),
            name: "phone".into(),
            hostname: "My Phone".into(),
            os: "iOS".into(),
            user_id: 1,
            login_name: "me@example.com".into(),
            tagged: false,
        }
    }

    fn addr() -> SocketAddr {
        "100.100.2.1:50000".parse().unwrap()
    }

    fn code(result: Result<(), Refusal>) -> &'static str {
        result.expect_err("should be refused").code
    }

    #[test]
    fn the_same_user_gets_past_the_owner_check() {
        assert_eq!(check_owner(&local(), addr(), Some(&peer())), Ok(()));
        let shouty = Peer {
            login_name: "Me@Example.com".into(),
            ..peer()
        };
        assert_eq!(check_owner(&local(), addr(), Some(&shouty)), Ok(()));
    }

    #[test]
    fn other_users_unknown_peers_and_tagged_devices_are_refused() {
        let other_id = Peer { user_id: 2, ..peer() };
        let other_login = Peer {
            login_name: "someone@example.com".into(),
            ..peer()
        };
        let tagged = Peer {
            tagged: true,
            ..peer()
        };

        assert_eq!(code(check_owner(&local(), addr(), Some(&other_id))), "wrong_user");
        assert_eq!(
            code(check_owner(&local(), addr(), Some(&other_login))),
            "wrong_user"
        );
        assert_eq!(
            code(check_owner(&local(), addr(), Some(&tagged))),
            "tagged_device"
        );
        assert_eq!(code(check_owner(&local(), addr(), None)), "unknown_peer");
        let refusal = check_owner(&local(), addr(), None).unwrap_err();
        assert_eq!(refusal.status, StatusCode::FORBIDDEN);
        assert!(refusal.message.contains("100.100.2.1"), "{}", refusal.message);
    }

    #[test]
    fn only_approved_devices_get_in() {
        let device = |state| Device {
            id: 4,
            node_id: "nPHONE".into(),
            name: "phone".into(),
            hostname: "My Phone".into(),
            os: "iOS".into(),
            login_name: "me@example.com".into(),
            state,
            last_ip: None,
            first_seen_at: jiff::Timestamp::UNIX_EPOCH,
            last_seen_at: jiff::Timestamp::UNIX_EPOCH,
            decided_at: None,
        };

        assert_eq!(check_device(&device(DeviceState::Approved), "my-mac"), Ok(()));
        let pending = check_device(&device(DeviceState::Pending), "my-mac").unwrap_err();
        assert_eq!(pending.code, "device_pending");
        assert!(
            pending.message.contains("On my-mac, run `cos devices approve 4`"),
            "{}",
            pending.message
        );
        let denied = check_device(&device(DeviceState::Denied), "my-mac").unwrap_err();
        assert_eq!(
            (denied.status, denied.code),
            (StatusCode::FORBIDDEN, "device_denied")
        );
    }

    #[test]
    fn reads_the_macos_firewall_answer() {
        assert!(firewall_blocks(
            "Incoming connection to /Users/me/bin/cos is blocked.\n"
        ));
        assert!(!firewall_blocks(
            "Incoming connection to /Users/me/bin/cos is permitted.\n"
        ));
        assert!(!firewall_blocks(""));
    }

    #[test]
    fn settings_default_to_off_on_port_7700() {
        let settings: TailnetSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(settings.port, 7700);
        assert!(!TailnetSettings::default().enabled);
    }
}
