//! The daemon's SQLite database.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::LoggedEvent;

/// Schema changes, applied in order. `PRAGMA user_version` records how many
/// have run. Never edit one that has shipped; add a new one.
pub(crate) const MIGRATIONS: &[&str] = &[
    "
    -- AUTOINCREMENT so a sequence number is never reused, even after deletes.
    -- Clients resume streams by sequence number.
    CREATE TABLE events (
        seq    INTEGER PRIMARY KEY AUTOINCREMENT,
        ts_ms  INTEGER NOT NULL,
        kind   TEXT    NOT NULL,
        run_id TEXT,
        data   TEXT    NOT NULL
    );
    CREATE INDEX events_by_run ON events (run_id, seq);
",
    "
    -- A registered git repo, by its main checkout.
    CREATE TABLE projects (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        root        TEXT    NOT NULL UNIQUE,
        name        TEXT    NOT NULL,
        base_branch TEXT    NOT NULL,
        created_ms  INTEGER NOT NULL
    );
    -- A worktree cos created for a project. Agents run inside it.
    CREATE TABLE workspaces (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id  INTEGER NOT NULL REFERENCES projects (id),
        name        TEXT    NOT NULL,
        title       TEXT    NOT NULL,
        branch      TEXT    NOT NULL,
        path        TEXT    NOT NULL UNIQUE,
        base        TEXT    NOT NULL CHECK (base IN ('origin', 'head')),
        base_ref    TEXT    NOT NULL,
        base_commit TEXT    NOT NULL,
        created_ms  INTEGER NOT NULL,
        UNIQUE (project_id, name)
    );
",
    "
    -- A tailnet device that has tried to reach the daemon over its tailnet
    -- listener. Only approved ones are let in.
    CREATE TABLE devices (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        node_id       TEXT    NOT NULL UNIQUE, -- Tailscale's stable node ID
        name          TEXT    NOT NULL,        -- MagicDNS name, such as 'my-iphone'
        hostname      TEXT    NOT NULL,        -- what the device calls itself
        os            TEXT    NOT NULL,
        login_name    TEXT    NOT NULL,        -- the Tailscale user it is logged in as
        state         TEXT    NOT NULL CHECK (state IN ('pending', 'approved', 'denied')),
        last_ip       TEXT,
        first_seen_ms INTEGER NOT NULL,
        last_seen_ms  INTEGER NOT NULL,
        decided_ms    INTEGER
    );
    -- Daemon settings that survive restarts, such as whether the tailnet
    -- listener is on. Values are JSON.
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
",
    "
    -- One goal in one workspace. No foreign key on the workspace: a run's
    -- history outlives the worktree it ran in.
    CREATE TABLE runs (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        workspace_id INTEGER NOT NULL,
        goal         TEXT    NOT NULL,
        state        TEXT    NOT NULL,
        preset       TEXT    NOT NULL,
        approval_cap INTEGER NOT NULL,
        report_path  TEXT,
        reason       TEXT,
        created_ms   INTEGER NOT NULL,
        updated_ms   INTEGER NOT NULL
    );
    CREATE INDEX runs_by_workspace ON runs (workspace_id, id);
    -- One task of a run's plan. files and depends_on are JSON arrays.
    CREATE TABLE tasks (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id       INTEGER NOT NULL REFERENCES runs (id),
        position     INTEGER NOT NULL,
        title        TEXT    NOT NULL,
        instructions TEXT    NOT NULL,
        files        TEXT    NOT NULL,
        test_command TEXT,
        depends_on   TEXT    NOT NULL,
        state        TEXT    NOT NULL,
        retries      INTEGER NOT NULL,
        branch       TEXT,
        worktree     TEXT,
        last_failure TEXT,
        created_ms   INTEGER NOT NULL,
        updated_ms   INTEGER NOT NULL,
        UNIQUE (run_id, position)
    );
",
    "
    -- A Claude session the daemon runs in a terminal, in a workspace's folder.
    -- Forgotten with its workspace.
    CREATE TABLE agents (
        id               INTEGER PRIMARY KEY AUTOINCREMENT,
        workspace_id     INTEGER NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
        run_id           INTEGER REFERENCES runs (id),
        role             TEXT    NOT NULL,
        title            TEXT    NOT NULL,
        session_id       TEXT    NOT NULL UNIQUE,
        pid              INTEGER,
        state            TEXT    NOT NULL,
        model            TEXT,
        permission_mode  TEXT    NOT NULL,
        -- 1: Claude loads the user's own settings; 0: only the repo's.
        user_settings    INTEGER NOT NULL,
        term_rows        INTEGER NOT NULL,
        term_cols        INTEGER NOT NULL,
        started_ms       INTEGER NOT NULL,
        last_activity_ms INTEGER NOT NULL,
        exited_ms        INTEGER,
        exit_code        INTEGER,
        last_message     TEXT
    );
    CREATE INDEX agents_by_workspace ON agents (workspace_id);
    -- A permission request from an agent, and how it was answered.
    CREATE TABLE approvals (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id     INTEGER NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
        workspace_id INTEGER NOT NULL,
        tool_name    TEXT    NOT NULL,
        tool_input   TEXT    NOT NULL,
        tool_use_id  TEXT,
        summary      TEXT    NOT NULL,
        state        TEXT    NOT NULL,
        decided_by   TEXT,
        message      TEXT,
        created_ms   INTEGER NOT NULL,
        decided_ms   INTEGER
    );
    CREATE INDEX approvals_by_agent ON approvals (agent_id, state);
    CREATE INDEX approvals_by_state ON approvals (state, id);
",
    "
    -- How cos-policy judged each approval: its tier (allowed, chief, ask_user,
    -- never_delegated), who it went to (auto, chief, user) and why. NULL on
    -- approvals from before the policy was wired in.
    ALTER TABLE approvals ADD COLUMN tier TEXT;
    ALTER TABLE approvals ADD COLUMN route TEXT;
    ALTER TABLE approvals ADD COLUMN reason TEXT;
    -- How many requests the chief has approved in the run, against approval_cap.
    ALTER TABLE runs ADD COLUMN chief_approvals INTEGER NOT NULL DEFAULT 0;
    -- Every answer to a run agent's request that cos, the chief or you gave,
    -- for the report. No foreign keys: it outlives the agents and approvals.
    -- record is a cos_policy::AuditRecord as JSON.
    CREATE TABLE approval_audit (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id      INTEGER NOT NULL,
        approval_id INTEGER,
        at_ms       INTEGER NOT NULL,
        record      TEXT    NOT NULL
    );
    CREATE INDEX approval_audit_by_run ON approval_audit (run_id, id);
",
    "
    -- The pipeline: which task a worker or reviewer works on, and where and
    -- how a run agent was started, so a resume starts it the same way.
    ALTER TABLE agents ADD COLUMN task_id INTEGER REFERENCES tasks (id);
    ALTER TABLE agents ADD COLUMN cwd TEXT;           -- NULL: the workspace folder
    ALTER TABLE agents ADD COLUMN system_prompt TEXT; -- --append-system-prompt
    ALTER TABLE agents ADD COLUMN mcp_config TEXT;    -- --mcp-config
    CREATE INDEX agents_by_task ON agents (task_id, id);
    -- The last checks and review of a task, as JSON.
    ALTER TABLE tasks ADD COLUMN checks_json TEXT;
    ALTER TABLE tasks ADD COLUMN review_json TEXT;
",
    "
    -- The chief: the agent that plans and steers a run.
    ALTER TABLE runs ADD COLUMN chief_agent_id INTEGER;
    -- One message the daemon typed into a chief, holding inbox items.
    CREATE TABLE wakeups (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id     INTEGER NOT NULL REFERENCES runs (id),
        agent_id   INTEGER NOT NULL, -- no foreign key: agents are forgotten with their workspace
        items      INTEGER NOT NULL,
        bytes      INTEGER NOT NULL,
        created_ms INTEGER NOT NULL
    );
    CREATE INDEX wakeups_by_run ON wakeups (run_id, created_ms);
    -- Something the chief should hear about, waiting for its next wakeup.
    CREATE TABLE inbox (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id       INTEGER NOT NULL REFERENCES runs (id),
        kind         TEXT    NOT NULL,
        text         TEXT    NOT NULL,
        created_ms   INTEGER NOT NULL,
        wakeup_id    INTEGER REFERENCES wakeups (id), -- set once delivered
        delivered_ms INTEGER
    );
    CREATE INDEX inbox_waiting ON inbox (run_id, wakeup_id, id);
    -- A question the chief asked you. choices is a JSON array.
    CREATE TABLE questions (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id      INTEGER NOT NULL REFERENCES runs (id),
        agent_id    INTEGER,
        question    TEXT    NOT NULL,
        choices     TEXT    NOT NULL,
        state       TEXT    NOT NULL CHECK (state IN ('open', 'answered')),
        answer      TEXT,
        created_ms  INTEGER NOT NULL,
        answered_ms INTEGER
    );
    CREATE INDEX questions_by_state ON questions (state, id);
",
    "
    -- The Chief of Staff. Workspace titles become optional (NULL: clients
    -- show the city), and the chief's own worktree is a workspace row of kind
    -- 'staff', which listings leave out. SQLite can't drop NOT NULL, so the
    -- table is rebuilt; migrations run with foreign keys off for this.
    CREATE TABLE workspaces_new (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id  INTEGER NOT NULL REFERENCES projects (id),
        kind        TEXT    NOT NULL DEFAULT 'workspace' CHECK (kind IN ('workspace', 'staff')),
        name        TEXT    NOT NULL,
        title       TEXT,
        branch      TEXT    NOT NULL,
        path        TEXT    NOT NULL UNIQUE,
        base        TEXT    NOT NULL CHECK (base IN ('origin', 'head')),
        base_ref    TEXT    NOT NULL,
        base_commit TEXT    NOT NULL,
        created_ms  INTEGER NOT NULL,
        UNIQUE (project_id, name)
    );
    INSERT INTO workspaces_new (id, project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
        SELECT id, project_id, name, title, branch, path, base, base_ref, base_commit, created_ms FROM workspaces;
    DROP TABLE workspaces;
    ALTER TABLE workspaces_new RENAME TO workspaces;
    -- The agent that started this one (the Chief of Staff), and Claude's
    -- transcript file, from the hooks.
    ALTER TABLE agents ADD COLUMN spawned_by INTEGER;
    ALTER TABLE agents ADD COLUMN transcript_path TEXT;
    CREATE INDEX agents_by_spawner ON agents (spawned_by);
    -- The Chief of Staff that started a run.
    ALTER TABLE runs ADD COLUMN spawned_by INTEGER;
    -- One Chief of Staff per project. agent_id has no foreign key: agents are
    -- forgotten with their workspace.
    CREATE TABLE staff (
        project_id INTEGER PRIMARY KEY REFERENCES projects (id),
        agent_id   INTEGER,
        model      TEXT,
        brief      TEXT    NOT NULL DEFAULT '',
        updated_ms INTEGER NOT NULL
    );
    -- One batch of updates typed into a Chief of Staff.
    CREATE TABLE staff_wakeups (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id INTEGER NOT NULL,
        agent_id   INTEGER NOT NULL,
        items      INTEGER NOT NULL,
        bytes      INTEGER NOT NULL,
        created_ms INTEGER NOT NULL
    );
    CREATE INDEX staff_wakeups_by_project ON staff_wakeups (project_id, created_ms);
    -- What a Chief of Staff should hear: your messages and its agents' and
    -- runs' news. Waiting until delivered_ms is set; updates are delivered in
    -- a wakeup, your messages one at a time.
    CREATE TABLE staff_inbox (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id   INTEGER NOT NULL,
        kind         TEXT    NOT NULL,
        agent_id     INTEGER,
        run_id       INTEGER,
        text         TEXT    NOT NULL,
        created_ms   INTEGER NOT NULL,
        wakeup_id    INTEGER REFERENCES staff_wakeups (id),
        delivered_ms INTEGER
    );
    CREATE INDEX staff_inbox_waiting ON staff_inbox (project_id, delivered_ms, id);
",
    "
    -- Environment variables for agents: a project's (project_id set) or the
    -- global list (project_id NULL). Values are secrets: never logged.
    CREATE TABLE env (
        project_id INTEGER REFERENCES projects (id) ON DELETE CASCADE,
        name       TEXT    NOT NULL,
        value      TEXT    NOT NULL,
        updated_ms INTEGER NOT NULL,
        UNIQUE (project_id, name)
    );
    -- UNIQUE treats NULLs as distinct, so this keeps global names unique too.
    CREATE UNIQUE INDEX env_by_name ON env (COALESCE(project_id, 0), name);
    -- The names (JSON array) of the extra variables an agent was last
    -- launched with. Never the values.
    ALTER TABLE agents ADD COLUMN env_names TEXT;
",
    "
    -- A Chief of Staff's coordination is paused: news waits in its inbox
    -- and only your messages go in.
    ALTER TABLE staff ADD COLUMN paused INTEGER NOT NULL DEFAULT 0;
    -- You stopped it yourself, so a daemon restart leaves it stopped. Any
    -- start clears it.
    ALTER TABLE staff ADD COLUMN stopped INTEGER NOT NULL DEFAULT 0;
",
    "
    -- jev (cos-judge), in shadow mode: which projects it may judge (off
    -- unless a row says on), and every decision it made.
    CREATE TABLE judge_projects (
        project_id INTEGER PRIMARY KEY REFERENCES projects (id) ON DELETE CASCADE,
        enabled    INTEGER NOT NULL,
        updated_ms INTEGER NOT NULL
    );
    -- No foreign keys: decisions are labelled examples, kept after their
    -- project, approval or inbox item is gone.
    CREATE TABLE judge_decisions (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        point       TEXT    NOT NULL,  -- 'permission' | 'wake'
        project_id  INTEGER,
        subject     TEXT    NOT NULL,  -- 'approval:12', 'inbox:5', 'staff_inbox:9'
        approval_id INTEGER,
        summary     TEXT    NOT NULL,  -- one line for people
        input_hash  TEXT    NOT NULL,  -- SHA-256 of the trimmed input
        input       TEXT    NOT NULL,  -- the trimmed input sent, as JSON
        answers     TEXT,              -- jev's typed answers, NULL on error
        would       TEXT    NOT NULL,  -- 'approve' | 'ask_you' | 'now' | 'batch' | 'none' | 'no_opinion'
        why         TEXT,
        no_opinion  INTEGER NOT NULL,
        actual      TEXT,              -- what you did: 'allowed' | 'denied' | 'terminal' | 'expired'
        actual_ms   INTEGER,
        latency_ms  INTEGER NOT NULL,
        error       TEXT,
        model       TEXT,
        created_ms  INTEGER NOT NULL
    );
    CREATE INDEX judge_decisions_by_point ON judge_decisions (point, id);
    CREATE INDEX judge_decisions_by_approval ON judge_decisions (approval_id);
",
    "
    -- Runs are gone: the Chief of Staff coordinates agents itself. Run
    -- agents (chiefs, workers, reviewers) and their approvals go with the
    -- run tables, and agents loses run_id and task_id, so it is rebuilt.
    DELETE FROM approvals WHERE agent_id IN (SELECT id FROM agents WHERE role NOT IN ('user', 'staff'));
    DELETE FROM agents WHERE role NOT IN ('user', 'staff');
    CREATE TABLE agents_new (
        id               INTEGER PRIMARY KEY AUTOINCREMENT,
        workspace_id     INTEGER NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
        role             TEXT    NOT NULL,
        title            TEXT    NOT NULL,
        session_id       TEXT    NOT NULL UNIQUE,
        pid              INTEGER,
        state            TEXT    NOT NULL,
        model            TEXT,
        permission_mode  TEXT    NOT NULL,
        -- 1: Claude loads the user's own settings; 0: only the repo's.
        user_settings    INTEGER NOT NULL,
        term_rows        INTEGER NOT NULL,
        term_cols        INTEGER NOT NULL,
        started_ms       INTEGER NOT NULL,
        last_activity_ms INTEGER NOT NULL,
        exited_ms        INTEGER,
        exit_code        INTEGER,
        last_message     TEXT,
        cwd              TEXT,    -- NULL: the workspace folder
        system_prompt    TEXT,    -- --append-system-prompt
        mcp_config       TEXT,    -- --mcp-config
        spawned_by       INTEGER, -- the Chief of Staff that started it
        transcript_path  TEXT,
        env_names        TEXT
    );
    INSERT INTO agents_new (id, workspace_id, role, title, session_id, pid, state, model, permission_mode,
            user_settings, term_rows, term_cols, started_ms, last_activity_ms, exited_ms, exit_code, last_message,
            cwd, system_prompt, mcp_config, spawned_by, transcript_path, env_names)
        SELECT id, workspace_id, role, title, session_id, pid, state, model, permission_mode,
            user_settings, term_rows, term_cols, started_ms, last_activity_ms, exited_ms, exit_code, last_message,
            cwd, system_prompt, mcp_config, spawned_by, transcript_path, env_names
        FROM agents;
    DROP TABLE agents;
    ALTER TABLE agents_new RENAME TO agents;
    CREATE INDEX agents_by_workspace ON agents (workspace_id);
    CREATE INDEX agents_by_spawner ON agents (spawned_by);
    -- Every approval goes to you now, so there is nobody to route it to.
    ALTER TABLE approvals DROP COLUMN route;
    DELETE FROM staff_inbox WHERE kind = 'run';
    ALTER TABLE staff_inbox DROP COLUMN run_id;
    DROP TABLE approval_audit;
    DROP TABLE questions;
    DROP TABLE inbox;
    DROP TABLE wakeups;
    DROP TABLE tasks;
    DROP TABLE runs;
",
    "
    -- A Chief of Staff's memory notes: named, with a one-line description
    -- that goes in its system prompt, and a body it reads when it needs it.
    CREATE TABLE staff_notes (
        project_id  INTEGER NOT NULL REFERENCES projects (id),
        name        TEXT    NOT NULL,
        description TEXT    NOT NULL,
        body        TEXT    NOT NULL,
        created_ms  INTEGER NOT NULL,
        updated_ms  INTEGER NOT NULL,
        PRIMARY KEY (project_id, name)
    );
",
    "
    -- Who a Chief of Staff is, written by the user: it goes at the top of
    -- its system prompt, and only the user can change it.
    ALTER TABLE staff ADD COLUMN identity TEXT NOT NULL DEFAULT '';
",
    "
    -- 0: a Chief of Staff started it fire-and-forget, so its news (turns,
    -- exits, waits) doesn't go to the chief's inbox.
    ALTER TABLE agents ADD COLUMN report INTEGER NOT NULL DEFAULT 1;
",
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error("the database has schema version {found}, but this cos only knows up to {known}; update cos")]
    TooNew { found: usize, known: usize },

    #[error("event #{seq} in the database is unreadable: {reason}")]
    Corrupt { seq: u64, reason: String },

    #[error("setting {key:?} in the database is unreadable: {reason}")]
    BadSetting { key: String, reason: String },
}

