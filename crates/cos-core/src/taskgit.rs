//! The git side of running a task: a short-lived worktree per task branched
//! from its workspace, the file-scope check, the test run, and the merge back
//! into the workspace. The rules are in `docs/daemon.md` ("Task git
//! lifecycle"). Nothing here touches the database; the orchestrator stores a
//! [`TaskWorktree`] and calls these in order.

mod limiter;
mod scope;
mod testrun;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use limiter::{DEFAULT_WORKERS_PER_WORKSPACE, DEFAULT_WORKERS_TOTAL, WorkerLimiter, WorkerPermit};
pub use scope::{Change, ChangeStatus, Scope, ScopeReport, is_protected};
pub use testrun::{
    DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_TEST_TIMEOUT, TestOptions, TestOutcome, TestStatus, run_tests,
};

use crate::{Workspace, git};

/// `t12-add-login`, then `t12-add-login-2` … `t12-add-login-99`.
const MAX_NAME_VERSION: u32 = 99;
const MAX_SLUG_CHARS: usize = 40;

#[derive(Debug, thiserror::Error)]
pub enum TaskGitError {
    /// Bad input, or git refused. The message is written for the user.
    #[error("{0}")]
    Invalid(String),
    /// A checkout has uncommitted changes where none are allowed.
    #[error("{0}")]
    Dirty(String),
}

impl From<git::GitError> for TaskGitError {
    fn from(error: git::GitError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// A task's worktree and branch. The orchestrator keeps this for the task's
/// lifetime; every later step is a method on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskWorktree {
    pub task_id: i64,
    /// The workspace's worktree. Merges happen there.
    pub workspace: PathBuf,
    /// The branch task branches merge into: `cos/<workspace name>`.
    pub workspace_branch: String,
    /// The task's own worktree, where the worker runs.
    pub path: PathBuf,
    /// `cos/<workspace name>-t<task id>-<slug>`.
    pub branch: String,
    /// The workspace branch's commit when the task started. Scope checks and
    /// diffs are measured from here.
    pub base_commit: String,
}

/// What [`TaskWorktree::merge`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// A new `--no-ff` merge commit on the workspace branch.
    Merged { commit: String },
    /// The workspace branch already contains the task branch; nothing changed.
    AlreadyMerged,
    /// The merge was aborted and the workspace is as it was. Mark the task
    /// blocked.
    Conflict { paths: Vec<String> },
}

/// What a task branch changes relative to its base, for the reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions: u64,
    pub deletions: u64,
    /// Files git counts as binary; they add no insertions or deletions.
    pub binary_files: usize,
    /// `git diff --stat`, ready to show.
    pub stat: String,
}

/// Creates task worktrees under one cos-owned folder, like
/// `<cos home>/tasks`.
pub struct TaskWorktrees {
    dir: PathBuf,
    /// One creation at a time, so two can't pick the same name.
    creating: tokio::sync::Mutex<()>,
}

impl TaskWorktrees {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            creating: tokio::sync::Mutex::new(()),
        }
    }

    /// Creates a worktree for the task at the tip of the workspace branch, on
    /// a new branch `cos/<workspace name>-t<task id>-<slug of title>`, in
    /// `<dir>/<workspace name>-<workspace id>/t<task id>-<slug>`. A taken name
    /// gets `-2`, `-3`, … Uncommitted changes in the workspace are not copied.
    pub async fn create(
        &self,
        workspace: &Workspace,
        task_id: i64,
        title: &str,
    ) -> Result<TaskWorktree, TaskGitError> {
        let _one_at_a_time = self.creating.lock().await;
        if !workspace.path.is_dir() {
            return Err(TaskGitError::Invalid(format!(
                "workspace {} is missing at {}",
                workspace.name,
                workspace.path.display()
            )));
        }
        let base_commit = git::commit_of(&workspace.path, &workspace.branch).await?;

        let folder = self.dir.join(format!("{}-{}", workspace.name, workspace.id));
        std::fs::create_dir_all(&folder).map_err(|error| {
            TaskGitError::Invalid(format!("could not create {}: {error}", folder.display()))
        })?;

        let stem = format!("t{task_id}-{}", slug(title));
        let mut free = None;
        for name in candidate_names(&stem) {
            let branch = task_branch(&workspace.name, &name);
            if !folder.join(&name).exists() && !git::branch_exists(&workspace.path, &branch).await {
                free = Some((name, branch));
                break;
            }
        }
        let (name, branch) = free
            .ok_or_else(|| TaskGitError::Invalid(format!("every branch name for task {task_id} is taken")))?;
        let path = folder.join(name);

        git::add_worktree(&workspace.path, &path, &branch, &base_commit).await?;
        Ok(TaskWorktree {
            task_id,
            workspace: workspace.path.clone(),
            workspace_branch: workspace.branch.clone(),
            path,
            branch,
            base_commit,
        })
    }
}

