//! The numbered event log: everything the daemon does is appended here, and
//! clients follow it live or catch up from any sequence number.

use std::collections::VecDeque;
use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::{Store, StoreError};

/// Event kinds. See `docs/daemon.md` for each one's data.
pub mod kind {
    pub const DAEMON_STARTED: &str = "daemon.started";
    pub const DAEMON_STOPPED: &str = "daemon.stopped";
    pub const PROJECT_ADDED: &str = "project.added";
    pub const PROJECT_REMOVED: &str = "project.removed";
    pub const WORKSPACE_CREATED: &str = "workspace.created";
    pub const WORKSPACE_RENAMED: &str = "workspace.renamed";
    pub const WORKSPACE_REMOVED: &str = "workspace.removed";
    pub const DEVICE_REQUESTED: &str = "device.requested";
    pub const DEVICE_APPROVED: &str = "device.approved";
    pub const DEVICE_DENIED: &str = "device.denied";
    pub const DEVICE_REMOVED: &str = "device.removed";
    pub const TAILNET_CHANGED: &str = "tailnet.changed";
    pub const RUN_CREATED: &str = "run.created";
    pub const RUN_STATE_CHANGED: &str = "run.state_changed";
    pub const PLAN_PROPOSED: &str = "plan.proposed";
    pub const PLAN_APPROVED: &str = "plan.approved";
    pub const TASK_STATE_CHANGED: &str = "task.state_changed";
    pub const TASK_WORKTREE_ASSIGNED: &str = "task.worktree_assigned";
    // Agents and approvals; see `docs/agents-api.md`.
    pub const AGENT_STARTED: &str = "agent.started";
    pub const AGENT_STATE_CHANGED: &str = "agent.state_changed";
    pub const AGENT_EXITED: &str = "agent.exited";
    pub const AGENT_REMOVED: &str = "agent.removed";
    pub const APPROVAL_REQUESTED: &str = "approval.requested";
    pub const APPROVAL_DECIDED: &str = "approval.decided";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggedEvent {
    /// Position in the log, starting at 1. Never reused.
    pub seq: u64,
    pub ts: Timestamp,
    /// Dotted name such as `daemon.started`.
    pub kind: String,
    /// The run this event belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub data: Value,
}

/// Appends to the store and publishes each event to live subscribers.
/// Cheap to clone.
#[derive(Clone)]
pub struct EventLog {
    store: Arc<Store>,
    live: broadcast::Sender<LoggedEvent>,
    page_size: usize,
}

impl EventLog {
    pub fn new(store: Arc<Store>) -> Self {
        Self::with_limits(store, 1024, 500)
    }

    /// `capacity`: events buffered per live subscriber before it has to catch
    /// up from the database. `page_size`: events read per catch-up query.
    fn with_limits(store: Arc<Store>, capacity: usize, page_size: usize) -> Self {
        Self {
            store,
            live: broadcast::channel(capacity).0,
            page_size,
        }
    }

    pub fn append(&self, kind: &str, run_id: Option<&str>, data: Value) -> Result<LoggedEvent, StoreError> {
        self.store.append_event(kind, run_id, data, |event| {
            // No subscribers is fine.
            let _ = self.live.send(event.clone());
        })
    }

    /// Up to `limit` events after sequence number `after`, oldest first.
    pub fn after(&self, after: u64, limit: usize) -> Result<Vec<LoggedEvent>, StoreError> {
        self.store.events_after(after, limit)
    }

    /// The number of stored events and the last sequence number (0 if empty).
    pub fn stats(&self) -> Result<(u64, u64), StoreError> {
        self.store.event_stats()
    }

