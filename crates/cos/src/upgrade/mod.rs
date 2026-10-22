//! `cos upgrade`: replaces this `cos` with a newer release, upgrades the Mac
//! app when it's installed, and restarts the daemon's service on the new
//! binary.
//!
//! The downloading and checking is `install.sh`'s: cos fetches the script
//! for the release's tag and runs it with `sh`, with `COS_INSTALL_DIR` set
//! to this binary's folder (`--app` and `COS_APP_DIR` for the app). Its
//! checksum check stays in one place, and cos needs no TLS stack. Everything
//! that touches the machine goes through [`System`], so the decisions here
//! are tested against a fake.

mod system;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, bail};
use cos_core::{Agent, AgentState};

use self::system::{DaemonInfo, System};
use self::version::Version;
use crate::home::CosHome;
use crate::service::Installation;

/// Where `install.sh` is fetched from, at the release's tag.
const RAW_BASE: &str = "https://raw.githubusercontent.com/raykibul/cos";
/// The installer's one-liner, for advice.
const ONE_LINER: &str = "curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh";
/// How long to wait for the restarted daemon: this many pauses of 250 ms.
const RESTART_CHECKS: usize = 80;
const RESTART_QUESTION: &str =
    "Restart the daemon now? Running agents are interrupted; Chiefs of Staff resume by themselves. [y/N]";

/// `cos upgrade`'s options. Its help text is on `Command::Upgrade` in
/// `main.rs`.
#[derive(clap::Args, Debug, Default)]
pub(crate) struct UpgradeArgs {
    /// Show the installed and the latest version, and change nothing.
    #[arg(long)]
    check: bool,
    /// Install this release instead of the latest, such as v0.1.5 (older ones too).
    #[arg(long, value_name = "vX.Y.Z")]
    version: Option<String>,
    /// Restart the daemon without asking, even with agents at work.
    #[arg(long, short = 'y')]
    yes: bool,
    /// Leave the daemon running the old version.
    #[arg(long)]
    no_restart: bool,
    /// Leave the Mac app alone.
    #[arg(long)]
    no_app: bool,
    /// Replace this cos even though it was built from source.
    #[arg(long)]
    force: bool,
}

pub(crate) async fn upgrade(home: &CosHome, args: UpgradeArgs) -> anyhow::Result<ExitCode> {
    let settings = Settings::from_env(home)?;
    run(&system::Real::new(home.socket()), &settings, &args).await
}

/// What `cos upgrade` works on, read once from the environment.
struct Settings {
    /// This binary, with symlinks resolved: what gets replaced.
    exe: PathBuf,
    /// This binary's version.
    current: Version,
    /// Where to look for `cos.app`, in order. Empty off macOS.
    app_dirs: Vec<PathBuf>,
    /// Running as root under `sudo`: the daemon and the app are the user's,
    /// so they're left alone.
    sudo: bool,
    /// The daemon's log, for when it doesn't come back.
    daemon_log: PathBuf,
}

impl Settings {
    fn from_env(home: &CosHome) -> anyhow::Result<Self> {
        let exe = std::env::current_exe()
            .and_then(|exe| exe.canonicalize())
            .context("could not find this cos binary")?;
        let mut app_dirs = Vec::new();
        if cfg!(target_os = "macos") {
            // As install.sh: COS_APP_DIR stands in for /Applications.
            let system = std::env::var_os("COS_APP_DIR").filter(|dir| !dir.is_empty());
            app_dirs.push(system.map_or_else(|| PathBuf::from("/Applications"), PathBuf::from));
            if let Some(home) = std::env::var_os("HOME") {
                app_dirs.push(Path::new(&home).join("Applications"));
            }
            app_dirs.dedup();
        }
        Ok(Self {
            exe,
            current: Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver"),
            app_dirs,
            sudo: rustix::process::getuid().is_root()
                && std::env::var_os("SUDO_UID").is_some_and(|uid| !uid.is_empty()),
            daemon_log: home.daemon_log(),
        })
    }
}