impl TaskWorktree {
    /// Deletes the task's folder. Refuses if it has uncommitted changes,
    /// unless `force`. The branch always stays; see [`Self::delete_branch`].
    pub async fn remove(&self, force: bool) -> Result<(), TaskGitError> {
        if !self.path.exists() {
            // Someone deleted the folder by hand; tell git it's gone.
            git::prune_worktrees(&self.workspace).await?;
            return Ok(());
        }
        if !force && git::has_uncommitted_changes(&self.path).await? {
            return Err(TaskGitError::Dirty(format!(
                "task {} has uncommitted changes in {}; its branch {} is kept either way",
                self.task_id,
                self.path.display(),
                self.branch
            )));
        }
        let path = self.path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path);
        run(&self.workspace, &args).await.map(drop)
    }

    /// Deletes the task branch, but only once the workspace branch contains
    /// it, so no commit is ever lost. Remove the worktree first; git won't
    /// delete a branch that is checked out.
    pub async fn delete_branch(&self) -> Result<(), TaskGitError> {
        if !self.is_merged().await? {
            return Err(TaskGitError::Invalid(format!(
                "{} is not merged into {}, so it was kept",
                self.branch, self.workspace_branch
            )));
        }
        run(&self.workspace, &["branch", "-D", &self.branch])
            .await
            .map(drop)
    }

    /// Whether the workspace branch already contains every commit of the
    /// task branch.
    pub async fn is_merged(&self) -> Result<bool, TaskGitError> {
        is_ancestor(&self.workspace, &self.branch, &self.workspace_branch).await
    }

    /// How many commits the task branch has on top of its base.
    pub async fn commits_ahead(&self) -> Result<u32, TaskGitError> {
        let range = format!("{}..{}", self.base_commit, self.branch);
        let count = run(&self.workspace, &["rev-list", "--count", &range]).await?;
        count
            .trim()
            .parse()
            .map_err(|_| TaskGitError::Invalid(format!("git rev-list printed {count:?}")))
    }

    /// What the task branch changes since its base, for the reviewer.
    pub async fn diff_summary(&self) -> Result<DiffSummary, TaskGitError> {
        let range = format!("{}...{}", self.base_commit, self.branch);
        let numstat = run(&self.workspace, &["diff", "--numstat", "-z", "-M", &range]).await?;
        let mut summary = DiffSummary {
            files_changed: 0,
            insertions: 0,
            deletions: 0,
            binary_files: 0,
            stat: String::new(),
        };
        // `<ins>\t<del>\t<path>\0`, or `<ins>\t<del>\t\0<from>\0<to>\0` for a rename.
        let mut fields = numstat.split('\0');
        while let Some(entry) = fields.next() {
            if entry.is_empty() {
                continue;
            }
            let mut parts = entry.splitn(3, '\t');
            let (Some(ins), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if path.is_empty() {
                fields.next();
                fields.next();
            }
            summary.files_changed += 1;
            match (ins.parse::<u64>(), del.parse::<u64>()) {
                (Ok(ins), Ok(del)) => {
                    summary.insertions += ins;
                    summary.deletions += del;
                }
                _ => summary.binary_files += 1,
            }
        }
        summary.stat = run(&self.workspace, &["diff", "--stat", "--no-color", "-M", &range])
            .await?
            .trim_end()
            .to_owned();
        Ok(summary)
    }

    /// Merges the task branch into the workspace branch with `--no-ff` and
    /// the message `Merge task <id>: <title>`, inside the workspace worktree.
    ///
    /// Refuses if the workspace has uncommitted changes or isn't on its
    /// branch. On a conflict the merge is aborted, the workspace is left as it
    /// was, and the conflicting paths come back. Callers must not run two
    /// merges into the same workspace at once.
    pub async fn merge(&self, title: &str) -> Result<MergeOutcome, TaskGitError> {
        let workspace = &self.workspace;
        let on = git::current_branch(workspace).await?;
        if on != self.workspace_branch {
            return Err(TaskGitError::Invalid(format!(
                "the workspace at {} is on {on}, not {}, so task {} was not merged",
                workspace.display(),
                self.workspace_branch,
                self.task_id
            )));
        }
        if git::has_uncommitted_changes(workspace).await? {
            return Err(TaskGitError::Dirty(format!(
                "the workspace at {} has uncommitted changes, so task {} was not merged; commit or discard them first",
                workspace.display(),
                self.task_id
            )));
        }
        if self.is_merged().await? {
            return Ok(MergeOutcome::AlreadyMerged);
        }

        let message = merge_message(self.task_id, title);
        let merge = git::output(
            workspace,
            &["merge", "--no-ff", "--no-edit", "-m", &message, &self.branch],
        )
        .await?;
        if merge.status.success() {
            return Ok(MergeOutcome::Merged {
                commit: git::commit_of(workspace, "HEAD").await?,
            });
        }

        // Read the conflicts before aborting wipes them.
        let conflicts = run(workspace, &["diff", "--name-only", "-z", "--diff-filter=U"])
            .await
            .map(|out| split_nul(&out))
            .unwrap_or_default();
        if merge_in_progress(workspace).await {
            run(workspace, &["merge", "--abort"]).await?;
        }
        if conflicts.is_empty() {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            return Err(TaskGitError::Invalid(format!(
                "git merge of {} failed: {}",
                self.branch,
                stderr.trim()
            )));
        }
        Ok(MergeOutcome::Conflict { paths: conflicts })
    }
}

