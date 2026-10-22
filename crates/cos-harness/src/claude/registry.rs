//! The Claude sessions running on this machine, including ones cos didn't
//! start, read from the files Claude Code keeps for itself.
//!
//! Each interactive `claude` writes `<config dir>/sessions/<pid>.json` with its
//! session id, folder and whether it is busy or idle. The config dir is
//! `$CLAUDE_CONFIG_DIR`, else `~/.claude`. The format is undocumented, so
//! everything is parsed leniently and unknown fields are ignored. A file can
//! outlive its process, so [`live_sessions`] checks the pid and its start time.
//! The `.key` files next to them are secrets and are never opened.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    /// `interactive`, or another kind such as a headless run.
    pub kind: Option<String>,
    /// The name Claude shows for the session, such as `cos-6e`.
    pub name: Option<String>,
    /// `busy` while working, `idle` while waiting for the user.
    pub status: Option<String>,
    /// Claude Code's version.
    pub version: Option<String>,
    pub entrypoint: Option<String>,
    pub started_at: Option<i64>,
    /// When the status last changed, in Unix milliseconds.
    pub status_updated_at: Option<i64>,
    pub updated_at: Option<i64>,
    /// The process start time as `TZ=UTC ps -o lstart` prints it, to tell a
    /// live session from a dead one whose pid was reused.
    pub proc_start: Option<String>,
    /// The config dir this entry came from. Not part of the file.
    #[serde(skip)]
    pub config_dir: PathBuf,
}

impl RunningSession {
    /// The session's transcript, if it has been written yet.
    pub fn transcript(&self) -> Option<PathBuf> {
        let projects = self.config_dir.join("projects");
        let file = format!("{}.jsonl", self.session_id);
        let usual = projects.join(project_folder(&self.cwd)).join(&file);
        if usual.is_file() {
            return Some(usual);
        }
        std::fs::read_dir(&projects)
            .ok()?
            .flatten()
            .map(|folder| folder.path().join(&file))
            .find(|path| path.is_file())
    }
}

/// Claude's folder name for a project: the path with every character that
/// isn't a letter or digit replaced by `-`.
fn project_folder(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The config dirs to look in: `$CLAUDE_CONFIG_DIR` and `~/.claude`, the ones
/// that exist, without duplicates.
pub fn config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".claude"));
    }
    let mut seen = Vec::new();
    dirs.retain(|dir| {
        let key = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        let keep = dir.is_dir() && !seen.contains(&key);
        seen.push(key);
        keep
    });
    dirs
}

/// Every session file in `config_dir`, alive or not. Files that don't parse
/// are skipped.
pub fn read_sessions(config_dir: &Path) -> Vec<RunningSession> {
    let Ok(entries) = std::fs::read_dir(config_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut sessions: Vec<RunningSession> = entries
        .flatten()
        .map(|entry| entry.path())
        // Only `<pid>.json`; never the `.key` files.
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| std::fs::read(&path).ok())
        .filter_map(|bytes| serde_json::from_slice::<RunningSession>(&bytes).ok())
        .map(|mut session| {
            session.config_dir = config_dir.to_owned();
            session
        })
        .collect();
    sessions.sort_by_key(|session| session.pid);
    sessions
}

/// The sessions in `config_dirs` whose process is still running.
pub async fn live_sessions(config_dirs: &[PathBuf]) -> Vec<RunningSession> {
    let sessions: Vec<RunningSession> = config_dirs.iter().flat_map(|dir| read_sessions(dir)).collect();
    let started = process_start_times(sessions.iter().map(|session| session.pid)).await;
    sessions
        .into_iter()
        .filter(|session| is_alive(session, &started))
        .collect()
}

fn is_alive(session: &RunningSession, started: &HashMap<u32, Option<String>>) -> bool {
    match (started.get(&session.pid), &session.proc_start) {
        (None, _) => false,
        // The pid is in use; make sure it's still the same process.
        (Some(Some(actual)), Some(recorded)) => same_time(actual, recorded),
        // Running, but its start time is unknown on one side.
        (Some(_), _) => true,
    }
}

