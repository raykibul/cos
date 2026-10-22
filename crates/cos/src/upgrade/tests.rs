//! `cos upgrade`'s decisions against a fake machine: nothing here downloads,
//! runs an installer, or touches launchd, systemd or the app.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use super::*;

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

/// A machine that records what `cos upgrade` asks of it.
struct Fake {
    latest: Version,
    lines: RefCell<Vec<String>>,
    installs: RefCell<Vec<Installer>>,
    /// What `cos --version` says; the CLI install changes it.
    binary: RefCell<String>,
    unwritable: Vec<PathBuf>,
    app_version: RefCell<Option<String>>,
    app_running: bool,
    opened: RefCell<Vec<PathBuf>>,
    service: Option<Installation>,
    restarts: Cell<usize>,
    /// The daemon's answers to `/v1/status`, in turn; the last one repeats.
    daemon: RefCell<VecDeque<Option<DaemonInfo>>>,
    agents: Vec<Agent>,
    agents_asked: Cell<bool>,
    interactive: bool,
    answer: bool,
    asked: RefCell<Vec<String>>,
    installer_fails: bool,
}

impl Fake {
    fn new(latest: &str) -> Self {
        Self {
            latest: v(latest),
            lines: RefCell::default(),
            installs: RefCell::default(),
            binary: RefCell::new("0.1.4".into()),
            unwritable: Vec::new(),
            app_version: RefCell::default(),
            app_running: false,
            opened: RefCell::default(),
            service: None,
            restarts: Cell::new(0),
            daemon: RefCell::new(VecDeque::from([None])),
            agents: Vec::new(),
            agents_asked: Cell::new(false),
            interactive: false,
            answer: false,
            asked: RefCell::default(),
            installer_fails: false,
        }
    }

    fn output(&self) -> String {
        self.lines.borrow().join("\n")
    }

    fn says(&self, text: &str) -> bool {
        self.output().contains(text)
    }

    /// The daemon answers with these, in turn.
    fn daemon_answers(&self, answers: &[Option<(&str, u32)>]) {
        *self.daemon.borrow_mut() = answers
            .iter()
            .map(|answer| {
                answer.map(|(version, pid)| DaemonInfo {
                    version: version.into(),
                    pid,
                })
            })
            .collect();
    }
}

impl System for Fake {
    fn say(&self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }

    fn latest(&self) -> anyhow::Result<Version> {
        Ok(self.latest.clone())
    }

    fn run_installer(&self, installer: &Installer) -> anyhow::Result<()> {
        self.installs.borrow_mut().push(installer.clone());
        if self.installer_fails {
            bail!("checksum mismatch");
        }
        if installer.args.contains(&"--app") {
            *self.app_version.borrow_mut() = Some(installer.version.to_string());
        } else {
            *self.binary.borrow_mut() = installer.version.to_string();
        }
        Ok(())
    }

    fn binary_version(&self, _exe: &Path) -> Option<String> {
        Some(self.binary.borrow().clone())
    }

    fn writable(&self, dir: &Path) -> bool {
        !self.unwritable.iter().any(|path| path == dir)
    }

    fn app_version(&self, _app: &Path) -> Option<String> {
        self.app_version.borrow().clone()
    }

    fn app_running(&self) -> bool {
        self.app_running
    }

    fn open_app(&self, app: &Path) -> anyhow::Result<()> {
        self.opened.borrow_mut().push(app.to_owned());
        Ok(())
    }

    fn service(&self) -> Option<Installation> {
        self.service.clone()
    }

    fn restart_service(&self) -> anyhow::Result<()> {
        self.restarts.set(self.restarts.get() + 1);
        Ok(())
    }

    async fn daemon(&self) -> Option<DaemonInfo> {
        let mut answers = self.daemon.borrow_mut();
        if answers.len() > 1 {
            answers.pop_front().flatten()
        } else {
            answers.front().cloned().flatten()
        }
    }

