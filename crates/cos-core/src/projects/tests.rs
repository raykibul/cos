//! Runs real git against throwaway repos: a bare "origin", a "seed" clone that
//! pushes to it, and the "repo" clone that gets registered.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// Runs git for test setup, with an identity and no signing so commits work
/// on any machine.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=cos test",
            "-c",
            "user.email=test@cos.local",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit_file(repo: &Path, file: &str) -> String {
    std::fs::write(repo.join(file), file).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", file]);
    git(repo, &["rev-parse", "HEAD"])
}

struct Fixture {
    dir: tempfile::TempDir,
    projects: Projects,
    log: EventLog,
}

impl Fixture {
    /// `origin.git` with one commit on `trunk` (not `main`, to prove the base
    /// branch comes from origin), a `seed` clone and a `repo` clone.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "--bare", "-b", "trunk", "origin.git"]);
        git(root, &["clone", "-q", "origin.git", "seed"]);
        let seed = root.join("seed");
        git(&seed, &["checkout", "-q", "-b", "trunk"]);
        commit_file(&seed, "first.txt");
        git(&seed, &["push", "-q", "origin", "trunk"]);
        git(root, &["clone", "-q", "origin.git", "repo"]);

        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        let projects = Projects::new(store, log.clone(), root.join("worktrees"));
        Self { dir, projects, log }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name).canonicalize().unwrap()
    }

    fn repo(&self) -> PathBuf {
        self.path("repo")
    }

    async fn add_repo(&self) -> Project {
        self.projects.add(&self.repo()).await.unwrap().0
    }

    fn event_kinds(&self) -> Vec<String> {
        self.log
            .after(0, 100)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }
}

#[tokio::test]
async fn a_repo_registers_once_whatever_path_points_into_it() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    std::fs::create_dir(repo.join("sub")).unwrap();
    git(&repo, &["worktree", "add", "-q", "../linked"]);

    let (from_sub, created) = fixture.projects.add(&repo.join("sub")).await.unwrap();
    let (again, created_again) = fixture.projects.add(&repo).await.unwrap();
    let (from_linked, _) = fixture.projects.add(&fixture.path("linked")).await.unwrap();

    assert!(created && !created_again);
    assert_eq!(from_sub.root, repo);
    assert_eq!(from_sub.name, "repo");
    assert_eq!((again.id, from_linked.id), (from_sub.id, from_sub.id));
    assert_eq!(fixture.projects.list().await.unwrap().len(), 1);
    assert_eq!(fixture.event_kinds(), [kind::PROJECT_ADDED]);
}

#[tokio::test]
async fn adding_something_that_is_not_a_repo_is_refused() {
    let fixture = Fixture::new();
    let plain = fixture.dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();

    let not_repo = fixture.projects.add(&plain).await.unwrap_err();
    let relative = fixture.projects.add(Path::new("repo")).await.unwrap_err();

    assert!(
        not_repo.to_string().contains("not inside a git repository"),
        "{not_repo}"
    );
    assert!(matches!(relative, ProjectError::Invalid(_)), "{relative}");
}

#[tokio::test]
async fn base_branch_follows_origin_not_the_checked_out_branch() {
    let fixture = Fixture::new();
    git(&fixture.repo(), &["checkout", "-q", "-b", "feature"]);

    let project = fixture.add_repo().await;

    assert_eq!(project.base_branch, "trunk");
    assert!(project.has_origin);
}

#[tokio::test]
async fn origin_workspace_starts_from_a_fresh_fetch() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    // Pushed after `repo` was cloned, so only a fetch can see it.
    let seed = fixture.path("seed");
    let latest = commit_file(&seed, "second.txt");
    git(&seed, &["push", "-q", "origin", "trunk"]);

    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Origin, None)
        .await
        .unwrap();

    assert_eq!(workspace.base_commit, latest);
    assert_eq!(workspace.base_ref, "origin/trunk");
    assert!(workspace.path.join("second.txt").exists());
    assert_eq!(workspace.branch, format!("repo/{}", workspace.name));
    assert_eq!(workspace.title, None);
    assert_eq!(workspace.city, workspace.name);
    assert_eq!(workspace.label(), workspace.name);
    assert!(CITIES.contains(&workspace.name.as_str()));
    assert_eq!(
        workspace.path,
        fixture
            .dir
            .path()
            .join("worktrees")
            .join(format!("repo-{}", project.id))
            .join(&workspace.name)
    );
    assert_eq!(
        git(&workspace.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        workspace.branch
    );
    assert_eq!(
        fixture.event_kinds(),
        [kind::PROJECT_ADDED, kind::WORKSPACE_CREATED]
    );
}

