//! Shell terminals: a login shell in a folder, for you to type in, kept by
//! the daemon like an agent's terminal but in memory only. The Mac app opens
//! one in a project's root or a session's folder. Routes, merged into
//! [`super::api::router`] (see "Shells" in `docs/agents-api.md`):
//!
//! - `GET /v1/shells`, `POST /v1/shells` with [`StartShell`]
//! - `POST /v1/shells/{id}/keys|resize`, `DELETE /v1/shells/{id}`
//! - `GET /v1/shells/{id}/terminal` (SSE, the same stream as an agent's)
//!
//! They are host-only: over the tailnet they answer 403 `host_only`, since a
//! shell is the machine itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use cos_harness::ClaudeTerminal;
use serde::{Deserialize, Serialize};

use super::agents::TerminalView;
use super::agents_api::{Keys, Resize, terminal_stream};
use super::api::{ApiError, AppState};

/// Shells running at once, at most. Exited ones don't count.
const MAX_SHELLS: usize = 32;
const DEFAULT_ROWS: u16 = 40;
const DEFAULT_COLS: u16 = 120;

/// A shell as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shell {
    pub id: i64,
    /// The project whose environment it got, if any.
    pub project_id: Option<i64>,
    pub cwd: String,
    pub pid: Option<u32>,
    pub started_ms: i64,
    pub exited: bool,
    /// Set once it has exited, unless a signal ended it.
    pub exit_code: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ShellsPage {
    pub shells: Vec<Shell>,
}

/// Body of `POST /v1/shells`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StartShell {
    /// An absolute path to an existing folder.
    pub cwd: String,
    /// Gives the shell this project's environment (`CLAUDE_CONFIG_DIR` and
    /// the rest), as its agents get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

struct Entry {
    shell: Shell,
    terminal: ClaudeTerminal,
}

impl Entry {
    fn view(&self) -> Shell {
        let exit = self.terminal.exit_status();
        Shell {
            exited: exit.is_some(),
            exit_code: exit.flatten(),
            ..self.shell.clone()
        }
    }
}

/// The daemon's shells. Cheap to clone. Dropping the last clone kills them.
#[derive(Clone)]
pub(crate) struct Shells {
    inner: Arc<Mutex<Inner>>,
    program: PathBuf,
}

#[derive(Default)]
struct Inner {
    next_id: i64,
    shells: BTreeMap<i64, Entry>,
}

impl Shells {
    /// Shells running `$SHELL` (else the OS's usual one) as a login shell.
    pub(crate) fn new() -> Self {
        Self::with_program(super::judge::login_shell())
    }

