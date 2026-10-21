//! The pipeline end to end, with `cos-fake-claude` playing scripted workers
//! and reviewers (see `crates/cos-harness/src/bin/cos-fake-claude/script.rs`)
//! and real git in a throwaway repo.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    AgentState, Agents, Base, EventLog, NewRun, PlanProposal, Projects, ProposedTask, ReviewVerdict, Role,
    RunState, Runs, Store, Task, TaskState, TestsStatus, Workspace,
};
use serde_json::{Value, json};

use super::{Pipeline, PipelineConfig};
use crate::daemon::agents::{AgentConfig, AgentManager};

const WAIT: Duration = Duration::from_secs(40);

/// `cos-fake-claude`, built next to this test binary by `cargo test --workspace`.
fn fake_claude() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("cos-fake-claude"))
        .unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    log: EventLog,
    projects: Arc<Projects>,
    runs: Arc<Runs>,
    manager: AgentManager,
    pipeline: Pipeline,
    workspace: Workspace,
}

impl Fixture {
    async fn start() -> Self {
        assert!(
            fake_claude().is_file(),
            "{} is missing; run the tests with `cargo test --workspace` (make test)",
            fake_claude().display()
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        // Repo-local, so the fake's commits and cos's merges have a name.
        git(&repo, &["config", "user.name", "cos test"]);
        git(&repo, &["config", "user.email", "test@cos.local"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README"), "hi\n").unwrap();
        git(&repo, &["add", "README"]);
        git(&repo, &["commit", "-q", "-m", "first"]);
        std::fs::create_dir_all(root.join("fake")).unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        let projects = Arc::new(Projects::new(store.clone(), log.clone(), root.join("w")));
        let (project, _) = projects.add(&repo).await.unwrap();
        let workspace = projects
            .create_workspace(project.id, Base::Head, None)
            .await
            .unwrap();
        let runs = Arc::new(Runs::new(store.clone(), log.clone()));
        let manager = start_manager(&store, &log, &projects, &runs, &root);
        let pipeline = start_pipeline(&runs, &manager, &projects, &log, &root);
        Self {
            dir,
            store,
            log,
            projects,
            runs,
            manager,
            pipeline,
            workspace,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    /// The fake's script: `{name: {worker: [turn…], reviewer: [turn…]}}`.
    fn script(&self, script: Value) {
        std::fs::write(self.root().join("fake/script.json"), script.to_string()).unwrap();
    }

    /// Every prompt the fake got, as `(name, role, prompt)`.
    fn prompts(&self) -> Vec<(String, String, String)> {
        std::fs::read_to_string(self.root().join("fake/prompts.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
                (text("name"), text("role"), text("prompt"))
            })
            .collect()
    }

    /// Starts a run with this plan and approves it. Returns the tasks.
    fn run(&self, tasks: Vec<ProposedTask>) -> (i64, Vec<Task>) {
        let run = self
            .runs
            .create(NewRun {
                workspace_id: self.workspace.id,
                goal: "test the pipeline".into(),
                preset: Default::default(),
                approval_cap: None,
            })
            .unwrap();
        self.runs.propose_plan(run.id, &PlanProposal { tasks }).unwrap();
        let board = self.runs.approve_plan(run.id).unwrap();
        (run.id, board.tasks)
    }

    async fn until_task(&self, id: i64, state: TaskState) -> Task {
        until(&format!("task {id} to be {state}"), || async {
            self.runs.task(id).ok().filter(|task| task.state == state)
        })
        .await
    }

    async fn until_run(&self, id: i64, state: RunState) {
        until(&format!("run {id} to be {state}"), || async {
            self.runs.run(id).ok().filter(|run| run.state == state)
        })
        .await;
    }

    /// Waits until every agent of the task has exited.
    async fn until_agents_exited(&self, task_id: i64) {
        until(&format!("task {task_id}'s agents to exit"), || async {
            let agents = self.manager.of_task(task_id).ok()?;
            agents.iter().all(|agent| !agent.state.is_running()).then_some(())
        })
        .await;
    }

    fn kinds(&self) -> Vec<String> {
        self.log
            .after(0, 10_000)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }

    /// What a daemon restart does: stop the pipeline, shut the agents down,
    /// then start both again on the same database.
    async fn restart(&mut self) {
        self.pipeline.shutdown();
        self.manager.shutdown().await;
        let root = self.root();
        self.manager = start_manager(&self.store, &self.log, &self.projects, &self.runs, &root);
        self.pipeline = start_pipeline(&self.runs, &self.manager, &self.projects, &self.log, &root);
    }
}

fn start_manager(
    store: &Arc<Store>,
    log: &EventLog,
    projects: &Arc<Projects>,
    runs: &Arc<Runs>,
    root: &Path,
) -> AgentManager {
    let mut config = AgentConfig::for_tests(root);
    config.env.push((
        "COS_FAKE_CLAUDE_SCRIPT".into(),
        root.join("fake/script.json").display().to_string(),
    ));
    AgentManager::start(
        Agents::new(store.clone(), log.clone()),
        projects.clone(),
        runs.clone(),
        config,
    )
    .unwrap()
}

fn start_pipeline(
    runs: &Arc<Runs>,
    manager: &AgentManager,
    projects: &Arc<Projects>,
    log: &EventLog,
    root: &Path,
) -> Pipeline {
    let config = PipelineConfig {
        tasks_dir: root.join("tasks"),
        model: None,
        tests: cos_core::TestOptions::default(),
        workers_per_workspace: 3,
        workers_total: 6,
        max_diff_bytes: 24 * 1024,
        max_test_tail_chars: 3_000,
        ready_timeout: Duration::from_secs(20),
        exit_timeout: Duration::from_secs(5),
        liveness_poll: Duration::from_millis(500),
        stop_settle: Duration::from_millis(100),
    };
    Pipeline::start(
        runs.clone(),
        manager.clone(),
        projects.clone(),
        log.clone(),
        config,
    )
    .unwrap()
}

async fn until<T, F: Future<Output = Option<T>>>(what: &str, mut check: impl FnMut() -> F) -> T {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn task(title: &str, name: &str, files: &[&str], depends_on: &[&str], test: Option<&str>) -> ProposedTask {
    ProposedTask {
        title: title.into(),
        instructions: format!("Do {title}. [fake:{name}]"),
        files: files.iter().map(|file| (*file).to_owned()).collect(),
        test_command: test.map(str::to_owned),
        depends_on: depends_on.iter().map(|dep| (*dep).to_owned()).collect(),
    }
}

/// A worker turn that writes `file` and commits it.
fn commit(file: &str, say: &str) -> Value {
    json!({ "sh": [format!("echo {file} >> {file}"), format!("git add {file}"), format!("git commit -qm 'add {file}'")], "say": say })
}

fn say(text: &str) -> Value {
    json!({ "say": text })
}

#[tokio::test]
async fn two_dependent_tasks_merge_in_order() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": { "worker": [commit("a.txt", "Added a.\nDONE")], "reviewer": [say("Fine.\nVERDICT: APPROVE")] },
        "b": { "worker": [commit("b.txt", "DONE")], "reviewer": [say("VERDICT: APPROVE")] },
    }));
    let (run, tasks) = fixture.run(vec![
        task("first", "a", &["a.txt"], &[], Some("test -f a.txt")),
        task("second", "b", &["b.txt"], &["first"], None),
    ]);
    let (first, second) = (tasks[0].id, tasks[1].id);

    fixture.until_run(run, RunState::Reporting).await;

    let first = fixture.runs.task(first).unwrap();
    let second = fixture.runs.task(second).unwrap();
    assert_eq!(
        (first.state, second.state),
        (TaskState::Merged, TaskState::Merged)
    );
    let checks = first.checks.as_ref().expect("checks recorded");
    assert!(checks.scope.ok);
    assert_eq!(
        checks.tests.as_ref().map(|tests| tests.status),
        Some(TestsStatus::Passed)
    );
    assert_eq!(checks.commits_ahead, 1);
    assert_eq!(
        second
            .checks
            .as_ref()
            .and_then(|checks| checks.tests.as_ref())
            .map(|tests| tests.status),
        Some(TestsStatus::Skipped)
    );
    assert_eq!(
        first.review.as_ref().map(|review| review.verdict),
        Some(ReviewVerdict::Approve)
    );

    // Merged into the workspace in dependency order, then cleaned up.
    let workspace = &fixture.workspace.path;
    assert!(workspace.join("a.txt").is_file() && workspace.join("b.txt").is_file());
    let merges = git(workspace, &["log", "--merges", "--format=%s"]);
    assert_eq!(
        merges.lines().collect::<Vec<_>>(),
        [
            format!("Merge task {}: second", second.id),
            format!("Merge task {}: first", first.id)
        ]
    );
    for task in [&first, &second] {
        fixture.until_agents_exited(task.id).await;
        until(
            &format!("task {}'s worktree and branch to go", task.id),
            || async {
                let branches = git(workspace, &["branch", "--list", task.branch.as_deref().unwrap()]);
                (!task.worktree.as_ref().unwrap().exists() && branches.is_empty()).then_some(())
            },
        )
        .await;
    }

    // The agents: a worker and a reviewer per task, set up as a run's agents.
    let agents = fixture.manager.of_task(first.id).unwrap();
    let roles: Vec<(Role, &str)> = agents
        .iter()
        .map(|agent| (agent.role, agent.permission_mode.as_str()))
        .collect();
    assert_eq!(roles, [(Role::Worker, "acceptEdits"), (Role::Reviewer, "plan")]);
    assert_eq!(first.agent_id, Some(agents[0].id));
    for agent in &agents {
        assert_eq!((agent.run_id, agent.user_settings), (Some(run), false));
        assert_eq!(agent.cwd.as_ref(), first.worktree.as_ref());
        assert!(agent.system_prompt.is_some());
        // Exited through one `/exit`, not killed: two typed at once would
        // arrive as one prompt, `/exit/exit`.
        assert_eq!(
            (agent.state, agent.exit_code),
            (AgentState::Exited, Some(0)),
            "{agent:?}"
        );
    }
    let worker_prompt = &fixture.prompts()[0].2;
    assert!(
        worker_prompt.contains("Task: first") && worker_prompt.contains("- a.txt"),
        "{worker_prompt}"
    );

    let kinds = fixture.kinds();
    for kind in [kind::TASK_CHECKED, kind::TASK_REVIEWED, kind::TASK_MERGED] {
        assert_eq!(kinds.iter().filter(|k| *k == kind).count(), 2, "{kind}");
    }
}

#[tokio::test]
async fn a_scope_violation_gets_one_retry_then_blocks() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": { "worker": [
            { "sh": ["echo a > a.txt", "echo stray > stray.txt", "git add -A", "git commit -qm work"], "say": "DONE" },
            say("Still DONE"),
            say("DONE"),
        ] },
    }));
    let (_, tasks) = fixture.run(vec![task("scoped", "a", &["a.txt"], &[], None)]);

    let task = fixture.until_task(tasks[0].id, TaskState::Blocked).await;

    assert_eq!(task.retries, 1);
    let reason = task.last_failure.as_deref().unwrap();
    assert!(reason.contains("stray.txt"), "{reason}");
    let checks = task.checks.unwrap();
    assert!(!checks.scope.ok);
    assert_eq!(checks.scope.out_of_scope, ["stray.txt"]);
    // The retry told the worker what was wrong, in the same session.
    let prompts = fixture.prompts();
    assert!(prompts[1].2.contains("stray.txt"), "{:?}", prompts[1]);
    assert_eq!(fixture.manager.of_task(task.id).unwrap().len(), 1);
    // Blocked: the worker stops, the worktree stays for a retry.
    fixture.until_agents_exited(task.id).await;
    assert!(task.worktree.unwrap().is_dir());
}

