//! Projects (registered git repos) and their workspaces (worktrees cos owns,
//! where agents run). The rules are in `docs/daemon.md`.

use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cities::CITIES;
use crate::events::kind;
use crate::{EventLog, Store, StoreError, git};

const MAX_TITLE_CHARS: usize = 200;
/// `lisbon`, then `lisbon-v1` … `lisbon-v99`.
const MAX_NAME_VERSION: u32 = 99;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    /// The repo folder's name.
    pub name: String,
    /// The main checkout.
    pub root: PathBuf,
    /// The branch new work is based on.
    pub base_branch: String,
    /// Whether workspaces can start from `origin`.
    pub has_origin: bool,
    pub created_at: Timestamp,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: i64,
    pub project_id: i64,
    /// A city name, unique within the project. Also the folder name.
    pub name: String,
    /// What people call it. Starts as the name.
    pub title: String,
    pub branch: String,
    pub path: PathBuf,
    pub base: Base,
    /// What it started from: `origin/<branch>`, or the local branch for `head`.
    pub base_ref: String,
    pub base_commit: String,
    pub created_at: Timestamp,
}

/// Where a new workspace starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Base {
    /// Freshly fetched `origin/<base branch>`.
    #[default]
    Origin,
    /// The main checkout's current `HEAD`, including unpushed commits.
    Head,
}

impl Base {
    fn as_str(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Head => "head",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// Bad input, or git refused. The message is written for the user.
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<git::GitError> for ProjectError {
    fn from(error: git::GitError) -> Self {
        Self::Invalid(error.to_string())
    }
}

impl From<rusqlite::Error> for ProjectError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error.into())
    }
}

pub struct Projects {
    store: Arc<Store>,
    log: EventLog,
    /// Workspace folders go here: `<cos home>/worktrees`.
    worktrees: PathBuf,
    /// One workspace creation at a time, so two can't pick the same name.
    creating: tokio::sync::Mutex<()>,
}

impl Projects {
    pub fn new(store: Arc<Store>, log: EventLog, worktrees: PathBuf) -> Self {
        Self {
            store,
            log,
            worktrees,
            creating: tokio::sync::Mutex::new(()),
        }
    }

    /// All projects in the order they were added, with their workspaces.
    pub async fn list(&self) -> Result<Vec<Project>, ProjectError> {
        let mut projects = self.store.projects()?;
        for project in &mut projects {
            project.has_origin = git::has_origin(&project.root).await;
        }
        Ok(projects)
    }

    pub async fn get(&self, id: i64) -> Result<Project, ProjectError> {
        let mut project = self.project(id)?;
        project.has_origin = git::has_origin(&project.root).await;
        Ok(project)
    }

    /// Registers the repo containing `path`. Returns the project and whether
    /// it is new. Adding a repo again refreshes its base branch.
    pub async fn add(&self, path: &Path) -> Result<(Project, bool), ProjectError> {
        if !path.is_absolute() {
            return Err(ProjectError::Invalid(format!(
                "{} is not an absolute path",
                path.display()
            )));
        }
        if !path.is_dir() {
            return Err(ProjectError::Invalid(format!(
                "{} is not a folder",
                path.display()
            )));
        }
        let root = git::main_checkout(path).await?;
        let name = root
            .file_name()
            .map_or_else(|| "repo".to_owned(), |name| name.to_string_lossy().into_owned());
        let base_branch = git::base_branch(&root).await;

        let (id, created) = self.store.upsert_project(&root, &name, &base_branch)?;
        if created {
            self.log.append(
                kind::PROJECT_ADDED,
                None,
                json!({ "project_id": id, "name": name, "root": root }),
            )?;
        }
        Ok((self.get(id).await?, created))
    }

    /// Unregisters a project. The repo itself is never touched.
    pub async fn remove(&self, id: i64) -> Result<(), ProjectError> {
        let project = self.project(id)?;
        if !project.workspaces.is_empty() {
            return Err(ProjectError::Conflict(format!(
                "{} still has {} workspace(s); remove them first",
                project.name,
                project.workspaces.len()
            )));
        }
        self.store.delete_project(id)?;
        self.log.append(
            kind::PROJECT_REMOVED,
            None,
            json!({ "project_id": id, "name": project.name }),
        )?;
        Ok(())
    }

    /// Creates a worktree for the project on a new `cos/<city>` branch.
    pub async fn create_workspace(
        &self,
        project_id: i64,
        base: Base,
        title: Option<&str>,
    ) -> Result<Workspace, ProjectError> {
        let city = CITIES[(RandomState::new().hash_one(Timestamp::now()) % CITIES.len() as u64) as usize];
        self.create_workspace_from(project_id, base, title, city).await
    }

