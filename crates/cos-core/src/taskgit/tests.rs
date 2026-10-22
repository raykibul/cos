//! Runs real git against a throwaway repo with one workspace worktree on
//! `cos/lisbon`, the way `Projects` sets one up.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use jiff::Timestamp;

use super::*;
use crate::Base;

/// Runs git for test setup and returns its trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
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

fn write(dir: &Path, file: &str, contents: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn commit_all(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

struct Fixture {
    dir: tempfile::TempDir,
    workspace: Workspace,
    tasks: TaskWorktrees,
}

impl Fixture {
    /// `repo` on `main` with `README.md` and `src/lib.rs`, and a workspace
    /// worktree on `cos/lisbon` one commit ahead of it.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q", "-b", "main", "repo"]);
        let repo = root.join("repo");
        // Repo-local, so merges cos makes (which add no identity) work too.
        git(&repo, &["config", "user.name", "cos test"]);
        git(&repo, &["config", "user.email", "test@cos.local"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        write(&repo, "README.md", "readme\n");
        write(&repo, "src/lib.rs", "// lib\n");
        write(&repo, ".gitignore", "target/\n");
        commit_all(&repo, "first");

        let path = root.join("worktrees/repo-1/lisbon");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "cos/lisbon",
                &path.to_string_lossy(),
            ],
        );
        write(&path, "src/workspace.rs", "// from the workspace\n");
        let base_commit = commit_all(&path, "workspace work");

        let workspace = Workspace {
            id: 4,
            project_id: 1,
            name: "lisbon".into(),
            city: "lisbon".into(),
            title: None,
            branch: "cos/lisbon".into(),
            path,
            base: Base::Head,
            base_ref: "main".into(),
            base_commit,
            created_at: Timestamp::now(),
            staff: false,
        };
        let tasks = TaskWorktrees::new(root.join("tasks"));
        Self {
            dir,
            workspace,
            tasks,
        }
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap().join("repo")
    }

    async fn task(&self, id: i64, title: &str) -> TaskWorktree {
        self.tasks.create(&self.workspace, id, title).await.unwrap()
    }
}

fn scope(patterns: &[&str]) -> Scope {
    Scope::new(patterns).unwrap()
}

// ---- Task worktrees ----

#[tokio::test]
async fn a_task_worktree_starts_at_the_workspace_tip_on_its_own_branch() {
    let fixture = Fixture::new();
    let workspace_tip = git(&fixture.workspace.path, &["rev-parse", "HEAD"]);

    let task = fixture.task(7, "Add the login page!").await;

    assert_eq!(task.task_id, 7);
    assert_eq!(task.branch, "cos/lisbon-t7-add-the-login-page");
    assert_eq!(task.base_commit, workspace_tip);
    assert_eq!(task.workspace, fixture.workspace.path);
    assert_eq!(task.workspace_branch, "cos/lisbon");
    assert_eq!(
        task.path,
        fixture
            .dir
            .path()
            .canonicalize()
            .unwrap()
            .join("tasks/lisbon-4/t7-add-the-login-page")
    );
    assert!(task.path.join("src/workspace.rs").exists());
    assert_eq!(
        git(&task.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        task.branch
    );
    assert_eq!(task.commits_ahead().await.unwrap(), 0);
    assert!(
        task.is_merged().await.unwrap(),
        "a branch with no commits is trivially merged"
    );
}

#[tokio::test]
async fn task_names_skip_branches_and_folders_that_exist() {
    let fixture = Fixture::new();
    git(&fixture.repo(), &["branch", "cos/lisbon-t1-fix"]);

    let first = fixture.task(1, "fix").await;
    let second = fixture.task(1, "fix").await;

    assert_eq!(first.branch, "cos/lisbon-t1-fix-2");
    assert_eq!(second.branch, "cos/lisbon-t1-fix-3");
    assert!(second.path.ends_with("t1-fix-3"));
}

#[test]
fn slugs_are_short_lowercase_and_never_empty() {
    assert_eq!(slug("Add OAuth (v2) login"), "add-oauth-v2-login");
    assert_eq!(slug("  --déjà vu--  "), "d-j-vu");
    assert_eq!(slug("!!!"), "task");
    assert_eq!(slug(&"x".repeat(100)).len(), MAX_SLUG_CHARS);
    assert_eq!(merge_message(3, "  Fix\nthe   bug "), "Merge task 3: Fix the bug");
    assert_eq!(merge_message(3, " "), "Merge task 3");
}

#[tokio::test]
async fn removing_refuses_unsaved_work_unless_forced_and_keeps_the_branch() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "draft").await;
    write(&task.path, "draft.txt", "unsaved");

    let refused = task.remove(false).await.unwrap_err();
    assert!(matches!(refused, TaskGitError::Dirty(_)), "{refused}");
    assert!(task.path.join("draft.txt").exists());

    task.remove(true).await.unwrap();

    assert!(!task.path.exists());
    git(
        &fixture.repo(),
        &["show-ref", "--verify", &format!("refs/heads/{}", task.branch)],
    );
    // Removing again, or after the folder vanished, is fine.
    task.remove(false).await.unwrap();
}