#[tokio::test]
async fn failing_tests_get_a_retry_with_their_output() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": {
            "worker": [commit("a.txt", "DONE"), commit("fixed.txt", "DONE")],
            "reviewer": [say("VERDICT: APPROVE")],
        },
    }));
    let (_, tasks) = fixture.run(vec![task(
        "tested",
        "a",
        &["a.txt", "fixed.txt"],
        &[],
        Some("echo boom-output; test -f fixed.txt"),
    )]);

    let task = fixture.until_task(tasks[0].id, TaskState::Merged).await;

    assert_eq!(task.retries, 1);
    assert!(task.last_failure.as_deref().unwrap().contains("boom-output"));
    let retry = &fixture.prompts()[1];
    assert_eq!(retry.1, "worker");
    assert!(
        retry.2.contains("boom-output") && retry.2.contains("exit code 1"),
        "{retry:?}"
    );
}

#[tokio::test]
async fn reviewer_changes_go_back_to_the_worker_then_merge() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": {
            "worker": [commit("a.txt", "DONE"), commit("a.txt", "Renamed.\nDONE")],
            "reviewer": [
                say("Close.\nVERDICT: CHANGES: rename the helper"),
                say("VERDICT: APPROVE"),
            ],
        },
    }));
    let (_, tasks) = fixture.run(vec![task("reviewed", "a", &["a.txt"], &[], None)]);

    let task = fixture.until_task(tasks[0].id, TaskState::Merged).await;

    assert_eq!(task.retries, 1);
    let prompts = fixture.prompts();
    let roles: Vec<&str> = prompts.iter().map(|(_, role, _)| role.as_str()).collect();
    assert_eq!(roles, ["worker", "reviewer", "worker", "reviewer"]);
    assert!(prompts[2].2.contains("rename the helper"), "{:?}", prompts[2]);
    // The reviewer saw the diff.
    assert!(prompts[1].2.contains("+a.txt"), "{:?}", prompts[1]);
    let agents = fixture.manager.of_task(task.id).unwrap();
    let roles: Vec<Role> = agents.iter().map(|agent| agent.role).collect();
    assert_eq!(roles, [Role::Worker, Role::Reviewer, Role::Reviewer]);
    assert_eq!(task.review.unwrap().verdict, ReviewVerdict::Approve);
    let kinds = fixture.kinds();
    assert_eq!(kinds.iter().filter(|k| *k == kind::TASK_REVIEWED).count(), 2);
}