    /// Every event after `after`: first the stored ones, then new ones as
    /// they are appended, with no gaps or repeats.
    pub fn subscribe(&self, after: u64) -> Subscription {
        Subscription {
            // Subscribe before reading the backlog, so nothing appended in
            // between is missed. Anything seen twice is skipped by sequence.
            live: self.live.subscribe(),
            store: self.store.clone(),
            page_size: self.page_size,
            cursor: after,
            backlog: VecDeque::new(),
            caught_up: false,
        }
    }
}

pub struct Subscription {
    // Not an `EventLog`: holding its sender would keep the channel open forever.
    store: Arc<Store>,
    page_size: usize,
    live: broadcast::Receiver<LoggedEvent>,
    /// Sequence number of the last event handed out.
    cursor: u64,
    backlog: VecDeque<LoggedEvent>,
    /// False while there may be stored events newer than `cursor` that the
    /// live channel won't deliver.
    caught_up: bool,
}

impl Subscription {
    /// The next event, waiting for one if needed. `None` once the log is gone.
    /// Safe to use in `select!`: cancelling it loses nothing.
    pub async fn next(&mut self) -> Result<Option<LoggedEvent>, StoreError> {
        loop {
            if let Some(event) = self.backlog.pop_front() {
                self.cursor = event.seq;
                return Ok(Some(event));
            }
            if !self.caught_up {
                let page = self.store.events_after(self.cursor, self.page_size)?;
                self.caught_up = page.len() < self.page_size;
                self.backlog.extend(page);
                continue;
            }
            match self.live.recv().await {
                Ok(event) if event.seq <= self.cursor => {}
                Ok(event) if event.seq == self.cursor + 1 => {
                    self.cursor = event.seq;
                    return Ok(Some(event));
                }
                // We fell behind the channel, or there's a gap before this
                // event. Either way the database has everything in order.
                Ok(_) | Err(RecvError::Lagged(_)) => self.caught_up = false,
                Err(RecvError::Closed) => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    fn log_with_limits(capacity: usize, page_size: usize) -> EventLog {
        EventLog::with_limits(Arc::new(Store::open_in_memory().unwrap()), capacity, page_size)
    }

    fn append(log: &EventLog, n: u64) {
        log.append("test", None, json!({ "n": n })).unwrap();
    }

    async fn next_seq(subscription: &mut Subscription) -> u64 {
        tokio::time::timeout(Duration::from_secs(5), subscription.next())
            .await
            .expect("timed out waiting for an event")
            .unwrap()
            .expect("log closed")
            .seq
    }

    #[tokio::test]
    async fn backlog_then_live_without_gaps() {
        let log = log_with_limits(16, 500);
        append(&log, 1);
        append(&log, 2);
        let mut subscription = log.subscribe(0);

        assert_eq!(next_seq(&mut subscription).await, 1);
        append(&log, 3);
        assert_eq!(next_seq(&mut subscription).await, 2);
        assert_eq!(next_seq(&mut subscription).await, 3);
    }

    #[tokio::test]
    async fn starts_after_the_given_sequence_number() {
        let log = log_with_limits(16, 500);
        for n in 1..=3 {
            append(&log, n);
        }

        let mut subscription = log.subscribe(2);

        assert_eq!(next_seq(&mut subscription).await, 3);
    }

    #[tokio::test]
    async fn waits_for_new_events() {
        let log = log_with_limits(16, 500);
        let mut subscription = log.subscribe(0);

        let waiting = tokio::spawn(async move { next_seq(&mut subscription).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        append(&log, 1);

        assert_eq!(waiting.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn backlog_bigger_than_a_page_is_read_in_pages() {
        let log = log_with_limits(16, 3);
        for n in 1..=10 {
            append(&log, n);
        }
        let mut subscription = log.subscribe(0);

        for expected in 1..=10 {
            assert_eq!(next_seq(&mut subscription).await, expected);
        }
    }

    #[tokio::test]
    async fn slow_subscriber_catches_up_from_the_database() {
        let log = log_with_limits(2, 500);
        let mut subscription = log.subscribe(0);
        // Get it onto the live channel, then overflow the channel.
        append(&log, 1);
        assert_eq!(next_seq(&mut subscription).await, 1);
        for n in 2..=20 {
            append(&log, n);
        }

        for expected in 2..=20 {
            assert_eq!(next_seq(&mut subscription).await, expected);
        }
    }

    #[tokio::test]
    async fn ends_when_the_log_is_dropped() {
        let log = log_with_limits(16, 500);
        let mut subscription = log.subscribe(0);
        drop(log);

        assert_eq!(subscription.next().await.unwrap(), None);
    }

    #[test]
    fn event_json_shape() {
        let event = LoggedEvent {
            seq: 7,
            ts: Timestamp::from_millisecond(1_792_600_000_000).unwrap(),
            kind: kind::DAEMON_STARTED.into(),
            run_id: None,
            data: json!({ "pid": 1 }),
        };

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(
            value,
            json!({ "seq": 7, "ts": "2026-10-21T16:26:40Z", "kind": "daemon.started", "data": { "pid": 1 } })
        );
        assert_eq!(serde_json::from_value::<LoggedEvent>(value).unwrap(), event);
    }
}