#[tokio::test]
async fn a_task_branch_is_deleted_only_once_merged() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "feature").await;
    write(&task.path, "src/feature.rs", "// feature\n");
    commit_all(&task.path, "feature");
    task.remove(false).await.unwrap();

    let refused = task.delete_branch().await.unwrap_err();
    assert!(refused.to_string().contains("not merged"), "{refused}");

    task.merge("Feature").await.unwrap();
    task.delete_branch().await.unwrap();

    let branches = git(&fixture.repo(), &["branch", "--list", "cos/lisbon-*"]);
    assert!(branches.is_empty(), "{branches}");
}

// ---- Scope ----

#[test]
fn globs_match_within_segments_and_across_folders() {
    let s = scope(&[
        "src/*.rs",
        "docs/**/*.md",
        "tests/?.txt",
        "app/[id].tsx",
        "Cargo.toml",
    ]);

    assert!(s.allows("src/lib.rs"));
    assert!(!s.allows("src/deep/lib.rs"), "* stays in one segment");
    assert!(!s.allows("src/lib.rsx"));
    assert!(s.allows("docs/a.md"), "** can be no folders");
    assert!(s.allows("docs/x/y/a.md"));
    assert!(!s.allows("docs/a.txt"));
    assert!(s.allows("tests/a.txt"));
    assert!(!s.allows("tests/ab.txt"));
    assert!(s.allows("app/[id].tsx"), "brackets are literal");
    assert!(!s.allows("app/i.tsx"));
    assert!(s.allows("Cargo.toml"));
    assert!(
        !s.allows("crates/x/Cargo.toml"),
        "patterns are anchored at the root"
    );
}

#[test]
fn a_pattern_naming_a_folder_covers_everything_in_it() {
    for pattern in ["src/auth", "src/auth/", "./src/auth/**", "/src/auth"] {
        let s = scope(&[pattern]);
        assert!(s.allows("src/auth/login.rs"), "{pattern}");
        assert!(s.allows("src/auth/deep/token.rs"), "{pattern}");
        assert!(!s.allows("src/authz.rs"), "{pattern}");
        assert!(!s.allows("src/lib.rs"), "{pattern}");
    }
    assert!(scope(&["**/*.md"]).allows("a/b/README.md"));
    assert!(scope(&["*a*b*"]).allows("xaxxbx"));
    assert!(!scope(&["*a*b*"]).allows("xbxxax"));
    assert!(!scope(&[]).allows("anything"));
}

#[test]
fn bad_patterns_are_refused() {
    for pattern in ["", "  ", "/", "./", "../secrets", "src/../../x"] {
        let error = Scope::new([pattern]).unwrap_err();
        assert!(matches!(error, TaskGitError::Invalid(_)), "{pattern:?}: {error}");
    }
}

