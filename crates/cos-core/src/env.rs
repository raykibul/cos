//! Environment variables for agents: a global list, and one per project
//! whose names win over the global ones. Every agent launched for a project
//! gets the merge (see [`Environment::effective`]); the rules are in the
//! "Environment" section of `docs/daemon.md`.
//!
//! Values are secrets (API keys, tokens). Nothing here puts a value in an
//! event, a log line or an error message; only names.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::events::kind;
use crate::{EventLog, Store, StoreError};

/// The longest name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// The longest value, in bytes.
pub const MAX_VALUE_BYTES: usize = 32 * 1024;

/// Variables by name, sorted.
pub type Variables = BTreeMap<String, String>;

/// `GET /v1/env`: the global list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalEnv {
    pub variables: Variables,
}

/// `GET /v1/projects/{id}/env`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectEnv {
    /// The project's own list.
    pub variables: Variables,
    /// The global list.
    pub global: Variables,
    /// What the project's agents get: the global list with the project's on top.
    pub effective: Variables,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    /// Bad input. The message is written for the user and never holds a value.
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for EnvError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error.into())
    }
}

/// Checks a name's shape: `^[A-Za-z_][A-Za-z0-9_]*$`, at most
/// [`MAX_NAME_CHARS`]. Which names cos keeps for itself is the daemon's call.
pub fn check_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let shaped = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !shaped {
        return Err(
            "a variable name is letters, digits and underscores, and doesn't start with a digit".into(),
        );
    }
    if name.len() > MAX_NAME_CHARS {
        return Err(format!("a variable name is at most {MAX_NAME_CHARS} characters"));
    }
    Ok(())
}

/// Checks a value: at most [`MAX_VALUE_BYTES`], no NUL. The message never
/// quotes the value.
pub fn check_value(name: &str, value: &str) -> Result<(), String> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(format!(
            "the value of {name} is {} bytes; the most is {MAX_VALUE_BYTES}",
            value.len()
        ));
    }
    if value.contains('\0') {
        return Err(format!("the value of {name} contains a NUL byte"));
    }
    Ok(())
}

/// `value` with a leading `~` (exactly `~`, or `~/…`) replaced by `home`.
/// People write `CLAUDE_CONFIG_DIR=~/.claude-personal`, and no shell expands
/// it for an agent. Anything else, `~user` included, is left as it is.
pub fn expand_home(value: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return value.to_owned();
    };
    if value == "~" {
        return home.display().to_string();
    }
    match value.strip_prefix("~/") {
        Some(rest) => home.join(rest).display().to_string(),
        None => value.to_owned(),
    }
}

/// The variables, global and per project. Cheap to clone.
#[derive(Clone)]
pub struct Environment {
    store: Arc<Store>,
    log: EventLog,
}

impl Environment {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// The global list.
    pub fn global(&self) -> Result<Variables, StoreError> {
        Ok(self.store.env_list(None)?)
    }

    /// A project's own list, the global list and their merge.
    pub fn project(&self, project_id: i64) -> Result<ProjectEnv, EnvError> {
        self.require_project(project_id)?;
        let variables = self.store.env_list(Some(project_id))?;
        let global = self.store.env_list(None)?;
        let mut effective = global.clone();
        effective.extend(variables.clone());
        Ok(ProjectEnv {
            variables,
            global,
            effective,
        })
    }

    /// What an agent of the project gets: the global list with the
    /// project's on top. Values as stored (`~` not yet expanded). An unknown
    /// project gets the global list.
    pub fn effective(&self, project_id: i64) -> Result<Variables, StoreError> {
        let mut effective = self.store.env_list(None)?;
        effective.extend(self.store.env_list(Some(project_id))?);
        Ok(effective)
    }

    /// Sets a variable, globally (`None`) or for a project. Logs
    /// `env.changed` with the name only.
    pub fn set(&self, project_id: Option<i64>, name: &str, value: &str) -> Result<(), EnvError> {
        check_name(name).map_err(EnvError::Invalid)?;
        check_value(name, value).map_err(EnvError::Invalid)?;
        if let Some(id) = project_id {
            self.require_project(id)?;
        }
        self.store.env_set(project_id, name, value)?;
        self.log_changed(project_id, name, "set")?;
        Ok(())
    }

    /// Removes a variable. Returns whether there was one; logs `env.changed`
    /// only then.
    pub fn unset(&self, project_id: Option<i64>, name: &str) -> Result<bool, EnvError> {
        check_name(name).map_err(EnvError::Invalid)?;
        if let Some(id) = project_id {
            self.require_project(id)?;
        }
        let removed = self.store.env_unset(project_id, name)?;
        if removed {
            self.log_changed(project_id, name, "removed")?;
        }
        Ok(removed)
    }

    fn require_project(&self, id: i64) -> Result<(), EnvError> {
        if self.store.project_exists(id)? {
            Ok(())
        } else {
            Err(EnvError::NotFound(format!("there is no project {id}")))
        }
    }

    fn log_changed(&self, project_id: Option<i64>, name: &str, action: &str) -> Result<(), StoreError> {
        self.log.append(
            kind::ENV_CHANGED,
            None,
            json!({ "project_id": project_id, "name": name, "action": action }),
        )?;
        Ok(())
    }
}

// ---- Storage ----

impl Store {
    fn env_list(&self, project_id: Option<i64>) -> rusqlite::Result<Variables> {
        let conn = self.lock();
        let mut query =
            conn.prepare_cached("SELECT name, value FROM env WHERE project_id IS ?1 ORDER BY name")?;
        let rows = query.query_map([project_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    fn env_set(&self, project_id: Option<i64>, name: &str, value: &str) -> rusqlite::Result<()> {
        let mut conn = self.lock();
        let now = Timestamp::now().as_millisecond();
        // An upsert can't name the expression index that keeps global names
        // unique, so update first and insert if nothing was there.
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE env SET value = ?3, updated_ms = ?4 WHERE project_id IS ?1 AND name = ?2",
            params![project_id, name, value, now],
        )?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO env (project_id, name, value, updated_ms) VALUES (?1, ?2, ?3, ?4)",
                params![project_id, name, value, now],
            )?;
        }
        tx.commit()
    }

    fn env_unset(&self, project_id: Option<i64>, name: &str) -> rusqlite::Result<bool> {
        let removed = self.lock().execute(
            "DELETE FROM env WHERE project_id IS ?1 AND name = ?2",
            params![project_id, name],
        )?;
        Ok(removed > 0)
    }

    fn project_exists(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .lock()
            .query_row("SELECT 1 FROM projects WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests;
