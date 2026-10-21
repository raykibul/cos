//! Runs against an in-memory database with a workspace row inserted by hand,
//! since nothing here touches git.

use std::path::Path;

use rusqlite::Connection;

use super::*;
use crate::store::MIGRATIONS;

struct Fixture {
    runs: Runs,
    log: EventLog,
    store: Arc<Store>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_store(Store::open_in_memory().unwrap())
    }

    fn with_store(store: Store) -> Self {
        let store = Arc::new(store);
        let log = EventLog::new(store.clone());
        Self {
            runs: Runs::new(store.clone(), log.clone()),
            log,
            store,
        }
    }

    /// Adds a project with one workspace and returns the workspace id.
    fn workspace(&self, name: &str) -> i64 {
        add_workspace(&self.store.lock(), name)
    }

    fn create(&self, workspace_id: i64) -> Run {
        self.runs
            .create(new_run(workspace_id, "add rate limiting"))
            .unwrap()
    }

    /// A run with the plan approved: `api`, `ui` after `api`, and `docs`.
    fn running(&self) -> (Run, Vec<Task>) {
        let run = self.create(self.workspace("lisbon"));
        self.runs
            .propose_plan(
                run.id,
                &plan(&[
                    ("api", &["src/api.rs"], &[]),
                    ("ui", &["src/ui/**"], &["api"]),
                    ("docs", &["README.md"], &[]),
                ]),
            )
            .unwrap();
        let board = self.runs.approve_plan(run.id).unwrap();
        (board.run, board.tasks)
    }

    fn kinds(&self) -> Vec<String> {
        self.log
            .after(0, 1000)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }

    fn states(&self, run_id: i64) -> Vec<(String, TaskState)> {
        self.runs
            .board(run_id)
            .unwrap()
            .tasks
            .into_iter()
            .map(|task| (task.title, task.state))
            .collect()
    }

    /// Walks a task from ready to merged.
    fn merge(&self, task_id: i64) -> Board {
        for state in [TaskState::Running, TaskState::Checking, TaskState::Review] {
            self.runs.update_task(task_id, state, None).unwrap();
        }
        self.runs.update_task(task_id, TaskState::Merged, None).unwrap()
    }
}