/// One run of `install.sh`: the CLI into a folder, or the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installer {
    version: Version,
    args: Vec<&'static str>,
    env: Vec<(&'static str, String)>,
}

impl Installer {
    /// The CLI, `version`, into `dir`. The installer's hint about restarting
    /// the service is left out: `cos upgrade` sees to that itself.
    fn cli(version: &Version, dir: &Path) -> Self {
        Self {
            version: version.clone(),
            args: Vec::new(),
            env: vec![
                ("COS_VERSION", version.tag()),
                ("COS_INSTALL_DIR", dir.to_string_lossy().into_owned()),
                ("COS_NO_SERVICE_HINT", "1".to_owned()),
            ],
        }
    }

    /// The Mac app, `version`, as `<dir>/cos.app`.
    fn app(version: &Version, dir: &Path) -> Self {
        Self {
            version: version.clone(),
            args: vec!["--app"],
            env: vec![
                ("COS_VERSION", version.tag()),
                ("COS_APP_DIR", dir.to_string_lossy().into_owned()),
            ],
        }
    }

    /// Where to get `install.sh`: `COS_INSTALL_SCRIPT` (a path or URL) when
    /// set, else the script at the release's tag, so it matches its release.
    pub(crate) fn script_source(&self, script_override: Option<&str>) -> String {
        script_override.map_or_else(
            || format!("{RAW_BASE}/{}/install.sh", self.version.tag()),
            str::to_owned,
        )
    }

    /// `sh -s -- <args>`, with the settings in its environment. The script
    /// goes on its stdin, as with the one-liner.
    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new("sh");
        command.args(["-s", "--"]).args(&self.args);
        command.envs(self.env.iter().map(|(name, value)| (name, value)));
        command
    }
}