/// The database, behind a lock. Calls are short (single-row writes, small
/// reads), so they run directly on the async runtime.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        // In-memory databases answer "memory" here, which is fine.
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Stores a new event. `then` runs before the lock is released, so
    /// whatever it does (publishing the event) happens in sequence order.
    pub(crate) fn append_event(
        &self,
        kind: &str,
        run_id: Option<&str>,
        data: Value,
        then: impl FnOnce(&LoggedEvent),
    ) -> Result<LoggedEvent, StoreError> {
        let conn = self.lock();
        // Stored with millisecond precision, so keep the same precision here.
        let ts = Timestamp::from_millisecond(Timestamp::now().as_millisecond())
            .expect("the current time is in range");
        conn.execute(
            "INSERT INTO events (ts_ms, kind, run_id, data) VALUES (?1, ?2, ?3, ?4)",
            params![ts.as_millisecond(), kind, run_id, data.to_string()],
        )?;
        let event = LoggedEvent {
            seq: conn.last_insert_rowid() as u64,
            ts,
            kind: kind.to_owned(),
            run_id: run_id.map(str::to_owned),
            data,
        };
        then(&event);
        Ok(event)
    }

    /// Up to `limit` events with a sequence number above `after`, oldest first.
    pub(crate) fn events_after(&self, after: u64, limit: usize) -> Result<Vec<LoggedEvent>, StoreError> {
        let conn = self.lock();
        let mut query = conn.prepare_cached(
            "SELECT seq, ts_ms, kind, run_id, data FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = query.query_map(params![after as i64, limit as i64], read_event)?;
        rows.map(|row| row?).collect()
    }

    /// How many events are stored, and the highest sequence number (0 if none).
    pub(crate) fn event_stats(&self) -> Result<(u64, u64), StoreError> {
        let (count, last): (i64, i64) =
            self.lock()
                .query_row("SELECT COUNT(*), COALESCE(MAX(seq), 0) FROM events", [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
        Ok((count as u64, last as u64))
    }

    /// A setting's value, or `None` if it was never set.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StoreError> {
        let value: Option<String> = self
            .lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| StoreError::BadSetting {
                    key: key.to_owned(),
                    reason: error.to_string(),
                })
            })
            .transpose()
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), StoreError> {
        let value = serde_json::to_string(value).map_err(|error| StoreError::BadSetting {
            key: key.to_owned(),
            reason: error.to_string(),
        })?;
        self.lock().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = applied as usize;
    if applied > MIGRATIONS.len() {
        return Err(StoreError::TooNew {
            found: applied,
            known: MIGRATIONS.len(),
        });
    }
    if applied == MIGRATIONS.len() {
        return Ok(());
    }
    // Off while migrating, so a migration can rebuild a table others refer
    // to (dropping it with foreign keys on would cascade). It can't change
    // inside a transaction, and every key is checked before committing.
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| {
        let tx = conn.transaction()?;
        for migration in &MIGRATIONS[applied..] {
            tx.execute_batch(migration)?;
        }
        let broken: i64 = tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
        if broken > 0 {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
                Some(format!("{broken} rows break a foreign key after migrating")),
            ));
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()
    })();
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(result?)
}

