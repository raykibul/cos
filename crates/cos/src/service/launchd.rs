//! The daemon as a launchd agent on macOS: starts at login and restarts if it
//! exits. The Mac app may take this over later with `SMAppService`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, bail};

use crate::home::CosHome;

pub(crate) const LABEL: &str = "local.cos.daemon";

pub(crate) fn plist_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(Path::new(&home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

pub(crate) fn is_installed() -> bool {
    plist_path().is_ok_and(|path| path.exists())
}

/// Writes the agent's plist and loads it, replacing any earlier install.
/// Returns the plist path.
pub(crate) fn install(home: &CosHome) -> anyhow::Result<PathBuf> {
    home.prepare()?;
    let exe = super::current_exe()?;
    // launchd starts agents with a bare PATH and none of your environment.
    let path_env = super::service_path();
    let extra = super::passthrough_env();
    let plist = plist_path()?;

    std::fs::create_dir_all(plist.parent().expect("plist path has a parent"))?;
    std::fs::write(&plist, render_plist(&exe, home, &path_env, &extra))
        .with_context(|| format!("could not write {}", plist.display()))?;

    let domain = gui_domain()?;
    // Not loaded yet is fine.
    let _ = launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
    // Right after a bootout launchd can still be tearing the old one down.
    let mut attempt = 0;
    loop {
        match launchctl(&["bootstrap", &domain, &plist.to_string_lossy()]) {
            Ok(()) => return Ok(plist),
            Err(_) if attempt < 10 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Stops the agent and removes its plist. Returns false if it wasn't installed.
pub(crate) fn uninstall() -> anyhow::Result<bool> {
    let plist = plist_path()?;
    let domain = gui_domain()?;
    let _ = launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
    if !plist.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&plist).with_context(|| format!("could not remove {}", plist.display()))?;
    Ok(true)
}

/// Restarts the agent on whatever binary its plist names now:
/// `launchctl kickstart -k`, which stops the running daemon (SIGTERM, so it
/// ends its agents first) and starts it again.
pub(crate) fn restart() -> anyhow::Result<()> {
    launchctl(&["kickstart", "-k", &format!("{}/{LABEL}", gui_domain()?)])
}

/// The command [`restart`] runs, for people to run themselves.
pub(crate) fn restart_command() -> String {
    let uid = rustix::process::getuid().as_raw();
    format!("launchctl kickstart -k gui/{uid}/{LABEL}")
}

/// The binary an installed plist runs: the first `ProgramArguments` string.
pub(crate) fn recorded_exe(plist: &str) -> Option<PathBuf> {
    let (_, rest) = plist.split_once("<key>ProgramArguments</key>")?;
    let (_, rest) = rest.split_once("<string>")?;
    let (exe, _) = rest.split_once("</string>")?;
    Some(PathBuf::from(xml_unescape(exe)))
}

fn render_plist(exe: &Path, home: &CosHome, path_env: &str, extra_env: &[(&str, String)]) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    let cos_home = xml_escape(&home.root().to_string_lossy());
    let log = xml_escape(&home.daemon_log().to_string_lossy());
    let path_env = xml_escape(path_env);
    let extra_env: String = extra_env
        .iter()
        .map(|(name, value)| {
            format!(
                "\n        <key>{}</key>\n        <string>{}</string>",
                xml_escape(name),
                xml_escape(value)
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>daemon</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>COS_HOME</key>
        <string>{cos_home}</string>
        <key>PATH</key>
        <string>{path_env}</string>{extra_env}
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// launchd's domain for agents in the current user's login session.
fn gui_domain() -> anyhow::Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("could not run `id -u`")?;
    let uid = String::from_utf8(output.stdout)?;
    Ok(format!("gui/{}", uid.trim()))
}

fn launchctl(args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .context("could not run launchctl")?;
    if !output.status.success() {
        bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::CLAUDE_CONFIG_DIR;
    use super::*;

    #[test]
    fn plist_runs_the_daemon_with_the_callers_path_and_home() {
        let home = CosHome::new("/Users/me/.cos");

        let plist = render_plist(
            Path::new("/opt/cos/bin/cos"),
            &home,
            "/usr/bin:/Users/me/.local/bin",
            &[],
        );

        assert!(plist.contains("<string>/opt/cos/bin/cos</string>\n        <string>daemon</string>"));
        assert!(plist.contains("<key>COS_HOME</key>\n        <string>/Users/me/.cos</string>"));
        assert!(plist.contains("<string>/usr/bin:/Users/me/.local/bin</string>"));
        assert!(plist.contains("<string>/Users/me/.cos/logs/daemon.log</string>"));
        assert!(plist.contains(&format!("<string>{LABEL}</string>")));
        assert!(!plist.contains(CLAUDE_CONFIG_DIR));
    }

    #[test]
    fn plist_passes_the_claude_config_dir_on() {
        let home = CosHome::new("/Users/me/.cos");

        let plist = render_plist(
            Path::new("/opt/cos/bin/cos"),
            &home,
            "/usr/bin",
            &[(CLAUDE_CONFIG_DIR, "/Users/me/.claude-work".into())],
        );

        assert!(plist.contains(
            "<key>CLAUDE_CONFIG_DIR</key>\n        <string>/Users/me/.claude-work</string>\n    </dict>"
        ));
    }

    #[test]
    fn reads_the_binary_back_from_the_plist() {
        let home = CosHome::new("/Users/me/.cos");
        for exe in ["/Users/me/.local/bin/cos", "/tmp/a&b <c>/\"cos\""] {
            let plist = render_plist(Path::new(exe), &home, "/usr/bin", &[]);
            assert_eq!(recorded_exe(&plist), Some(PathBuf::from(exe)));
        }
        assert_eq!(recorded_exe("<plist></plist>"), None);
    }

    #[test]
    fn plist_values_are_escaped() {
        let home = CosHome::new("/tmp/a&b");

        let plist = render_plist(Path::new("/tmp/<cos>"), &home, "\"x\"", &[]);

        assert!(plist.contains("/tmp/a&amp;b"));
        assert!(plist.contains("/tmp/&lt;cos&gt;"));
        assert!(plist.contains("&quot;x&quot;"));
    }
}