    /// [`Self::create_workspace`] with the first name to try chosen by the caller.
    async fn create_workspace_from(
        &self,
        project_id: i64,
        base: Base,
        title: Option<&str>,
        city: &str,
    ) -> Result<Workspace, ProjectError> {
        let _one_at_a_time = self.creating.lock().await;
        let project = self.project(project_id)?;
        let root = &project.root;
        if !root.is_dir() {
            return Err(ProjectError::Invalid(format!(
                "{} is missing; was the repo moved?",
                root.display()
            )));
        }

        let (base_ref, base_commit) = match base {
            Base::Origin => {
                if !git::has_origin(root).await {
                    return Err(ProjectError::Invalid(format!(
                        "{} has no origin remote, so a workspace can only start from your current checkout (head)",
                        project.name
                    )));
                }
                let branch = &project.base_branch;
                git::fetch(root, branch).await.map_err(|error| {
                    ProjectError::Invalid(format!(
                        "could not fetch origin/{branch}, so no workspace was created: {error}"
                    ))
                })?;
                let base_ref = format!("origin/{branch}");
                let commit = git::commit_of(root, &base_ref).await?;
                (base_ref, commit)
            }
            Base::Head => (
                git::current_branch(root).await?,
                git::commit_of(root, "HEAD").await?,
            ),
        };

        let folder = self.worktrees.join(format!("{}-{}", project.name, project.id));
        std::fs::create_dir_all(&folder).map_err(|error| {
            ProjectError::Invalid(format!("could not create {}: {error}", folder.display()))
        })?;

        let mut free_name = None;
        for name in candidate_names(city) {
            let taken = self.store.workspace_name_taken(project_id, &name)?
                || folder.join(&name).exists()
                || git::branch_exists(root, &branch_for(&name)).await;
            if !taken {
                free_name = Some(name);
                break;
            }
        }
        let name = free_name.ok_or_else(|| {
            ProjectError::Conflict(format!("every name based on {city} is taken; try again"))
        })?;
        let branch = branch_for(&name);
        let path = folder.join(&name);

        git::add_worktree(root, &path, &branch, &base_commit).await?;
        let new = NewWorkspace {
            project_id,
            title: clean_title(title).unwrap_or_else(|| name.clone()),
            name,
            branch,
            path: path.clone(),
            base,
            base_ref,
            base_commit,
        };
        let workspace = match self.store.insert_workspace(new) {
            Ok(workspace) => workspace,
            Err(error) => {
                // Don't leave a worktree nobody knows about.
                let _ = git::remove_worktree(root, &path).await;
                return Err(error.into());
            }
        };

        self.log.append(
            kind::WORKSPACE_CREATED,
            None,
            json!({
                "workspace_id": workspace.id,
                "project_id": project_id,
                "name": workspace.name,
                "branch": workspace.branch,
                "path": workspace.path,
                "base": workspace.base,
            }),
        )?;
        Ok(workspace)
    }

    pub async fn rename_workspace(&self, id: i64, title: &str) -> Result<Workspace, ProjectError> {
        let title =
            clean_title(Some(title)).ok_or_else(|| ProjectError::Invalid("a title can't be empty".into()))?;
        let workspace = self.workspace(id)?;
        self.store.rename_workspace(id, &title)?;
        self.log.append(
            kind::WORKSPACE_RENAMED,
            None,
            json!({ "workspace_id": id, "project_id": workspace.project_id, "title": title }),
        )?;
        Ok(Workspace { title, ..workspace })
    }

    /// Deletes the workspace's folder and forgets it. Refuses if the folder
    /// has uncommitted changes. The branch, and so every commit, stays.
    pub async fn remove_workspace(&self, id: i64) -> Result<(), ProjectError> {
        let workspace = self.workspace(id)?;
        let project = self.project(workspace.project_id)?;

        if workspace.path.exists() {
            if git::has_uncommitted_changes(&workspace.path).await? {
                return Err(ProjectError::Conflict(format!(
                    "{} has uncommitted changes in {}. Commit or discard them first; its branch {} is kept either way.",
                    workspace.title,
                    workspace.path.display(),
                    workspace.branch
                )));
            }
            git::remove_worktree(&project.root, &workspace.path).await?;
        } else {
            // Someone deleted the folder by hand; tell git it's gone.
            git::prune_worktrees(&project.root).await?;
        }

        self.store.delete_workspace(id)?;
        self.log.append(
            kind::WORKSPACE_REMOVED,
            None,
            json!({
                "workspace_id": id,
                "project_id": workspace.project_id,
                "name": workspace.name,
                "branch": workspace.branch,
            }),
        )?;
        Ok(())
    }

    fn project(&self, id: i64) -> Result<Project, ProjectError> {
        self.store
            .project(id)?
            .ok_or_else(|| ProjectError::NotFound(format!("there is no project {id}")))
    }

    fn workspace(&self, id: i64) -> Result<Workspace, ProjectError> {
        self.store
            .workspace(id)?
            .ok_or_else(|| ProjectError::NotFound(format!("there is no workspace {id}")))
    }
}

fn branch_for(name: &str) -> String {
    format!("cos/{name}")
}

fn candidate_names(city: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(city.to_owned())
        .chain((1..=MAX_NAME_VERSION).map(move |version| format!("{city}-v{version}")))
}

fn clean_title(title: Option<&str>) -> Option<String> {
    let title = title?.trim();
    (!title.is_empty()).then(|| title.chars().take(MAX_TITLE_CHARS).collect())
}

