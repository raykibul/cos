//! The chief's CLI against a real `cos daemon`, with `cos-fake-claude` as
//! Claude: a run starts its chief, and `cos questions`, `cos say` and
//! `cos report` reach it. Also makes Cargo build the `cos` binary, which the
//! daemon's unit tests run as the chief's MCP server.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const COS: &str = env!("CARGO_BIN_EXE_cos");

/// `cos-fake-claude`, built next to `cos` by `cargo test --workspace`.
fn fake_claude() -> PathBuf {
    Path::new(COS).with_file_name("cos-fake-claude")
}

struct Daemon {
    home: tempfile::TempDir,
    child: Child,
}

impl Daemon {
    fn start() -> Self {
        let home = tempfile::tempdir().unwrap();
        let child = Command::new(COS)
            .arg("daemon")
            .env("COS_HOME", home.path())
            .env("COS_CLAUDE_BIN", fake_claude())
            .env_remove("COS_CHIEF_MODEL")
            .env_remove("COS_HOST")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Self { home, child };
        wait_for("the daemon", || daemon.cos(&["status"]).status.success());
        daemon
    }

    fn cos(&self, args: &[&str]) -> Output {
        Command::new(COS)
            .args(args)
            .env("COS_HOME", self.home.path())
            .env_remove("COS_HOST")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    /// Runs `cos` and returns its stdout, failing the test if it fails.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.cos(args);
        assert!(
            output.status.success(),
            "cos {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // SIGTERM, so it stops its agents on the way down.
        let _ = Command::new("kill").arg(self.child.id().to_string()).status();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A git repo with one commit and no remote.
fn git_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=cos test",
                "-c",
                "user.email=test@cos.local",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&["add", "README"]);
    git(&["commit", "-q", "-m", "first"]);
    repo
}

#[test]
fn a_run_gets_a_chief_you_can_talk_to() {
    assert!(
        fake_claude().is_file(),
        "{} is missing; run the tests with `cargo test --workspace` (make test)",
        fake_claude().display()
    );
    let daemon = Daemon::start();
    let scratch = tempfile::tempdir().unwrap();
    let repo = git_repo(scratch.path());
    daemon.ok(&["project", "add", repo.to_str().unwrap()]);
    daemon.ok(&["workspace", "new", "repo", "--from", "head"]);

    let started = daemon.ok(&["run", "-w", "1", "add a one-line comment to README"]);
    assert!(started.contains("chief"), "{started}");
    let board = daemon.home.path().join("runs/1/board.md");
    wait_for("board.md", || board.is_file());
    let agents = daemon.ok(&["agents"]);
    assert!(agents.contains("Chief of run 1"), "{agents}");

    assert!(
        daemon
            .ok(&["questions"])
            .contains("No questions waiting for you.")
    );
    let said = daemon.ok(&["say", "1", "keep it short"]);
    assert!(said.contains("Sent to the chief of run 1"), "{said}");
    let report = daemon.cos(&["report", "1"]);
    assert!(!report.status.success());
    assert!(
        String::from_utf8_lossy(&report.stderr).contains("no report yet"),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    daemon.ok(&["cancel", "1"]);
}