async fn run<S: System>(sys: &S, settings: &Settings, args: &UpgradeArgs) -> anyhow::Result<ExitCode> {
    let current = &settings.current;
    let pinned = args.version.is_some();
    let target = match &args.version {
        Some(text) => {
            Version::parse(text).with_context(|| format!("`{text}` isn't a version such as v0.1.5"))?
        }
        None => sys.latest().context("could not find the latest cos release")?,
    };
    // Without --version, only ever upgrade; with it, go wherever it says.
    let wanted = |have: &Version| if pinned { *have != target } else { *have < target };
    let app = if args.no_app {
        None
    } else {
        find_app(&settings.app_dirs).map(|path| {
            let version = sys.app_version(&path);
            (path, version)
        })
    };

    if args.check {
        return check(sys, settings, &target, pinned, app.as_ref()).await;
    }

    let mut summary = Vec::new();
    let installed = if wanted(current) {
        check_location(sys, settings, args.force)?;
        let exe = &settings.exe;
        sys.say(&format!(
            "Upgrading cos {current} → {target} at {}",
            exe.display()
        ));
        let dir = exe.parent().context("this cos has no folder")?;
        sys.run_installer(&Installer::cli(&target, dir))?;
        let reported = sys.binary_version(exe);
        let installed = reported
            .as_deref()
            .and_then(Version::parse)
            .with_context(|| format!("{} doesn't run after the upgrade", exe.display()))?;
        if installed != target {
            bail!(
                "installed {} but it says it's cos {installed}, not {target}",
                exe.display()
            );
        }
        summary.push(format!("cos {current} → {installed}"));
        installed
    } else {
        if *current == target {
            sys.say(&format!("cos {current} is up to date"));
        } else {
            sys.say(&format!(
                "cos {current} is newer than the latest release ({target}); nothing to install"
            ));
        }
        current.clone()
    };

    let mut ok = true;
    if settings.sudo {
        sys.say("Under sudo, the daemon and the Mac app are left alone: run `cos upgrade` again without sudo for those.");
    } else {
        if let Some((path, version)) = &app {
            ok &= upgrade_app(sys, path, version.as_deref(), &target, pinned, &mut summary);
        }
        ok &= restart_daemon(sys, settings, args, &installed, &mut summary).await;
    }

    if !summary.is_empty() {
        sys.say("");
        for line in &summary {
            sys.say(line);
        }
    }
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// `cos upgrade --check`: what's installed and what would be.
async fn check<S: System>(
    sys: &S,
    settings: &Settings,
    target: &Version,
    pinned: bool,
    app: Option<&(PathBuf, Option<String>)>,
) -> anyhow::Result<ExitCode> {
    let current = &settings.current;
    let wanted = |have: &Version| if pinned { have != target } else { have < target };
    let mut behind = wanted(current);
    sys.say(&format!("installed  cos {current} at {}", settings.exe.display()));
    sys.say(&format!(
        "{}  cos {target}",
        if pinned { "wanted   " } else { "latest   " }
    ));
    if let Some((path, version)) = app {
        sys.say(&format!(
            "app        {} at {}",
            version.as_deref().unwrap_or("(unknown version)"),
            path.display()
        ));
        behind |= version
            .as_deref()
            .and_then(Version::parse)
            .is_none_or(|have| wanted(&have));
    }
    if let Some(daemon) = sys.daemon().await {
        sys.say(&format!("daemon     {} (pid {})", daemon.version, daemon.pid));
    }
    if behind {
        let flag = if pinned {
            format!(" --version {}", target.tag())
        } else {
            String::new()
        };
        sys.say(&format!("Run `cos upgrade{flag}` to install it."));
    } else {
        sys.say("Up to date.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Refuses to replace a cos that a rebuild would overwrite anyway, one the
/// installer wouldn't write (it only writes `cos`), or one in a folder this
/// user can't write to.
fn check_location<S: System>(sys: &S, settings: &Settings, force: bool) -> anyhow::Result<()> {
    let exe = &settings.exe;
    if !force && let Some(checkout) = source_checkout(exe) {
        bail!(
            "This cos was built from source at {}; update it with `git pull && make build` \
             (or pass --force to replace it with a release build)",
            checkout.display()
        );
    }
    if exe.file_name().is_none_or(|name| name != "cos") {
        bail!(
            "{} isn't named `cos`, and the installer only writes `cos`. Install it with: {ONE_LINER} | sh",
            exe.display()
        );
    }
    let dir = exe.parent().context("this cos has no folder")?;
    if !sys.writable(dir) {
        bail!(
            "{} isn't writable by you, so this cos can't be replaced. Run `sudo cos upgrade`, \
             or install a copy of your own: {ONE_LINER} | COS_INSTALL_DIR=$HOME/.local/bin sh",
            dir.display()
        );
    }
    Ok(())
}

/// Where this cos was built from source, if it was: the folder holding the
/// Cargo `target/` it sits in, or a git checkout of a Cargo project.
fn source_checkout(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().skip(1).find_map(|dir| {
        if dir.file_name().is_some_and(|name| name == "target") {
            Some(dir.parent().unwrap_or(dir).to_path_buf())
        } else if dir.join(".git").exists() && dir.join("Cargo.toml").is_file() {
            Some(dir.to_path_buf())
        } else {
            None
        }
    })
}

/// The first `cos.app` in `dirs`.
fn find_app(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join("cos.app"))
        .find(|app| app.is_dir())
}

/// Upgrades the app at `app` to `target`, unless it's there already or
/// can't be written. Reopens it if it was running, since the installer
/// quits it. Returns false if the upgrade failed.
fn upgrade_app<S: System>(
    sys: &S,
    app: &Path,
    version: Option<&str>,
    target: &Version,
    pinned: bool,
    summary: &mut Vec<String>,
) -> bool {
    let have = version.and_then(Version::parse);
    let needed = have
        .as_ref()
        .is_none_or(|have| if pinned { have != target } else { have < target });
    let old = version.unwrap_or("(unknown version)");
    if !needed {
        sys.say(&format!("The cos app {old} at {} is up to date", app.display()));
        return true;
    }
    let contents = app.join("Contents");
    if !sys.writable(app) || (contents.is_dir() && !sys.writable(&contents)) {
        sys.say(&format!(
            "{} isn't writable by you, so the app stays at {old}. To upgrade it: {ONE_LINER} | sudo sh -s -- --app",
            app.display()
        ));
        return true;
    }
    let Some(dir) = app.parent() else {
        return true;
    };
    let was_running = sys.app_running();
    sys.say(&format!(
        "Upgrading the cos app {old} → {target} at {}",
        app.display()
    ));
    if let Err(error) = sys.run_installer(&Installer::app(target, dir)) {
        sys.say(&format!("The app wasn't upgraded: {error:#}"));
        if was_running {
            let _ = sys.open_app(app);
        }
        return false;
    }
    let new = sys.app_version(app).unwrap_or_else(|| target.to_string());
    let mut line = format!("app {old} → {new} at {}", app.display());
    if was_running {
        match sys.open_app(app) {
            Ok(()) => line.push_str(", reopened"),
            Err(error) => sys.say(&format!(
                "Could not reopen the app ({error:#}); open it with: open '{}'",
                app.display()
            )),
        }
    }
    summary.push(line);
    true
}

/// The daemon, as far as restarting it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonState {
    NotRunning,
    /// It runs the version just installed.
    Current,
    Outdated,
}

/// The daemon's service, as far as restarting it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceState {
    None,
    /// It runs the binary just upgraded.
    ThisBinary,
    OtherBinary,
}

#[derive(Debug, Clone, Copy)]
struct Situation {
    no_restart: bool,
    daemon: DaemonState,
    service: ServiceState,
    /// Agents doing something a restart would interrupt.
    busy: usize,
    yes: bool,
    interactive: bool,
}

/// What to do about the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restart {
    /// `--no-restart`.
    Skip,
    /// Not running, or already on the new version.
    Nothing,
    /// Running, but not as a service: say so.
    Unmanaged,
    /// The service runs another binary: say so, leave it.
    OtherBinary,
    Now,
    /// Agents are at work: ask first.
    Ask,
    /// Agents are at work and there's nobody to ask: print the command.
    NoTerminal,
}