fn add_workspace(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO projects (root, name, base_branch, created_ms) VALUES (?1, 'repo', 'main', 0)",
        [format!("/repos/{name}")],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
         VALUES (?1, ?2, ?2, 'cos/' || ?2, '/wt/' || ?2, 'head', 'main', 'abc', 0)",
        rusqlite::params![project, name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn new_run(workspace_id: i64, goal: &str) -> NewRun {
    NewRun {
        workspace_id,
        goal: goal.into(),
        preset: ApprovalPreset::default(),
        approval_cap: None,
    }
}

fn plan(tasks: &[(&str, &[&str], &[&str])]) -> PlanProposal {
    PlanProposal {
        tasks: tasks
            .iter()
            .map(|(title, files, deps)| ProposedTask {
                title: (*title).into(),
                instructions: format!("do {title}"),
                files: files.iter().map(|f| (*f).to_owned()).collect(),
                test_command: Some("cargo test".into()),
                depends_on: deps.iter().map(|d| (*d).to_owned()).collect(),
            })
            .collect(),
    }
}

fn id_of(tasks: &[Task], title: &str) -> i64 {
    tasks.iter().find(|task| task.title == title).unwrap().id
}

#[test]
fn a_new_run_starts_planning_with_default_limits() {
    let fixture = Fixture::new();
    let workspace = fixture.workspace("lisbon");

    let run = fixture
        .runs
        .create(new_run(workspace, "  add rate limiting \n"))
        .unwrap();

    assert_eq!(run.goal, "add rate limiting");
    assert_eq!(run.state, RunState::Planning);
    assert_eq!((run.preset, run.approval_cap), (ApprovalPreset::Normal, 20));
    assert_eq!(fixture.runs.run(run.id).unwrap(), run);
    let event = fixture.log.after(0, 1).unwrap().remove(0);
    assert_eq!(event.kind, kind::RUN_CREATED);
    assert_eq!(event.run_id, Some(run.id.to_string()));
    assert_eq!(event.data["goal"], "add rate limiting");
}

#[test]
fn bad_runs_are_refused() {
    let fixture = Fixture::new();
    let workspace = fixture.workspace("lisbon");

    let empty = fixture.runs.create(new_run(workspace, "  ")).unwrap_err();
    let nowhere = fixture.runs.create(new_run(999, "x")).unwrap_err();
    let mut greedy = new_run(workspace, "x");
    greedy.approval_cap = Some(5000);
    let greedy = fixture.runs.create(greedy).unwrap_err();

    assert!(matches!(empty, RunError::Invalid(_)), "{empty}");
    assert!(matches!(nowhere, RunError::NotFound(_)), "{nowhere}");
    assert!(matches!(greedy, RunError::Invalid(_)), "{greedy}");
    assert!(fixture.kinds().is_empty());
}

#[test]
fn one_unfinished_run_per_workspace() {
    let fixture = Fixture::new();
    let lisbon = fixture.workspace("lisbon");
    let first = fixture.create(lisbon);

    let second = fixture.runs.create(new_run(lisbon, "more")).unwrap_err();
    fixture.create(fixture.workspace("porto"));

    assert!(matches!(second, RunError::Conflict(_)), "{second}");
    assert!(
        second.to_string().contains(&format!("run {}", first.id)),
        "{second}"
    );
    fixture.runs.cancel(first.id, None).unwrap();
    fixture.create(lisbon);
}

#[test]
fn proposing_a_plan_puts_it_on_the_board() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));

    let board = fixture
        .runs
        .propose_plan(
            run.id,
            &plan(&[("ui", &["ui"], &["api"]), ("api", &["api"], &[])]),
        )
        .unwrap();

    assert_eq!(board.run.state, RunState::AwaitingPlanApproval);
    assert_eq!(board.counts.proposed, 2);
    let (ui, api) = (&board.tasks[0], &board.tasks[1]);
    assert_eq!((ui.position, api.position), (0, 1));
    assert_eq!(ui.depends_on, [api.id]);
    assert_eq!(ui.test_command.as_deref(), Some("cargo test"));
    assert_eq!(
        fixture.kinds(),
        [kind::RUN_CREATED, kind::PLAN_PROPOSED, kind::RUN_STATE_CHANGED]
    );
}

#[test]
fn proposing_again_before_approval_replaces_the_plan() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));
    fixture
        .runs
        .propose_plan(run.id, &plan(&[("a", &["a"], &[]), ("b", &["b"], &[])]))
        .unwrap();

    let board = fixture
        .runs
        .propose_plan(run.id, &plan(&[("c", &["c"], &[])]))
        .unwrap();

    assert_eq!(fixture.states(run.id), [("c".to_owned(), TaskState::Proposed)]);
    assert_eq!(board.run.state, RunState::AwaitingPlanApproval);
    let last = fixture.log.after(0, 100).unwrap().pop().unwrap();
    assert_eq!(last.kind, kind::PLAN_PROPOSED);
    assert_eq!(
        (last.data["tasks"].as_u64(), last.data["replaced"].as_u64()),
        (Some(1), Some(2))
    );
}

#[test]
fn an_invalid_plan_changes_nothing() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));

    let error = fixture
        .runs
        .propose_plan(run.id, &plan(&[("a", &["a"], &["b"]), ("b", &["b"], &["a"])]))
        .unwrap_err();

    assert!(matches!(error, RunError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("circle"), "{error}");
    let board = fixture.runs.board(run.id).unwrap();
    assert_eq!((board.run.state, board.tasks.len()), (RunState::Planning, 0));
}