#[test]
fn protected_paths_are_found_at_any_depth_and_case() {
    for path in [
        ".claude/settings.json",
        "pkg/.claude/settings.local.json",
        ".Claude/settings.json",
        ".cos/board.md",
        ".codex/config.toml",
        ".env",
        ".env.local",
        "web/.envrc",
        ".mcp.json",
        "sub/.git/config",
    ] {
        assert!(is_protected(path), "{path}");
    }
    for path in [
        "src/env.rs",
        "claude.md",
        "docs/.cosmetic",
        "environment/x",
        ".github/ci.yml",
    ] {
        assert!(!is_protected(path), "{path}");
    }
}

#[tokio::test]
async fn committed_changes_inside_the_scope_pass() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "auth").await;
    write(&task.path, "src/auth/login.rs", "// login\n");
    write(&task.path, "src/lib.rs", "// lib, changed\n");
    write(&task.path, "target/debug/junk", "ignored build output");
    commit_all(&task.path, "auth");

    let report = task
        .check_scope(&scope(&["src/auth/**", "src/lib.rs"]))
        .await
        .unwrap();

    assert!(report.is_ok(), "{report:?}");
    assert_eq!(
        report.changes,
        [
            Change {
                path: "src/auth/login.rs".into(),
                status: ChangeStatus::Added,
                from: None
            },
            Change {
                path: "src/lib.rs".into(),
                status: ChangeStatus::Modified,
                from: None
            },
        ]
    );
}

#[tokio::test]
async fn files_outside_the_scope_and_protected_files_are_violations() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "auth").await;
    write(&task.path, "src/auth.rs", "// auth\n");
    write(&task.path, "README.md", "rewritten\n");
    write(&task.path, ".claude/settings.json", "{}\n");
    write(&task.path, ".env.local", "TOKEN=x\n");
    std::fs::remove_file(task.path.join("src/workspace.rs")).unwrap();
    commit_all(&task.path, "too much");

    // `**` allows every path, but never a protected one.
    let everything = task.check_scope(&scope(&["**"])).await.unwrap();
    let narrow = task.check_scope(&scope(&["src/auth.rs"])).await.unwrap();

    assert!(!everything.is_ok());
    assert!(everything.out_of_scope.is_empty(), "{everything:?}");
    assert_eq!(everything.protected, [".claude/settings.json", ".env.local"]);
    assert_eq!(narrow.out_of_scope, ["README.md", "src/workspace.rs"]);
    assert_eq!(narrow.protected, [".claude/settings.json", ".env.local"]);
    assert!(
        narrow
            .changes
            .iter()
            .any(|c| c.path == "src/workspace.rs" && c.status == ChangeStatus::Deleted)
    );
}

#[tokio::test]
async fn a_rename_needs_both_its_old_and_new_path_in_scope() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "move").await;
    std::fs::create_dir(task.path.join("lib")).unwrap();
    git(&task.path, &["mv", "src/workspace.rs", "lib/workspace.rs"]);
    commit_all(&task.path, "move");

    let only_new = task.check_scope(&scope(&["lib/**"])).await.unwrap();
    let both = task
        .check_scope(&scope(&["lib/**", "src/workspace.rs"]))
        .await
        .unwrap();

    assert_eq!(
        only_new.changes,
        [Change {
            path: "lib/workspace.rs".into(),
            status: ChangeStatus::Renamed,
            from: Some("src/workspace.rs".into()),
        }]
    );
    assert_eq!(only_new.out_of_scope, ["src/workspace.rs"]);
    assert!(both.is_ok(), "{both:?}");
}

#[tokio::test]
async fn uncommitted_work_is_a_violation_but_ignored_files_are_not() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "lazy").await;
    write(&task.path, "src/lib.rs", "// edited, not committed\n");
    write(&task.path, "src/new.rs", "// untracked\n");
    write(&task.path, "target/out", "ignored");

    let report = task.check_scope(&scope(&["src/**"])).await.unwrap();

    assert!(!report.is_ok());
    assert!(report.changes.is_empty());
    assert_eq!(report.uncommitted, ["src/lib.rs", "src/new.rs"]);
}

#[tokio::test]
async fn switching_away_from_the_task_branch_is_a_violation() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "wander").await;
    git(&task.path, &["checkout", "-q", "-b", "elsewhere"]);

    let report = task.check_scope(&scope(&["**"])).await.unwrap();

    assert_eq!(report.wrong_branch.as_deref(), Some("elsewhere"));
    assert!(!report.is_ok());
}