    pub(crate) fn with_program(program: PathBuf) -> Self {
        Self {
            inner: Arc::default(),
            program,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn list(&self) -> Vec<Shell> {
        self.lock().shells.values().map(Entry::view).collect()
    }

    fn start(
        &self,
        cwd: &Path,
        project_id: Option<i64>,
        env: &[(String, String)],
        rows: u16,
        cols: u16,
    ) -> Result<Shell, ApiError> {
        let mut inner = self.lock();
        // Exited shells go when a new one starts; their streams already ended.
        inner
            .shells
            .retain(|_, entry| entry.terminal.exit_status().is_none());
        if inner.shells.len() >= MAX_SHELLS {
            return Err(error(
                StatusCode::CONFLICT,
                format!("{MAX_SHELLS} terminals are open already; close one first"),
            ));
        }
        let (terminal, _events) =
            ClaudeTerminal::spawn_program(&self.program, &["-l".into()], cwd, env, rows, cols)
                .map_err(|error| error_500(error.to_string()))?;
        inner.next_id += 1;
        let shell = Shell {
            id: inner.next_id,
            project_id,
            cwd: cwd.display().to_string(),
            pid: terminal.pid(),
            started_ms: now_ms(),
            exited: false,
            exit_code: None,
        };
        inner.shells.insert(
            shell.id,
            Entry {
                shell: shell.clone(),
                terminal,
            },
        );
        Ok(shell)
    }

    fn with<T>(&self, id: i64, f: impl FnOnce(&Entry) -> T) -> Result<T, ApiError> {
        self.lock().shells.get(&id).map(f).ok_or_else(no_such_shell)
    }

    /// Kills it (if it's still running) and forgets it.
    fn close(&self, id: i64) -> Result<(), ApiError> {
        let entry = self.lock().shells.remove(&id).ok_or_else(no_such_shell)?;
        drop(entry);
        Ok(())
    }

    /// Kills every shell, when the daemon stops.
    pub(crate) fn close_all(&self) {
        let shells = std::mem::take(&mut self.lock().shells);
        drop(shells);
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/shells", get(list_shells).post(start_shell))
        .route("/v1/shells/{id}", axum::routing::delete(close_shell))
        .route("/v1/shells/{id}/keys", post(keys))
        .route("/v1/shells/{id}/resize", post(resize))
        .route("/v1/shells/{id}/terminal", get(terminal))
}

type Id = Result<UrlPath<i64>, PathRejection>;

fn shell_id(id: Id) -> Result<i64, ApiError> {
    id.map(|UrlPath(id)| id).map_err(|_| no_such_shell())
}

async fn list_shells(State(state): State<AppState>) -> Json<ShellsPage> {
    Json(ShellsPage {
        shells: state.shells.list(),
    })
}

async fn start_shell(
    State(state): State<AppState>,
    body: Result<Json<StartShell>, JsonRejection>,
) -> Result<(StatusCode, Json<Shell>), ApiError> {
    let Json(body) = body?;
    let cwd = PathBuf::from(body.cwd.trim());
    if !cwd.is_absolute() {
        return Err(bad_request("`cwd` must be an absolute path".into()));
    }
    if !cwd.is_dir() {
        return Err(bad_request(format!("{} isn't a folder", cwd.display())));
    }
    let env: Vec<(String, String)> = match body.project_id {
        Some(project_id) => {
            state.projects.get(project_id).await?;
            state.agents.env().effective(project_id)?.into_iter().collect()
        }
        None => Vec::new(),
    };
    let rows = body.rows.filter(|rows| *rows > 0).unwrap_or(DEFAULT_ROWS);
    let cols = body.cols.filter(|cols| *cols > 0).unwrap_or(DEFAULT_COLS);
    let shell = state.shells.start(&cwd, body.project_id, &env, rows, cols)?;
    Ok((StatusCode::CREATED, Json(shell)))
}

async fn close_shell(State(state): State<AppState>, id: Id) -> Result<StatusCode, ApiError> {
    state.shells.close(shell_id(id)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn keys(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Keys>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = shell_id(id)?;
    let Json(body) = body?;
    let bytes = BASE64
        .decode(body.data.trim())
        .map_err(|error| bad_request(format!("`data` isn't valid base64: {error}")))?;
    state
        .shells
        .with(id, |entry| entry.terminal.write(&bytes))?
        .map_err(|error| error_500(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resize(
    State(state): State<AppState>,
    id: Id,
    body: Result<Json<Resize>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = shell_id(id)?;
    let Json(body) = body?;
    if body.rows == 0 || body.cols == 0 {
        return Err(bad_request("`rows` and `cols` must be at least 1".into()));
    }
    state
        .shells
        .with(id, |entry| entry.terminal.resize(body.rows, body.cols))?
        .map_err(|error| error_500(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn terminal(State(state): State<AppState>, id: Id) -> Result<impl IntoResponse, ApiError> {
    let attach = state
        .shells
        .with(shell_id(id)?, |entry| entry.terminal.attach())?;
    Ok(terminal_stream(
        TerminalView::Live(attach),
        state.stopping.clone(),
    ))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| {
        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
    })
}

fn no_such_shell() -> ApiError {
    ApiError::not_found("there is no such terminal")
}

fn bad_request(message: String) -> ApiError {
    error(StatusCode::BAD_REQUEST, message)
}

fn error_500(message: String) -> ApiError {
    error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn error(status: StatusCode, message: String) -> ApiError {
    ApiError { status, message }
}