#[tokio::test]
async fn a_merge_conflict_blocks_with_the_paths() {
    let fixture = Fixture::start().await;
    let workspace = fixture.workspace.path.display().to_string();
    fixture.script(json!({
        "a": {
            "worker": [{
                "sh": [
                    "echo task > a.txt", "git add a.txt", "git commit -qm task",
                    // Meanwhile someone commits a different a.txt to the workspace.
                    format!("cd '{workspace}' && echo other > a.txt && git add a.txt && git commit -qm other"),
                ],
                "say": "DONE",
            }],
            "reviewer": [say("VERDICT: APPROVE")],
        },
    }));
    let (_, tasks) = fixture.run(vec![task("conflicting", "a", &["a.txt"], &[], None)]);

    let task = fixture.until_task(tasks[0].id, TaskState::Blocked).await;

    let reason = task.last_failure.as_deref().unwrap();
    assert!(reason.contains("conflicts in: a.txt"), "{reason}");
    assert_eq!(git(&fixture.workspace.path, &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn stuck_and_silent_workers_block() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": { "worker": [say("I can't.\nSTUCK: the API key is missing")] },
        "b": { "worker": [say("Hmm."), say("Still thinking.")] },
    }));
    let (_, tasks) = fixture.run(vec![
        task("stuck", "a", &["a.txt"], &[], None),
        task("silent", "b", &["b.txt"], &[], None),
    ]);

    let stuck = fixture.until_task(tasks[0].id, TaskState::Blocked).await;
    let silent = fixture.until_task(tasks[1].id, TaskState::Blocked).await;

    assert_eq!(
        stuck.last_failure.as_deref(),
        Some("the worker is stuck: the API key is missing")
    );
    assert!(silent.last_failure.unwrap().contains("no verdict line"));
    // The silent one was asked once for its verdict.
    let asked = fixture
        .prompts()
        .into_iter()
        .filter(|(name, _, prompt)| name == "b" && prompt.contains("end your reply"))
        .count();
    assert_eq!(asked, 1);
    assert_eq!(stuck.retries + silent.retries, 0);
}