    async fn agents(&self) -> anyhow::Result<Vec<Agent>> {
        self.agents_asked.set(true);
        Ok(self.agents.clone())
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn confirm(&self, question: &str) -> bool {
        self.asked.borrow_mut().push(question.to_owned());
        self.answer
    }

    async fn pause(&self) {}
}

/// A cos at `<tmp>/bin/cos`, 0.1.4, on Linux (no app folders).
struct Setup {
    tmp: tempfile::TempDir,
    settings: Settings,
}

impl Setup {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // Canonical, as the real exe path is (macOS's /var is a symlink).
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let settings = Settings {
            exe: root.join("bin/cos"),
            current: v("0.1.4"),
            app_dirs: Vec::new(),
            sudo: false,
            daemon_log: root.join("home/logs/daemon.log"),
        };
        Self { tmp, settings }
    }

    /// Makes `<tmp>/Applications/cos.app` and looks there, as on a Mac.
    fn with_app(mut self) -> Self {
        let apps = self
            .settings
            .exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("Applications");
        std::fs::create_dir_all(apps.join("cos.app/Contents")).unwrap();
        self.settings.app_dirs = vec![apps.clone(), self.tmp.path().join("home/Applications")];
        self
    }

    fn app(&self) -> PathBuf {
        self.settings.app_dirs[0].join("cos.app")
    }

    fn service(&self) -> Installation {
        Installation {
            file: PathBuf::from("/home/me/.config/systemd/user/cos.service"),
            exe: Some(self.settings.exe.clone()),
            restart: "systemctl --user restart cos.service".into(),
        }
    }
}

async fn upgrade(fake: &Fake, setup: &Setup, args: UpgradeArgs) -> ExitCode {
    run(fake, &setup.settings, &args).await.unwrap()
}

fn agent(id: i64, state: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "id": id, "workspace_id": 1, "run_id": null, "role": "user", "title": format!("agent {id}"),
        "session_id": "s", "pid": 100, "state": state, "model": null, "permission_mode": "default",
        "user_settings": true, "rows": 40, "cols": 120, "started_ms": 0, "last_activity_ms": 0,
        "exited_ms": null, "exit_code": null, "last_message": null, "pending_approval_id": null,
    }))
    .unwrap()
}

#[tokio::test]
async fn up_to_date_installs_nothing() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.4");

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert!(fake.says("cos 0.1.4 is up to date"), "{}", fake.output());
    assert!(fake.installs.borrow().is_empty());
}

#[tokio::test]
async fn a_newer_build_than_the_latest_release_is_kept() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.3");

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert!(
        fake.says("newer than the latest release (0.1.3)"),
        "{}",
        fake.output()
    );
    assert!(fake.installs.borrow().is_empty());
}

#[tokio::test]
async fn check_reports_and_changes_nothing() {
    let setup = Setup::new().with_app();
    let fake = Fake::new("0.1.5");
    *fake.app_version.borrow_mut() = Some("0.1.4".into());
    fake.daemon_answers(&[Some(("0.1.4", 42))]);
    let args = UpgradeArgs {
        check: true,
        ..Default::default()
    };

    assert_eq!(upgrade(&fake, &setup, args).await, ExitCode::SUCCESS);

    let out = fake.output();
    assert!(out.contains("installed  cos 0.1.4 at"), "{out}");
    assert!(out.contains("latest     cos 0.1.5"), "{out}");
    assert!(out.contains("app        0.1.4 at"), "{out}");
    assert!(out.contains("daemon     0.1.4 (pid 42)"), "{out}");
    assert!(out.contains("Run `cos upgrade` to install it."), "{out}");
    assert!(fake.installs.borrow().is_empty());
    assert_eq!(fake.restarts.get(), 0);
}

