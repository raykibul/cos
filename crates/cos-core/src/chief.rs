//! The chief's books: its inbox (what it should hear about, delivered in
//! batched wakeups) and the questions it asks you. The daemon decides when to
//! wake the chief and types the wakeup; this module keeps what's waiting and
//! what was delivered. The contract is `docs/orchestration.md`.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::events::kind;
use crate::{EventLog, RunError, Store, StoreError};

/// The most inbox items one wakeup holds.
pub const MAX_WAKEUP_ITEMS: usize = 20;
/// The most bytes of inbox text one wakeup holds (the brief comes on top).
pub const MAX_WAKEUP_BYTES: usize = 24 * 1024;
/// The most wakeups a chief gets in an hour; past that, items wait.
pub const MAX_WAKEUPS_PER_HOUR: usize = 30;
/// One item's text is cut to this many bytes, so any item fits in a wakeup.
pub const MAX_ITEM_BYTES: usize = 4 * 1024;
const MAX_QUESTION_CHARS: usize = 4_000;
const MAX_CHOICES: usize = 10;
const MAX_CHOICE_CHARS: usize = 200;
const MAX_ANSWER_CHARS: usize = 10_000;

/// What an inbox item is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    /// A task merged, was blocked or failed.
    Task,
    /// An approval was routed to the chief.
    Approval,
    /// You answered one of the chief's questions.
    Answer,
    /// You sent the chief a message.
    Message,
    /// The run is waiting for the report.
    Reporting,
}

impl InboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Approval => "approval",
            Self::Answer => "answer",
            Self::Message => "message",
            Self::Reporting => "reporting",
        }
    }
}

impl FromStr for InboxKind {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [
            Self::Task,
            Self::Approval,
            Self::Answer,
            Self::Message,
            Self::Reporting,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == text)
        .ok_or_else(|| format!("unknown inbox kind `{text}`"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: i64,
    pub run_id: i64,
    pub kind: InboxKind,
    /// What the chief reads, already worded for it.
    pub text: String,
    pub created_ms: i64,
    /// The wakeup that delivered it; `None` while it waits.
    pub wakeup_id: Option<i64>,
    pub delivered_ms: Option<i64>,
}

/// One message typed into a chief.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wakeup {
    pub id: i64,
    pub run_id: i64,
    pub agent_id: i64,
    pub items: usize,
    pub bytes: usize,
    pub created_ms: i64,
}

/// The inbox and wakeups tables.
#[derive(Clone)]
pub struct Inbox {
    store: Arc<Store>,
}

impl Inbox {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Adds an item for the run's chief. Text over [`MAX_ITEM_BYTES`] is cut.
    pub fn add(&self, run_id: i64, kind: InboxKind, text: &str) -> Result<InboxItem, StoreError> {
        let text = cut(text.trim(), MAX_ITEM_BYTES);
        let conn = self.store.lock();
        conn.execute(
            "INSERT INTO inbox (run_id, kind, text, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, kind.as_str(), text, now_ms()],
        )?;
        let id = conn.last_insert_rowid();
        Ok(conn
            .prepare_cached(&format!("SELECT {INBOX_COLUMNS} FROM inbox WHERE id = ?1"))?
            .query_row([id], read_item)?)
    }