#[test]
fn approving_starts_tasks_without_dependencies() {
    let fixture = Fixture::new();

    let (run, _) = fixture.running();

    assert_eq!(run.state, RunState::Running);
    assert_eq!(
        fixture.states(run.id),
        [
            ("api".to_owned(), TaskState::Ready),
            ("ui".to_owned(), TaskState::Proposed),
            ("docs".to_owned(), TaskState::Ready),
        ]
    );
    let kinds = fixture.kinds();
    assert_eq!(
        kinds[3..],
        [
            kind::PLAN_APPROVED,
            kind::RUN_STATE_CHANGED,
            kind::TASK_STATE_CHANGED,
            kind::TASK_STATE_CHANGED,
        ]
    );
}

#[test]
fn plans_are_approved_once_and_only_when_proposed() {
    let fixture = Fixture::new();
    let planning = fixture.create(fixture.workspace("porto"));
    let (running, _) = fixture.running();

    let too_early = fixture.runs.approve_plan(planning.id).unwrap_err();
    let again = fixture.runs.approve_plan(running.id).unwrap_err();
    let replan = fixture
        .runs
        .propose_plan(running.id, &plan(&[("x", &["x"], &[])]))
        .unwrap_err();

    for error in [too_early, again, replan] {
        assert!(matches!(error, RunError::Conflict(_)), "{error}");
    }
}

#[test]
fn a_task_is_ready_only_once_its_dependencies_are_merged() {
    let fixture = Fixture::new();
    let (run, tasks) = fixture.running();
    let (api, ui) = (id_of(&tasks, "api"), id_of(&tasks, "ui"));

    let early = fixture.runs.update_task(ui, TaskState::Ready, None).unwrap_err();
    assert!(matches!(early, RunError::Conflict(_)), "{early}");
    assert!(early.to_string().contains("waits for task"), "{early}");

    fixture.runs.update_task(api, TaskState::Running, None).unwrap();
    fixture.runs.update_task(api, TaskState::Checking, None).unwrap();
    fixture.runs.update_task(api, TaskState::Review, None).unwrap();
    assert_eq!(fixture.states(run.id)[1].1, TaskState::Proposed);

    let board = fixture.runs.update_task(api, TaskState::Merged, None).unwrap();
    assert_eq!(board.tasks[1].state, TaskState::Ready);
}

#[test]
fn a_task_waits_for_every_dependency() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));
    fixture
        .runs
        .propose_plan(
            run.id,
            &plan(&[("a", &["a"], &[]), ("b", &["b"], &[]), ("c", &["c"], &["a", "b"])]),
        )
        .unwrap();
    let tasks = fixture.runs.approve_plan(run.id).unwrap().tasks;

    let after_a = fixture.merge(id_of(&tasks, "a"));
    assert_eq!(after_a.tasks[2].state, TaskState::Proposed);
    let after_b = fixture.merge(id_of(&tasks, "b"));
    assert_eq!(after_b.tasks[2].state, TaskState::Ready);
}

#[test]
fn illegal_task_moves_are_refused() {
    let fixture = Fixture::new();
    let (run, tasks) = fixture.running();
    let api = id_of(&tasks, "api");

    let skip = fixture
        .runs
        .update_task(api, TaskState::Merged, None)
        .unwrap_err();
    let unknown = fixture
        .runs
        .update_task(999, TaskState::Running, None)
        .unwrap_err();

    assert!(matches!(skip, RunError::Conflict(_)), "{skip}");
    assert!(
        skip.to_string().contains("can't go from ready to merged"),
        "{skip}"
    );
    assert!(matches!(unknown, RunError::NotFound(_)), "{unknown}");
    assert_eq!(fixture.states(run.id)[0].1, TaskState::Ready);
}

#[test]
fn tasks_only_move_while_the_run_is_running() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));
    let tasks = fixture
        .runs
        .propose_plan(run.id, &plan(&[("a", &["a"], &[])]))
        .unwrap()
        .tasks;

    let error = fixture
        .runs
        .update_task(tasks[0].id, TaskState::Ready, None)
        .unwrap_err();

    assert!(matches!(error, RunError::Conflict(_)), "{error}");
    assert!(error.to_string().contains("awaiting_plan_approval"), "{error}");
}

