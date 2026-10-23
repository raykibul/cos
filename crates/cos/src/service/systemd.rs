//! The daemon as a systemd user unit on Linux, `~/.config/systemd/user/cos.service`:
//! started now and at login, and again after a crash. Without lingering
//! (`loginctl enable-linger`), systemd stops your user's services when your
//! last session ends, so on a server the daemon would die with your SSH
//! session; `install` says so.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::home::CosHome;

pub(crate) const UNIT: &str = "cos.service";
/// systemd's runtime folder, there only when systemd is PID 1.
const SYSTEMD_RUNTIME: &str = "/run/systemd/system";
/// An optional file of `NAME=value` lines in the cos home, read by the unit:
/// the place for secrets such as `ANTHROPIC_API_KEY`, which `install` never
/// copies into the unit.
pub(crate) const ENV_FILE: &str = "daemon.env";

const NO_SYSTEMD: &str = "systemd isn't running here (no /run/systemd/system), so `cos daemon install` \
     can't set up a service. Run `cos daemon` under your own supervisor instead";

/// Fails with advice when there's no systemd to install into.
pub(crate) fn check_available() -> anyhow::Result<()> {
    if !Path::new(SYSTEMD_RUNTIME).is_dir() {
        bail!(NO_SYSTEMD);
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME/systemd/user/cos.service`, or under `~/.config`.
pub(crate) fn unit_path() -> anyhow::Result<PathBuf> {
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(&std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(config.join("systemd/user").join(UNIT))
}

/// Writes the unit, enables it and (re)starts it. Returns the unit's path
/// and notes for you, such as lingering being off.
pub(crate) fn install(home: &CosHome) -> anyhow::Result<(PathBuf, Vec<String>)> {
    check_available()?;
    let mut notes = Vec::new();
    let user = user_name();
    // Lingering keeps the daemon running with nobody logged in, and starts it
    // at boot. Many systems let you turn it on for yourself without sudo.
    if linger(&user) == Some(false) && enable_linger(&user) {
        notes.push(format!(
            "turned on lingering for {user}: the daemon keeps running after you log out, and starts at boot"
        ));
        wait_for_user_bus();
    }
    // Fails early, before writing anything, when there's no user manager to
    // talk to (a `su` shell, say).
    systemctl(&["show-environment"])?;
    home.prepare()?;
    let exe = super::current_exe()?;
    let unit = unit_path()?;
    std::fs::create_dir_all(unit.parent().expect("unit path has a parent"))?;
    let rendered = render_unit(&exe, home, &super::service_path(), &super::passthrough_env());
    std::fs::write(&unit, rendered).with_context(|| format!("could not write {}", unit.display()))?;

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", UNIT])?;
    // `restart` rather than `enable --now`, so installing again picks up a
    // new binary or environment.
    systemctl(&["restart", UNIT])?;

    if linger(&user) == Some(false) {
        notes.push(format!(
            "lingering is off for {user}, so systemd stops the daemon when your last login session \
             (such as SSH) ends. To keep it running, and start it at boot: sudo loginctl enable-linger {user}"
        ));
    }
    Ok((unit, notes))
}

/// `loginctl enable-linger` for yourself, without a password prompt.
/// Returns whether it worked; polkit refuses it on some systems, and then
/// it takes sudo.
fn enable_linger(user: &str) -> bool {
    Command::new("loginctl")
        .args(["enable-linger", "--no-ask-password", user])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Once lingering is turned on, systemd starts the user's manager in the
/// background; waits up to 5 s for its bus, when the shell has no session
/// to find it by.
fn wait_for_user_bus() {
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| !dir.is_empty()) {
        return;
    }
    let bus = PathBuf::from(format!("/run/user/{}/bus", rustix::process::getuid().as_raw()));
    for _ in 0..50 {
        if bus.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Stops and disables the unit and removes its file. Returns false if it
/// wasn't installed.
pub(crate) fn uninstall() -> anyhow::Result<bool> {
    // Not loaded or not enabled is fine.
    let _ = systemctl(&["disable", "--now", UNIT]);
    let removed = remove_unit_file()?;
    if removed {
        let _ = systemctl(&["daemon-reload"]);
        let _ = systemctl(&["reset-failed", UNIT]);
    }
    Ok(removed)
}

/// Restarts the unit, so it runs whatever binary `ExecStart` names now.
pub(crate) fn restart() -> anyhow::Result<()> {
    systemctl(&["restart", UNIT])
}

/// The command [`restart`] runs, for people to run themselves.
pub(crate) fn restart_command() -> String {
    format!("systemctl --user restart {UNIT}")
}

/// The binary an installed unit runs: `ExecStart`'s first word, quoted as
/// [`exec_quote`] wrote it.
pub(crate) fn recorded_exe(unit: &str) -> Option<PathBuf> {
    let line = unit.lines().find_map(|line| line.strip_prefix("ExecStart="))?;
    let mut chars = line.strip_prefix('"')?.chars();
    let mut exe = String::new();
    while let Some(char) = chars.next() {
        match char {
            '"' => return Some(PathBuf::from(exe)),
            '\\' => match chars.next()? {
                'n' => exe.push('\n'),
                other => exe.push(other),
            },
            // `$$` and `%%` each stand for one.
            '$' | '%' if chars.next()? == char => exe.push(char),
            '$' | '%' => return None,
            other => exe.push(other),
        }
    }
    None
}

/// Removes the unit file. Returns false if there was none.
pub(crate) fn remove_unit_file() -> anyhow::Result<bool> {
    let unit = unit_path()?;
    if !unit.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&unit).with_context(|| format!("could not remove {}", unit.display()))?;
    Ok(true)
}

fn render_unit(exe: &Path, home: &CosHome, path_env: &str, extra_env: &[(&str, String)]) -> String {
    let exec = format!("{} daemon", exec_quote(&exe.to_string_lossy()));
    let mut env = vec![
        environment("COS_HOME", &home.root().to_string_lossy()),
        environment("PATH", path_env),
    ];
    env.extend(extra_env.iter().map(|(name, value)| environment(name, value)));
    let env = env.join("\n");
    let env_file = specifiers(&home.root().join(ENV_FILE).to_string_lossy());
    let log = specifiers(&home.daemon_log().to_string_lossy());
    format!(
        "# Written by `cos daemon install`; `cos daemon uninstall` removes it.
[Unit]
Description=cos daemon, chief of staff for coding agents

[Service]
Type=simple
ExecStart={exec}
{env}
EnvironmentFile=-{env_file}
Restart=on-failure
RestartSec=10
# SIGTERM goes to the daemon alone, which stops its agents itself; anything
# left is killed when the timeout runs out.
KillMode=mixed
TimeoutStopSec=30
StandardOutput=append:{log}
StandardError=append:{log}

[Install]
WantedBy=default.target
"
    )
}

/// An `Environment=` line, quoted so spaces, quotes and `%` survive.
fn environment(name: &str, value: &str) -> String {
    format!("Environment=\"{}={}\"", escape(name), escape(value))
}

/// One `ExecStart=` word: quoted, with `$` kept from variable expansion.
fn exec_quote(word: &str) -> String {
    format!("\"{}\"", escape(word).replace('$', "$$"))
}

/// Escapes for a double-quoted unit-file value: backslashes, quotes and
/// newlines, and `%`, which starts a specifier.
fn escape(text: &str) -> String {
    specifiers(
        &text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n"),
    )
}

/// `%` doubled, for values systemd expands specifiers in.
fn specifiers(text: &str) -> String {
    text.replace('%', "%%")
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
    let bus = user_bus_env(
        runtime_dir.as_deref(),
        rustix::process::getuid().as_raw(),
        |path| path.exists(),
    );
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .envs(bus)
        .output()
        .context("could not run systemctl")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.contains("Failed to connect to bus") || stderr.contains("XDG_RUNTIME_DIR") {
        let user = user_name();
        bail!(
            "`systemctl --user {}` can't reach your user's systemd ({stderr}). This shell has no \
             login session, as with AWS Session Manager, `su` or `sudo -u`. Run `sudo loginctl \
             enable-linger {user}`, which starts {user}'s systemd and keeps the daemon running \
             after you log out, then try again",
            args.join(" ")
        );
    }
    bail!("`systemctl --user {}` failed: {stderr}", args.join(" "))
}

/// Where to find the user's systemd when the shell doesn't say. Shells
/// without a login session (AWS Session Manager, `su`, `sudo -u`) have no
/// `XDG_RUNTIME_DIR`, yet the user's manager may be running anyway, because
/// lingering is on; its bus is then at `/run/user/<uid>/bus`.
fn user_bus_env(
    runtime_dir: Option<&std::ffi::OsStr>,
    uid: u32,
    exists: impl Fn(&Path) -> bool,
) -> Vec<(&'static str, String)> {
    if runtime_dir.is_some_and(|dir| !dir.is_empty()) {
        return Vec::new();
    }
    let dir = format!("/run/user/{uid}");
    let bus = format!("{dir}/bus");
    if !exists(Path::new(&bus)) {
        return Vec::new();
    }
    vec![
        ("XDG_RUNTIME_DIR", dir),
        ("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={bus}")),
    ]
}

fn user_name() -> String {
    std::env::var("USER")
        .ok()
        .filter(|user| !user.is_empty())
        .or_else(|| {
            let output = Command::new("id").arg("-un").output().ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .unwrap_or_default()
}

/// Whether `user`'s services keep running without a login session, or
/// `None` if `loginctl` didn't say.
fn linger(user: &str) -> Option<bool> {
    let output = Command::new("loginctl")
        .args(["show-user", user, "--property=Linger"])
        .output()
        .ok()?;
    if !output.status.success() {
        // `loginctl` only knows users with a session or lingering on, so a
        // user it doesn't know isn't lingering.
        return Some(false);
    }
    parse_linger(&String::from_utf8_lossy(&output.stdout))
}

/// Reads `loginctl show-user … --property=Linger`: `Linger=yes`.
fn parse_linger(output: &str) -> Option<bool> {
    output
        .lines()
        .find_map(|line| match line.trim().strip_prefix("Linger=")? {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::super::CLAUDE_CONFIG_DIR;
    use super::*;

    #[test]
    fn the_unit_runs_the_daemon_with_the_callers_path_and_home() {
        let home = CosHome::new("/home/me/.cos");

        let unit = render_unit(
            Path::new("/home/me/.local/bin/cos"),
            &home,
            "/usr/bin:/home/me/.local/bin",
            &[],
        );

        assert!(
            unit.contains("\nExecStart=\"/home/me/.local/bin/cos\" daemon\n"),
            "{unit}"
        );
        assert!(unit.contains("\nEnvironment=\"COS_HOME=/home/me/.cos\"\n"));
        assert!(unit.contains("\nEnvironment=\"PATH=/usr/bin:/home/me/.local/bin\"\n"));
        assert!(unit.contains("\nEnvironmentFile=-/home/me/.cos/daemon.env\n"));
        assert!(unit.contains("\nRestart=on-failure\n"));
        assert!(unit.contains("\nKillMode=mixed\n"));
        assert!(unit.contains("\nStandardOutput=append:/home/me/.cos/logs/daemon.log\n"));
        assert!(unit.contains("\nStandardError=append:/home/me/.cos/logs/daemon.log\n"));
        assert!(unit.contains("\n[Install]\nWantedBy=default.target\n"));
        assert!(!unit.contains(CLAUDE_CONFIG_DIR));
    }

    #[test]
    fn the_unit_passes_the_extra_environment_on() {
        let home = CosHome::new("/home/me/.cos");

        let unit = render_unit(
            Path::new("/usr/local/bin/cos"),
            &home,
            "/usr/bin",
            &[
                (CLAUDE_CONFIG_DIR, "/home/me/.claude-work".into()),
                ("COS_STAFF_MODEL", "haiku".into()),
            ],
        );

        assert!(unit.contains(
            "Environment=\"PATH=/usr/bin\"\n\
             Environment=\"CLAUDE_CONFIG_DIR=/home/me/.claude-work\"\n\
             Environment=\"COS_STAFF_MODEL=haiku\"\n\
             EnvironmentFile="
        ));
    }

    #[test]
    fn unit_values_are_escaped() {
        let home = CosHome::new("/tmp/100% \"cos\"");

        let unit = render_unit(Path::new("/opt/my $cos/cos"), &home, "a\\b", &[]);

        assert!(unit.contains("ExecStart=\"/opt/my $$cos/cos\" daemon"), "{unit}");
        assert!(
            unit.contains("Environment=\"COS_HOME=/tmp/100%% \\\"cos\\\"\""),
            "{unit}"
        );
        assert!(unit.contains("Environment=\"PATH=a\\\\b\""), "{unit}");
        assert!(
            unit.contains("StandardOutput=append:/tmp/100%% \"cos\"/logs/daemon.log"),
            "{unit}"
        );
    }

    #[test]
    fn reads_the_binary_back_from_the_unit() {
        let home = CosHome::new("/home/me/.cos");
        for exe in ["/home/me/.local/bin/cos", "/opt/my $cos/100% \"c\\os\""] {
            let unit = render_unit(Path::new(exe), &home, "/usr/bin", &[]);
            assert_eq!(recorded_exe(&unit), Some(PathBuf::from(exe)), "{unit}");
        }
        assert_eq!(recorded_exe("[Service]\nExecStart=cos daemon\n"), None);
    }

    #[test]
    fn finds_the_users_bus_when_the_shell_has_no_session() {
        let runtime = std::ffi::OsStr::new("/run/user/1000");
        // The shell knows its session: leave it alone.
        assert!(user_bus_env(Some(runtime), 1000, |_| true).is_empty());
        // No session, but the user's systemd runs (lingering).
        assert_eq!(
            user_bus_env(None, 1001, |path| path == Path::new("/run/user/1001/bus")),
            vec![
                ("XDG_RUNTIME_DIR", "/run/user/1001".to_owned()),
                (
                    "DBUS_SESSION_BUS_ADDRESS",
                    "unix:path=/run/user/1001/bus".to_owned()
                ),
            ]
        );
        assert_eq!(user_bus_env(Some(std::ffi::OsStr::new("")), 7, |_| true).len(), 2);
        // No session and no user systemd: nothing to point at.
        assert!(user_bus_env(None, 1001, |_| false).is_empty());
    }

    #[test]
    fn reads_loginctls_linger_answer() {
        assert_eq!(parse_linger("Linger=yes\n"), Some(true));
        assert_eq!(parse_linger("Linger=no\n"), Some(false));
        assert_eq!(parse_linger(""), None);
        assert_eq!(parse_linger("Linger=maybe"), None);
    }

    #[test]
    fn the_unit_lives_in_the_xdg_config_dir() {
        // Only checks the shape; the env var may or may not be set here.
        let path = unit_path().unwrap();
        assert!(path.ends_with("systemd/user/cos.service"), "{}", path.display());
    }
}
