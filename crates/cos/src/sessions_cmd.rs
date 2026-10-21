//! `cos sessions`: the Claude sessions running on this Mac, from Claude's own
//! session files. Read-only; works without the daemon.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cos_core::Project;
use cos_harness::claude::registry::{self, RunningSession};
use jiff::Timestamp;
use serde_json::json;

use crate::client::Client;
use crate::commands::human_duration;
use crate::daemon::agents::place;
use crate::home::CosHome;

#[derive(clap::Args)]
pub(crate) struct SessionsArgs {
    /// Claude config dir to read. Repeatable. Default: $CLAUDE_CONFIG_DIR and ~/.claude.
    #[arg(long = "claude-config", value_name = "DIR")]
    config_dirs: Vec<PathBuf>,
    /// Print JSON, one session per line.
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(home: &CosHome, args: SessionsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    if client.is_remote() {
        // Another machine's sessions: only its daemon can read them.
        return remote(&client, args.json).await;
    }
    let config_dirs = if args.config_dirs.is_empty() {
        registry::config_dirs()
    } else {
        args.config_dirs
    };
    let sessions = registry::live_sessions(&config_dirs).await;
    // Matching sessions to workspaces is a bonus; skip it if the daemon is down.
    let projects = Client::new(home.socket()).projects().await.unwrap_or_default();

    if args.json {
        for session in &sessions {
            let place = place(&projects, &session.cwd);
            println!(
                "{}",
                json!({
                    "pid": session.pid,
                    "session_id": session.session_id,
                    "cwd": session.cwd,
                    "name": session.name,
                    "kind": session.kind,
                    "status": session.status,
                    "version": session.version,
                    "updated_at_ms": session.status_updated_at.or(session.updated_at),
                    "config_dir": session.config_dir,
                    "transcript": session.transcript(),
                    "project_id": place.as_ref().map(|place| place.project_id),
                    "workspace_id": place.as_ref().and_then(|place| place.workspace_id),
                })
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    if sessions.is_empty() {
        println!("No Claude sessions running (looked in {}).", list(&config_dirs));
    }
    for session in &sessions {
        print_session(session, &projects);
    }
    Ok(ExitCode::SUCCESS)
}

/// The sessions on the `--host` machine, as its daemon sees them (minus the
/// agents it runs itself).
async fn remote(client: &Client, as_json: bool) -> anyhow::Result<ExitCode> {
    let sessions = client.sessions().await?;
    if sessions.is_empty() && !as_json {
        println!("No other Claude sessions running there.");
    }
    for session in &sessions {
        if as_json {
            println!("{}", serde_json::to_string(session)?);
            continue;
        }
        let status = session.status.as_deref().unwrap_or("?");
        let name = session.name.as_deref().unwrap_or("-");
        println!("{status:<5} {name:<22} {}", session.cwd.display());
        let mut details = vec![
            format!("pid {}", session.pid),
            format!("session {}", session.session_id),
        ];
        if let Some(ms) = session.status_updated_ms {
            details.push(format!("{status} for {}", ago(ms)));
        }
        println!("      {}", details.join(" · "));
    }
    Ok(ExitCode::SUCCESS)
}

fn print_session(session: &RunningSession, projects: &[Project]) {
    let status = session.status.as_deref().unwrap_or("?");
    let name = session.name.as_deref().unwrap_or("-");
    println!("{status:<5} {name:<22} {}", tilde(&session.cwd));

    let mut details = vec![format!("pid {}", session.pid)];
    if let Some(kind) = session.kind.as_deref().filter(|kind| *kind != "interactive") {
        details.push(kind.to_owned());
    }
    if let Some(version) = &session.version {
        details.push(version.clone());
    }
    details.push(format!("session {}", session.session_id));
    if let Some(ms) = session.status_updated_at.or(session.updated_at) {
        details.push(format!("{status} for {}", ago(ms)));
    }
    if session.transcript().is_none() {
        details.push("no transcript yet".into());
    }
    println!("      {}", details.join(" · "));
    if let Some(place) = place(projects, &session.cwd) {
        println!("      {}", place.label);
    }
}

fn ago(ms: i64) -> String {
    let now = Timestamp::now().as_millisecond();
    human_duration(u64::try_from((now - ms) / 1000).unwrap_or(0))
}

fn tilde(path: &Path) -> String {
    match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn list(dirs: &[PathBuf]) -> String {
    if dirs.is_empty() {
        return "no Claude config dir found".into();
    }
    dirs.iter()
        .map(|dir| tilde(dir))
        .collect::<Vec<_>>()
        .join(" and ")
}