#[tokio::test]
async fn upgrades_the_cli_in_its_own_folder() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.5");

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    let installs = fake.installs.borrow();
    assert_eq!(
        *installs,
        [Installer::cli(&v("0.1.5"), setup.settings.exe.parent().unwrap())]
    );
    assert!(fake.says("cos 0.1.4 → 0.1.5"), "{}", fake.output());
}

#[tokio::test]
async fn a_pinned_version_may_go_back() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.5");
    let args = UpgradeArgs {
        version: Some("v0.1.2".into()),
        ..Default::default()
    };

    assert_eq!(upgrade(&fake, &setup, args).await, ExitCode::SUCCESS);

    assert_eq!(fake.installs.borrow()[0].version, v("0.1.2"));
    assert!(fake.says("cos 0.1.4 → 0.1.2"), "{}", fake.output());
}

#[tokio::test]
async fn a_bad_pinned_version_is_refused() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.5");
    let args = UpgradeArgs {
        version: Some("latest".into()),
        ..Default::default()
    };

    let error = run(&fake, &setup.settings, &args).await.unwrap_err();

    assert!(error.to_string().contains("isn't a version"), "{error}");
}

#[tokio::test]
async fn a_failed_install_stops_there() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.installer_fails = true;
    fake.service = Some(setup.service());
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    let error = run(&fake, &setup.settings, &UpgradeArgs::default())
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("checksum mismatch"), "{error:#}");
    assert_eq!(fake.restarts.get(), 0);
}

#[tokio::test]
async fn a_source_build_is_refused_unless_forced() {
    let mut setup = Setup::new();
    let root = setup.settings.exe.parent().unwrap().parent().unwrap().to_owned();
    std::fs::create_dir_all(root.join("repo/target/release")).unwrap();
    setup.settings.exe = root.join("repo/target/release/cos");
    let fake = Fake::new("0.1.5");

    let error = run(&fake, &setup.settings, &UpgradeArgs::default())
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "This cos was built from source at {}; update it with `git pull && make build` \
             (or pass --force to replace it with a release build)",
            root.join("repo").display()
        )
    );
    assert!(fake.installs.borrow().is_empty());

    let args = UpgradeArgs {
        force: true,
        ..Default::default()
    };
    assert_eq!(upgrade(&fake, &setup, args).await, ExitCode::SUCCESS);
    assert_eq!(fake.installs.borrow().len(), 1);
}

#[test]
fn finds_source_checkouts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert_eq!(
        source_checkout(&root.join("cos/target/release/cos")),
        Some(root.join("cos"))
    );
    assert_eq!(
        source_checkout(&root.join("cos/target/docker/debug/cos")),
        Some(root.join("cos"))
    );
    // A git checkout of a Cargo project, binary anywhere inside.
    std::fs::create_dir_all(root.join("repo/.git")).unwrap();
    std::fs::write(root.join("repo/Cargo.toml"), "").unwrap();
    assert_eq!(
        source_checkout(&root.join("repo/bin/cos")),
        Some(root.join("repo"))
    );
    // A git repo that isn't a Cargo project, such as dotfiles in a home.
    std::fs::create_dir_all(root.join("home/.git")).unwrap();
    assert_eq!(source_checkout(&root.join("home/.local/bin/cos")), None);
    assert_eq!(source_checkout(Path::new("/usr/local/bin/cos")), None);
}

#[tokio::test]
async fn an_unwritable_folder_is_refused_with_advice() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.unwritable = vec![setup.settings.exe.parent().unwrap().to_owned()];

    let error = run(&fake, &setup.settings, &UpgradeArgs::default())
        .await
        .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("isn't writable by you"), "{message}");
    assert!(message.contains("sudo cos upgrade"), "{message}");
    assert!(
        message.contains("COS_INSTALL_DIR=$HOME/.local/bin sh"),
        "{message}"
    );
    assert!(fake.installs.borrow().is_empty());
}

