//! Each agent's transcript: where Claude keeps it (every hook says, and it's
//! recorded when it changes) and when it grew, logged as `agent.transcript`
//! at most once a second. `UserPromptSubmit`, `PostToolUse` and `Stop` are
//! the signals, so the daemon never watches the file.
//!
//! Claude may write a line after the hook that announced it, so the file is
//! read [`SETTLE`] after the newest signal, never only after an earlier one,
//! and once more [`TRAIL`] after `Stop`: a turn's last reply has no later
//! hook to bring it in.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cos_core::StoreError;
use cos_harness::claude::hooks::Hook;
use serde_json::Value;

use super::{Inner, Live};

/// After a signal, wait this long for Claude to finish writing.
const SETTLE: Duration = Duration::from_millis(250);
/// After `Stop`, look once more this late, for a reply written after the hook.
const TRAIL: Duration = Duration::from_secs(2);
/// At most one `agent.transcript` per agent this often.
const MIN_GAP: Duration = Duration::from_secs(1);

/// What the daemon knows about one agent's transcript.
#[derive(Default)]
pub(super) struct Note {
    path: Option<PathBuf>,
    /// The file's size when `agent.transcript` was last logged.
    logged_len: u64,
    last_logged: Option<Instant>,
    /// A task is on its way to check the file.
    scheduled: bool,
    /// The file should be read at or after this: [`SETTLE`] after the newest signal.
    settle_at: Option<Instant>,
    /// And again at or after this: [`TRAIL`] after the last `Stop`.
    trail_at: Option<Instant>,
}

impl Note {
    /// A signal came at `now`. True when no task is on its way yet, so the
    /// caller starts one.
    fn signal(&mut self, now: Instant, turn_ended: bool) -> bool {
        self.settle_at = self.settle_at.max(Some(now + SETTLE));
        if turn_ended {
            self.trail_at = self.trail_at.max(Some(now + TRAIL));
        }
        !std::mem::replace(&mut self.scheduled, true)
    }

    /// When to check next: the earliest read still wanted, but not sooner
    /// than [`MIN_GAP`] after the last log. `None` when nothing is wanted.
    fn due(&self) -> Option<Instant> {
        let wanted = match (self.settle_at, self.trail_at) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some(match self.last_logged {
            Some(at) => wanted.max(at + MIN_GAP),
            None => wanted,
        })
    }

    /// The file was read as of `at`: the reads wanted by then are done.
    /// True when another one is still wanted.
    fn checked(&mut self, at: Instant) -> bool {
        self.settle_at = self.settle_at.filter(|due| *due > at);
        self.trail_at = self.trail_at.filter(|due| *due > at);
        let more = self.settle_at.is_some() || self.trail_at.is_some();
        self.scheduled = more;
        more
    }
}

impl Inner {
    /// Records the hook's `transcript_path`, and schedules a check of the
    /// file after the hooks that mean it grew.
    pub(super) fn note_transcript(
        self: &Arc<Self>,
        live: &mut Live,
        id: i64,
        hook: &Hook,
    ) -> Result<(), StoreError> {
        let note = live.transcripts.entry(id).or_default();
        let path = hook
            .input
            .get("transcript_path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        if let Some(path) = path.filter(|path| note.path.as_ref() != Some(path)) {
            self.agents.set_transcript_path(id, &path)?;
            note.path = Some(path);
        }
        let grew = matches!(hook.event.as_str(), "UserPromptSubmit" | "PostToolUse" | "Stop");
        if !grew || note.path.is_none() || !note.signal(Instant::now(), hook.event == "Stop") {
            return Ok(());
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(due) = weak
                    .upgrade()
                    .and_then(|inner| inner.live().transcripts.get(&id).and_then(Note::due))
                else {
                    return;
                };
                tokio::time::sleep_until(due.into()).await;
                let Some(inner) = weak.upgrade() else { return };
                let at = Instant::now();
                inner.clone().log_transcript(id).await;
                let more = inner
                    .live()
                    .transcripts
                    .get_mut(&id)
                    .is_some_and(|note| note.checked(at));
                if !more {
                    return;
                }
            }
        });
        Ok(())
    }

    /// Logs `agent.transcript` if the file grew since the last one.
    async fn log_transcript(self: Arc<Self>, id: i64) {
        let (path, logged_len) = {
            let live = self.live();
            let Some(note) = live.transcripts.get(&id) else {
                return;
            };
            let Some(path) = note.path.clone() else {
                return;
            };
            (path, note.logged_len)
        };
        let inner = self.clone();
        let read = tokio::task::spawn_blocking(move || -> std::io::Result<Option<(u64, usize)>> {
            let len = std::fs::metadata(&path)?.len();
            if len == logged_len {
                return Ok(None);
            }
            Ok(Some((len, inner.parsed.read(&path, None)?.next)))
        })
        .await;
        let (len, next) = match read {
            Ok(Ok(Some(grown))) => grown,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                tracing::debug!(%error, agent = id, "could not read a transcript");
                return;
            }
            Err(error) => {
                tracing::error!(%error, agent = id, "reading a transcript failed");
                return;
            }
        };
        if let Some(note) = self.live().transcripts.get_mut(&id) {
            note.logged_len = len;
            note.last_logged = Some(Instant::now());
        }
        if let Err(error) = self.agents.log_transcript(id, next) {
            tracing::error!(%error, agent = id, "could not log a transcript change");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            path: Some(PathBuf::from("/t.jsonl")),
            ..Note::default()
        }
    }

    #[test]
    fn a_signal_while_a_check_waits_moves_the_read_after_it() {
        let t0 = Instant::now();
        let mut note = note();
        assert!(note.signal(t0, false), "the first signal starts a task");
        assert_eq!(note.due(), Some(t0 + SETTLE));
        // `Stop` lands just before that check: the read waits for its settle too.
        let stop = t0 + Duration::from_millis(200);
        assert!(!note.signal(stop, true), "a task is already on its way");
        assert_eq!(note.due(), Some(stop + SETTLE));
        // Checking at the first due time leaves the later ones wanted.
        assert!(note.checked(t0 + SETTLE));
        assert_eq!(note.due(), Some(stop + SETTLE));
        assert!(note.checked(stop + SETTLE));
        // Then once more, late, for a last reply written after `Stop`.
        assert_eq!(note.due(), Some(stop + TRAIL));
        assert!(!note.checked(stop + TRAIL), "nothing more is wanted");
        assert_eq!(note.due(), None);
        assert!(
            note.signal(stop + TRAIL, false),
            "the next signal starts a task again"
        );
    }

    #[test]
    fn checks_stay_a_gap_apart_after_a_log() {
        let t0 = Instant::now();
        let mut note = note();
        note.last_logged = Some(t0);
        note.signal(t0, false);
        assert_eq!(note.due(), Some(t0 + MIN_GAP));
    }

    #[test]
    fn only_stop_asks_for_a_late_read() {
        let t0 = Instant::now();
        let mut note = note();
        note.signal(t0, false);
        assert!(!note.checked(t0 + SETTLE));
        assert_eq!(note.due(), None);
    }
}
