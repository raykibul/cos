//! jev's books, in shadow mode: which projects it may judge, every decision
//! it made with what you then did, and the numbers the apps show ("would
//! have auto-approved 41 of 45, disagreed on 4"). The daemon asks jev
//! (`cos-judge`); this module only stores. See `docs/judge.md`.
//!
//! Inputs are stored (it's your own database) but never put in an event:
//! `judge.decided` carries ids, the would-have and the latency.

use std::collections::BTreeMap;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::kind;
use crate::{EventLog, Store, StoreError};

/// Disagreements listed per decision point.
pub const MAX_DISAGREEMENTS: usize = 20;

/// Where in the daemon jev was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPoint {
    /// Sorting a permission request that goes to you.
    Permission,
    /// Whether an inbox item should wake its chief.
    Wake,
}

impl DecisionPoint {
    pub const ALL: [DecisionPoint; 2] = [DecisionPoint::Permission, DecisionPoint::Wake];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Wake => "wake",
        }
    }
}

/// One decision to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDecision {
    pub point: DecisionPoint,
    pub project_id: Option<i64>,
    /// What was judged: `approval:12`, `inbox:5`, `staff_inbox:9`.
    pub subject: String,
    pub approval_id: Option<i64>,
    pub summary: String,
    pub input_hash: String,
    /// The trimmed input jev was sent.
    pub input: Value,
    /// jev's typed answers; `None` when the call failed.
    pub answers: Option<Value>,
    /// What jev would have done.
    pub would: String,
    pub why: String,
    pub no_opinion: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub model: Option<String>,
}

/// A stored decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub point: DecisionPoint,
    pub project_id: Option<i64>,
    pub subject: String,
    pub approval_id: Option<i64>,
    pub summary: String,
    pub input_hash: String,
    pub input: Value,
    pub answers: Option<Value>,
    pub would: String,
    pub why: Option<String>,
    pub no_opinion: bool,
    /// What you did: `allowed`, `denied`, `terminal` (answered in the
    /// terminal, outcome unknown) or `expired`. `None` until then, and for
    /// wake decisions.
    pub actual: Option<String>,
    pub actual_ms: Option<i64>,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub model: Option<String>,
    pub created_ms: i64,
}

/// A project and whether jev may judge it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSwitch {
    pub project_id: i64,
    pub name: String,
    pub enabled: bool,
}

/// One decision point's numbers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PointStats {
    pub point: String,
    /// Calls made.
    pub decisions: u64,
    /// Calls that failed or timed out.
    pub errors: u64,
    /// Errors, timeouts and answers below the minimum confidence.
    pub no_opinion: u64,
    /// How many of each would-have: `approve`/`ask_you`, or
    /// `now`/`batch`/`none`/`no_opinion`.
    pub would: BTreeMap<String, u64>,
    /// Permission requests you allowed or denied through cos.
    pub answered: u64,
    /// Of `answered`, how many jev would have approved.
    pub would_approve: u64,
    /// jev would have approved and you allowed, or it would have asked and
    /// you denied (with an opinion).
    pub agreed: u64,
    /// jev would have approved but you denied.
    pub disagreed: u64,
    /// jev would have asked (with an opinion) but you allowed.
    pub missed: u64,
    /// The disagreements, newest first, at most [`MAX_DISAGREEMENTS`].
    pub disagreements: Vec<Example>,
}

/// A decision as the apps list it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Example {
    pub id: i64,
    pub point: String,
    pub project_id: Option<i64>,
    pub subject: String,
    pub summary: String,
    pub would: String,
    pub why: Option<String>,
    pub actual: Option<String>,
    pub answers: Option<Value>,
    pub created_ms: i64,
}

/// An item added to a Chief of Staff's inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffInboxItem {
    pub id: i64,
    pub project_id: i64,
    pub kind: String,
    pub text: String,
}

/// The judge tables.
#[derive(Clone)]
pub struct Judgments {
    store: Arc<Store>,
    log: EventLog,
}

impl Judgments {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    // ---- Per-project switch ----

