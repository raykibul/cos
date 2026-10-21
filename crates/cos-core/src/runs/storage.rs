//! SQL for runs and tasks. Every function takes a connection the caller has
//! locked, so a whole change can run in one transaction.

use std::collections::HashMap;
use std::path::PathBuf;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::plan::CheckedTask;
use super::{ApprovalPreset, Run, Task, TaskCounts, TaskState};

const RUN_COLUMNS: &str =
    "id, workspace_id, goal, state, preset, approval_cap, report_path, reason, created_ms, updated_ms";
const TASK_COLUMNS: &str = "id, run_id, position, title, instructions, files, test_command, depends_on, \
     state, retries, branch, worktree, last_failure, created_ms, updated_ms, checks_json, review_json, \
     (SELECT MAX(a.id) FROM agents a WHERE a.task_id = tasks.id AND a.role = 'worker')";

/// The current time at the precision it's stored with, so what's returned
/// equals what's read back later.
pub(super) fn now() -> Timestamp {
    Timestamp::from_millisecond(Timestamp::now().as_millisecond()).expect("the current time is in range")
}

/// Inserts a run and returns its id. `run.id` is ignored.
pub(super) fn insert_run(conn: &Connection, run: &Run) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO runs (workspace_id, goal, state, preset, approval_cap, report_path, reason, created_ms, updated_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run.workspace_id,
            run.goal,
            run.state.as_str(),
            run.preset.as_str(),
            run.approval_cap,
            run.report_path.as_ref().map(|path| path.to_string_lossy()),
            run.reason,
            run.created_at.as_millisecond(),
            run.updated_at.as_millisecond(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Saves a run's state, reason, report and update time.
pub(super) fn save_run(conn: &Connection, run: &Run) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET state = ?2, reason = ?3, report_path = ?4, updated_ms = ?5 WHERE id = ?1",
        params![
            run.id,
            run.state.as_str(),
            run.reason,
            run.report_path.as_ref().map(|path| path.to_string_lossy()),
            run.updated_at.as_millisecond(),
        ],
    )?;
    Ok(())
}

pub(super) fn run(conn: &Connection, id: i64) -> rusqlite::Result<Option<Run>> {
    conn.prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"))?
        .query_row([id], read_run)
        .optional()
}

/// Every run, newest first.
pub(super) fn runs(conn: &Connection) -> rusqlite::Result<Vec<Run>> {
    conn.prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM runs ORDER BY id DESC"))?
        .query_map([], read_run)?
        .collect()
}

/// The newest run in the workspace that hasn't finished, if any.
pub(super) fn unfinished_run_in(conn: &Connection, workspace_id: i64) -> rusqlite::Result<Option<Run>> {
    let runs: Vec<Run> = conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE workspace_id = ?1 ORDER BY id DESC"
        ))?
        .query_map([workspace_id], read_run)?
        .collect::<Result<_, _>>()?;
    Ok(runs.into_iter().find(|run| !run.state.is_finished()))
}

/// Task counts by state for every run that has tasks.
pub(super) fn task_counts(conn: &Connection) -> rusqlite::Result<HashMap<i64, TaskCounts>> {
    let mut counts: HashMap<i64, TaskCounts> = HashMap::new();
    let mut query =
        conn.prepare_cached("SELECT run_id, state, COUNT(*) FROM tasks GROUP BY run_id, state")?;
    let rows = query.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            parse::<TaskState>(row, 1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (run_id, state, count) = row?;
        counts.entry(run_id).or_default().add(state, count as usize);
    }
    Ok(counts)
}

/// A run's tasks in plan order.
pub(super) fn tasks_of(conn: &Connection, run_id: i64) -> rusqlite::Result<Vec<Task>> {
    conn.prepare_cached(&format!(
        "SELECT {TASK_COLUMNS} FROM tasks WHERE run_id = ?1 ORDER BY position"
    ))?
    .query_map([run_id], read_task)?
    .collect()
}

pub(super) fn task(conn: &Connection, id: i64) -> rusqlite::Result<Option<Task>> {
    conn.prepare_cached(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"))?
        .query_row([id], read_task)
        .optional()
}

/// The run a task belongs to.
pub(super) fn task_run(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT run_id FROM tasks WHERE id = ?1", [task_id], |row| {
        row.get(0)
    })
    .optional()
}

pub(super) fn delete_tasks(conn: &Connection, run_id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM tasks WHERE run_id = ?1", [run_id])?;
    Ok(())
}