#[tokio::test]
async fn only_a_binary_named_cos_is_replaced() {
    let mut setup = Setup::new();
    setup.settings.exe.set_file_name("cos-0.1.4");
    let fake = Fake::new("0.1.5");

    let error = run(&fake, &setup.settings, &UpgradeArgs::default())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("isn't named `cos`"), "{error}");
}

#[test]
fn the_installer_gets_the_folder_version_and_script() {
    let cli = Installer::cli(&v("0.1.5"), Path::new("/opt/cos bin"));
    let command = cli.command();
    assert_eq!(command.get_program(), "sh");
    assert_eq!(command.get_args().collect::<Vec<_>>(), ["-s", "--"]);
    let env: Vec<_> = command
        .get_envs()
        .map(|(name, value)| (name.to_str().unwrap(), value.unwrap().to_str().unwrap()))
        .collect();
    assert_eq!(
        env,
        [
            ("COS_INSTALL_DIR", "/opt/cos bin"),
            ("COS_NO_SERVICE_HINT", "1"),
            ("COS_VERSION", "v0.1.5"),
        ]
    );
    assert_eq!(
        cli.script_source(None),
        "https://raw.githubusercontent.com/raykibul/cos/v0.1.5/install.sh"
    );
    assert_eq!(cli.script_source(Some("/src/install.sh")), "/src/install.sh");

    let app = Installer::app(&v("0.2.0-rc.1"), Path::new("/Users/me/Applications"));
    let command = app.command();
    assert_eq!(command.get_args().collect::<Vec<_>>(), ["-s", "--", "--app"]);
    let env: Vec<_> = command
        .get_envs()
        .map(|(name, value)| (name.to_str().unwrap(), value.unwrap().to_str().unwrap()))
        .collect();
    assert_eq!(
        env,
        [
            ("COS_APP_DIR", "/Users/me/Applications"),
            ("COS_VERSION", "v0.2.0-rc.1")
        ]
    );
}