    /// Whether jev may judge the project. Off unless switched on.
    pub fn enabled(&self, project_id: i64) -> Result<bool, StoreError> {
        Ok(self
            .store
            .lock()
            .query_row(
                "SELECT enabled FROM judge_projects WHERE project_id = ?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Switches jev on or off for a project. `None` if there's no such
    /// project. Logs `judge.changed`.
    pub fn set_enabled(&self, project_id: i64, enabled: bool) -> Result<Option<ProjectSwitch>, StoreError> {
        {
            let conn = self.store.lock();
            let exists: Option<i64> = conn
                .query_row("SELECT id FROM projects WHERE id = ?1", [project_id], |row| {
                    row.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Ok(None);
            }
            conn.execute(
                "INSERT INTO judge_projects (project_id, enabled, updated_ms) VALUES (?1, ?2, ?3)
                 ON CONFLICT (project_id) DO UPDATE SET enabled = excluded.enabled, updated_ms = excluded.updated_ms",
                params![project_id, enabled, now_ms()],
            )?;
        }
        self.log.append(
            kind::JUDGE_CHANGED,
            None,
            json!({ "project_id": project_id, "enabled": enabled }),
        )?;
        Ok(self
            .switches()?
            .into_iter()
            .find(|switch| switch.project_id == project_id))
    }

    /// Every project, with its switch.
    pub fn switches(&self) -> Result<Vec<ProjectSwitch>, StoreError> {
        let conn = self.store.lock();
        let mut query = conn.prepare_cached(
            "SELECT p.id, p.name, COALESCE(j.enabled, 0)
             FROM projects p LEFT JOIN judge_projects j ON j.project_id = p.id ORDER BY p.id",
        )?;
        let rows = query.query_map([], |row| {
            Ok(ProjectSwitch {
                project_id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ---- Decisions ----

    /// Stores a decision. Logs `judge.decided`, without the input.
    pub fn record(&self, new: NewDecision) -> Result<Decision, StoreError> {
        let id = {
            let conn = self.store.lock();
            conn.execute(
                "INSERT INTO judge_decisions (point, project_id, subject, approval_id, summary, input_hash,
                     input, answers, would, why, no_opinion, latency_ms, error, model, created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    new.point.as_str(),
                    new.project_id,
                    new.subject,
                    new.approval_id,
                    new.summary,
                    new.input_hash,
                    new.input.to_string(),
                    new.answers.as_ref().map(Value::to_string),
                    new.would,
                    new.why,
                    new.no_opinion,
                    new.latency_ms as i64,
                    new.error,
                    new.model,
                    now_ms(),
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.log.append(
            kind::JUDGE_DECIDED,
            None,
            json!({
                "decision_id": id,
                "point": new.point,
                "project_id": new.project_id,
                "subject": new.subject,
                "would": new.would,
                "latency_ms": new.latency_ms,
                "error": new.error.is_some(),
            }),
        )?;
        self.decision(id)?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    /// Records what you did with an approval jev judged, if it judged it and
    /// nothing is recorded yet. Returns whether a decision changed.
    pub fn record_actual(&self, approval_id: i64, actual: &str) -> Result<bool, StoreError> {
        let changed = self.store.lock().execute(
            "UPDATE judge_decisions SET actual = ?2, actual_ms = ?3
             WHERE approval_id = ?1 AND point = 'permission' AND actual IS NULL",
            params![approval_id, actual, now_ms()],
        )?;
        Ok(changed > 0)
    }

    pub fn decision(&self, id: i64) -> Result<Option<Decision>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {DECISION_COLUMNS} FROM judge_decisions WHERE id = ?1"
            ))?
            .query_row([id], read_decision)
            .optional()?)
    }

    /// The newest decisions, newest first, optionally of one point.
    pub fn recent(&self, point: Option<DecisionPoint>, limit: usize) -> Result<Vec<Decision>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {DECISION_COLUMNS} FROM judge_decisions
                 WHERE (?1 IS NULL OR point = ?1) ORDER BY id DESC LIMIT ?2"
            ))?
            .query_map(
                params![point.map(DecisionPoint::as_str), limit as i64],
                read_decision,
            )?
            .collect::<Result<_, _>>()?)
    }

    /// Each decision point's numbers.
    pub fn stats(&self) -> Result<Vec<PointStats>, StoreError> {
        DecisionPoint::ALL
            .into_iter()
            .map(|point| self.point_stats(point))
            .collect()
    }

    fn point_stats(&self, point: DecisionPoint) -> Result<PointStats, StoreError> {
        let conn = self.store.lock();
        let name = point.as_str();
        let counts = conn.query_row(
            "SELECT COUNT(*),
                 COALESCE(SUM(error IS NOT NULL), 0),
                 COALESCE(SUM(no_opinion), 0),
                 COALESCE(SUM(actual IN ('allowed', 'denied')), 0),
                 COALESCE(SUM(actual IN ('allowed', 'denied') AND would = 'approve'), 0),
                 COALESCE(SUM(no_opinion = 0 AND ((would = 'approve' AND actual = 'allowed')
                                               OR (would = 'ask_you' AND actual = 'denied'))), 0),
                 COALESCE(SUM(would = 'approve' AND actual = 'denied'), 0),
                 COALESCE(SUM(no_opinion = 0 AND would = 'ask_you' AND actual = 'allowed'), 0)
             FROM judge_decisions WHERE point = ?1",
            [name],
            |row| {
                Ok([
                    row.get::<_, i64>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ])
            },
        )?;
        let [
            decisions,
            errors,
            no_opinion,
            answered,
            would_approve,
            agreed,
            disagreed,
            missed,
        ] = counts.map(|count| count.max(0) as u64);
        let would = conn
            .prepare_cached("SELECT would, COUNT(*) FROM judge_decisions WHERE point = ?1 GROUP BY would")?
            .query_map([name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<Result<_, _>>()?;
        let disagreements = conn
            .prepare_cached(&format!(
                "SELECT {DECISION_COLUMNS} FROM judge_decisions
                 WHERE point = ?1 AND would = 'approve' AND actual = 'denied' ORDER BY id DESC LIMIT ?2"
            ))?
            .query_map(params![name, MAX_DISAGREEMENTS as i64], read_decision)?
            .map(|decision| decision.map(Example::from))
            .collect::<Result<_, _>>()?;
        Ok(PointStats {
            point: name.to_owned(),
            decisions,
            errors,
            no_opinion,
            would,
            answered,
            would_approve,
            agreed,
            disagreed,
            missed,
            disagreements,
        })
    }

    // ---- Inboxes, read for the wake decision ----

    /// The newest id of the Chiefs of Staff's inboxes (0 when empty): where
    /// following them starts.
    pub fn inbox_cursor(&self) -> Result<i64, StoreError> {
        Ok(self
            .store
            .lock()
            .query_row("SELECT COALESCE(MAX(id), 0) FROM staff_inbox", [], |row| {
                row.get(0)
            })?)
    }

    /// Chiefs of Staff's inbox items after `after`, oldest first.
    pub fn staff_inbox_after(&self, after: i64, limit: usize) -> Result<Vec<StaffInboxItem>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(
                "SELECT id, project_id, kind, text FROM staff_inbox WHERE id > ?1 ORDER BY id LIMIT ?2",
            )?
            .query_map(params![after, limit as i64], |row| {
                Ok(StaffInboxItem {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    kind: row.get(2)?,
                    text: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
}

impl From<Decision> for Example {
    fn from(decision: Decision) -> Self {
        Self {
            id: decision.id,
            point: decision.point.as_str().to_owned(),
            project_id: decision.project_id,
            subject: decision.subject,
            summary: decision.summary,
            would: decision.would,
            why: decision.why,
            actual: decision.actual,
            answers: decision.answers,
            created_ms: decision.created_ms,
        }
    }
}

const DECISION_COLUMNS: &str =
    "id, point, project_id, subject, approval_id, summary, input_hash, input, answers,
    would, why, no_opinion, actual, actual_ms, latency_ms, error, model, created_ms";

fn read_decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
    let point = match row.get::<_, String>(1)?.as_str() {
        "wake" => DecisionPoint::Wake,
        _ => DecisionPoint::Permission,
    };
    let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
    Ok(Decision {
        id: row.get(0)?,
        point,
        project_id: row.get(2)?,
        subject: row.get(3)?,
        approval_id: row.get(4)?,
        summary: row.get(5)?,
        input_hash: row.get(6)?,
        input: json(row.get(7)?).unwrap_or(Value::Null),
        answers: json(row.get(8)?),
        would: row.get(9)?,
        why: row.get(10)?,
        no_opinion: row.get(11)?,
        actual: row.get(12)?,
        actual_ms: row.get(13)?,
        latency_ms: row.get::<_, i64>(14)?.max(0) as u64,
        error: row.get(15)?,
        model: row.get(16)?,
        created_ms: row.get(17)?,
    })
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books() -> (Judgments, Arc<Store>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        store
            .lock()
            .execute_batch(
                "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/r', 'app', 'main', 0);
                 INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/s', 'site', 'main', 0);",
            )
            .unwrap();
        (Judgments::new(store.clone(), log), store)
    }

    fn permission(approval_id: i64, would: &str, no_opinion: bool) -> NewDecision {
        NewDecision {
            point: DecisionPoint::Permission,
            project_id: Some(1),
            subject: format!("approval:{approval_id}"),
            approval_id: Some(approval_id),
            summary: format!("Bash step {approval_id}"),
            input_hash: "h".into(),
            input: json!({ "request": { "tool": "Bash" } }),
            answers: Some(json!({ "serves_task": { "probability": 0.9 } })),
            would: would.into(),
            why: "because".into(),
            no_opinion,
            latency_ms: 120,
            error: None,
            model: Some("jev".into()),
        }
    }

    #[test]
    fn projects_are_off_until_switched_on() {
        let (books, _) = books();

        assert!(!books.enabled(1).unwrap());
        let on = books.set_enabled(1, true).unwrap().unwrap();
        assert_eq!(
            on,
            ProjectSwitch {
                project_id: 1,
                name: "app".into(),
                enabled: true
            }
        );
        assert!(books.enabled(1).unwrap());
        assert_eq!(books.set_enabled(99, true).unwrap(), None);
        let switches: Vec<bool> = books.switches().unwrap().iter().map(|s| s.enabled).collect();
        assert_eq!(switches, [true, false]);
        books.set_enabled(1, false).unwrap();
        assert!(!books.enabled(1).unwrap());
    }

    #[test]
    fn a_decision_is_logged_without_its_input() {
        let (books, store) = books();

        let decision = books.record(permission(7, "approve", false)).unwrap();

        assert_eq!(decision.input["request"]["tool"], "Bash");
        let event = store.events_after(0, 10).unwrap().pop().unwrap();
        assert_eq!(event.kind, kind::JUDGE_DECIDED);
        assert_eq!(event.data["decision_id"], decision.id);
        assert_eq!(event.data["would"], "approve");
        assert!(event.data.get("input").is_none());
        assert!(!event.data.to_string().contains("Bash"));
    }

    #[test]
    fn stats_compare_jev_with_you() {
        let (books, _) = books();
        // 1: approve, you allowed. 2: approve, you denied. 3: ask, you
        // allowed. 4: ask, you denied. 5: no opinion, you allowed.
        // 6: approve, answered in the terminal. 7: approve, not answered.
        for (id, would, no_opinion, actual) in [
            (1, "approve", false, Some("allowed")),
            (2, "approve", false, Some("denied")),
            (3, "ask_you", false, Some("allowed")),
            (4, "ask_you", false, Some("denied")),
            (5, "ask_you", true, Some("allowed")),
            (6, "approve", false, Some("terminal")),
            (7, "approve", false, None),
        ] {
            books.record(permission(id, would, no_opinion)).unwrap();
            if let Some(actual) = actual {
                assert!(books.record_actual(id, actual).unwrap());
            }
        }
        // Only the first answer counts.
        assert!(!books.record_actual(1, "denied").unwrap());

        let stats = books.stats().unwrap();

        let permission = &stats[0];
        assert_eq!(permission.point, "permission");
        assert_eq!(permission.decisions, 7);
        assert_eq!(permission.no_opinion, 1);
        assert_eq!(permission.answered, 5);
        assert_eq!(permission.would_approve, 2);
        assert_eq!(permission.agreed, 2);
        assert_eq!(permission.disagreed, 1);
        assert_eq!(permission.missed, 1);
        assert_eq!(permission.would["approve"], 4);
        assert_eq!(permission.disagreements.len(), 1);
        assert_eq!(permission.disagreements[0].summary, "Bash step 2");
        assert_eq!(permission.disagreements[0].actual.as_deref(), Some("denied"));
        assert_eq!(stats[1].point, "wake");
        assert_eq!(stats[1].decisions, 0);
    }

    #[test]
    fn inbox_items_come_with_their_project() {
        let (books, store) = books();
        store
            .lock()
            .execute_batch(
                "INSERT INTO staff_inbox (project_id, kind, text, created_ms) VALUES (1, 'agent', 'Agent 3 is done.', 0);",
            )
            .unwrap();

        let staff = books.staff_inbox_after(0, 10).unwrap();

        assert_eq!((staff[0].project_id, staff[0].text.as_str()), (1, "Agent 3 is done."));
        assert_eq!(books.inbox_cursor().unwrap(), 1);
        assert!(books.staff_inbox_after(1, 10).unwrap().is_empty());
    }
}
