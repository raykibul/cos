//! What `cos upgrade` asks of the machine, behind [`System`] so tests can
//! fake it: downloads (through `curl` or `wget`, since cos has no TLS stack
//! of its own), running `install.sh`, the Mac app, the daemon and its
//! service. [`Real`] does it for real.

use std::cell::OnceCell;
use std::io::{BufRead, ErrorKind, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};
use cos_core::Agent;

use super::Installer;
use super::version::{Version, tag_from_location};
use crate::client::Client;
use crate::service::{self, Installation};

/// Where releases live, unless `COS_DOWNLOAD_BASE` says otherwise (as for
/// `install.sh`).
pub(crate) const DEFAULT_BASE: &str = "https://github.com/raykibul/cos/releases";
/// The app's executable, as `pgrep` sees it. `install.sh` reads the same
/// variable.
const APP_PROCESS_ENV: &str = "COS_APP_PROCESS";

/// The running daemon, as `/v1/status` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonInfo {
    pub version: String,
    pub pid: u32,
}

/// The machine, as `cos upgrade` sees it.
#[allow(async_fn_in_trait)]
pub(crate) trait System {
    /// Prints a line for the user.
    fn say(&self, line: &str);
    /// The newest release's version.
    fn latest(&self) -> anyhow::Result<Version>;
    /// Downloads `install.sh` and runs it as `installer` says, its output
    /// going straight to the terminal.
    fn run_installer(&self, installer: &Installer) -> anyhow::Result<()>;
    /// What `<exe> --version` says, without the leading `cos `.
    fn binary_version(&self, exe: &Path) -> Option<String>;
    /// Whether this user can create files in `dir`.
    fn writable(&self, dir: &Path) -> bool;
    /// The app's `CFBundleShortVersionString`.
    fn app_version(&self, app: &Path) -> Option<String>;
    /// Whether the app is running for this user.
    fn app_running(&self) -> bool;
    fn open_app(&self, app: &Path) -> anyhow::Result<()>;
    /// The daemon's service, if one is installed.
    fn service(&self) -> Option<Installation>;
    fn restart_service(&self) -> anyhow::Result<()>;
    /// The local daemon, if it answers.
    async fn daemon(&self) -> Option<DaemonInfo>;
    async fn agents(&self) -> anyhow::Result<Vec<Agent>>;
    /// Whether there's someone at a terminal to ask.
    fn interactive(&self) -> bool;
    /// Asks a yes/no question; anything but yes is no.
    fn confirm(&self, question: &str) -> bool;
    /// Waits a moment between checks on a restarting daemon.
    async fn pause(&self);
}

pub(crate) struct Real {
    client: Client,
    /// `COS_DOWNLOAD_BASE`, else GitHub.
    base: String,
    /// `COS_INSTALL_SCRIPT`: a path or URL to use instead of the tag's
    /// `install.sh`.
    script_override: Option<String>,
    /// The script, downloaded once for both the CLI and the app.
    script: OnceCell<Vec<u8>>,
}

impl Real {
    pub(crate) fn new(socket: PathBuf) -> Self {
        let env = |name| {
            std::env::var(name)
                .ok()
                .filter(|value: &String| !value.is_empty())
        };
        Self {
            client: Client::new(socket),
            base: env("COS_DOWNLOAD_BASE").unwrap_or_else(|| DEFAULT_BASE.to_owned()),
            script_override: env("COS_INSTALL_SCRIPT"),
            script: OnceCell::new(),
        }
    }
}

impl System for Real {
    fn say(&self, line: &str) {
        println!("{line}");
    }

    fn latest(&self) -> anyhow::Result<Version> {
        let base = self.base.trim_end_matches('/');
        // A local release folder (the tests') has no redirect to read, so it
        // names the latest tag in a file.
        if let Some(dir) = base.strip_prefix("file://") {
            let file = Path::new(dir).join("latest/tag");
            let tag = std::fs::read_to_string(&file)
                .with_context(|| format!("could not read the latest tag from {}", file.display()))?;
            return Version::parse(&tag).with_context(|| format!("{} holds no version", file.display()));
        }
        let url = format!("{base}/latest");
        let headers = head(&url)?;
        tag_from_location(&headers).with_context(|| format!("{url} doesn't redirect to a release tag"))
    }

