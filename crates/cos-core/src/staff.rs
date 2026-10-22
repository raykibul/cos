//! The Chief of Staff's books: one row per project (its agent, model and
//! brief) and its inbox. The daemon decides when to wake the chief and types
//! the wakeup; this module keeps what waits and what was delivered. Batching
//! and the caps are the run chief's ([`crate::chief`]). The contract is
//! `docs/chief-of-staff.md`.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::chief::{MAX_ITEM_BYTES, cut};
use crate::events::kind;
use crate::{EventLog, RunError, Store, StoreError};

/// The most bytes a brief holds.
pub const MAX_BRIEF_BYTES: usize = 4 * 1024;

/// A project's Chief of Staff as stored. A project without a row has the
/// defaults: no agent yet, no model, an empty brief.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaffRecord {
    pub project_id: i64,
    /// Its agent, once it first started.
    pub agent_id: Option<i64>,
    /// The model set for this project; `None` means the daemon's default.
    pub model: Option<String>,
    /// What it keeps for its next start, with `brief_update`.
    pub brief: String,
    /// Coordination is paused: news waits, your messages still go in.
    #[serde(default)]
    pub paused: bool,
    /// You stopped it, so a daemon restart doesn't bring it back.
    #[serde(default)]
    pub stopped: bool,
}

/// What an inbox item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaffItemKind {
    /// Something you typed. Delivered on its own, never batched.
    Message,
    /// News from an agent it started: a turn ended, it exited, or it waits
    /// for you.
    Agent,
    /// A run it started changed state.
    Run,
}

impl StaffItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Agent => "agent",
            Self::Run => "run",
        }
    }
}