/// Inserts a validated plan as `proposed` tasks and returns them in order.
/// Dependencies are indexes into the plan until the ids exist, so they are
/// filled in second.
pub(super) fn insert_plan(
    conn: &Connection,
    run_id: i64,
    plan: &[CheckedTask],
    now: Timestamp,
) -> rusqlite::Result<Vec<Task>> {
    let mut tasks = Vec::with_capacity(plan.len());
    for (position, checked) in plan.iter().enumerate() {
        let mut task = Task {
            id: 0,
            run_id,
            position: position as u32,
            title: checked.title.clone(),
            instructions: checked.instructions.clone(),
            files: checked.files.clone(),
            test_command: checked.test_command.clone(),
            depends_on: Vec::new(),
            state: TaskState::Proposed,
            retries: 0,
            branch: None,
            worktree: None,
            last_failure: None,
            agent_id: None,
            checks: None,
            review: None,
            created_at: now,
            updated_at: now,
        };
        conn.execute(
            "INSERT INTO tasks (run_id, position, title, instructions, files, test_command, depends_on,
                                state, retries, created_ms, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7, 0, ?8, ?8)",
            params![
                run_id,
                task.position,
                task.title,
                task.instructions,
                to_json(&task.files),
                task.test_command,
                task.state.as_str(),
                now.as_millisecond(),
            ],
        )?;
        task.id = conn.last_insert_rowid();
        tasks.push(task);
    }
    for (index, checked) in plan.iter().enumerate() {
        if checked.depends_on.is_empty() {
            continue;
        }
        let ids: Vec<i64> = checked.depends_on.iter().map(|&dep| tasks[dep].id).collect();
        conn.execute(
            "UPDATE tasks SET depends_on = ?2 WHERE id = ?1",
            params![tasks[index].id, to_json(&ids)],
        )?;
        tasks[index].depends_on = ids;
    }
    Ok(tasks)
}

/// Saves what can change on a task after it's created.
pub(super) fn save_task(conn: &Connection, task: &Task) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET state = ?2, retries = ?3, branch = ?4, worktree = ?5, last_failure = ?6, updated_ms = ?7
         WHERE id = ?1",
        params![
            task.id,
            task.state.as_str(),
            task.retries,
            task.branch,
            task.worktree.as_ref().map(|path| path.to_string_lossy()),
            task.last_failure,
            task.updated_at.as_millisecond(),
        ],
    )?;
    Ok(())
}

fn read_run(row: &Row<'_>) -> rusqlite::Result<Run> {
    let preset: String = row.get(4)?;
    Ok(Run {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        goal: row.get(2)?,
        state: parse(row, 3)?,
        preset: ApprovalPreset::parse(&preset)
            .ok_or_else(|| bad_value(4, format!("unknown approval preset {preset:?}")))?,
        approval_cap: row.get(5)?,
        report_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
        reason: row.get(7)?,
        created_at: timestamp(row, 8)?,
        updated_at: timestamp(row, 9)?,
    })
}

fn read_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        run_id: row.get(1)?,
        position: row.get(2)?,
        title: row.get(3)?,
        instructions: row.get(4)?,
        files: from_json(row, 5)?,
        test_command: row.get(6)?,
        depends_on: from_json(row, 7)?,
        state: parse(row, 8)?,
        retries: row.get(9)?,
        branch: row.get(10)?,
        worktree: row.get::<_, Option<String>>(11)?.map(PathBuf::from),
        last_failure: row.get(12)?,
        created_at: timestamp(row, 13)?,
        updated_at: timestamp(row, 14)?,
        checks: optional_json(row, 15)?,
        review: optional_json(row, 16)?,
        agent_id: row.get(17)?,
    })
}

/// Sets `checks_json` or `review_json` on a task.
pub(super) fn save_task_json(
    conn: &Connection,
    task_id: i64,
    column: &'static str,
    json: &str,
    now: Timestamp,
) -> rusqlite::Result<()> {
    debug_assert!(matches!(column, "checks_json" | "review_json"));
    conn.execute(
        &format!("UPDATE tasks SET {column} = ?2, updated_ms = ?3 WHERE id = ?1"),
        params![task_id, json, now.as_millisecond()],
    )?;
    Ok(())
}

fn parse<T: std::str::FromStr<Err = String>>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    row.get_ref(column)?
        .as_str()?
        .parse()
        .map_err(|error| bad_value(column, error))
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("lists of strings and numbers serialize")
}

fn from_json<T: serde::de::DeserializeOwned>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    serde_json::from_str(row.get_ref(column)?.as_str()?).map_err(|error| bad_value(column, error.to_string()))
}

fn optional_json<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(column)?
        .map(|text| serde_json::from_str(&text).map_err(|error| bad_value(column, error.to_string())))
        .transpose()
}

fn timestamp(row: &Row<'_>, column: usize) -> rusqlite::Result<Timestamp> {
    Timestamp::from_millisecond(row.get(column)?)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(column, Type::Integer, Box::new(error)))
}

fn bad_value(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}
