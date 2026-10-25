//! `cos daemon`: the long-running process. It owns the database and the
//! event log and serves the API on `<home>/cos.sock`, and on the tailnet when
//! remote access is on (see [`tailnet`]). It runs agents in terminals (see
//! [`agents`]).

pub(crate) mod agents;
pub(crate) mod agents_api;
pub(crate) mod api;
pub(crate) mod attachments;
pub(crate) mod attachments_api;
pub(crate) mod env_api;
pub(crate) mod judge;
pub(crate) mod judge_api;
pub(crate) mod peers;
pub(crate) mod remote_api;
pub(crate) mod shells;
pub(crate) mod staff;
pub(crate) mod staff_api;
pub(crate) mod tailnet;
#[cfg(test)]
mod tests;
pub(crate) mod transcript;

use std::fs::{self, File, TryLockError};
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use cos_core::events::kind;
use cos_core::{Agents, Devices, Environment, EventLog, Projects, Store};
use jiff::Timestamp;
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use self::agents::{AgentConfig, AgentManager};
use crate::home::CosHome;
use crate::tailscale::{Tailscale, TailscaleCli};

/// How long open connections get to finish once the daemon is stopping.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Runs the daemon until a stop signal (SIGINT or SIGTERM) arrives.
pub(crate) async fn run(home: &CosHome) -> anyhow::Result<()> {
    run_until(home, stop_signal()).await
}

/// Runs the daemon until `stop` resolves to the reason for stopping.
pub(crate) async fn run_until(home: &CosHome, stop: impl Future<Output = String>) -> anyhow::Result<()> {
    run_with(home, Arc::new(TailscaleCli), tailnet::Timing::default(), stop).await
}

/// Runs the daemon with the given Tailscale (a fake in tests) until `stop`
/// resolves to the reason for stopping.
pub(crate) async fn run_with(
    home: &CosHome,
    tailscale: Arc<dyn Tailscale>,
    timing: tailnet::Timing,
    stop: impl Future<Output = String>,
) -> anyhow::Result<()> {
    home.prepare()?;
    // Held until this function returns; the OS releases it if we crash.
    let _lock = lock_single_instance(home)?;

    let store = Store::open(&home.database())
        .with_context(|| format!("could not open {}", home.database().display()))?;
    let store = Arc::new(store);
    // On Linux systemd loads it into our environment as well; on macOS
    // launchd doesn't, so it's read here on both. jev looks for its key in it.
    let _ = crate::daemon_env::load(home);
    let log = EventLog::new(store.clone());
    let projects = Arc::new(Projects::new(store.clone(), log.clone(), home.worktrees()));
    let devices = Arc::new(Devices::new(store.clone(), log.clone()));
    let agents = AgentManager::start(
        Agents::new(store.clone(), log.clone()),
        projects.clone(),
        Environment::new(store.clone(), log.clone()),
        AgentConfig::for_home(home)?,
    )?;
    let tailnet = tailnet::Tailnet::load(store.clone(), log.clone());
    let judge = judge::Judge::start(
        store.clone(),
        log.clone(),
        projects.clone(),
        agents.clone(),
        judge::JudgeConfig::for_home(home),
    )?;
    let staff = staff::Staff::start(
        store,
        log.clone(),
        projects.clone(),
        agents.clone(),
        staff::StaffConfig::from_env(),
    )?;

    let socket = home.socket();
    // We hold the lock, so any socket file left behind is from a dead daemon.
    if socket.exists() {
        fs::remove_file(&socket).with_context(|| format!("could not remove stale {}", socket.display()))?;
    }
    let listener =
        UnixListener::bind(&socket).with_context(|| format!("could not listen on {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

    let (stopping_tx, stopping) = watch::channel(false);
    let shells = shells::Shells::new();
    let state = api::AppState {
        shells: shells.clone(),
        log: log.clone(),
        projects,
        devices,
        tailnet,
        peers: peers::Peers::new(tailscale.clone()),
        agents: agents.clone(),
        staff,
        judge,
        started_at: Timestamp::now(),
        home: home.root().to_path_buf(),
        stopping: stopping.clone(),
    };
    let server = serve(listener, state.clone(), stopping);

    log.append(
        kind::DAEMON_STARTED,
        None,
        json!({ "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") }),
    )?;
    tracing::info!(socket = %socket.display(), "cos daemon started");
    // Chiefs of Staff the restart interrupted come back, one a second.
    state.staff.resume_chiefs();
    let remote = tailnet::spawn(state, tailscale, timing);

    let reason = stop.await;
    tracing::info!(%reason, "cos daemon stopping");
    let _ = stopping_tx.send(true);
    // Bounded: agents that don't exit in time are killed. They end up
    // `interrupted`, ready to resume.
    agents.shutdown().await;
    shells.close_all();
    match tokio::time::timeout(SHUTDOWN_GRACE, server).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => tracing::error!(%error, "server failed"),
        Ok(Err(error)) => tracing::error!(%error, "server task failed"),
        Err(_) => tracing::warn!("connections still open after {SHUTDOWN_GRACE:?}; stopping anyway"),
    }
    // It closes its own listeners once `stopping` flips.
    if tokio::time::timeout(SHUTDOWN_GRACE, remote).await.is_err() {
        tracing::warn!("tailnet listener still closing after {SHUTDOWN_GRACE:?}; stopping anyway");
    }

    log.append(kind::DAEMON_STOPPED, None, json!({ "reason": reason }))?;
    let _ = fs::remove_file(&socket);
    Ok(())
}

/// Serves the API until `stopping` flips to true.
pub(crate) fn serve(
    listener: UnixListener,
    state: api::AppState,
    mut stopping: watch::Receiver<bool>,
) -> JoinHandle<std::io::Result<()>> {
    let shutdown = async move {
        let _ = stopping.wait_for(|stopping| *stopping).await;
    };
    let server = axum::serve(listener, api::router(state)).with_graceful_shutdown(shutdown);
    tokio::spawn(server.into_future())
}

/// Takes an exclusive lock on `<home>/daemon.lock` and writes our pid in it.
fn lock_single_instance(home: &CosHome) -> anyhow::Result<File> {
    let path = home.lock_file();
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("could not open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let pid = fs::read_to_string(&path).unwrap_or_default();
            bail!("another cos daemon is already running (pid {})", pid.trim());
        }
        Err(TryLockError::Error(error)) => {
            return Err(error).with_context(|| format!("could not lock {}", path.display()));
        }
    }
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    Ok(file)
}

async fn stop_signal() -> String {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).expect("can listen for SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => "SIGINT".into(),
        _ = terminate.recv() => "SIGTERM".into(),
    }
}