/// Reads one `events` row. A row that can't be decoded is reported as
/// corrupt rather than as a SQLite error.
fn read_event(row: &Row<'_>) -> rusqlite::Result<Result<LoggedEvent, StoreError>> {
    let seq = row.get::<_, i64>(0)? as u64;
    let ts_ms: i64 = row.get(1)?;
    let data: String = row.get(4)?;
    let corrupt = |reason: String| StoreError::Corrupt { seq, reason };

    let ts = match Timestamp::from_millisecond(ts_ms) {
        Ok(ts) => ts,
        Err(error) => return Ok(Err(corrupt(error.to_string()))),
    };
    let data = match serde_json::from_str(&data) {
        Ok(data) => data,
        Err(error) => return Ok(Err(corrupt(error.to_string()))),
    };
    Ok(Ok(LoggedEvent {
        seq,
        ts,
        kind: row.get(2)?,
        run_id: row.get(3)?,
        data,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn append(store: &Store, kind: &str) -> LoggedEvent {
        store.append_event(kind, None, json!({}), |_| {}).unwrap()
    }

    #[test]
    fn events_get_increasing_sequence_numbers() {
        let store = Store::open_in_memory().unwrap();

        let first = append(&store, "a");
        let second = append(&store, "b");

        assert_eq!((first.seq, second.seq), (1, 2));
        assert_eq!(store.event_stats().unwrap(), (2, 2));
    }

    #[test]
    fn events_after_pages_in_order() {
        let store = Store::open_in_memory().unwrap();
        for kind in ["a", "b", "c", "d"] {
            append(&store, kind);
        }

        let page = store.events_after(1, 2).unwrap();

        assert_eq!(
            page.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert!(store.events_after(4, 10).unwrap().is_empty());
    }

    #[test]
    fn stored_events_read_back_unchanged() {
        let store = Store::open_in_memory().unwrap();
        let written = store
            .append_event("run.started", Some("run-1"), json!({ "goal": "x" }), |_| {})
            .unwrap();

        let read = store.events_after(0, 1).unwrap().remove(0);

        assert_eq!(read, written);
    }

    #[test]
    fn database_survives_reopening_and_never_reuses_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cos.db");
        {
            let store = Store::open(&path).unwrap();
            append(&store, "a");
            append(&store, "b");
            store
                .lock()
                .execute("DELETE FROM events WHERE seq = 2", [])
                .unwrap();
        }

        let store = Store::open(&path).unwrap();
        assert_eq!(store.event_stats().unwrap(), (1, 1));
        assert_eq!(append(&store, "c").seq, 3);
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::open_in_memory().unwrap();

        assert_eq!(store.setting::<bool>("x").unwrap(), None);
        store.set_setting("x", &json!({ "on": true })).unwrap();
        store.set_setting("x", &json!({ "on": false })).unwrap();

        assert_eq!(store.setting("x").unwrap(), Some(json!({ "on": false })));
        let error = store.setting::<bool>("x").expect_err("wrong type");
        assert!(matches!(error, StoreError::BadSetting { .. }), "{error}");
    }

    #[test]
    fn an_older_database_gains_the_device_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cos.db");
        {
            // A database as the previous release left it: two migrations in.
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            for migration in &MIGRATIONS[..2] {
                tx.execute_batch(migration).unwrap();
            }
            tx.pragma_update(None, "user_version", 2).unwrap();
            tx.execute(
                "INSERT INTO events (ts_ms, kind, data) VALUES (0, 'old', '{}')",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
        }

        let store = Store::open(&path).unwrap();

        let version: i64 = store
            .lock()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        assert_eq!(store.event_stats().unwrap(), (1, 1));
        let tables: Vec<String> = store
            .lock()
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name IN ('devices', 'settings') ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, ["devices", "settings"]);
    }

    #[test]
    fn rebuilding_workspaces_keeps_their_agents_and_frees_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cos.db");
        {
            // As the release before the Chief of Staff left it.
            let mut conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            let tx = conn.transaction().unwrap();
            for migration in &MIGRATIONS[..8] {
                tx.execute_batch(migration).unwrap();
            }
            tx.pragma_update(None, "user_version", 8).unwrap();
            tx.execute_batch(
                "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/r', 'r', 'main', 0);
                 INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
                     VALUES (1, 'lisbon', 'Login fix', 'cos/lisbon', '/w/lisbon', 'head', 'main', 'abc', 0);
                 INSERT INTO agents (workspace_id, role, title, session_id, state, permission_mode, user_settings,
                     term_rows, term_cols, started_ms, last_activity_ms)
                     VALUES (1, 'user', 'a', 's', 'exited', 'default', 1, 40, 120, 0, 0);
                 INSERT INTO approvals (agent_id, workspace_id, tool_name, tool_input, summary, state, created_ms)
                     VALUES (1, 1, 'Bash', '{}', 'Bash ls', 'settled', 0);",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        let store = Store::open(&path).unwrap();

        let conn = store.lock();
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!((count("agents"), count("approvals")), (1, 1));
        let (title, kind): (Option<String>, String) = conn
            .query_row("SELECT title, kind FROM workspaces WHERE id = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(
            (title.as_deref(), kind.as_str()),
            (Some("Login fix"), "workspace")
        );
        conn.execute("UPDATE workspaces SET title = NULL WHERE id = 1", [])
            .unwrap();
        let foreign_keys: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(foreign_keys);
        // The cascade still points at the rebuilt table.
        conn.execute("DELETE FROM workspaces WHERE id = 1", []).unwrap();
        assert_eq!((count("agents"), count("approvals")), (0, 0));
    }

    #[test]
    fn removing_runs_keeps_your_agents_and_drops_the_run_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cos.db");
        {
            // As the last release with runs left it.
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            for migration in &MIGRATIONS[..12] {
                tx.execute_batch(migration).unwrap();
            }
            tx.pragma_update(None, "user_version", 12).unwrap();
            tx.execute_batch(
                "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/r', 'r', 'main', 0);
                 INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
                     VALUES (1, 'lisbon', NULL, 'r/lisbon', '/w/lisbon', 'head', 'main', 'abc', 0);
                 INSERT INTO runs (workspace_id, goal, state, preset, approval_cap, created_ms, updated_ms)
                     VALUES (1, 'Ship it', 'running', 'normal', 20, 0, 0);
                 INSERT INTO agents (workspace_id, run_id, role, title, session_id, state, permission_mode,
                     user_settings, term_rows, term_cols, started_ms, last_activity_ms, spawned_by)
                     VALUES (1, NULL, 'user', 'mine', 's1', 'idle', 'default', 1, 40, 120, 0, 0, 3),
                            (1, 1, 'worker', 'worker', 's2', 'exited', 'acceptEdits', 0, 40, 120, 0, 0, NULL);
                 INSERT INTO approvals (agent_id, workspace_id, tool_name, tool_input, summary, state, created_ms, route)
                     VALUES (1, 1, 'Bash', '{}', 'Bash ls', 'pending', 0, 'user'),
                            (2, 1, 'Bash', '{}', 'Bash rm', 'allowed', 0, 'chief');
                 INSERT INTO staff_inbox (project_id, kind, agent_id, run_id, text, created_ms)
                     VALUES (1, 'agent', 1, NULL, 'done', 0), (1, 'run', NULL, 1, 'run 1 is done', 0);",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        let store = Store::open(&path).unwrap();

        let conn = store.lock();
        let ids = |sql: &str| -> Vec<i64> {
            conn.prepare(sql)
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(ids("SELECT id FROM agents"), [1]);
        assert_eq!(ids("SELECT spawned_by FROM agents"), [3]);
        assert_eq!(ids("SELECT id FROM approvals"), [1]);
        assert_eq!(ids("SELECT id FROM staff_inbox"), [1]);
        let gone = ids("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
             AND name IN ('runs', 'tasks', 'inbox', 'wakeups', 'questions', 'approval_audit')");
        assert_eq!(gone, [0]);
        // The cascade points at the rebuilt table.
        conn.execute("DELETE FROM workspaces WHERE id = 1", []).unwrap();
        assert_eq!(ids("SELECT COUNT(*) FROM approvals"), [0]);
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cos.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();

        let error = Store::open(&path).err().expect("open should fail");

        assert!(matches!(error, StoreError::TooNew { found: 99, .. }), "{error}");
    }

    #[test]
    fn unreadable_rows_are_reported_as_corrupt() {
        let store = Store::open_in_memory().unwrap();
        store
            .lock()
            .execute(
                "INSERT INTO events (ts_ms, kind, data) VALUES (0, 'x', 'not json')",
                [],
            )
            .unwrap();

        let error = store.events_after(0, 10).expect_err("read should fail");

        assert!(matches!(error, StoreError::Corrupt { seq: 1, .. }), "{error}");
    }
}