fn decide(situation: &Situation) -> Restart {
    if situation.no_restart {
        return Restart::Skip;
    }
    match (situation.daemon, situation.service) {
        (DaemonState::NotRunning | DaemonState::Current, _) => Restart::Nothing,
        (DaemonState::Outdated, ServiceState::None) => Restart::Unmanaged,
        (DaemonState::Outdated, ServiceState::OtherBinary) => Restart::OtherBinary,
        (DaemonState::Outdated, ServiceState::ThisBinary) if situation.busy == 0 || situation.yes => {
            Restart::Now
        }
        (DaemonState::Outdated, ServiceState::ThisBinary) if situation.interactive => Restart::Ask,
        (DaemonState::Outdated, ServiceState::ThisBinary) => Restart::NoTerminal,
    }
}

/// An agent a restart would interrupt mid-work: anything not idle or ended.
fn is_busy(state: AgentState) -> bool {
    !matches!(
        state,
        AgentState::Idle | AgentState::Exited | AgentState::Interrupted
    )
}

/// Whether the service's recorded binary is `exe` (already canonical).
fn same_binary(recorded: &Path, exe: &Path) -> bool {
    recorded == exe || recorded.canonicalize().is_ok_and(|recorded| recorded == exe)
}

/// Restarts the daemon's service on `new` when it runs this binary and
/// nothing stops it. Returns false if a restart failed.
async fn restart_daemon<S: System>(
    sys: &S,
    settings: &Settings,
    args: &UpgradeArgs,
    new: &Version,
    summary: &mut Vec<String>,
) -> bool {
    if args.no_restart {
        sys.say("Not restarting the daemon (--no-restart).");
        return true;
    }
    let daemon = sys.daemon().await;
    let daemon_state = match &daemon {
        None => DaemonState::NotRunning,
        Some(info) if Version::parse(&info.version).as_ref() == Some(new) => DaemonState::Current,
        Some(_) => DaemonState::Outdated,
    };
    let service = sys.service();
    let service_state = match &service {
        None => ServiceState::None,
        Some(Installation { exe: Some(exe), .. }) if same_binary(exe, &settings.exe) => {
            ServiceState::ThisBinary
        }
        Some(_) => ServiceState::OtherBinary,
    };
    let mut busy = Vec::new();
    let mut busy_unknown = false;
    if daemon_state == DaemonState::Outdated && service_state == ServiceState::ThisBinary && !args.yes {
        match sys.agents().await {
            Ok(agents) => busy = agents.into_iter().filter(|agent| is_busy(agent.state)).collect(),
            Err(error) => {
                sys.say(&format!("Could not list the daemon's agents: {error:#}"));
                busy_unknown = true;
            }
        }
    }
    let decision = decide(&Situation {
        no_restart: args.no_restart,
        daemon: daemon_state,
        service: service_state,
        busy: busy.len() + usize::from(busy_unknown),
        yes: args.yes,
        interactive: sys.interactive(),
    });

    // `decide` only says more than Nothing with a daemon running, and only
    // restarts or asks with a service.
    let (Some(daemon), Some(service)) = (daemon.as_ref(), service.as_ref()) else {
        if let (Restart::Unmanaged, Some(daemon)) = (decision, &daemon) {
            sys.say(&format!(
                "A cos daemon (pid {}) is running {}, not as a service: restart it to use the new version ({new}).",
                daemon.pid, daemon.version
            ));
        }
        return true;
    };
    match decision {
        Restart::Skip | Restart::Nothing | Restart::Unmanaged => true,
        Restart::OtherBinary => {
            let other = service
                .exe
                .as_deref()
                .map_or_else(|| "another binary".to_owned(), |exe| exe.display().to_string());
            sys.say(&format!(
                "The daemon's service ({}) runs {other}, not this cos, so the daemon stays on {}. \
                 To run this cos as the service instead: {} daemon install",
                service.file.display(),
                daemon.version,
                settings.exe.display()
            ));
            true
        }
        Restart::NoTerminal => {
            list_busy(sys, &busy);
            sys.say(&format!(
                "Not restarting the daemon, which is still on {}: agents are at work and there's no terminal to ask. \
                 Restart it with `{}`, or run `cos upgrade --yes`.",
                daemon.version, service.restart
            ));
            true
        }
        Restart::Ask => {
            list_busy(sys, &busy);
            if sys.confirm(RESTART_QUESTION) {
                restart(sys, settings, service, new, summary).await
            } else {
                sys.say(&format!(
                    "Not restarted: the daemon stays on {} until it restarts. Restart it with `{}`.",
                    daemon.version, service.restart
                ));
                true
            }
        }
        Restart::Now => restart(sys, settings, service, new, summary).await,
    }
}