#[tokio::test]
async fn head_workspace_has_unpushed_commits_but_not_uncommitted_files() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    let local = commit_file(&repo, "local.txt");
    std::fs::write(repo.join("scratch.txt"), "not committed").unwrap();
    let project = fixture.add_repo().await;

    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Head, Some("  Try the thing  "))
        .await
        .unwrap();

    assert_eq!(workspace.base_commit, local);
    assert_eq!(workspace.base_ref, "trunk");
    assert_eq!(workspace.title.as_deref(), Some("Try the thing"));
    assert!(workspace.path.join("local.txt").exists());
    assert!(!workspace.path.join("scratch.txt").exists());
}

#[tokio::test]
async fn origin_base_needs_a_remote() {
    let fixture = Fixture::new();
    let lonely = fixture.dir.path().join("lonely");
    git(fixture.dir.path(), &["init", "-q", "-b", "main", "lonely"]);
    commit_file(&lonely, "a.txt");
    let (project, _) = fixture.projects.add(&lonely).await.unwrap();

    let error = fixture
        .projects
        .create_workspace(project.id, Base::Origin, None)
        .await
        .unwrap_err();

    assert!(!project.has_origin);
    assert_eq!(project.base_branch, "main");
    assert!(error.to_string().contains("no origin remote"), "{error}");
}

#[tokio::test]
async fn failed_fetch_creates_nothing() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    git(
        &fixture.repo(),
        &["remote", "set-url", "origin", "/nonexistent/origin.git"],
    );

    let error = fixture
        .projects
        .create_workspace(project.id, Base::Origin, None)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("could not fetch origin/trunk"),
        "{error}"
    );
    assert!(
        fixture
            .projects
            .get(project.id)
            .await
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert_eq!(fixture.event_kinds(), [kind::PROJECT_ADDED]);
}

#[tokio::test]
async fn names_skip_branches_that_already_exist() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    git(&fixture.repo(), &["branch", "repo/lisbon"]);

    let first = fixture
        .projects
        .create_workspace_from(project.id, Base::Head, None, "lisbon")
        .await
        .unwrap();
    let second = fixture
        .projects
        .create_workspace_from(project.id, Base::Head, None, "lisbon")
        .await
        .unwrap();

    assert_eq!(
        (first.name.as_str(), second.name.as_str()),
        ("lisbon-v1", "lisbon-v2")
    );
    assert_ne!(first.path, second.path);
}

#[tokio::test]
async fn removing_a_workspace_keeps_its_branch_and_refuses_unsaved_work() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();
    std::fs::write(workspace.path.join("draft.txt"), "unsaved").unwrap();

    let refused = fixture.projects.remove_workspace(workspace.id).await.unwrap_err();
    assert!(matches!(refused, ProjectError::Conflict(_)), "{refused}");
    assert!(workspace.path.join("draft.txt").exists());

    std::fs::remove_file(workspace.path.join("draft.txt")).unwrap();
    fixture.projects.remove_workspace(workspace.id).await.unwrap();

    assert!(!workspace.path.exists());
    git(
        &fixture.repo(),
        &[
            "show-ref",
            "--verify",
            &format!("refs/heads/{}", workspace.branch),
        ],
    );
    assert!(
        fixture
            .projects
            .get(project.id)
            .await
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert_eq!(
        fixture.event_kinds().last().map(String::as_str),
        Some(kind::WORKSPACE_REMOVED)
    );
}

#[tokio::test]
async fn a_workspace_folder_deleted_by_hand_can_still_be_removed() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();
    std::fs::remove_dir_all(&workspace.path).unwrap();

    fixture.projects.remove_workspace(workspace.id).await.unwrap();

    let worktrees = git(&fixture.repo(), &["worktree", "list"]);
    assert!(!worktrees.contains(&workspace.name), "{worktrees}");
}

#[tokio::test]
async fn a_project_is_removed_only_without_workspaces_and_the_repo_stays() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();

    let refused = fixture.projects.remove(project.id).await.unwrap_err();
    assert!(matches!(refused, ProjectError::Conflict(_)), "{refused}");

    fixture.projects.remove_workspace(workspace.id).await.unwrap();
    fixture.projects.remove(project.id).await.unwrap();

    assert!(fixture.projects.list().await.unwrap().is_empty());
    assert!(fixture.repo().join("first.txt").exists());
    let missing = fixture.projects.remove(project.id).await.unwrap_err();
    assert!(matches!(missing, ProjectError::NotFound(_)), "{missing}");
}