#[test]
fn one_retry_then_fail_or_block() {
    let fixture = Fixture::new();
    let (_, tasks) = fixture.running();
    let api = id_of(&tasks, "api");
    fixture.runs.update_task(api, TaskState::Running, None).unwrap();
    fixture.runs.update_task(api, TaskState::Checking, None).unwrap();

    let silent = fixture
        .runs
        .update_task(api, TaskState::Running, None)
        .unwrap_err();
    assert!(matches!(silent, RunError::Invalid(_)), "{silent}");

    fixture
        .runs
        .update_task(api, TaskState::Running, Some("tests failed: 2 errors"))
        .unwrap();
    let task = fixture.runs.task(api).unwrap();
    assert_eq!(task.retries, 1);
    assert_eq!(task.last_failure.as_deref(), Some("tests failed: 2 errors"));

    fixture.runs.update_task(api, TaskState::Checking, None).unwrap();
    fixture.runs.update_task(api, TaskState::Review, None).unwrap();
    let second = fixture
        .runs
        .update_task(api, TaskState::Running, Some("reviewer wants changes"))
        .unwrap_err();
    assert!(matches!(second, RunError::Conflict(_)), "{second}");
    assert!(second.to_string().contains("used its retry"), "{second}");

    fixture
        .runs
        .update_task(api, TaskState::Blocked, Some("merge conflict in src/api.rs"))
        .unwrap();
    let unblock = fixture
        .runs
        .update_task(api, TaskState::Ready, Some("try again"))
        .unwrap_err();
    assert!(matches!(unblock, RunError::Conflict(_)), "{unblock}");
}

#[test]
fn a_failed_task_blocks_everything_after_it_and_the_run_reports_when_all_finish() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));
    fixture
        .runs
        .propose_plan(
            run.id,
            &plan(&[
                ("a", &["a"], &[]),
                ("b", &["b"], &["a"]),
                ("c", &["c"], &["b"]),
                ("d", &["d"], &[]),
            ]),
        )
        .unwrap();
    let tasks = fixture.runs.approve_plan(run.id).unwrap().tasks;
    let (a, b, c, d) = (tasks[0].id, tasks[1].id, tasks[2].id, tasks[3].id);

    fixture.runs.update_task(a, TaskState::Running, None).unwrap();
    let board = fixture
        .runs
        .update_task(a, TaskState::Failed, Some("worker gave up"))
        .unwrap();
    assert_eq!(
        board.tasks.iter().map(|task| task.state).collect::<Vec<_>>(),
        [
            TaskState::Failed,
            TaskState::Blocked,
            TaskState::Blocked,
            TaskState::Ready
        ]
    );
    assert!(
        board.tasks[2]
            .last_failure
            .as_deref()
            .unwrap()
            .contains("\"a\", which failed"),
        "{:?}",
        board.tasks[2].last_failure
    );

    fixture.merge(d);
    fixture.runs.update_task(b, TaskState::Cancelled, None).unwrap();
    assert_eq!(fixture.runs.run(run.id).unwrap().state, RunState::Running);
    let board = fixture
        .runs
        .update_task(c, TaskState::Failed, Some("nothing to build on"))
        .unwrap();
    assert_eq!(board.run.state, RunState::Reporting);

    let done = fixture
        .runs
        .finish(run.id, Path::new("reports/2026-10-22-rate-limiting.md"))
        .unwrap();
    assert_eq!(done.run.state, RunState::Done);
    assert_eq!(
        done.run.report_path.as_deref(),
        Some(Path::new("reports/2026-10-22-rate-limiting.md"))
    );
    let last = fixture.log.after(0, 1000).unwrap().pop().unwrap();
    assert_eq!(last.data["report"], "reports/2026-10-22-rate-limiting.md");
}