// ---- Test runs ----

#[tokio::test]
async fn a_passing_and_a_failing_test_command() {
    let fixture = Fixture::new();
    let task = fixture.task(1, "tests").await;
    let options = TestOptions::default();

    let passed = task
        .run_tests("test -f src/workspace.rs && echo out && echo err >&2", &options)
        .await
        .unwrap();
    let failed = task.run_tests("echo boom; exit 3", &options).await.unwrap();

    assert_eq!(passed.status, TestStatus::Passed);
    assert_eq!(passed.exit_code, Some(0));
    assert_eq!(passed.output, "out\nerr\n");
    assert!(!passed.truncated);
    assert_eq!(failed.status, TestStatus::Failed);
    assert_eq!(failed.exit_code, Some(3));
    assert_eq!(failed.output, "boom\n");
    assert!(task.run_tests("  ", &options).await.is_err());
}

#[tokio::test]
async fn only_the_tail_of_the_output_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let options = TestOptions {
        max_output_bytes: 10,
        ..TestOptions::default()
    };

    let outcome = run_tests(dir.path(), "seq 1 1000", &options).await.unwrap();

    assert!(outcome.truncated);
    assert_eq!(outcome.output, "\n999\n1000\n");
}

/// Whether `pid` is still running. A zombie (killed, not yet reaped by
/// whoever inherited it) counts as gone.
fn is_running(pid: &str) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&output.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

async fn wait_until_gone(pid: &str) -> bool {
    for _ in 0..50 {
        if !is_running(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn a_timeout_kills_the_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let options = TestOptions {
        timeout: Duration::from_millis(500),
        ..TestOptions::default()
    };

    let started = std::time::Instant::now();
    let outcome = run_tests(
        dir.path(),
        "sleep 60 & echo $! > bg.pid; echo started; sleep 60",
        &options,
    )
    .await
    .unwrap();

    assert_eq!(outcome.status, TestStatus::TimedOut);
    assert_eq!(outcome.exit_code, None);
    assert!(outcome.signal.is_some());
    assert_eq!(outcome.output, "started\n");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let background = std::fs::read_to_string(dir.path().join("bg.pid")).unwrap();
    assert!(
        wait_until_gone(background.trim()).await,
        "pid {background} survived"
    );
}

#[tokio::test]
async fn processes_left_behind_by_a_finished_command_are_killed() {
    let dir = tempfile::tempdir().unwrap();

    let started = std::time::Instant::now();
    // The background sleep keeps the output pipe open; the run must not wait for it.
    let outcome = run_tests(
        dir.path(),
        "sleep 60 & echo $! > bg.pid; echo done",
        &TestOptions::default(),
    )
    .await
    .unwrap();

    assert_eq!(outcome.status, TestStatus::Passed);
    assert_eq!(outcome.output, "done\n");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let background = std::fs::read_to_string(dir.path().join("bg.pid")).unwrap();
    assert!(
        wait_until_gone(background.trim()).await,
        "pid {background} survived"
    );
}

// ---- Merges ----

#[tokio::test]
async fn a_task_merges_with_a_merge_commit_and_a_summary() {
    let fixture = Fixture::new();
    let task = fixture.task(5, "Add login").await;
    write(&task.path, "src/login.rs", "a\nb\nc\n");
    commit_all(&task.path, "login");
    write(&task.path, "src/lib.rs", "// lib\n// more\n");
    commit_all(&task.path, "lib");
    let before = git(&fixture.workspace.path, &["rev-parse", "HEAD"]);

    assert_eq!(task.commits_ahead().await.unwrap(), 2);
    assert!(!task.is_merged().await.unwrap());
    let summary = task.diff_summary().await.unwrap();
    assert_eq!(
        (
            summary.files_changed,
            summary.insertions,
            summary.deletions,
            summary.binary_files
        ),
        (2, 4, 0, 0)
    );
    assert!(summary.stat.contains("src/login.rs"), "{}", summary.stat);
    assert!(summary.stat.contains("2 files changed"), "{}", summary.stat);

    let outcome = task.merge("Add login").await.unwrap();

    let MergeOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}")
    };
    let ws = &fixture.workspace.path;
    assert_eq!(git(ws, &["rev-parse", "HEAD"]), commit);
    assert_eq!(git(ws, &["log", "-1", "--format=%s"]), "Merge task 5: Add login");
    assert_eq!(
        git(ws, &["rev-parse", "HEAD^1"]),
        before,
        "the first parent is the workspace"
    );
    assert!(ws.join("src/login.rs").exists());
    assert!(task.is_merged().await.unwrap());
    assert_eq!(
        task.merge("Add login").await.unwrap(),
        MergeOutcome::AlreadyMerged
    );
}