#[tokio::test]
async fn cancelling_a_run_stops_its_agents_and_keeps_only_work() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": { "worker": [{ "hang": true }] },
        "b": { "worker": [{ "sh": ["echo b > b.txt", "git add b.txt", "git commit -qm b"], "hang": true }] },
    }));
    let (run, tasks) = fixture.run(vec![
        task("idle", "a", &["a.txt"], &[], None),
        task("busy", "b", &["b.txt"], &[], None),
    ]);
    let idle = fixture.until_task(tasks[0].id, TaskState::Running).await;
    let busy = fixture.until_task(tasks[1].id, TaskState::Running).await;
    let busy_branch = busy.branch.clone().unwrap();
    until("the busy worker to commit", || async {
        (git(
            &fixture.workspace.path,
            &["rev-list", "--count", &format!("HEAD..{busy_branch}")],
        ) == "1")
            .then_some(())
    })
    .await;

    fixture.runs.cancel(run, Some("changed my mind")).unwrap();

    for task in [&idle, &busy] {
        fixture.until_agents_exited(task.id).await;
    }
    until("the idle task's worktree to go", || async {
        (!idle.worktree.as_ref().unwrap().exists()).then_some(())
    })
    .await;
    let workspace = &fixture.workspace.path;
    assert!(git(workspace, &["branch", "--list", idle.branch.as_deref().unwrap()]).is_empty());
    // Work is never thrown away.
    assert!(busy.worktree.as_ref().unwrap().is_dir());
    assert!(!git(workspace, &["branch", "--list", &busy_branch]).is_empty());
}