    /// The run's undelivered items, oldest first.
    pub fn waiting(&self, run_id: i64) -> Result<Vec<InboxItem>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {INBOX_COLUMNS} FROM inbox WHERE run_id = ?1 AND wakeup_id IS NULL ORDER BY id"
            ))?
            .query_map([run_id], read_item)?
            .collect::<Result<_, _>>()?)
    }

    /// Every item of the run, oldest first.
    pub fn items(&self, run_id: i64) -> Result<Vec<InboxItem>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {INBOX_COLUMNS} FROM inbox WHERE run_id = ?1 ORDER BY id"
            ))?
            .query_map([run_id], read_item)?
            .collect::<Result<_, _>>()?)
    }

    /// Runs with undelivered items.
    pub fn runs_waiting(&self) -> Result<Vec<i64>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached("SELECT DISTINCT run_id FROM inbox WHERE wakeup_id IS NULL ORDER BY run_id")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// How many wakeups the run's chief got since `since_ms`.
    pub fn wakeups_since(&self, run_id: i64, since_ms: i64) -> Result<usize, StoreError> {
        let count: i64 = self.store.lock().query_row(
            "SELECT COUNT(*) FROM wakeups WHERE run_id = ?1 AND created_ms > ?2",
            params![run_id, since_ms],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// The oldest wakeup of the run's chief since `since_ms`, if any: when
    /// the hourly cap frees up again.
    pub fn oldest_wakeup_since(&self, run_id: i64, since_ms: i64) -> Result<Option<i64>, StoreError> {
        Ok(self.store.lock().query_row(
            "SELECT MIN(created_ms) FROM wakeups WHERE run_id = ?1 AND created_ms > ?2",
            params![run_id, since_ms],
            |row| row.get(0),
        )?)
    }

    /// Records a wakeup that delivered `items` to `agent_id`. Items already
    /// delivered are left alone.
    pub fn deliver(&self, run_id: i64, agent_id: i64, items: &[InboxItem]) -> Result<Wakeup, StoreError> {
        let mut conn = self.store.lock();
        let tx = conn.transaction()?;
        let now = now_ms();
        let bytes: usize = items.iter().map(|item| item.text.len()).sum();
        tx.execute(
            "INSERT INTO wakeups (run_id, agent_id, items, bytes, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, agent_id, items.len() as i64, bytes as i64, now],
        )?;
        let id = tx.last_insert_rowid();
        for item in items {
            tx.execute(
                "UPDATE inbox SET wakeup_id = ?2, delivered_ms = ?3 WHERE id = ?1 AND wakeup_id IS NULL",
                params![item.id, id, now],
            )?;
        }
        tx.commit()?;
        Ok(Wakeup {
            id,
            run_id,
            agent_id,
            items: items.len(),
            bytes,
            created_ms: now,
        })
    }
}

/// The items the next wakeup holds: oldest first, at most
/// [`MAX_WAKEUP_ITEMS`] and [`MAX_WAKEUP_BYTES`] of text, and always at least
/// one if any wait.
pub fn next_batch(waiting: &[InboxItem]) -> &[InboxItem] {
    next_batch_by(waiting, |item| item.text.len())
}

/// [`next_batch`] for any kind of item, `bytes` giving each one's size.
/// The Chief of Staff's inbox uses it too.
pub fn next_batch_by<T>(waiting: &[T], bytes: impl Fn(&T) -> usize) -> &[T] {
    let mut total = 0;
    let mut count = 0;
    for item in waiting.iter().take(MAX_WAKEUP_ITEMS) {
        total += bytes(item);
        if count > 0 && total > MAX_WAKEUP_BYTES {
            break;
        }
        count += 1;
    }
    &waiting[..count]
}

// ---- Questions ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Open,
    Answered,
}

impl QuestionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Answered => "answered",
        }
    }
}

impl FromStr for QuestionState {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "open" => Ok(Self::Open),
            "answered" => Ok(Self::Answered),
            _ => Err(format!("unknown question state `{text}`; use open or answered")),
        }
    }
}

/// A question the chief asked you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: i64,
    pub run_id: i64,
    /// The chief that asked.
    pub agent_id: Option<i64>,
    pub question: String,
    /// Suggested answers; any answer is accepted.
    pub choices: Vec<String>,
    pub state: QuestionState,
    pub answer: Option<String>,
    pub created_ms: i64,
    pub answered_ms: Option<i64>,
}

/// The questions table, with every change logged.
#[derive(Clone)]
pub struct Questions {
    store: Arc<Store>,
    log: EventLog,
}

