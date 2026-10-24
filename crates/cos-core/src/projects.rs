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
    /// A city name with `-vN` if needed, unique within the project. Also the
    /// folder name.
    pub name: String,
    /// The last segment of the branch, which clients show when there's no
    /// title. The same as `name`.
    #[serde(default)]
    pub city: String,
    /// What people call it, if anyone named it.
    pub title: Option<String>,
    /// `<project slug>/<city>`; `cos/<city>` for workspaces from before.
    pub branch: String,
    pub path: PathBuf,
    pub base: Base,
    /// What it started from: `origin/<branch>`, or the local branch for `head`.
    pub base_ref: String,
    pub base_commit: String,
    pub created_at: Timestamp,
    /// The Chief of Staff's own worktree, which isn't listed as a workspace.
    #[serde(skip)]
    pub staff: bool,
}

impl Workspace {
    /// The title, or the city when it has none: what to show people.
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
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

    /// Creates a worktree for the project on a new `<project slug>/<city>` branch.
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

        let folder = self.folder(&project)?;
        let prefix = branch_prefix(&project, root).await?;

        let mut free_name = None;
        for name in candidate_names(city) {
            let taken = self.store.workspace_name_taken(project_id, &name)?
                || folder.join(&name).exists()
                || git::branch_exists(root, &format!("{prefix}/{name}")).await;
            if !taken {
                free_name = Some(name);
                break;
            }
        }
        let name = free_name.ok_or_else(|| {
            ProjectError::Conflict(format!("every name based on {city} is taken; try again"))
        })?;
        let branch = format!("{prefix}/{name}");
        let path = folder.join(&name);

        git::add_worktree(root, &path, &branch, &base_commit).await?;
        let new = NewWorkspace {
            project_id,
            staff: false,
            title: clean_title(title),
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
                "city": workspace.city,
                "title": workspace.title,
                "branch": workspace.branch,
                "path": workspace.path,
                "base": workspace.base,
            }),
        )?;
        Ok(workspace)
    }

    /// The Chief of Staff's worktree for the project: `<project slug>/chief`
    /// from the main checkout's `HEAD` (no fetch), in `<folder>/chief`.
    /// Created the first time, and again if its folder was deleted. It is a
    /// workspace row with `staff` set, so listings leave it out.
    pub async fn staff_workspace(&self, project_id: i64) -> Result<Workspace, ProjectError> {
        let _one_at_a_time = self.creating.lock().await;
        let project = self.project(project_id)?;
        let root = &project.root;
        if !root.is_dir() {
            return Err(ProjectError::Invalid(format!(
                "{} is missing; was the repo moved?",
                root.display()
            )));
        }
        if let Some(workspace) = self.store.staff_workspace(project_id)? {
            if !workspace.path.is_dir() {
                git::prune_worktrees(root).await?;
                if git::branch_exists(root, &workspace.branch).await {
                    git::add_worktree_on(root, &workspace.path, &workspace.branch).await?;
                } else {
                    let head = git::commit_of(root, "HEAD").await?;
                    git::add_worktree(root, &workspace.path, &workspace.branch, &head).await?;
                }
            }
            return Ok(workspace);
        }
        let path = self.folder(&project)?.join(STAFF_NAME);
        let branch = format!("{}/{STAFF_NAME}", branch_prefix(&project, root).await?);
        let base_ref = git::current_branch(root).await?;
        let base_commit = git::commit_of(root, "HEAD").await?;
        if path.is_dir() {
            // Left behind by a database that was reset; git still knows it.
        } else if git::branch_exists(root, &branch).await {
            git::prune_worktrees(root).await?;
            git::add_worktree_on(root, &path, &branch).await?;
        } else {
            git::add_worktree(root, &path, &branch, &base_commit).await?;
        }
        let new = NewWorkspace {
            project_id,
            staff: true,
            name: STAFF_NAME.into(),
            title: None,
            branch,
            path,
            base: Base::Head,
            base_ref,
            base_commit,
        };
        Ok(self.store.insert_workspace(new)?)
    }

    /// The Chief of Staff's worktree, if it was ever made.
    pub fn existing_staff_workspace(&self, project_id: i64) -> Result<Option<Workspace>, ProjectError> {
        Ok(self.store.staff_workspace(project_id)?)
    }

    /// Where the Chief of Staff's worktree is, or will be, and its branch.
    pub fn staff_location(&self, project_id: i64) -> Result<(PathBuf, String), ProjectError> {
        if let Some(workspace) = self.store.staff_workspace(project_id)? {
            return Ok((workspace.path, workspace.branch));
        }
        let project = self.project(project_id)?;
        let path = self
            .worktrees
            .join(format!("{}-{}", project.name, project.id))
            .join(STAFF_NAME);
        Ok((path, format!("{}/{STAFF_NAME}", slug(&project.name))))
    }

    /// Gives a workspace a title, or clears it with `None` or an empty one.
    pub async fn rename_workspace(&self, id: i64, title: Option<&str>) -> Result<Workspace, ProjectError> {
        let title = clean_title(title);
        let workspace = self.listed_workspace(id)?;
        self.store.rename_workspace(id, title.as_deref())?;
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
                    workspace.label(),
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
        if workspace.staff {
            return Ok(());
        }
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

    /// A workspace, or the Chief of Staff's worktree.
    pub fn workspace(&self, id: i64) -> Result<Workspace, ProjectError> {
        self.store
            .workspace(id)?
            .ok_or_else(|| ProjectError::NotFound(format!("there is no workspace {id}")))
    }

    /// A workspace people see: not the Chief of Staff's worktree.
    pub fn listed_workspace(&self, id: i64) -> Result<Workspace, ProjectError> {
        self.workspace(id)
            .ok()
            .filter(|workspace| !workspace.staff)
            .ok_or_else(|| ProjectError::NotFound(format!("there is no workspace {id}")))
    }

    /// `<cos home>/worktrees/<project name>-<id>`, created if needed.
    fn folder(&self, project: &Project) -> Result<PathBuf, ProjectError> {
        let folder = self.worktrees.join(format!("{}-{}", project.name, project.id));
        std::fs::create_dir_all(&folder).map_err(|error| {
            ProjectError::Invalid(format!("could not create {}: {error}", folder.display()))
        })?;
        Ok(folder)
    }
}

