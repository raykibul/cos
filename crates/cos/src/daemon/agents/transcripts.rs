//! Each agent's transcript: where Claude keeps it (every hook says, and it's
//! recorded when it changes) and when it grew, logged as `agent.transcript`
//! at most once a second. `UserPromptSubmit`, `PostToolUse` and `Stop` are
//! the signals, so the daemon never watches the file.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cos_core::StoreError;
use cos_harness::claude::hooks::Hook;
use serde_json::Value;

use super::{Inner, Live};
use crate::daemon::transcript;

/// After a signal, wait this long for Claude to finish writing.
const SETTLE: Duration = Duration::from_millis(250);
/// At most one `agent.transcript` per agent this often.
const MIN_GAP: Duration = Duration::from_secs(1);

/// What the daemon knows about one agent's transcript.
#[derive(Default)]
pub(super) struct Note {
    path: Option<PathBuf>,
    /// The file's size when `agent.transcript` was last logged.
    logged_len: u64,
    last_logged: Option<Instant>,
    /// A check is already on its way.
    scheduled: bool,
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
        if !grew || note.scheduled || note.path.is_none() {
            return Ok(());
        }
        note.scheduled = true;
        let wait = note
            .last_logged
            .map_or(SETTLE, |at| MIN_GAP.saturating_sub(at.elapsed()).max(SETTLE));
        let inner = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            if let Some(inner) = inner.upgrade() {
                inner.log_transcript(id).await;
            }
        });
        Ok(())
    }

    /// Logs `agent.transcript` if the file grew since the last one.
    async fn log_transcript(self: Arc<Self>, id: i64) {
        let (path, logged_len) = {
            let mut live = self.live();
            let Some(note) = live.transcripts.get_mut(&id) else {
                return;
            };
            note.scheduled = false;
            let Some(path) = note.path.clone() else {
                return;
            };
            (path, note.logged_len)
        };
        let read = tokio::task::spawn_blocking(move || -> std::io::Result<Option<(u64, usize)>> {
            let len = std::fs::metadata(&path)?.len();
            if len == logged_len {
                return Ok(None);
            }
            Ok(Some((len, transcript::read(&path)?.next)))
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