#[tokio::test]
async fn a_restart_mid_run_resumes_the_worker() {
    let mut fixture = Fixture::start().await;
    fixture.script(json!({
        "a": {
            "worker": [{ "hang": true }, commit("a.txt", "Picked up where I left off.\nDONE")],
            "reviewer": [say("VERDICT: APPROVE")],
        },
    }));
    let (run, tasks) = fixture.run(vec![task("long", "a", &["a.txt"], &[], None)]);
    fixture.until_task(tasks[0].id, TaskState::Running).await;
    let worker = until("the worker to be working", || async {
        let agents = fixture.manager.of_task(tasks[0].id).ok()?;
        agents
            .into_iter()
            .find(|agent| agent.state == AgentState::Working)
    })
    .await;

    fixture.restart().await;

    let task = fixture.until_task(tasks[0].id, TaskState::Merged).await;
    fixture.until_run(run, RunState::Reporting).await;
    assert_eq!(task.retries, 0);
    // The same worker, resumed, and told to carry on.
    assert_eq!(task.agent_id, Some(worker.id));
    let prompts = fixture.prompts();
    assert!(prompts[1].2.contains("cos restarted"), "{:?}", prompts[1]);
}

#[tokio::test]
async fn a_task_retried_after_blocking_resumes_its_worker() {
    let fixture = Fixture::start().await;
    fixture.script(json!({
        "a": {
            "worker": [say("STUCK: which file?"), commit("a.txt", "DONE")],
            "reviewer": [say("VERDICT: APPROVE")],
        },
    }));
    let (_, tasks) = fixture.run(vec![task("unclear", "a", &["a.txt"], &[], None)]);
    fixture.until_task(tasks[0].id, TaskState::Blocked).await;
    fixture.until_agents_exited(tasks[0].id).await;

    // What the chief's `task_update retry` does.
    fixture
        .runs
        .update_task(tasks[0].id, TaskState::Ready, Some("use a.txt"))
        .unwrap();

    let task = fixture.until_task(tasks[0].id, TaskState::Merged).await;
    assert_eq!(fixture.manager.of_task(task.id).unwrap()[0].role, Role::Worker);
    let prompts = fixture.prompts();
    assert!(prompts[1].2.contains("use a.txt"), "{:?}", prompts[1]);
}
