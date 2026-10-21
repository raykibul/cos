//! The Claude sessions running on this machine that the daemon didn't start,
//! read from Claude's own `sessions/<pid>.json` files (see
//! [`cos_harness::claude::registry`]). Read-only: the daemon can't show their
//! screens or answer their prompts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Weak;

use cos_core::Project;
use cos_core::events::kind;
use cos_harness::claude::registry::{self, RunningSession};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Inner;

/// One entry of `GET /v1/sessions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExternalSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub name: Option<String>,
    /// `busy` or `idle`.
    pub status: Option<String>,
    pub version: Option<String>,
    pub status_updated_ms: Option<i64>,
    pub project_id: Option<i64>,
    pub workspace_id: Option<i64>,
}

impl ExternalSession {
    pub(super) fn new(session: RunningSession, projects: &[Project]) -> Self {
        let place = place(projects, &session.cwd);
        Self {
            pid: session.pid,
            session_id: session.session_id,
            cwd: session.cwd,
            name: session.name,
            status: session.status,
            version: session.version,
            status_updated_ms: session.status_updated_at.or(session.updated_at),
            project_id: place.as_ref().map(|place| place.project_id),
            workspace_id: place.and_then(|place| place.workspace_id),
        }
    }
}

/// The cos workspace or project a folder belongs to.
pub(crate) struct Place {
    pub project_id: i64,
    pub workspace_id: Option<i64>,
    /// Such as "workspace lisbon of project cos".
    pub label: String,
}

/// The workspace containing `cwd`, else the project, else `None`.
pub(crate) fn place(projects: &[Project], cwd: &Path) -> Option<Place> {
    for project in projects {
        if let Some(workspace) = project
            .workspaces
            .iter()
            .find(|workspace| cwd.starts_with(&workspace.path))
        {
            return Some(Place {
                project_id: project.id,
                workspace_id: Some(workspace.id),
                label: format!("workspace {} of project {}", workspace.title, project.name),
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

/// What `session.changed` reports a change of: which sessions run, and
/// whether each is busy.
type Fingerprint = Vec<(u32, String, Option<String>)>;

impl Inner {
    /// Live Claude sessions, minus the daemon's own agents.
    pub(super) async fn external_sessions(&self) -> Vec<RunningSession> {
        let sessions = registry::live_sessions(&self.config.claude_config_dirs).await;
        let (own_sessions, own_pids): (HashSet<String>, HashSet<u32>) = {
            let live = self.live();
            (
                live.by_session.keys().cloned().collect(),
                live.terminals
                    .values()
                    .filter(|running| !running.exited)
                    .filter_map(|running| running.terminal.pid())
                    .collect(),
            )
        };
        sessions
            .into_iter()
            .filter(|session| !own_sessions.contains(&session.session_id) && !own_pids.contains(&session.pid))
            .collect()
    }
}

/// Logs `session.changed` whenever the set of other sessions, or one's
/// status, changes. The first look only sets the baseline.
pub(super) async fn watch(inner: Weak<Inner>) {
    let mut last: Option<Fingerprint> = None;
    loop {
        let Some(strong) = inner.upgrade() else { return };
        let poll = strong.config.session_poll;
        if strong.live().stopping {
            return;
        }
        let now = fingerprint(&strong.external_sessions().await);
        if last.as_ref().is_some_and(|last| *last != now)
            && let Err(error) = strong.log.append(kind::SESSION_CHANGED, None, json!({}))
        {
            tracing::error!(%error, "could not log a session change");
        }
        last = Some(now);
        drop(strong);
        tokio::time::sleep(poll).await;
    }
}

fn fingerprint(sessions: &[RunningSession]) -> Fingerprint {
    let mut print: Fingerprint = sessions
        .iter()
        .map(|session| (session.pid, session.session_id.clone(), session.status.clone()))
        .collect();
    print.sort();
    print
}
