//! `cos sessions`: the Claude sessions running on this machine, from Claude's own
//! session files. Read-only; works without the daemon.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cos_core::Project;
use cos_harness::claude::registry::{self, RunningSession};
use jiff::Timestamp;
use serde_json::json;

use crate::client::Client;
use crate::commands::human_duration;
use crate::home::CosHome;

#[derive(clap::Args)]
pub(crate) struct SessionsArgs {
    /// Claude config dir to read. Repeatable. Default: $CLAUDE_CONFIG_DIR, ~/.claude, and
    /// the CLAUDE_CONFIG_DIR set for agents in cos (`cos env`), globally or per project.
    #[arg(long = "claude-config", value_name = "DIR")]
    config_dirs: Vec<PathBuf>,
    /// Print JSON, one session per line.
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(home: &CosHome, args: SessionsArgs) -> anyhow::Result<ExitCode> {
    anyhow::ensure!(
        !Client::new(home.socket()).is_remote(),
        "`cos sessions` reads this machine's Claude sessions; it doesn't work with --host"
    );
    let client = Client::new(home.socket());
    // Matching sessions to workspaces is a bonus; skip it if the daemon is down.
    let projects = client.projects().await.unwrap_or_default();
    let config_dirs = if args.config_dirs.is_empty() {
        let mut dirs = registry::config_dirs();
        add_dirs(&mut dirs, agent_config_dirs(&client, &projects).await);
        dirs
    } else {
        args.config_dirs
    };
    let sessions = registry::live_sessions(&config_dirs).await;

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

/// The Claude config dirs cos's agents use: `CLAUDE_CONFIG_DIR` in the
/// global variables and each project's, with `~` expanded. Empty if the
/// daemon is down.
async fn agent_config_dirs(client: &Client, projects: &[Project]) -> Vec<PathBuf> {
    const NAME: &str = "CLAUDE_CONFIG_DIR";
    let mut values = Vec::new();
    if let Ok(global) = client.global_env().await {
        values.extend(global.variables.get(NAME).cloned());
    }
    for project in projects {
        if let Ok(env) = client.project_env(project.id).await {
            values.extend(env.variables.get(NAME).cloned());
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    values
        .iter()
        .map(|value| PathBuf::from(cos_core::env::expand_home(value, home.as_deref())))
        .collect()
}

/// Adds the folders in `extra` that exist and aren't in `dirs` yet.
fn add_dirs(dirs: &mut Vec<PathBuf>, extra: Vec<PathBuf>) {
    let key = |dir: &PathBuf| dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let mut seen: Vec<PathBuf> = dirs.iter().map(key).collect();
    for dir in extra {
        let canonical = key(&dir);
        if dir.is_dir() && !seen.contains(&canonical) {
            seen.push(canonical);
            dirs.push(dir);
        }
    }
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

/// The cos workspace or project a folder belongs to.
struct Place {
    project_id: i64,
    workspace_id: Option<i64>,
    /// Such as "workspace lisbon of project cos".
    label: String,
}

/// The workspace containing `cwd`, else the project, else `None`.
fn place(projects: &[Project], cwd: &Path) -> Option<Place> {
    for project in projects {
        if let Some(workspace) = project
            .workspaces
            .iter()
            .find(|workspace| cwd.starts_with(&workspace.path))
        {
            return Some(Place {
                project_id: project.id,
                workspace_id: Some(workspace.id),
                label: format!("workspace {} of project {}", workspace.label(), project.name),
            });
        }
    }
    projects
        .iter()
        .find(|project| cwd.starts_with(&project.root))
        .map(|project| Place {
            project_id: project.id,
            workspace_id: None,
            label: format!("project {}", project.name),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_only_new_existing_config_dirs() {
        let root = tempfile::tempdir().unwrap();
        let [a, b] = ["a", "b"].map(|name| {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            dir
        });
        let mut dirs = vec![a.clone()];
        add_dirs(
            &mut dirs,
            vec![a.join("."), b.clone(), root.path().join("missing"), b.clone()],
        );
        assert_eq!(dirs, [a, b]);
    }
}