#[test]
fn finishing_needs_the_run_to_be_reporting() {
    let fixture = Fixture::new();
    let (run, _) = fixture.running();

    let error = fixture.runs.finish(run.id, Path::new("r.md")).unwrap_err();

    assert!(matches!(error, RunError::Conflict(_)), "{error}");
    assert_eq!(fixture.runs.run(run.id).unwrap().report_path, None);
}

#[test]
fn cancelling_a_run_cancels_its_open_tasks() {
    let fixture = Fixture::new();
    let (run, tasks) = fixture.running();
    let docs = id_of(&tasks, "docs");
    fixture.merge(docs);
    let before = fixture.kinds().len();

    let board = fixture.runs.cancel(run.id, Some("  changed my mind ")).unwrap();

    assert_eq!(board.run.state, RunState::Cancelled);
    assert_eq!(board.run.reason.as_deref(), Some("changed my mind"));
    assert_eq!(
        board.tasks.iter().map(|task| task.state).collect::<Vec<_>>(),
        [TaskState::Cancelled, TaskState::Cancelled, TaskState::Merged]
    );
    assert_eq!(
        fixture.kinds()[before..],
        [
            kind::TASK_STATE_CHANGED,
            kind::TASK_STATE_CHANGED,
            kind::RUN_STATE_CHANGED
        ]
    );

    let again = fixture.runs.cancel(run.id, None).unwrap_err();
    let moved = fixture
        .runs
        .update_task(docs, TaskState::Running, None)
        .unwrap_err();
    assert!(matches!(again, RunError::Conflict(_)), "{again}");
    assert!(matches!(moved, RunError::Conflict(_)), "{moved}");
}

#[test]
fn a_failed_run_keeps_its_reason() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));

    assert!(matches!(
        fixture.runs.fail(run.id, " "),
        Err(RunError::Invalid(_))
    ));
    let board = fixture.runs.fail(run.id, "the chief crashed twice").unwrap();

    assert_eq!(board.run.state, RunState::Failed);
    assert_eq!(board.run.reason.as_deref(), Some("the chief crashed twice"));
}

#[test]
fn worktrees_are_recorded_on_the_task() {
    let fixture = Fixture::new();
    let (_, tasks) = fixture.running();
    let api = id_of(&tasks, "api");

    let task = fixture
        .runs
        .assign_worktree(api, "cos/lisbon-api", Path::new("/wt/lisbon-api"))
        .unwrap();

    assert_eq!(task.branch.as_deref(), Some("cos/lisbon-api"));
    assert_eq!(fixture.runs.task(api).unwrap(), task);
    assert_eq!(fixture.kinds().last().unwrap(), kind::TASK_WORKTREE_ASSIGNED);
}

#[test]
fn list_is_newest_first_with_counts() {
    let fixture = Fixture::new();
    let (older, _) = fixture.running();
    let newer = fixture.create(fixture.workspace("porto"));

    let runs = fixture.runs.list().unwrap();

    assert_eq!(
        runs.iter().map(|r| r.run.id).collect::<Vec<_>>(),
        [newer.id, older.id]
    );
    assert_eq!(runs[0].tasks, TaskCounts::default());
    assert_eq!(
        (runs[1].tasks.ready, runs[1].tasks.proposed, runs[1].tasks.total),
        (2, 1, 3)
    );
}

#[test]
fn a_summary_flattens_the_run_into_json() {
    let fixture = Fixture::new();
    let (run, _) = fixture.running();

    let json = serde_json::to_value(&fixture.runs.list().unwrap()[0]).unwrap();

    assert_eq!(json["id"], run.id);
    assert_eq!(json["state"], "running");
    assert_eq!(json["preset"], "normal");
    assert_eq!(json["tasks"]["ready"], 2);
    assert_eq!(serde_json::from_value::<RunSummary>(json).unwrap().run, run);
}