#[tokio::test]
async fn a_conflict_aborts_the_merge_and_leaves_the_workspace_clean() {
    let fixture = Fixture::new();
    let task = fixture.task(2, "readme").await;
    write(&task.path, "README.md", "from the task\n");
    write(&task.path, "src/fine.rs", "// no conflict here\n");
    commit_all(&task.path, "task readme");
    let ws = &fixture.workspace.path;
    write(ws, "README.md", "from the workspace\n");
    let before = commit_all(ws, "workspace readme");

    let outcome = task.merge("Readme").await.unwrap();

    assert_eq!(
        outcome,
        MergeOutcome::Conflict {
            paths: vec!["README.md".into()]
        }
    );
    assert_eq!(git(ws, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(ws, &["status", "--porcelain"]), "");
    assert!(!ws.join("src/fine.rs").exists());
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "from the workspace\n"
    );
    assert!(!task.is_merged().await.unwrap());
}

#[tokio::test]
async fn merging_into_a_dirty_or_switched_workspace_is_refused() {
    let fixture = Fixture::new();
    let task = fixture.task(3, "x").await;
    write(&task.path, "src/x.rs", "x\n");
    commit_all(&task.path, "x");
    let ws = &fixture.workspace.path;

    write(ws, "scratch.txt", "untracked");
    let dirty = task.merge("x").await.unwrap_err();
    assert!(matches!(dirty, TaskGitError::Dirty(_)), "{dirty}");
    std::fs::remove_file(ws.join("scratch.txt")).unwrap();

    git(ws, &["checkout", "-q", "--detach"]);
    let switched = task.merge("x").await.unwrap_err();
    assert!(switched.to_string().contains("not cos/lisbon"), "{switched}");

    git(ws, &["checkout", "-q", "cos/lisbon"]);
    assert!(matches!(
        task.merge("x").await.unwrap(),
        MergeOutcome::Merged { .. }
    ));
}

// ---- Worker limits ----

#[tokio::test]
async fn workers_are_capped_per_workspace_and_in_total() {
    let limiter = WorkerLimiter::new(2, 3);

    let a1 = limiter.try_acquire(1).unwrap();
    let _a2 = limiter.try_acquire(1).unwrap();
    assert!(limiter.try_acquire(1).is_none(), "workspace 1 is full");
    assert_eq!(limiter.available(1), 0);
    assert_eq!(limiter.available(2), 1);

    let _b1 = limiter.try_acquire(2).unwrap();
    assert!(limiter.try_acquire(2).is_none(), "the total is full");

    drop(a1);
    assert!(limiter.try_acquire(2).is_some());
}

#[tokio::test]
async fn a_waiting_worker_starts_when_a_slot_frees() {
    let limiter = std::sync::Arc::new(WorkerLimiter::default());
    let held: Vec<_> = (0..DEFAULT_WORKERS_PER_WORKSPACE)
        .map(|_| limiter.try_acquire(9).unwrap())
        .collect();

    let waiting = tokio::spawn({
        let limiter = limiter.clone();
        async move { limiter.acquire(9).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiting.is_finished());

    drop(held);
    let permit = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    assert_eq!(limiter.available(9), DEFAULT_WORKERS_PER_WORKSPACE);
    assert_eq!(WorkerLimiter::new(0, 0).available(1), 1);
}
