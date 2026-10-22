//! `cos daemon install|uninstall`: runs the daemon as a service of the OS,
//! so it starts by itself and again after a crash. That is a launchd agent
//! on macOS ([`launchd`]) and a systemd user unit on Linux ([`systemd`]).
//! Elsewhere, run `cos daemon` under a supervisor of your own.
//!
//! Both modules build everywhere, so their rendering is tested on any OS;
//! only [`manager`] picks one by the OS cos runs on.

mod launchd;
mod systemd;

use std::path::{Path, PathBuf};

use anyhow::bail;

use crate::home::CosHome;

/// Claude's config dir, when it isn't `~/.claude`.
const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";

/// What `install` did.
pub(crate) struct Installed {
    /// The plist or unit file written.
    pub file: PathBuf,
    /// When the daemon starts again by itself, for the closing message.
    pub restarts: &'static str,
    /// Things you may still have to do, such as enabling lingering.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manager {
    Launchd,
    Systemd,
}

/// The service manager on this OS, or why there's none to use.
fn manager() -> anyhow::Result<Manager> {
    if cfg!(target_os = "macos") {
        Ok(Manager::Launchd)
    } else if cfg!(target_os = "linux") {
        systemd::check_available()?;
        Ok(Manager::Systemd)
    } else {
        bail!(
            "`cos daemon install` supports launchd (macOS) and systemd (Linux) only. \
             Run `cos daemon` under your own supervisor instead"
        )
    }
}

/// One line for `cos status`: whether the service is installed, and as what.
pub(crate) fn describe() -> String {
    if cfg!(target_os = "macos") {
        if launchd::is_installed() {
            format!("installed (launchd agent {})", launchd::LABEL)
        } else {
            "not installed".into()
        }
    } else if cfg!(target_os = "linux") {
        match systemd::unit_path() {
            Ok(path) if path.exists() => format!("installed (systemd user unit {})", path.display()),
            _ => "not installed".into(),
        }
    } else {
        "not supported on this OS".into()
    }
}

/// An installed service, as `cos upgrade` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installation {
    /// The plist or unit file.
    pub file: PathBuf,
    /// The binary it runs, or `None` if the file doesn't say.
    pub exe: Option<PathBuf>,
    /// The command that restarts it, for people to run.
    pub restart: String,
}

/// The service installed on this OS, if any.
pub(crate) fn installation() -> Option<Installation> {
    let (file, exe): (PathBuf, fn(&str) -> Option<PathBuf>) = if cfg!(target_os = "macos") {
        (launchd::plist_path().ok()?, launchd::recorded_exe)
    } else if cfg!(target_os = "linux") {
        (systemd::unit_path().ok()?, systemd::recorded_exe)
    } else {
        return None;
    };
    let text = std::fs::read_to_string(&file).ok()?;
    Some(Installation {
        exe: exe(&text),
        file,
        restart: restart_command(),
    })
}

/// Restarts the installed service, so it runs the binary its file names
/// now: `launchctl kickstart -k` or `systemctl --user restart`.
pub(crate) fn restart() -> anyhow::Result<()> {
    match manager()? {
        Manager::Launchd => launchd::restart(),
        Manager::Systemd => systemd::restart(),
    }
}

fn restart_command() -> String {
    if cfg!(target_os = "macos") {
        launchd::restart_command()
    } else {
        systemd::restart_command()
    }
}

/// Installs the service and starts it, replacing any earlier install.
pub(crate) fn install(home: &CosHome) -> anyhow::Result<Installed> {
    let manager = manager()?;
    // Before the daemon starts, so it finds the key straight away.
    let copied = copy_judge_key(home)?;
    let mut installed = install_with(manager, home)?;
    installed.notes.extend(copied);
    Ok(installed)
}

fn install_with(manager: Manager, home: &CosHome) -> anyhow::Result<Installed> {
    match manager {
        Manager::Launchd => Ok(Installed {
            file: launchd::install(home)?,
            restarts: "It starts again at every login.",
            notes: Vec::new(),
        }),
        Manager::Systemd => {
            let (file, notes) = systemd::install(home)?;
            Ok(Installed {
                file,
                restarts: "systemd starts it again after a crash, and at login (or at boot, with lingering on).",
                notes,
            })
        }
    }
}

/// Stops the service and removes it. Returns false if it wasn't installed.
pub(crate) fn uninstall() -> anyhow::Result<bool> {
    match manager() {
        Ok(Manager::Launchd) => launchd::uninstall(),
        Ok(Manager::Systemd) => systemd::uninstall(),
        // Without systemd there is nothing to stop, but a unit file left
        // from earlier can still go.
        Err(_) if cfg!(target_os = "linux") => systemd::remove_unit_file(),
        Err(error) => Err(error),
    }
}

/// What the service manager is called here, for messages.
pub(crate) fn kind() -> &'static str {
    if cfg!(target_os = "macos") {
        "launchd agent"
    } else {
        "systemd user unit"
    }
}

/// Copies jev's API key from this shell into `daemon.env` (mode 0600), when
/// it's set. Secrets never go in the plist or the unit; the daemon reads
/// `daemon.env` itself. Returns a note for the closing message.
fn copy_judge_key(home: &CosHome) -> anyhow::Result<Option<String>> {
    let Some(value) = std::env::var(cos_judge::KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    home.prepare()?;
    let path = crate::daemon_env::store(home, cos_judge::KEY_ENV, value.trim())?;
    Ok(Some(format!(
        "copied {} from this shell into {} (mode 0600), for jev",
        cos_judge::KEY_ENV,
        path.display()
    )))
}

/// What [`passthrough_env`] passes, when set. Never a secret: those go in
/// `daemon.env`.
const PASSTHROUGH: [&str; 6] = [
    CLAUDE_CONFIG_DIR,
    crate::which::CLAUDE_BIN_ENV,
    crate::tailscale::TAILSCALE_ENV,
    crate::daemon::chief::MODEL_ENV,
    crate::daemon::staff::MODEL_ENV,
    crate::daemon::pipeline::RUN_MODEL_ENV,
];

/// The environment the daemon needs that a service manager wouldn't pass on:
/// Claude's config dir, the `claude` and `tailscale` binaries, and the
/// chiefs' and workers' models, those that are set.
fn passthrough_env() -> Vec<(&'static str, String)> {
    PASSTHROUGH
        .into_iter()
        .filter_map(|name| Some((name, std::env::var(name).ok().filter(|value| !value.is_empty())?)))
        .collect()
}

/// Your `PATH`, so the daemon finds `claude`, `git`, `tailscale` and what
/// the workers run, plus `claude`'s folder if it isn't on it.
fn service_path() -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    crate::which::service_path(&std::env::var("PATH").unwrap_or_default(), home.as_deref())
}

/// The binary to install: this one, with symlinks resolved.
fn current_exe() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()?.canonicalize()?)
}

/// Whether `exe` sits in a Cargo build folder, which a rebuild replaces.
pub(crate) fn is_build_output(exe: &Path) -> bool {
    exe.components().any(|part| part.as_os_str() == "target")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jev_key_never_goes_in_the_plist_or_the_unit() {
        assert!(!PASSTHROUGH.contains(&cos_judge::KEY_ENV));
    }
}