/// The folder and branch name of the Chief of Staff's worktree.
const STAFF_NAME: &str = "chief";

/// A project's name as a git-safe branch prefix: lowercase, every run of
/// characters other than `[a-z0-9]` made one `-`, no `-` at either end.
pub fn slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "project".into()
    } else {
        slug.into()
    }
}

/// The prefix of the project's workspace branches. Git can't hold a branch
/// `x` and branches under `x/` at once, so a branch named like the slug is
/// refused with a message rather than a git error.
async fn branch_prefix(project: &Project, root: &Path) -> Result<String, ProjectError> {
    let prefix = slug(&project.name);
    if git::branch_exists(root, &prefix).await {
        return Err(ProjectError::Conflict(format!(
            "{} has a branch named {prefix}, so cos can't make branches under {prefix}/; rename that branch",
            project.name
        )));
    }
    Ok(prefix)
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
    staff: bool,
    name: String,
    title: Option<String>,
    branch: String,
    path: PathBuf,
    base: Base,
    base_ref: String,
    base_commit: String,
}

const PROJECT_COLUMNS: &str = "id, root, name, base_branch, created_ms";
const WORKSPACE_COLUMNS: &str =
    "id, project_id, name, title, branch, path, base, base_ref, base_commit, created_ms, kind";

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
            .prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE kind = 'workspace' ORDER BY id"
            ))?
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
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE project_id = ?1 AND kind = 'workspace' ORDER BY id"
            ))?
            .query_map([id], read_workspace)?
            .collect::<Result<_, _>>()?;
        Ok(Some(project))
    }

    /// Deletes the project and its Chief of Staff's books. Its staff
    /// worktree must be gone already.
    pub(crate) fn delete_project(&self, id: i64) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM staff_inbox WHERE project_id = ?1", [id])?;
        tx.execute("DELETE FROM staff_wakeups WHERE project_id = ?1", [id])?;
        tx.execute("DELETE FROM staff_notes WHERE project_id = ?1", [id])?;
        tx.execute("DELETE FROM staff WHERE project_id = ?1", [id])?;
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn workspace(&self, id: i64) -> Result<Option<Workspace>, StoreError> {
        Ok(self
            .lock()
            .prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"
            ))?
            .query_row([id], read_workspace)
            .optional()?)
    }

    fn staff_workspace(&self, project_id: i64) -> Result<Option<Workspace>, StoreError> {
        Ok(self
            .lock()
            .prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE project_id = ?1 AND kind = 'staff'"
            ))?
            .query_row([project_id], read_workspace)
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
            "INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                if new.staff { "staff" } else { "workspace" },
            ],
        )?;
        Ok(Workspace {
            id: conn.last_insert_rowid(),
            project_id: new.project_id,
            city: new.name.clone(),
            name: new.name,
            title: new.title,
            branch: new.branch,
            path: new.path,
            base: new.base,
            base_ref: new.base_ref,
            base_commit: new.base_commit,
            created_at: timestamp(created_ms).expect("the current time is in range"),
            staff: new.staff,
        })
    }

    fn rename_workspace(&self, id: i64, title: Option<&str>) -> Result<(), StoreError> {
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
    let name: String = row.get(2)?;
    Ok(Workspace {
        id: row.get(0)?,
        project_id: row.get(1)?,
        city: name.clone(),
        name,
        title: row.get(3)?,
        branch: row.get(4)?,
        path: PathBuf::from(row.get::<_, String>(5)?),
        base,
        base_ref: row.get(7)?,
        base_commit: row.get(8)?,
        created_at: timestamp(row.get(9)?)?,
        staff: row.get_ref(10)?.as_str()? == "staff",
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
