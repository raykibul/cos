//! Where cos keeps its files: `~/.cos`, or `$COS_HOME` if set.

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// macOS limits Unix socket paths to 104 bytes, including the final NUL.
const MAX_SOCKET_PATH: usize = 103;

#[derive(Debug, Clone)]
pub(crate) struct CosHome {
    root: PathBuf,
}

impl CosHome {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn from_env() -> anyhow::Result<Self> {
        if let Some(root) = std::env::var_os("COS_HOME") {
            return Ok(Self::new(root));
        }
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(Self::new(Path::new(&home).join(".cos")))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn socket(&self) -> PathBuf {
        self.root.join("cos.sock")
    }

    /// Where the hooks of the daemon's agents report (see `cos hook`).
    pub(crate) fn hook_socket(&self) -> PathBuf {
        self.root.join("hooks.sock")
    }

    pub(crate) fn database(&self) -> PathBuf {
        self.root.join("cos.db")
    }

    pub(crate) fn lock_file(&self) -> PathBuf {
        self.root.join("daemon.lock")
    }

    /// Where workspace worktrees go.
    pub(crate) fn worktrees(&self) -> PathBuf {
        self.root.join("worktrees")
    }

    /// Where task worktrees go: one per task a worker is on.
    pub(crate) fn tasks(&self) -> PathBuf {
        self.root.join("tasks")
    }

    pub(crate) fn daemon_log(&self) -> PathBuf {
        self.root.join("logs").join("daemon.log")
    }

    /// Creates the directories, readable by the current user only.
    pub(crate) fn prepare(&self) -> anyhow::Result<()> {
        let logs = self.root.join("logs");
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&logs)
            .with_context(|| format!("could not create {}", logs.display()))?;
        // `mode` only applies to directories it creates; tighten an existing one too.
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;

        // The hook socket has the longer name, so it is the one to check.
        let socket = self.hook_socket();
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            bail!(
                "socket path {} is longer than {MAX_SOCKET_PATH} bytes; set COS_HOME to a shorter directory",
                socket.display()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_creates_private_directories() {
        let dir = tempfile::tempdir().unwrap();
        let home = CosHome::new(dir.path().join("cos"));

        home.prepare().unwrap();

        let mode = fs::metadata(home.root()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(home.daemon_log().parent().unwrap().is_dir());
    }

    #[test]
    fn prepare_rejects_a_socket_path_macos_cannot_bind() {
        let dir = tempfile::tempdir().unwrap();
        let home = CosHome::new(dir.path().join("x".repeat(110)));

        let error = home.prepare().unwrap_err();

        assert!(error.to_string().contains("COS_HOME"), "{error}");
    }
}