    fn run_installer(&self, installer: &Installer) -> anyhow::Result<()> {
        let script = match self.script.get() {
            Some(script) => script,
            None => {
                let source = installer.script_source(self.script_override.as_deref());
                let script = if source.contains("://") {
                    download(&source)?
                } else {
                    std::fs::read(&source).with_context(|| format!("could not read {source}"))?
                };
                self.script.get_or_init(|| script)
            }
        };
        let mut child = installer
            .command()
            .stdin(Stdio::piped())
            .spawn()
            .context("could not run sh")?;
        let written = child.stdin.take().expect("stdin is piped").write_all(script);
        let status = child.wait().context("could not wait for the installer")?;
        if !status.success() {
            bail!("the installer failed ({status})");
        }
        written.context("could not pass install.sh to sh")
    }

    fn binary_version(&self, exe: &Path) -> Option<String> {
        let output = Command::new(exe).arg("--version").output().ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        Some(text.strip_prefix("cos ").unwrap_or(text).to_owned())
    }

    fn writable(&self, dir: &Path) -> bool {
        // Tried for real: permission bits miss ACLs, read-only mounts and root.
        let probe = dir.join(format!(".cos-upgrade-probe.{}", std::process::id()));
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .is_ok();
        if created {
            let _ = std::fs::remove_file(&probe);
        }
        created
    }

    fn app_version(&self, app: &Path) -> Option<String> {
        let plist = app.join("Contents/Info.plist");
        let output = Command::new("plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
            .arg(plist)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn app_running(&self) -> bool {
        let process = std::env::var(APP_PROCESS_ENV)
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "cos-desktop".to_owned());
        let uid = rustix::process::getuid().as_raw().to_string();
        Command::new("pgrep")
            .args(["-x", "-U", &uid, &process])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn open_app(&self, app: &Path) -> anyhow::Result<()> {
        let status = Command::new("open")
            .arg(app)
            .status()
            .context("could not run open")?;
        if !status.success() {
            bail!("`open {}` failed ({status})", app.display());
        }
        Ok(())
    }

    fn service(&self) -> Option<Installation> {
        service::installation()
    }

    fn restart_service(&self) -> anyhow::Result<()> {
        service::restart()
    }

    async fn daemon(&self) -> Option<DaemonInfo> {
        let status = tokio::time::timeout(Duration::from_secs(3), self.client.status())
            .await
            .ok()?
            .ok()?;
        Some(DaemonInfo {
            version: status.version,
            pid: status.pid,
        })
    }

    async fn agents(&self) -> anyhow::Result<Vec<Agent>> {
        self.client.agents(None).await
    }

    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn confirm(&self, question: &str) -> bool {
        print!("{question} ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }

    async fn pause(&self) {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Runs a download tool, `curl` or else `wget`, with the arguments for it.
/// Returns which one ran and its output.
fn tool(curl: &[&str], wget: &[&str]) -> anyhow::Result<(&'static str, std::process::Output)> {
    for (program, args) in [("curl", curl), ("wget", wget)] {
        match Command::new(program).args(args).stdin(Stdio::null()).output() {
            Ok(output) => return Ok((program, output)),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("could not run {program}")),
        }
    }
    bail!("`cos upgrade` needs curl or wget")
}

/// Downloads `url` (`file://` too) into memory.
fn download(url: &str) -> anyhow::Result<Vec<u8>> {
    let (_, output) = tool(&["-fsSL", "--retry", "3", url], &["-q", "-O", "-", url])?;
    if !output.status.success() {
        bail!(
            "could not download {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// The response headers for `url`, without following redirects.
fn head(url: &str) -> anyhow::Result<String> {
    let (program, output) = tool(
        &["-fsSI", "--retry", "3", url],
        &["-S", "--spider", "--max-redirect=0", url],
    )?;
    if program == "curl" && !output.status.success() {
        bail!(
            "could not reach {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    // wget writes headers to stderr, and fails on the redirect it won't follow.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if text.trim().is_empty() {
        bail!("could not reach {url}");
    }
    Ok(text)
}