impl FromStr for StaffItemKind {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [Self::Message, Self::Agent, Self::Run]
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| format!("unknown staff inbox kind `{text}`"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaffItem {
    pub id: i64,
    pub project_id: i64,
    pub kind: StaffItemKind,
    /// The agent it is about, if any.
    pub agent_id: Option<i64>,
    /// The run it is about, if any.
    pub run_id: Option<i64>,
    /// What the chief reads.
    pub text: String,
    pub created_ms: i64,
    /// `None` while it waits.
    pub delivered_ms: Option<i64>,
}

/// The staff, staff_inbox and staff_wakeups tables. Inbox changes log
/// `staff.inbox`.
#[derive(Clone)]
pub struct StaffBooks {
    store: Arc<Store>,
    log: EventLog,
}

impl StaffBooks {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// The project's record, or the defaults if it has none yet.
    pub fn record(&self, project_id: i64) -> Result<StaffRecord, StoreError> {
        let record = self
            .store
            .lock()
            .query_row(
                &format!("SELECT {RECORD_COLUMNS} FROM staff WHERE project_id = ?1"),
                [project_id],
                read_record,
            )
            .optional()?;
        Ok(record.unwrap_or(StaffRecord {
            project_id,
            ..StaffRecord::default()
        }))
    }

    /// Every project's record that has a chief, by project id.
    pub fn records(&self) -> Result<Vec<StaffRecord>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {RECORD_COLUMNS} FROM staff WHERE agent_id IS NOT NULL ORDER BY project_id"
            ))?
            .query_map([], read_record)?
            .collect::<Result<_, _>>()?)
    }

    /// Pauses or resumes its coordination.
    pub fn set_paused(&self, project_id: i64, paused: bool) -> Result<(), StoreError> {
        self.update(project_id, "paused", &paused)
    }

    /// Records whether you stopped it (true) or it was started (false).
    pub fn set_stopped(&self, project_id: i64, stopped: bool) -> Result<(), StoreError> {
        self.update(project_id, "stopped", &stopped)
    }

    pub fn set_agent(&self, project_id: i64, agent_id: i64) -> Result<(), StoreError> {
        self.update(project_id, "agent_id", &agent_id)
    }

    /// `None` goes back to the daemon's default.
    pub fn set_model(&self, project_id: i64, model: Option<&str>) -> Result<(), StoreError> {
        self.update(project_id, "model", &model)
    }

    /// Saves the brief, trimmed. At most [`MAX_BRIEF_BYTES`].
    pub fn set_brief(&self, project_id: i64, brief: &str) -> Result<String, RunError> {
        let brief = brief.trim();
        if brief.len() > MAX_BRIEF_BYTES {
            return Err(RunError::Invalid(format!(
                "the brief is {} bytes; keep it under {MAX_BRIEF_BYTES} and move detail elsewhere",
                brief.len()
            )));
        }
        self.update(project_id, "brief", &brief)?;
        Ok(brief.to_owned())
    }

    /// The project whose Chief of Staff is `agent_id`.
    pub fn project_of(&self, agent_id: i64) -> Result<Option<i64>, StoreError> {
        Ok(self
            .store
            .lock()
            .query_row(
                "SELECT project_id FROM staff WHERE agent_id = ?1",
                [agent_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn update(&self, project_id: i64, column: &str, value: &dyn rusqlite::ToSql) -> Result<(), StoreError> {
        let conn = self.store.lock();
        conn.execute(
            "INSERT INTO staff (project_id, updated_ms) VALUES (?1, ?2) ON CONFLICT (project_id) DO NOTHING",
            params![project_id, now_ms()],
        )?;
        conn.execute(
            &format!("UPDATE staff SET {column} = ?2, updated_ms = ?3 WHERE project_id = ?1"),
            params![project_id, value, now_ms()],
        )?;
        Ok(())
    }

    // ---- Inbox ----

    /// Adds an item. News is cut to [`MAX_ITEM_BYTES`]; your messages are
    /// kept whole. Logs `staff.inbox`.
    pub fn add(
        &self,
        project_id: i64,
        kind: StaffItemKind,
        agent_id: Option<i64>,
        run_id: Option<i64>,
        text: &str,
    ) -> Result<StaffItem, StoreError> {
        let text = match kind {
            StaffItemKind::Message => text.trim().to_owned(),
            _ => cut(text.trim(), MAX_ITEM_BYTES),
        };
        let item = {
            let conn = self.store.lock();
            conn.execute(
                "INSERT INTO staff_inbox (project_id, kind, agent_id, run_id, text, created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![project_id, kind.as_str(), agent_id, run_id, text, now_ms()],
            )?;
            let id = conn.last_insert_rowid();
            conn.prepare_cached(&format!("SELECT {ITEM_COLUMNS} FROM staff_inbox WHERE id = ?1"))?
                .query_row([id], read_item)?
        };
        self.log_count(project_id)?;
        Ok(item)
    }

    /// The project's undelivered items, oldest first.
    pub fn waiting(&self, project_id: i64) -> Result<Vec<StaffItem>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {ITEM_COLUMNS} FROM staff_inbox
                 WHERE project_id = ?1 AND delivered_ms IS NULL ORDER BY id"
            ))?
            .query_map([project_id], read_item)?
            .collect::<Result<_, _>>()?)
    }

    /// News from agents and runs, newest first, delivered or not: what the
    /// apps show. Your own messages are left out.
    pub fn recent(&self, project_id: i64, limit: usize) -> Result<Vec<StaffItem>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {ITEM_COLUMNS} FROM staff_inbox
                 WHERE project_id = ?1 AND kind != 'message' ORDER BY id DESC LIMIT ?2"
            ))?
            .query_map(params![project_id, limit as i64], read_item)?
            .collect::<Result<_, _>>()?)
    }

    /// Projects with undelivered items.
    pub fn projects_waiting(&self) -> Result<Vec<i64>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(
                "SELECT DISTINCT project_id FROM staff_inbox WHERE delivered_ms IS NULL ORDER BY project_id",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// How many wakeups the project's chief got since `since_ms`.
    pub fn wakeups_since(&self, project_id: i64, since_ms: i64) -> Result<usize, StoreError> {
        let count: i64 = self.store.lock().query_row(
            "SELECT COUNT(*) FROM staff_wakeups WHERE project_id = ?1 AND created_ms > ?2",
            params![project_id, since_ms],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Records a wakeup that delivered `items` to `agent_id`. Logs
    /// `staff.inbox`.
    pub fn deliver(&self, project_id: i64, agent_id: i64, items: &[StaffItem]) -> Result<i64, StoreError> {
        let id = {
            let mut conn = self.store.lock();
            let tx = conn.transaction()?;
            let now = now_ms();
            let bytes: usize = items.iter().map(|item| item.text.len()).sum();
            tx.execute(
                "INSERT INTO staff_wakeups (project_id, agent_id, items, bytes, created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![project_id, agent_id, items.len() as i64, bytes as i64, now],
            )?;
            let id = tx.last_insert_rowid();
            for item in items {
                tx.execute(
                    "UPDATE staff_inbox SET wakeup_id = ?2, delivered_ms = ?3 WHERE id = ?1 AND delivered_ms IS NULL",
                    params![item.id, id, now],
                )?;
            }
            tx.commit()?;
            id
        };
        self.log_count(project_id)?;
        Ok(id)
    }

    /// Marks one of your messages as typed in. Logs `staff.inbox`.
    pub fn delivered(&self, item: &StaffItem) -> Result<(), StoreError> {
        self.store.lock().execute(
            "UPDATE staff_inbox SET delivered_ms = ?2 WHERE id = ?1 AND delivered_ms IS NULL",
            params![item.id, now_ms()],
        )?;
        self.log_count(item.project_id)
    }

    /// How many items of news (not your messages) wait for the project's
    /// chief.
    pub fn news_waiting(&self, project_id: i64) -> Result<usize, StoreError> {
        let count: i64 = self.store.lock().query_row(
            "SELECT COUNT(*) FROM staff_inbox
             WHERE project_id = ?1 AND delivered_ms IS NULL AND kind != 'message'",
            [project_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// How many items wait for the project's chief.
    pub fn waiting_count(&self, project_id: i64) -> Result<usize, StoreError> {
        let count: i64 = self.store.lock().query_row(
            "SELECT COUNT(*) FROM staff_inbox WHERE project_id = ?1 AND delivered_ms IS NULL",
            [project_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    fn log_count(&self, project_id: i64) -> Result<(), StoreError> {
        let count = self.waiting_count(project_id)?;
        self.log.append(
            kind::STAFF_INBOX,
            None,
            json!({ "project_id": project_id, "count": count }),
        )?;
        Ok(())
    }
}

const RECORD_COLUMNS: &str = "project_id, agent_id, model, brief, paused, stopped";

fn read_record(row: &Row<'_>) -> rusqlite::Result<StaffRecord> {
    Ok(StaffRecord {
        project_id: row.get(0)?,
        agent_id: row.get(1)?,
        model: row.get(2)?,
        brief: row.get(3)?,
        paused: row.get(4)?,
        stopped: row.get(5)?,
    })
}

const ITEM_COLUMNS: &str = "id, project_id, kind, agent_id, run_id, text, created_ms, delivered_ms";

fn read_item(row: &Row<'_>) -> rusqlite::Result<StaffItem> {
    Ok(StaffItem {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get_ref(2)?.as_str()?.parse().map_err(|error: String| {
            rusqlite::Error::FromSqlConversionFailure(2, Type::Text, error.into())
        })?,
        agent_id: row.get(3)?,
        run_id: row.get(4)?,
        text: row.get(5)?,
        created_ms: row.get(6)?,
        delivered_ms: row.get(7)?,
    })
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chief::next_batch_by;

    fn books() -> (StaffBooks, EventLog, i64) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .lock()
            .execute(
                "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/r', 'r', 'main', 0)",
                [],
            )
            .unwrap();
        let log = EventLog::new(store.clone());
        (StaffBooks::new(store, log.clone()), log, 1)
    }

    #[test]
    fn a_project_starts_with_the_defaults_and_keeps_what_is_set() {
        let (books, _, project) = books();

        assert_eq!(
            books.record(project).unwrap(),
            StaffRecord {
                project_id: project,
                ..StaffRecord::default()
            }
        );
        books.set_agent(project, 7).unwrap();
        books.set_model(project, Some("haiku")).unwrap();
        assert_eq!(books.set_brief(project, "  ship v2 \n").unwrap(), "ship v2");

        let record = books.record(project).unwrap();
        assert_eq!(record.agent_id, Some(7));
        assert_eq!(record.model.as_deref(), Some("haiku"));
        assert_eq!(record.brief, "ship v2");
        assert_eq!(books.project_of(7).unwrap(), Some(project));
        assert_eq!(books.project_of(8).unwrap(), None);
        books.set_model(project, None).unwrap();
        assert_eq!(books.record(project).unwrap().model, None);

        assert!(!record.paused && !record.stopped);
        books.set_paused(project, true).unwrap();
        books.set_stopped(project, true).unwrap();
        let record = books.record(project).unwrap();
        assert!(record.paused && record.stopped);
        assert_eq!(books.records().unwrap(), [record]);
    }

    #[test]
    fn news_waiting_leaves_out_your_messages() {
        let (books, _, project) = books();
        assert!(books.records().unwrap().is_empty(), "no chief yet");
        books
            .add(project, StaffItemKind::Message, None, None, "hi")
            .unwrap();
        books
            .add(project, StaffItemKind::Agent, Some(3), None, "done")
            .unwrap();
        assert_eq!(books.news_waiting(project).unwrap(), 1);
        assert_eq!(books.waiting_count(project).unwrap(), 2);
    }

    #[test]
    fn a_brief_over_4_kib_is_refused() {
        let (books, _, project) = books();

        let error = books
            .set_brief(project, &"x".repeat(MAX_BRIEF_BYTES + 1))
            .unwrap_err();

        assert!(matches!(error, RunError::Invalid(_)), "{error}");
        assert!(books.set_brief(project, &"x".repeat(MAX_BRIEF_BYTES)).is_ok());
    }

    #[test]
    fn items_wait_until_delivered_and_every_change_is_logged() {
        let (books, log, project) = books();

        let message = books
            .add(project, StaffItemKind::Message, None, None, &"long ".repeat(2000))
            .unwrap();
        let news = books
            .add(
                project,
                StaffItemKind::Agent,
                Some(3),
                None,
                &"é".repeat(MAX_ITEM_BYTES),
            )
            .unwrap();
        books
            .add(project, StaffItemKind::Run, None, Some(2), "run 2 is done")
            .unwrap();

        assert_eq!(
            message.text.len(),
            "long ".repeat(2000).trim().len(),
            "messages are kept whole"
        );
        assert!(news.text.len() <= MAX_ITEM_BYTES && news.text.ends_with('…'));
        assert_eq!(books.projects_waiting().unwrap(), [project]);
        assert_eq!(books.waiting_count(project).unwrap(), 3);

        books.delivered(&message).unwrap();
        let waiting = books.waiting(project).unwrap();
        assert_eq!(waiting.len(), 2);
        books.deliver(project, 9, &waiting[..1]).unwrap();
        assert_eq!(books.waiting(project).unwrap().len(), 1);
        assert_eq!(books.wakeups_since(project, 0).unwrap(), 1);

        let recent = books.recent(project, 10).unwrap();
        assert_eq!(
            recent.iter().map(|item| item.kind).collect::<Vec<_>>(),
            [StaffItemKind::Run, StaffItemKind::Agent],
            "newest first, without your messages"
        );
        assert!(recent[1].delivered_ms.is_some());

        let counts: Vec<u64> = log
            .after(0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == kind::STAFF_INBOX)
            .map(|event| event.data["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, [1, 2, 3, 2, 1]);
    }

    #[test]
    fn batches_follow_the_run_chiefs_caps() {
        let sizes = [10, 10, 30 * 1024];
        assert_eq!(next_batch_by(&sizes, |size| *size).len(), 2);
        assert_eq!(next_batch_by(&sizes[2..], |size| *size).len(), 1);
    }
}