fn list_busy<S: System>(sys: &S, busy: &[Agent]) {
    if busy.is_empty() {
        return;
    }
    sys.say("Agents at work, which a restart interrupts:");
    for agent in busy {
        let state = serde_json::to_value(agent.state)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        let role = serde_json::to_value(agent.role)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        sys.say(&format!("  #{} {} ({role}): {state}", agent.id, agent.title));
    }
}

/// Restarts the service and waits up to 20 s for the daemon to answer on
/// `new`.
async fn restart<S: System>(
    sys: &S,
    settings: &Settings,
    service: &Installation,
    new: &Version,
    summary: &mut Vec<String>,
) -> bool {
    sys.say(&format!("Restarting the daemon: {}", service.restart));
    if let Err(error) = sys.restart_service() {
        sys.say(&format!(
            "Could not restart the daemon: {error:#}. Restart it with `{}`.",
            service.restart
        ));
        return false;
    }
    for _ in 0..RESTART_CHECKS {
        if let Some(DaemonInfo { version, pid }) = sys.daemon().await
            && Version::parse(&version).as_ref() == Some(new)
        {
            summary.push(format!("daemon restarted on {new} (pid {pid})"));
            return true;
        }
        sys.pause().await;
    }
    sys.say(&format!(
        "The daemon isn't answering on {new} 20 s after the restart. Check `cos status` and {}.",
        settings.daemon_log.display()
    ));
    false
}

#[cfg(test)]
mod tests;
