//! `daemon.env` in the cos home: `NAME=value` lines for secrets the daemon
//! needs, such as `TYPESAFEAI_API_KEY` for jev. `cos daemon install` never
//! puts secrets in the launchd plist or the systemd unit.
//!
//! - On Linux the systemd unit loads it into the daemon's environment
//!   (`EnvironmentFile=`), so agents get its variables too.
//! - On both macOS and Linux the daemon reads it itself at startup
//!   ([`load`]), for its own lookups (the jev key). It refuses a file other
//!   users can read or write.
//! - `cos daemon install` copies `TYPESAFEAI_API_KEY` from your shell into
//!   it ([`store`]), mode 0600.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context;
use cos_judge::key::{EnvFile, EnvFileError, read_env_file, set_in_env_file};

use crate::home::CosHome;

/// Its name in the cos home.
pub(crate) const FILE: &str = "daemon.env";

pub(crate) fn path(home: &CosHome) -> PathBuf {
    home.root().join(FILE)
}

/// Reads the file at startup, logging the names it sets (never the values)
/// or why it wasn't read. A missing file is normal.
pub(crate) fn load(home: &CosHome) -> Result<EnvFile, EnvFileError> {
    let file = read_env_file(&path(home));
    match &file {
        Ok(file) => {
            let names: Vec<&str> = file.names().collect();
            tracing::info!(file = %path(home).display(), names = ?names, "read daemon.env");
        }
        Err(EnvFileError::Missing(_)) => {}
        Err(error) => tracing::warn!("{error}"),
    }
    file
}

/// Sets `name` in the file, creating it if needed, and leaves it mode 0600.
/// Other lines stay as they are. Returns the file's path.
pub(crate) fn store(home: &CosHome, name: &str, value: &str) -> anyhow::Result<PathBuf> {
    let path = path(home);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("could not read {}", path.display())),
    };
    write_private(&path, &set_in_env_file(&existing, name, value))?;
    Ok(path)
}

/// Writes `text` to `path` through a temporary file created mode 0600, so
/// the value is never readable by others, even for a moment.
fn write_private(path: &Path, text: &str) -> anyhow::Result<()> {
    let temporary = path.with_extension("env.tmp");
    let _ = fs::remove_file(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| format!("could not create {}", temporary.display()))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use cos_judge::KEY_ENV;

    use super::*;

    #[test]
    fn stores_the_key_privately_and_keeps_other_lines() {
        let dir = tempfile::tempdir().unwrap();
        let home = CosHome::new(dir.path());
        fs::write(path(&home), "ANTHROPIC_API_KEY=sk-ant\n").unwrap();
        fs::set_permissions(path(&home), fs::Permissions::from_mode(0o644)).unwrap();

        store(&home, KEY_ENV, "sk-one").unwrap();
        store(&home, KEY_ENV, "sk-two").unwrap();

        let mode = fs::metadata(path(&home)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let file = load(&home).unwrap();
        assert_eq!(file.get(KEY_ENV), Some("sk-two"));
        assert_eq!(file.get("ANTHROPIC_API_KEY"), Some("sk-ant"));
        assert_eq!(
            fs::read_to_string(path(&home)).unwrap(),
            "ANTHROPIC_API_KEY=sk-ant\nTYPESAFEAI_API_KEY=sk-two\n"
        );
    }

    #[test]
    fn creates_the_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let home = CosHome::new(dir.path());

        assert!(matches!(load(&home), Err(EnvFileError::Missing(_))));
        store(&home, KEY_ENV, "sk-new").unwrap();

        assert_eq!(load(&home).unwrap().get(KEY_ENV), Some("sk-new"));
    }
}
