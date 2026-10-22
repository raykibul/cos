//! `cos upgrade` end to end against a fake release folder, laid out as
//! `scripts/fake-release.sh` lays one out (and GitHub serves one), with the
//! repo's `install.sh` doing the install. The "new" cos in the release is a
//! shell script that prints its version. HOME, COS_HOME and the app folder
//! are temporary, so no service, daemon or app of yours is involved.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const COS: &str = env!("CARGO_BIN_EXE_cos");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NEW: &str = "9.9.9";
const TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];

fn install_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh")
}

struct Setup {
    tmp: tempfile::TempDir,
}

impl Setup {
    /// A copy of this build's `cos` in `<tmp>/bin`, and a release folder
    /// with `NEW` (the latest) and a tampered `v0.0.0-bad`.
    fn new() -> Self {
        let setup = Self {
            tmp: tempfile::tempdir().unwrap(),
        };
        for dir in ["bin", "home", "cos-home"] {
            std::fs::create_dir_all(setup.path(dir)).unwrap();
        }
        std::fs::copy(COS, setup.cos()).unwrap();
        setup.release(NEW, NEW);
        setup.release("0.0.0-bad", NEW);
        let sums = setup.path("release/download/v0.0.0-bad/SHA256SUMS");
        let tampered: String = std::fs::read_to_string(&sums)
            .unwrap()
            .lines()
            .map(|line| format!("{}  {}\n", "0".repeat(64), line.split_once("  ").unwrap().1))
            .collect();
        std::fs::write(&sums, tampered).unwrap();
        setup.set_latest(NEW);
        setup
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.tmp.path().join(relative)
    }

    fn cos(&self) -> PathBuf {
        self.path("bin/cos")
    }

