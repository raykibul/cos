//! Finding the programs the daemon runs, `claude` and `tailscale`. A daemon
//! started by launchd or systemd can have a shorter `PATH` than your shell,
//! so each also has a few places it is usually installed.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The environment variable naming the `claude` binary agents run.
pub(crate) const CLAUDE_BIN_ENV: &str = "COS_CLAUDE_BIN";

/// The first executable `name` in the folders of `path` (a `PATH` value).
pub(crate) fn find_on_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

pub(crate) fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The `claude` agents run: `$COS_CLAUDE_BIN` if set; else `claude` on
/// `PATH`, kept as the bare name so it is looked up again at every start;
/// else the first of [`claude_places`] that exists; else the bare name, which
/// fails at start with "not found".
pub(crate) fn claude() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_claude(
        std::env::var_os(CLAUDE_BIN_ENV).as_deref(),
        std::env::var_os("PATH").as_deref(),
        home.as_deref(),
    )
}

fn resolve_claude(explicit: Option<&OsStr>, path: Option<&OsStr>, home: Option<&Path>) -> PathBuf {
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    if find_on_path("claude", path).is_some() {
        return PathBuf::from("claude");
    }
    home.into_iter()
        .flat_map(claude_places)
        .find(|candidate| is_executable(candidate))
        .unwrap_or_else(|| PathBuf::from("claude"))
}

/// Where Claude Code's installers put `claude`, most likely first: the
/// native installer (`~/.local/bin`), the old local install
/// (`~/.claude/local`), npm with a user prefix (`~/.npm-global`, as Claude's
/// docs suggest) or under nvm (newest Node first), and Homebrew.
pub(crate) fn claude_places(home: &Path) -> Vec<PathBuf> {
    let mut places = vec![
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        home.join(".npm-global/bin/claude"),
    ];
    let mut nvm: Vec<PathBuf> = std::fs::read_dir(home.join(".nvm/versions/node"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("bin/claude"))
        .collect();
    // Version folders (`v22.11.0`) sort well enough for "newest first".
    nvm.sort();
    places.extend(nvm.into_iter().rev());
    places.extend(["/usr/local/bin/claude", "/opt/homebrew/bin/claude"].map(PathBuf::from));
    places
}

/// The `PATH` to give an installed daemon: `path` (yours, at install time),
/// plus the folder of `claude` if it isn't on `path` but is in one of
/// [`claude_places`], so agents still find it.
pub(crate) fn service_path(path: &str, home: Option<&Path>) -> String {
    if find_on_path("claude", Some(OsStr::new(path))).is_some() {
        return path.to_owned();
    }
    let Some(dir) = home
        .into_iter()
        .flat_map(claude_places)
        .find(|candidate| is_executable(candidate))
        .and_then(|claude| claude.parent().map(Path::to_path_buf))
    else {
        return path.to_owned();
    };
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    dirs.push(dir);
    std::env::join_paths(dirs).map_or_else(
        |_| path.to_owned(),
        |joined| joined.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_env_var_wins() {
        let found = resolve_claude(
            Some(OsStr::new("/opt/claude")),
            Some(OsStr::new("/usr/bin")),
            None,
        );
        assert_eq!(found, PathBuf::from("/opt/claude"));
    }

    #[test]
    fn claude_on_path_stays_a_bare_name() {
        let dir = tempfile::tempdir().unwrap();
        executable(&dir.path().join("bin/claude"));
        let path = dir.path().join("bin");

        let found = resolve_claude(None, Some(path.as_os_str()), Some(dir.path()));

        assert_eq!(found, PathBuf::from("claude"));
    }

    #[test]
    fn falls_back_to_where_installers_put_it() {
        let home = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        executable(&home.path().join(".claude/local/claude"));

        let found = resolve_claude(None, Some(empty.path().as_os_str()), Some(home.path()));

        assert_eq!(found, home.path().join(".claude/local/claude"));
    }

    #[test]
    fn prefers_the_newest_nvm_node() {
        let home = tempfile::tempdir().unwrap();
        executable(&home.path().join(".nvm/versions/node/v20.1.0/bin/claude"));
        executable(&home.path().join(".nvm/versions/node/v22.11.0/bin/claude"));

        let places = claude_places(home.path());
        let first = places.iter().find(|place| is_executable(place)).unwrap();

        assert_eq!(first, &home.path().join(".nvm/versions/node/v22.11.0/bin/claude"));
    }

    #[test]
    fn a_file_that_isnt_executable_doesnt_count() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".local/bin/claude"), "").unwrap();

        let found = resolve_claude(None, Some(OsStr::new("")), Some(home.path()));

        assert_eq!(found, PathBuf::from("claude"));
    }

    #[test]
    fn the_service_path_gains_claudes_folder() {
        let home = tempfile::tempdir().unwrap();
        executable(&home.path().join(".local/bin/claude"));

        let path = service_path("/usr/bin:/bin", Some(home.path()));

        assert_eq!(
            path,
            format!("/usr/bin:/bin:{}", home.path().join(".local/bin").display())
        );
    }

    #[test]
    fn the_service_path_is_unchanged_when_claude_is_on_it() {
        let home = tempfile::tempdir().unwrap();
        executable(&home.path().join(".local/bin/claude"));
        let path = format!("/usr/bin:{}", home.path().join(".local/bin").display());

        assert_eq!(service_path(&path, Some(home.path())), path);
    }
}