#[tokio::test]
async fn the_app_is_upgraded_and_reopened_if_it_was_running() {
    let setup = Setup::new().with_app();
    let mut fake = Fake::new("0.1.5");
    fake.app_running = true;
    *fake.app_version.borrow_mut() = Some("0.1.4".into());

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    let installs = fake.installs.borrow();
    assert_eq!(installs.len(), 2);
    assert_eq!(
        installs[1],
        Installer::app(&v("0.1.5"), &setup.settings.app_dirs[0])
    );
    assert_eq!(*fake.opened.borrow(), [setup.app()]);
    assert!(
        fake.says(&format!(
            "app 0.1.4 → 0.1.5 at {}, reopened",
            setup.app().display()
        )),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn an_app_that_wasnt_running_stays_closed() {
    let setup = Setup::new().with_app();
    let fake = Fake::new("0.1.5");
    *fake.app_version.borrow_mut() = Some("0.1.4".into());

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(fake.installs.borrow().len(), 2);
    assert!(fake.opened.borrow().is_empty());
}

#[tokio::test]
async fn an_app_on_the_version_already_is_skipped() {
    let setup = Setup::new().with_app();
    let mut fake = Fake::new("0.1.5");
    fake.app_running = true;
    *fake.app_version.borrow_mut() = Some("0.1.5".into());

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(fake.installs.borrow().len(), 1, "only the CLI");
    assert!(fake.opened.borrow().is_empty());
    assert!(fake.says("is up to date"), "{}", fake.output());
}

#[tokio::test]
async fn an_old_app_is_upgraded_even_with_the_cli_current() {
    let setup = Setup::new().with_app();
    let fake = Fake::new("0.1.4");
    *fake.app_version.borrow_mut() = Some("0.1.3".into());

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    let installs = fake.installs.borrow();
    assert_eq!(
        *installs,
        [Installer::app(&v("0.1.4"), &setup.settings.app_dirs[0])]
    );
    assert!(fake.says("cos 0.1.4 is up to date"), "{}", fake.output());
}

#[tokio::test]
async fn no_app_leaves_the_app_alone() {
    let setup = Setup::new().with_app();
    let fake = Fake::new("0.1.5");
    *fake.app_version.borrow_mut() = Some("0.1.4".into());
    let args = UpgradeArgs {
        no_app: true,
        ..Default::default()
    };

    upgrade(&fake, &setup, args).await;

    assert_eq!(fake.installs.borrow().len(), 1, "only the CLI");
}

#[tokio::test]
async fn an_app_that_isnt_writable_is_left_with_advice() {
    let setup = Setup::new().with_app();
    let mut fake = Fake::new("0.1.5");
    fake.unwritable = vec![setup.app()];
    *fake.app_version.borrow_mut() = Some("0.1.4".into());

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert_eq!(fake.installs.borrow().len(), 1, "only the CLI");
    assert!(fake.says("| sudo sh -s -- --app"), "{}", fake.output());
}

#[tokio::test]
async fn under_sudo_the_app_and_daemon_are_left_alone() {
    let mut setup = Setup::new().with_app();
    setup.settings.sudo = true;
    let mut fake = Fake::new("0.1.5");
    *fake.app_version.borrow_mut() = Some("0.1.4".into());
    fake.service = Some(setup.service());
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(fake.installs.borrow().len(), 1, "only the CLI");
    assert_eq!(fake.restarts.get(), 0);
    assert!(fake.says("again without sudo"), "{}", fake.output());
}

#[tokio::test]
async fn restarts_a_service_running_this_binary_and_waits_for_the_new_version() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.agents = vec![agent(1, "idle"), agent(2, "exited"), agent(3, "interrupted")];
    // Before; the old daemon still answering; down; the new one.
    fake.daemon_answers(&[
        Some(("0.1.4", 42)),
        Some(("0.1.4", 42)),
        None,
        Some(("0.1.5", 43)),
    ]);

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert_eq!(fake.restarts.get(), 1);
    assert!(fake.asked.borrow().is_empty(), "nothing busy, so no question");
    assert!(
        fake.says("daemon restarted on 0.1.5 (pid 43)"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn a_daemon_that_doesnt_come_back_fails_the_upgrade() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.daemon_answers(&[Some(("0.1.4", 42)), None]);

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::FAILURE
    );

    assert_eq!(fake.restarts.get(), 1);
    assert!(
        fake.says("isn't answering on 0.1.5 20 s after the restart"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn busy_agents_are_listed_and_the_user_asked() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.agents = vec![
        agent(7, "working"),
        agent(8, "idle"),
        agent(9, "waiting_approval"),
    ];
    fake.interactive = true;
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(*fake.asked.borrow(), [RESTART_QUESTION]);
    assert!(fake.says("  #7 agent 7 (user): working"), "{}", fake.output());
    assert!(
        fake.says("  #9 agent 9 (user): waiting_approval"),
        "{}",
        fake.output()
    );
    assert!(!fake.says("#8"), "{}", fake.output());
    assert_eq!(fake.restarts.get(), 0, "answered no");
    assert!(
        fake.says("systemctl --user restart cos.service"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn a_yes_to_the_question_restarts() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.agents = vec![agent(7, "working")];
    fake.interactive = true;
    fake.answer = true;
    fake.daemon_answers(&[Some(("0.1.4", 42)), Some(("0.1.5", 50))]);

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert_eq!(fake.restarts.get(), 1);
}

#[tokio::test]
async fn busy_agents_without_a_terminal_get_the_command_instead() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.agents = vec![agent(7, "working")];
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    assert_eq!(
        upgrade(&fake, &setup, UpgradeArgs::default()).await,
        ExitCode::SUCCESS
    );

    assert!(fake.asked.borrow().is_empty());
    assert_eq!(fake.restarts.get(), 0);
    assert!(
        fake.says("Restart it with `systemctl --user restart cos.service`"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn yes_restarts_without_asking() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.agents = vec![agent(7, "working")];
    fake.daemon_answers(&[Some(("0.1.4", 42)), Some(("0.1.5", 50))]);
    let args = UpgradeArgs {
        yes: true,
        ..Default::default()
    };

    upgrade(&fake, &setup, args).await;

    assert!(fake.asked.borrow().is_empty());
    assert!(!fake.agents_asked.get(), "--yes needs no agent list");
    assert_eq!(fake.restarts.get(), 1);
}

#[tokio::test]
async fn a_service_running_another_binary_is_left_alone() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    let mut service = setup.service();
    service.exe = Some(PathBuf::from("/elsewhere/cos"));
    fake.service = Some(service);
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(fake.restarts.get(), 0);
    assert!(
        fake.says("runs /elsewhere/cos, not this cos"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn a_daemon_without_a_service_is_pointed_at() {
    let setup = Setup::new();
    let fake = Fake::new("0.1.5");
    fake.daemon_answers(&[Some(("0.1.4", 42))]);

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert_eq!(fake.restarts.get(), 0);
    assert!(
        fake.says("restart it to use the new version (0.1.5)"),
        "{}",
        fake.output()
    );
}

#[tokio::test]
async fn no_restart_skips_the_daemon_entirely() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.5");
    fake.service = Some(setup.service());
    fake.daemon_answers(&[Some(("0.1.4", 42))]);
    let args = UpgradeArgs {
        no_restart: true,
        ..Default::default()
    };

    upgrade(&fake, &setup, args).await;

    assert_eq!(fake.restarts.get(), 0);
    assert!(!fake.agents_asked.get());
}

#[tokio::test]
async fn an_old_daemon_is_restarted_even_with_the_cli_current() {
    let setup = Setup::new();
    let mut fake = Fake::new("0.1.4");
    fake.service = Some(setup.service());
    fake.daemon_answers(&[Some(("0.1.3", 42)), Some(("0.1.4", 43))]);

    upgrade(&fake, &setup, UpgradeArgs::default()).await;

    assert!(fake.installs.borrow().is_empty());
    assert_eq!(fake.restarts.get(), 1);
}

#[test]
fn the_restart_decision_table() {
    use DaemonState::{Current, NotRunning, Outdated};
    use Restart::*;
    use ServiceState::{None as NoService, OtherBinary as Other, ThisBinary as This};

    let cases = [
        // (daemon, service, busy, yes, interactive, no_restart) → decision
        ((Outdated, This, 0, false, false, false), Now),
        ((Outdated, This, 2, false, true, false), Ask),
        ((Outdated, This, 2, false, false, false), NoTerminal),
        ((Outdated, This, 2, true, false, false), Now),
        ((Outdated, This, 2, true, true, false), Now),
        ((Outdated, Other, 0, true, true, false), OtherBinary),
        ((Outdated, NoService, 0, true, true, false), Unmanaged),
        ((Current, This, 2, false, true, false), Nothing),
        ((NotRunning, This, 0, false, true, false), Nothing),
        ((NotRunning, NoService, 0, false, true, false), Nothing),
        ((Outdated, This, 0, true, true, true), Skip),
        ((Outdated, NoService, 0, false, false, true), Skip),
    ];
    for ((daemon, service, busy, yes, interactive, no_restart), expected) in cases {
        let situation = Situation {
            daemon,
            service,
            busy,
            yes,
            interactive,
            no_restart,
        };
        assert_eq!(decide(&situation), expected, "{situation:?}");
    }
}

#[test]
fn only_settled_agents_count_as_idle() {
    for state in [AgentState::Idle, AgentState::Exited, AgentState::Interrupted] {
        assert!(!is_busy(state), "{state:?}");
    }
    for state in [
        AgentState::Starting,
        AgentState::NeedsTrust,
        AgentState::NeedsLogin,
        AgentState::Working,
        AgentState::WaitingApproval,
    ] {
        assert!(is_busy(state), "{state:?}");
    }
}
