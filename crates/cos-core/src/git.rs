//! The git commands cos needs, run as child processes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

/// Long enough for a slow fetch; a hung git (a credential prompt, a dead
/// network) must not block the daemon forever.
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GitError(String);

/// Runs `git -C <dir> <args>` and returns its trimmed stdout.
async fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Fail instead of waiting for a password nobody can type.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(TIMEOUT, output)
        .await
        .map_err(|_| GitError(format!("git {} took longer than {TIMEOUT:?}", args.join(" "))))?
        .map_err(|error| GitError(format!("could not run git: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The main checkout of the repository containing `path`. A subfolder or a
/// linked worktree resolves to the same place.
pub(crate) async fn main_checkout(path: &Path) -> Result<PathBuf, GitError> {
    let list = git(path, &["worktree", "list", "--porcelain"])
        .await
        .map_err(|_| GitError(format!("{} is not inside a git repository", path.display())))?;
    let first = list
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .ok_or_else(|| GitError(format!("{} has no working checkout", path.display())))?;
    std::fs::canonicalize(first).map_err(|error| GitError(format!("could not resolve {first}: {error}")))
}

pub(crate) async fn has_origin(root: &Path) -> bool {
    git(root, &["remote"])
        .await
        .is_ok_and(|remotes| remotes.lines().any(|remote| remote == "origin"))
}

/// The branch new work is based on: what `origin/HEAD` points to, else the
/// checked-out branch, else `main`.
pub(crate) async fn base_branch(root: &Path) -> String {
    if let Ok(head) = git(root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).await
        && let Some(branch) = head.strip_prefix("origin/")
    {
        return branch.to_owned();
    }
    match git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok(branch) if branch != "HEAD" => branch,
        _ => "main".to_owned(),
    }
}

pub(crate) async fn fetch(root: &Path, branch: &str) -> Result<(), GitError> {
    git(root, &["fetch", "--quiet", "origin", branch]).await.map(drop)
}

/// The full commit id `rev` points to.
pub(crate) async fn commit_of(root: &Path, rev: &str) -> Result<String, GitError> {
    git(root, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")]).await
}

/// The checked-out branch's name, or `HEAD` when detached.
pub(crate) async fn current_branch(root: &Path) -> Result<String, GitError> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await
}

pub(crate) async fn branch_exists(root: &Path, branch: &str) -> bool {
    git(
        root,
        &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")],
    )
    .await
    .is_ok()
}

/// Creates a new branch at `commit` checked out in a new worktree at `path`.
pub(crate) async fn add_worktree(
    root: &Path,
    path: &Path,
    branch: &str,
    commit: &str,
) -> Result<(), GitError> {
    let path = path.to_string_lossy();
    git(root, &["worktree", "add", "--quiet", "-b", branch, &path, commit])
        .await
        .map(drop)
}

/// True if the checkout has changes `git worktree remove` would lose,
/// including untracked files.
pub(crate) async fn has_uncommitted_changes(path: &Path) -> Result<bool, GitError> {
    Ok(!git(path, &["status", "--porcelain"]).await?.is_empty())
}

/// Removes a worktree's folder. The branch stays.
pub(crate) async fn remove_worktree(root: &Path, path: &Path) -> Result<(), GitError> {
    git(root, &["worktree", "remove", &path.to_string_lossy()])
        .await
        .map(drop)
}

/// Forgets worktrees whose folders were deleted by hand.
pub(crate) async fn prune_worktrees(root: &Path) -> Result<(), GitError> {
    git(root, &["worktree", "prune"]).await.map(drop)
}
