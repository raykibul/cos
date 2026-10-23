//! Requests nobody answers are refused after five minutes, and the agent is
//! told to carry on without that step.

use jiff::{SignedDuration, Timestamp};

/// How long a request waits for an answer.
pub const UNANSWERED_AFTER: SignedDuration = SignedDuration::from_secs(5 * 60);

/// What the worker sees as the tool's error when its request times out.
pub const UNANSWERED_MESSAGE: &str = "Nobody answered this permission request within 5 minutes, so it was \
     refused. Carry on without this step, and mention in your report what you couldn't do.";

/// When a request asked at `asked_at` is refused if still unanswered.
pub fn refuse_at(asked_at: Timestamp) -> Timestamp {
    asked_at.checked_add(UNANSWERED_AFTER).unwrap_or(Timestamp::MAX)
}

/// The refusal message once a request asked at `asked_at` has waited too
/// long, or `None` while it may still be answered.
pub fn unanswered(asked_at: Timestamp, now: Timestamp) -> Option<&'static str> {
    (now >= refuse_at(asked_at)).then_some(UNANSWERED_MESSAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_after_five_minutes() {
        let asked: Timestamp = "2026-10-22T10:00:00Z".parse().unwrap();
        let just_before: Timestamp = "2026-10-22T10:04:59Z".parse().unwrap();
        let at: Timestamp = "2026-10-22T10:05:00Z".parse().unwrap();
        assert_eq!(refuse_at(asked), at);
        assert_eq!(unanswered(asked, just_before), None);
        assert_eq!(unanswered(asked, at), Some(UNANSWERED_MESSAGE));
        assert!(UNANSWERED_MESSAGE.contains("Carry on without this step"));
        assert_eq!(refuse_at(Timestamp::MAX), Timestamp::MAX);
    }
}