impl Questions {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// Records an open question. Logs `question.asked`.
    pub fn ask(
        &self,
        run_id: i64,
        agent_id: Option<i64>,
        question: &str,
        choices: &[String],
    ) -> Result<Question, RunError> {
        let question = question.trim();
        if question.is_empty() {
            return Err(RunError::Invalid("the question is empty".into()));
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return Err(RunError::Invalid(format!(
                "a question can be at most {MAX_QUESTION_CHARS} characters"
            )));
        }
        let choices: Vec<String> = choices
            .iter()
            .map(|choice| choice.trim().to_owned())
            .filter(|choice| !choice.is_empty())
            .collect();
        if choices.len() > MAX_CHOICES {
            return Err(RunError::Invalid(format!("give at most {MAX_CHOICES} choices")));
        }
        if choices
            .iter()
            .any(|choice| choice.chars().count() > MAX_CHOICE_CHARS)
        {
            return Err(RunError::Invalid(format!(
                "a choice can be at most {MAX_CHOICE_CHARS} characters"
            )));
        }
        let id = {
            let conn = self.store.lock();
            conn.execute(
                "INSERT INTO questions (run_id, agent_id, question, choices, state, created_ms)
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5)",
                params![
                    run_id,
                    agent_id,
                    question,
                    serde_json::to_string(&choices).expect("strings serialize"),
                    now_ms()
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.log.append(
            kind::QUESTION_ASKED,
            Some(&run_id.to_string()),
            json!({ "question_id": id, "run_id": run_id }),
        )?;
        self.get(id)
    }

    /// Records your answer. 409 if it was answered already. Logs
    /// `question.answered`.
    pub fn answer(&self, id: i64, answer: &str) -> Result<Question, RunError> {
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(RunError::Invalid("the answer is empty".into()));
        }
        if answer.chars().count() > MAX_ANSWER_CHARS {
            return Err(RunError::Invalid(format!(
                "an answer can be at most {MAX_ANSWER_CHARS} characters"
            )));
        }
        let question = self.get(id)?;
        let changed = self.store.lock().execute(
            "UPDATE questions SET state = 'answered', answer = ?2, answered_ms = ?3
             WHERE id = ?1 AND state = 'open'",
            params![id, answer, now_ms()],
        )?;
        if changed == 0 {
            return Err(RunError::Conflict(format!("question {id} was answered already")));
        }
        self.log.append(
            kind::QUESTION_ANSWERED,
            Some(&question.run_id.to_string()),
            json!({ "question_id": id, "run_id": question.run_id }),
        )?;
        self.get(id)
    }

    pub fn get(&self, id: i64) -> Result<Question, RunError> {
        question(&self.store.lock(), id)?
            .ok_or_else(|| RunError::NotFound(format!("there is no question {id}")))
    }

    /// Questions, oldest first, optionally only those in one state or run.
    pub fn list(
        &self,
        state: Option<QuestionState>,
        run_id: Option<i64>,
    ) -> Result<Vec<Question>, StoreError> {
        let conn = self.store.lock();
        Ok(conn
            .prepare_cached(&format!(
                "SELECT {QUESTION_COLUMNS} FROM questions
                 WHERE (?1 IS NULL OR state = ?1) AND (?2 IS NULL OR run_id = ?2) ORDER BY id"
            ))?
            .query_map(params![state.map(QuestionState::as_str), run_id], read_question)?
            .collect::<Result<_, _>>()?)
    }
}

// ---- Storage ----

const INBOX_COLUMNS: &str = "id, run_id, kind, text, created_ms, wakeup_id, delivered_ms";
const QUESTION_COLUMNS: &str =
    "id, run_id, agent_id, question, choices, state, answer, created_ms, answered_ms";

fn read_item(row: &Row<'_>) -> rusqlite::Result<InboxItem> {
    Ok(InboxItem {
        id: row.get(0)?,
        run_id: row.get(1)?,
        kind: parse(row, 2)?,
        text: row.get(3)?,
        created_ms: row.get(4)?,
        wakeup_id: row.get(5)?,
        delivered_ms: row.get(6)?,
    })
}

fn question(conn: &Connection, id: i64) -> rusqlite::Result<Option<Question>> {
    conn.prepare_cached(&format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE id = ?1"))?
        .query_row([id], read_question)
        .optional()
}

fn read_question(row: &Row<'_>) -> rusqlite::Result<Question> {
    let choices: String = row.get(4)?;
    Ok(Question {
        id: row.get(0)?,
        run_id: row.get(1)?,
        agent_id: row.get(2)?,
        question: row.get(3)?,
        choices: serde_json::from_str(&choices).map_err(|error| bad_value(4, error.to_string()))?,
        state: parse(row, 5)?,
        answer: row.get(6)?,
        created_ms: row.get(7)?,
        answered_ms: row.get(8)?,
    })
}

fn parse<T: FromStr<Err = String>>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    row.get_ref(column)?
        .as_str()?
        .parse()
        .map_err(|error| bad_value(column, error))
}

fn bad_value(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}

/// `text` cut to at most `max` bytes on a character boundary, with `…` added
/// if it was cut.
pub fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests;