#[test]
fn everything_survives_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cos.db");
    let before = {
        let fixture = Fixture::with_store(Store::open(&path).unwrap());
        let (run, tasks) = fixture.running();
        let api = id_of(&tasks, "api");
        fixture.runs.update_task(api, TaskState::Running, None).unwrap();
        fixture.runs.update_task(api, TaskState::Checking, None).unwrap();
        fixture
            .runs
            .update_task(api, TaskState::Running, Some("lint"))
            .unwrap();
        fixture
            .runs
            .assign_worktree(api, "cos/lisbon-api", Path::new("/wt/api"))
            .unwrap();
        fixture.runs.board(run.id).unwrap()
    };

    let fixture = Fixture::with_store(Store::open(&path).unwrap());

    assert_eq!(fixture.runs.board(before.run.id).unwrap(), before);
}

#[test]
fn the_runs_migration_upgrades_an_existing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cos.db");
    let runs_migration = MIGRATIONS
        .iter()
        .position(|sql| sql.contains("CREATE TABLE runs"))
        .unwrap();
    let workspace = {
        // A database as the previous cos left it.
        let conn = Connection::open(&path).unwrap();
        for sql in &MIGRATIONS[..runs_migration] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", runs_migration as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO events (ts_ms, kind, data) VALUES (0, 'daemon.started', '{}')",
            [],
        )
        .unwrap();
        add_workspace(&conn, "lisbon")
    };

    let fixture = Fixture::with_store(Store::open(&path).unwrap());

    let version: i64 = fixture
        .store
        .lock()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version as usize, MIGRATIONS.len());
    assert_eq!(fixture.kinds(), ["daemon.started"]);
    let run = fixture.create(workspace);
    assert_eq!(
        fixture.runs.board(run.id).unwrap().workspace.unwrap().name,
        "lisbon"
    );
}

#[test]
fn the_board_renders_as_markdown() {
    let fixture = Fixture::new();
    let (run, tasks) = fixture.running();
    let api = id_of(&tasks, "api");
    fixture.merge(api);
    let docs = id_of(&tasks, "docs");
    fixture.runs.update_task(docs, TaskState::Running, None).unwrap();
    fixture
        .runs
        .update_task(docs, TaskState::Blocked, Some("needs a | decision\non wording"))
        .unwrap();
    fixture
        .runs
        .assign_worktree(api, "cos/lisbon-api", Path::new("/wt/api"))
        .unwrap();

    let markdown = render_board(&fixture.runs.board(run.id).unwrap());

    assert!(
        markdown.starts_with(&format!("# Run {}: running\n\n> add rate limiting\n", run.id)),
        "{markdown}"
    );
    assert!(
        markdown.contains("- Workspace: lisbon (`cos/lisbon`)"),
        "{markdown}"
    );
    assert!(
        markdown.contains("- Approvals: normal preset, the chief may approve up to 20"),
        "{markdown}"
    );
    assert!(
        markdown.contains("- Tasks: 1 blocked, 1 merged, 1 ready (3 total)"),
        "{markdown}"
    );
    assert!(
        markdown.contains(&format!(
            "| {api} | api | merged |  | `src/api.rs` | `cos/lisbon-api` |"
        )),
        "{markdown}"
    );
    let ui = id_of(&tasks, "ui");
    assert!(
        markdown.contains(&format!("| {ui} | ui | ready | #{api} | `src/ui/**` |  |")),
        "{markdown}"
    );
    assert!(
        markdown.contains(&format!(
            "- #{docs} docs (blocked): needs a | decision on wording"
        )),
        "{markdown}"
    );
}

#[test]
fn an_empty_board_says_the_chief_is_planning() {
    let fixture = Fixture::new();
    let run = fixture.create(fixture.workspace("lisbon"));
    fixture
        .store
        .lock()
        .execute("DELETE FROM workspaces", [])
        .unwrap();

    let markdown = render_board(&fixture.runs.board(run.id).unwrap());

    assert!(markdown.contains("- Workspace: #1 (removed)"), "{markdown}");
    assert!(markdown.contains("- Tasks: no tasks yet"), "{markdown}");
    assert!(
        markdown.contains("No plan yet; the chief is working on it."),
        "{markdown}"
    );
}