// ---- Storage ----

struct NewWorkspace {
    project_id: i64,
    name: String,
    title: String,
    branch: String,
    path: PathBuf,
    base: Base,
    base_ref: String,
    base_commit: String,
}

const PROJECT_COLUMNS: &str = "id, root, name, base_branch, created_ms";
const WORKSPACE_COLUMNS: &str =
    "id, project_id, name, title, branch, path, base, base_ref, base_commit, created_ms";

impl Store {
    /// Returns the project id and whether the row is new.
    fn upsert_project(&self, root: &Path, name: &str, base_branch: &str) -> Result<(i64, bool), StoreError> {
        let conn = self.lock();
        let root = root.to_string_lossy();
        let existing: Option<i64> = conn
            .query_row("SELECT id FROM projects WHERE root = ?1", [&root], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(id) = existing {
            conn.execute(
                "UPDATE projects SET base_branch = ?2 WHERE id = ?1",
                params![id, base_branch],
            )?;
            return Ok((id, false));
        }
        conn.execute(
            "INSERT INTO projects (root, name, base_branch, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![root, name, base_branch, now_ms()],
        )?;
        Ok((conn.last_insert_rowid(), true))
    }

    /// `has_origin` is left false; it comes from git, not the database.
    fn projects(&self) -> Result<Vec<Project>, StoreError> {
        let conn = self.lock();
        let mut projects = conn
            .prepare_cached(&format!("SELECT {PROJECT_COLUMNS} FROM projects ORDER BY id"))?
            .query_map([], read_project)?
            .collect::<Result<Vec<_>, _>>()?;
        let workspaces = conn
            .prepare_cached(&format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces ORDER BY id"))?
            .query_map([], read_workspace)?
            .collect::<Result<Vec<_>, _>>()?;
        for workspace in workspaces {
            if let Some(project) = projects.iter_mut().find(|p| p.id == workspace.project_id) {
                project.workspaces.push(workspace);
            }
        }
        Ok(projects)
    }

    fn project(&self, id: i64) -> Result<Option<Project>, StoreError> {
        let conn = self.lock();
        let Some(mut project) = conn
            .prepare_cached(&format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"))?
            .query_row([id], read_project)
            .optional()?
        else {
            return Ok(None);
        };
        project.workspaces = conn
            .prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE project_id = ?1 ORDER BY id"
            ))?
            .query_map([id], read_workspace)?
            .collect::<Result<_, _>>()?;
        Ok(Some(project))
    }

    fn delete_project(&self, id: i64) -> Result<(), StoreError> {
        self.lock().execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    fn workspace(&self, id: i64) -> Result<Option<Workspace>, StoreError> {
        Ok(self
            .lock()
            .prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"
            ))?
            .query_row([id], read_workspace)
            .optional()?)
    }

    fn workspace_name_taken(&self, project_id: i64, name: &str) -> Result<bool, StoreError> {
        Ok(self.lock().query_row(
            "SELECT EXISTS (SELECT 1 FROM workspaces WHERE project_id = ?1 AND name = ?2)",
            params![project_id, name],
            |row| row.get(0),
        )?)
    }

    fn insert_workspace(&self, new: NewWorkspace) -> Result<Workspace, StoreError> {
        let conn = self.lock();
        let created_ms = now_ms();
        conn.execute(
            "INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                new.project_id,
                new.name,
                new.title,
                new.branch,
                new.path.to_string_lossy(),
                new.base.as_str(),
                new.base_ref,
                new.base_commit,
                created_ms,
            ],
        )?;
        Ok(Workspace {
            id: conn.last_insert_rowid(),
            project_id: new.project_id,
            name: new.name,
            title: new.title,
            branch: new.branch,
            path: new.path,
            base: new.base,
            base_ref: new.base_ref,
            base_commit: new.base_commit,
            created_at: timestamp(created_ms).expect("the current time is in range"),
        })
    }

    fn rename_workspace(&self, id: i64, title: &str) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE workspaces SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    fn delete_workspace(&self, id: i64) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM workspaces WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn read_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        root: PathBuf::from(row.get::<_, String>(1)?),
        name: row.get(2)?,
        base_branch: row.get(3)?,
        has_origin: false,
        created_at: timestamp(row.get(4)?)?,
        workspaces: Vec::new(),
    })
}

fn read_workspace(row: &Row<'_>) -> rusqlite::Result<Workspace> {
    let base = match row.get_ref(6)?.as_str()? {
        "origin" => Base::Origin,
        "head" => Base::Head,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                6,
                Type::Text,
                format!("unknown workspace base {other:?}").into(),
            ));
        }
    };
    Ok(Workspace {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        title: row.get(3)?,
        branch: row.get(4)?,
        path: PathBuf::from(row.get::<_, String>(5)?),
        base,
        base_ref: row.get(7)?,
        base_commit: row.get(8)?,
        created_at: timestamp(row.get(9)?)?,
    })
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

fn timestamp(ms: i64) -> rusqlite::Result<Timestamp> {
    Timestamp::from_millisecond(ms)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error)))
}

#[cfg(test)]
mod tests;
