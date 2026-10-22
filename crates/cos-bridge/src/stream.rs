//! What the streams share: where their messages go, the handle that closes
//! them, and the reconnect delay.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::Backoff;

/// Where a stream's messages go. A Tauri `Channel` in the app, anything in
/// tests. Closures `FnMut(M) -> bool` are sinks.
pub trait Sink<M>: Send + 'static {
    /// Delivers one message. `false` means nobody is listening any more,
    /// and the stream stops.
    fn send(&mut self, message: M) -> bool;
}

impl<M, F> Sink<M> for F
where
    F: FnMut(M) -> bool + Send + 'static,
{
    fn send(&mut self, message: M) -> bool {
        self(message)
    }
}

/// Set once the stream is closed. A message is only delivered while holding
/// it, so once [`StreamHandle::close`] returns, nothing more arrives.
type Gate = Arc<Mutex<bool>>;

fn lock(gate: &Gate) -> MutexGuard<'_, bool> {
    gate.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A stream's sink behind its gate.
pub(crate) struct Outbox<S> {
    gate: Gate,
    sink: S,
}

impl<S> Outbox<S> {
    /// Delivers `message` unless the stream is closed. `false` when the
    /// stream should stop.
    pub(crate) fn send<M>(&mut self, message: M) -> bool
    where
        S: Sink<M>,
    {
        let mut closed = lock(&self.gate);
        if *closed {
            return false;
        }
        if !self.sink.send(message) {
            *closed = true;
            return false;
        }
        true
    }
}

/// A running stream. Closing it (or dropping it) stops the reconnects and
/// drops the connection; no message is delivered after [`Self::close`].
#[derive(Debug)]
#[must_use = "dropping the handle closes the stream"]
pub struct StreamHandle {
    gate: Gate,
    task: AbortHandle,
}

impl StreamHandle {
    pub fn close(&self) {
        *lock(&self.gate) = true;
        self.task.abort();
    }

    /// Whether the stream has ended: closed, its sink gone, or (for a
    /// terminal) the agent exited.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.close();
    }
}

/// Runs `stream` on the Tokio runtime with `sink` behind a fresh gate.
pub(crate) fn spawn<S, F, Fut>(sink: S, stream: F) -> StreamHandle
where
    F: FnOnce(Outbox<S>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let gate = Gate::default();
    let outbox = Outbox {
        gate: gate.clone(),
        sink,
    };
    let task = tokio::spawn(stream(outbox)).abort_handle();
    StreamHandle { gate, task }
}

/// The delay before each reconnect.
pub(crate) struct Retry {
    backoff: Backoff,
    next: Duration,
}

impl Retry {
    pub(crate) fn new(backoff: Backoff) -> Self {
        Self {
            backoff,
            next: backoff.initial,
        }
    }

    /// How long to wait now; the next wait is twice as long, up to the max.
    pub(crate) fn delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.backoff.max);
        delay
    }

    /// Called after a successful connect.
    pub(crate) fn reset(&mut self) {
        self.next = self.backoff.initial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_doubles_up_to_the_max_and_resets() {
        let mut retry = Retry::new(Backoff::default());
        let delays: Vec<u64> = (0..6)
            .map(|_| u64::try_from(retry.delay().as_millis()).unwrap())
            .collect();
        assert_eq!(delays, [500, 1000, 2000, 4000, 5000, 5000]);
        retry.reset();
        assert_eq!(retry.delay(), Duration::from_millis(500));
    }

    #[tokio::test]
    async fn nothing_is_delivered_after_close() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let (go, wait) = tokio::sync::oneshot::channel::<()>();
        let handle = spawn(
            move |message: u32| sender.send(message).is_ok(),
            |mut outbox| async move {
                outbox.send(1);
                let _ = wait.await;
                outbox.send(2);
            },
        );
        assert_eq!(receiver.recv().await, Some(1));
        handle.close();
        let _ = go.send(());
        assert_eq!(receiver.recv().await, None);
        assert!(handle.is_finished());
    }

    #[tokio::test]
    async fn a_gone_sink_stops_the_stream() {
        let calls = Arc::new(Mutex::new(0));
        let counted = calls.clone();
        let handle = spawn(
            move |_: u32| {
                *counted.lock().unwrap() += 1;
                false
            },
            |mut outbox| async move {
                assert!(!outbox.send(1));
                // Later sends don't reach the sink.
                assert!(!outbox.send(2));
            },
        );
        while !handle.is_finished() {
            tokio::task::yield_now().await;
        }
        assert_eq!(*calls.lock().unwrap(), 1);
    }
}