/// Runs git and returns its untrimmed stdout, or an error with its stderr.
async fn run(dir: &Path, args: &[&str]) -> Result<String, TaskGitError> {
    let output = git::output(dir, args).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TaskGitError::Invalid(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether every commit of `rev` is in `of`.
async fn is_ancestor(dir: &Path, rev: &str, of: &str) -> Result<bool, TaskGitError> {
    let output = git::output(dir, &["merge-base", "--is-ancestor", rev, of]).await?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(TaskGitError::Invalid(format!(
            "could not compare {rev} with {of}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

async fn merge_in_progress(dir: &Path) -> bool {
    git::output(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .await
        .is_ok_and(|output| output.status.success())
}

fn split_nul(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

fn merge_message(task_id: i64, title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        format!("Merge task {task_id}")
    } else {
        format!("Merge task {task_id}: {title}")
    }
}

fn task_branch(workspace_name: &str, name: &str) -> String {
    // Not `cos/<workspace>/…`: git can't have both a branch `cos/lisbon` and
    // branches under `cos/lisbon/`.
    format!("cos/{workspace_name}-{name}")
}

fn candidate_names(stem: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(stem.to_owned())
        .chain((2..=MAX_NAME_VERSION).map(move |version| format!("{stem}-{version}")))
}

/// Lowercase ASCII letters and digits joined by single dashes, at most
/// [`MAX_SLUG_CHARS`] long; `task` if nothing is left.
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if slug.len() >= MAX_SLUG_CHARS {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "task".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests;