    /// `download/v<tag>/`: a tarball per target holding a fake `cos` that
    /// says it's `version`, and SHA256SUMS.
    fn release(&self, tag: &str, version: &str) {
        let dir = self.path(&format!("release/download/v{tag}"));
        let build = self.path(&format!("build/{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("cos"), format!("#!/bin/sh\necho \"cos {version}\"\n")).unwrap();
        run(Command::new("chmod").arg("755").arg(build.join("cos")));
        let mut sums = String::new();
        for target in TARGETS {
            let asset = format!("cos-{target}.tar.gz");
            run(Command::new("tar")
                .arg("-czf")
                .arg(dir.join(&asset))
                .arg("-C")
                .arg(&build)
                .arg("cos")
                .env("COPYFILE_DISABLE", "1"));
            sums.push_str(&format!("{}  {asset}\n", sha256(&dir.join(&asset))));
        }
        std::fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    }

    /// What `releases/latest` points at: a file here, a redirect on GitHub.
    fn set_latest(&self, version: &str) {
        std::fs::create_dir_all(self.path("release/latest")).unwrap();
        std::fs::write(self.path("release/latest/tag"), format!("v{version}\n")).unwrap();
    }

    fn command(&self, exe: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(exe);
        command
            .args(args)
            .env("HOME", self.path("home"))
            .env("COS_HOME", self.path("cos-home"))
            .env(
                "COS_DOWNLOAD_BASE",
                format!("file://{}", self.path("release").display()),
            )
            .env("COS_INSTALL_SCRIPT", install_script())
            .env("COS_APP_DIR", self.path("Applications"))
            .env("COS_APP_NO_QUIT", "1")
            .env("COS_APP_PROCESS", "cos-test-none")
            .env("COS_CLAUDE_BIN", Path::new(COS).with_file_name("cos-fake-claude"));
        for name in [
            "COS_HOST",
            "COS_VERSION",
            "COS_INSTALL_DIR",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
            "SUDO_UID",
        ] {
            command.env_remove(name);
        }
        command.stdin(Stdio::null());
        command
    }

    /// Runs the installed copy: `<tmp>/bin/cos <args>`.
    fn upgrade(&self, args: &[&str]) -> Output {
        let mut all = vec!["upgrade"];
        all.extend_from_slice(args);
        self.command(&self.cos(), &all).output().unwrap()
    }
}

fn run(command: &mut Command) {
    let status = command.status().unwrap();
    assert!(status.success(), "{command:?} failed");
}

fn sha256(file: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(file)
        .output()
        .or_else(|_| Command::new("shasum").args(["-a", "256"]).arg(file).output())
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn check_changes_nothing() {
    let setup = Setup::new();
    let before = std::fs::read(setup.cos()).unwrap();

    let output = setup.upgrade(&["--check"]);

    let out = text(&output);
    assert!(output.status.success(), "{out}");
    assert!(out.contains(&format!("installed  cos {VERSION} at ")), "{out}");
    assert!(out.contains(&format!("latest     cos {NEW}")), "{out}");
    assert!(out.contains("Run `cos upgrade` to install it."), "{out}");
    assert_eq!(std::fs::read(setup.cos()).unwrap(), before);
}

#[test]
fn up_to_date_exits_zero_and_changes_nothing() {
    let setup = Setup::new();
    setup.set_latest(VERSION);
    let before = std::fs::read(setup.cos()).unwrap();

    let output = setup.upgrade(&[]);

    let out = text(&output);
    assert!(output.status.success(), "{out}");
    assert!(out.contains(&format!("cos {VERSION} is up to date")), "{out}");
    assert!(!out.contains("cos install:"), "ran the installer: {out}");
    assert_eq!(std::fs::read(setup.cos()).unwrap(), before);
}

#[test]
fn upgrades_the_binary_in_place() {
    let setup = Setup::new();

    let output = setup.upgrade(&[]);

    let out = text(&output);
    assert!(output.status.success(), "{out}");
    let installed = Command::new(setup.cos()).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&installed.stdout).trim(),
        format!("cos {NEW}")
    );
    assert!(
        out.contains(&format!("cos install: installed cos {NEW} at")),
        "{out}"
    );
    assert!(out.contains(&format!("cos {VERSION} → {NEW}")), "{out}");
    // No service, so none of the installer's restart advice.
    assert!(!out.contains("daemon service"), "{out}");
}

#[test]
fn a_tampered_release_is_refused_and_changes_nothing() {
    let setup = Setup::new();
    let before = std::fs::read(setup.cos()).unwrap();

    let output = setup.upgrade(&["--version", "v0.0.0-bad"]);

    let out = text(&output);
    assert!(!output.status.success(), "{out}");
    assert!(out.contains("checksum mismatch"), "{out}");
    assert_eq!(std::fs::read(setup.cos()).unwrap(), before);
}

#[test]
fn a_build_from_source_is_refused() {
    let setup = Setup::new();

    let output = setup.command(Path::new(COS), &["upgrade"]).output().unwrap();

    let out = text(&output);
    assert!(!output.status.success(), "{out}");
    assert!(out.contains("This cos was built from source at "), "{out}");
    assert!(out.contains("update it with `git pull && make build`"), "{out}");
}

#[test]
fn a_remote_host_is_refused() {
    let setup = Setup::new();

    let output = setup.upgrade(&["--host", "my-mac"]);

    let out = text(&output);
    assert!(!output.status.success(), "{out}");
    assert!(out.contains("run `cos upgrade` on that machine"), "{out}");
}

/// A daemon this test started, killed when it ends.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_daemon_that_isnt_a_service_is_left_running_with_advice() {
    let setup = Setup::new();
    let _daemon = Daemon(
        setup
            .command(&setup.cos(), &["daemon"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while !setup
        .command(&setup.cos(), &["status"])
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the daemon didn't start"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let output = setup.upgrade(&[]);

    let out = text(&output);
    assert!(output.status.success(), "{out}");
    assert!(
        out.contains(&format!(
            "is running {VERSION}, not as a service: restart it to use the new version ({NEW})"
        )),
        "{out}"
    );
}