/// `ps` pads the day of the month with a space, so compare word by word.
fn same_time(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// When each running process among `pids` started, as `ps -o lstart` prints
/// it in UTC, which is how Claude records it. A pid that is running but
/// whose start time is unknown maps to `None`: that happens on Linux images
/// without `ps` (procps), where `/proc` still says which pids exist.
async fn process_start_times(pids: impl Iterator<Item = u32>) -> HashMap<u32, Option<String>> {
    let pids: Vec<u32> = pids.collect();
    if pids.is_empty() {
        return HashMap::new();
    }
    let list: Vec<String> = pids.iter().map(ToString::to_string).collect();
    let output = Command::new("ps")
        .args(["-o", "pid=,lstart=", "-p", &list.join(",")])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()
        .await;
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, started) = line.trim().split_once(char::is_whitespace)?;
                Some((pid.parse().ok()?, Some(started.trim().to_owned())))
            })
            .collect(),
        Err(_) if Path::new("/proc/self").is_dir() => pids
            .into_iter()
            .filter(|pid| Path::new(&format!("/proc/{pid}")).is_dir())
            .map(|pid| (pid, None))
            .collect(),
        Err(_) => HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), contents).unwrap();
    }

    async fn own_start_time() -> String {
        let started = process_start_times(std::iter::once(std::process::id())).await;
        started[&std::process::id()]
            .clone()
            .expect("ps is installed where tests run")
    }

    #[test]
    fn reads_session_files_leniently() {
        let config = tempfile::tempdir().unwrap();
        let sessions = config.path().join("sessions");
        let entry = json!({
            "pid": 42, "sessionId": "abc", "cwd": "/work/repo", "kind": "interactive",
            "name": "repo-1", "status": "busy", "version": "2.1.306", "somethingNew": [1, 2],
        });
        write(&sessions, "42.json", &entry.to_string());
        write(&sessions, "7.json", "not json");
        write(&sessions, "8.json", r#"{"pid": 8}"#);
        write(&sessions, "42.secret.key", "never read");

        let found = read_sessions(config.path());

        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].session_id, "abc");
        assert_eq!(found[0].status.as_deref(), Some("busy"));
        assert_eq!(found[0].config_dir, config.path());
    }

    #[tokio::test]
    async fn only_running_processes_count() {
        let config = tempfile::tempdir().unwrap();
        let sessions = config.path().join("sessions");
        let me = std::process::id();
        let started = own_start_time().await;
        let entry = |pid: u32, session: &str, proc_start: Option<&str>| {
            json!({ "pid": pid, "sessionId": session, "cwd": "/", "procStart": proc_start }).to_string()
        };
        write(&sessions, "a.json", &entry(me, "alive", Some(&started)));
        write(
            &sessions,
            "b.json",
            &entry(me, "pid-reused", Some("Mon Jan  1 00:00:00 2001")),
        );
        // Pids this high don't exist on Linux or macOS.
        write(&sessions, "c.json", &entry(99_999_999, "gone", None));

        let live = live_sessions(&[config.path().to_owned()]).await;

        let ids: Vec<&str> = live.iter().map(|session| session.session_id.as_str()).collect();
        assert_eq!(ids, ["alive"]);
    }

    #[test]
    fn finds_the_transcript() {
        let config = tempfile::tempdir().unwrap();
        let session = RunningSession {
            pid: 1,
            session_id: "abc".into(),
            cwd: "/Users/me/my.repo".into(),
            kind: None,
            name: None,
            status: None,
            version: None,
            entrypoint: None,
            started_at: None,
            status_updated_at: None,
            updated_at: None,
            proc_start: None,
            config_dir: config.path().to_owned(),
        };
        assert_eq!(session.transcript(), None);

        let folder = config.path().join("projects/-Users-me-my-repo");
        write(&folder, "abc.jsonl", "{}\n");
        assert_eq!(session.transcript(), Some(folder.join("abc.jsonl")));
    }

    #[test]
    fn day_padding_does_not_matter() {
        assert!(same_time("Wed Oct  1 16:53:55 2026", "Wed Oct 1 16:53:55 2026"));
        assert!(!same_time("Wed Oct 1 16:53:55 2026", "Wed Oct 1 16:53:56 2026"));
    }
}