#[tokio::test]
async fn renaming_trims_and_an_empty_title_clears_it() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    let workspace = fixture
        .projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap();

    let renamed = fixture
        .projects
        .rename_workspace(workspace.id, Some("  Login fix "))
        .await
        .unwrap();
    assert_eq!(renamed.title.as_deref(), Some("Login fix"));
    assert_eq!(
        fixture.projects.get(project.id).await.unwrap().workspaces[0]
            .title
            .as_deref(),
        Some("Login fix")
    );

    let cleared = fixture
        .projects
        .rename_workspace(workspace.id, Some("   "))
        .await
        .unwrap();
    assert_eq!(cleared.title, None);
    assert_eq!(cleared.label(), workspace.name);
    assert_eq!(
        fixture.projects.get(project.id).await.unwrap().workspaces[0].title,
        None
    );
    assert_eq!(
        fixture.event_kinds().last().map(String::as_str),
        Some(kind::WORKSPACE_RENAMED)
    );
}

#[tokio::test]
async fn names_skip_folders_that_already_exist() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    let folder = fixture
        .dir
        .path()
        .join("worktrees")
        .join(format!("repo-{}", project.id));
    std::fs::create_dir_all(folder.join("oslo")).unwrap();

    let workspace = fixture
        .projects
        .create_workspace_from(project.id, Base::Head, None, "oslo")
        .await
        .unwrap();

    assert_eq!(workspace.name, "oslo-v1");
    assert_eq!(workspace.branch, "repo/oslo-v1");
    assert_eq!(workspace.path, folder.join("oslo-v1"));
}

#[tokio::test]
async fn branches_start_with_the_project_slug() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    git(root, &["init", "-q", "-b", "main", "My App_2"]);
    let repo = root.join("My App_2");
    commit_file(&repo, "a.txt");
    let (project, _) = fixture.projects.add(&repo).await.unwrap();

    let workspace = fixture
        .projects
        .create_workspace_from(project.id, Base::Head, None, "lisbon")
        .await
        .unwrap();

    assert_eq!(workspace.branch, "my-app-2/lisbon");
    assert_eq!(workspace.city, "lisbon");
}

#[tokio::test]
async fn a_branch_named_like_the_slug_is_refused_clearly() {
    let fixture = Fixture::new();
    let project = fixture.add_repo().await;
    git(&fixture.repo(), &["branch", "repo"]);

    let error = fixture
        .projects
        .create_workspace(project.id, Base::Head, None)
        .await
        .unwrap_err();

    assert!(matches!(error, ProjectError::Conflict(_)), "{error}");
    assert!(error.to_string().contains("a branch named repo"), "{error}");
}

#[test]
fn slugs_are_git_safe() {
    assert_eq!(slug("topflexi"), "topflexi");
    assert_eq!(slug("My App"), "my-app");
    assert_eq!(slug("--Ünïcode__ & stuff--"), "n-code-stuff");
    assert_eq!(slug("a..b//c"), "a-b-c");
    assert_eq!(slug("***"), "project");
}

#[tokio::test]
async fn the_staff_worktree_is_made_once_from_head_and_never_listed() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    let local = commit_file(&repo, "local.txt");
    let project = fixture.add_repo().await;
    let (path, branch) = fixture.projects.staff_location(project.id).unwrap();

    let staff = fixture.projects.staff_workspace(project.id).await.unwrap();
    let again = fixture.projects.staff_workspace(project.id).await.unwrap();

    assert!(staff.staff);
    assert_eq!(again.id, staff.id);
    assert_eq!((staff.path.clone(), staff.branch.clone()), (path, branch));
    assert_eq!(staff.branch, "repo/chief");
    assert_eq!(staff.base_commit, local);
    assert!(staff.path.join("local.txt").exists());
    assert!(
        fixture
            .projects
            .get(project.id)
            .await
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert!(fixture.projects.listed_workspace(staff.id).is_err());
    assert!(
        fixture
            .projects
            .rename_workspace(staff.id, Some("x"))
            .await
            .is_err()
    );
    // Not logged as a workspace.
    assert_eq!(fixture.event_kinds(), [kind::PROJECT_ADDED]);

    // A folder deleted by hand comes back on its branch.
    std::fs::remove_dir_all(&staff.path).unwrap();
    let back = fixture.projects.staff_workspace(project.id).await.unwrap();
    assert_eq!(back.id, staff.id);
    assert!(back.path.join("local.txt").exists());
    assert_eq!(
        git(&back.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "repo/chief"
    );

    // Removing the worktree lets the project go.
    fixture.projects.remove_workspace(staff.id).await.unwrap();
    fixture.projects.remove(project.id).await.unwrap();
}

#[test]
fn candidate_names_count_up_from_the_city() {
    let names: Vec<String> = candidate_names("oslo").take(3).collect();

    assert_eq!(names, ["oslo", "oslo-v1", "oslo-v2"]);
    assert_eq!(candidate_names("oslo").count(), 1 + MAX_NAME_VERSION as usize);
}
