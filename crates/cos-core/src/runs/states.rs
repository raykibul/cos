//! The run and task state machines. Every change of state goes through
//! [`RunState::can_become`] or [`TaskState::transition`], so an illegal move
//! is refused in one place.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Where a run is. Moves forward only:
///
/// ```text
/// planning → awaiting_plan_approval → running → reporting → done
///      (any state that isn't finished) → failed | cancelled
/// ```
///
/// The chief proposing a plan moves `planning` to `awaiting_plan_approval`;
/// proposing again while it waits replaces the plan. Approving it starts the
/// run. The run moves to `reporting` by itself once every task has finished
/// (merged, failed or cancelled), and to `done` when the report is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// The chief is working out the tasks.
    Planning,
    /// A plan is on the board, waiting for you.
    AwaitingPlanApproval,
    /// Workers are on the tasks.
    Running,
    /// Every task has finished; the chief is writing the report.
    Reporting,
    Done,
    Failed,
    Cancelled,
}

impl RunState {
    pub const ALL: [Self; 7] = [
        Self::Planning,
        Self::AwaitingPlanApproval,
        Self::Running,
        Self::Reporting,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::AwaitingPlanApproval => "awaiting_plan_approval",
            Self::Running => "running",
            Self::Reporting => "reporting",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Done, failed or cancelled: nothing more will happen.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Whether a run in this state may move to `to`.
    pub fn can_become(self, to: Self) -> bool {
        use RunState::*;
        match (self, to) {
            (from, Failed | Cancelled) => !from.is_finished(),
            (Planning, AwaitingPlanApproval)
            | (AwaitingPlanApproval, Running)
            | (Running, Reporting)
            | (Reporting, Done) => true,
            _ => false,
        }
    }
}

/// Where a task is:
///
/// ```text
/// proposed → ready → running → checking → review → merged
///                       ↑          │          │
///                       └──────────┴──────────┘  retry (once)
/// ```
///
/// Plus `blocked` (needs a decision from the chief or you), `failed` and
/// `cancelled`. A task becomes `ready` only once its run is running and every
/// task it depends on is merged; the daemon does that by itself. Going back
/// for another attempt (`checking → running`, `review → running`,
/// `running → ready` after a worker crash, `blocked → ready`) uses the task's
/// one retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// In a plan that hasn't been approved, or waiting on its dependencies.
    Proposed,
    /// Can start: the plan is approved and its dependencies are merged.
    Ready,
    /// A worker is on it.
    Running,
    /// The worker finished; cos is checking the file scope and running the tests.
    Checking,
    /// Checks passed; the reviewer is looking at it.
    Review,
    Merged,
    /// Stuck (a merge conflict, a failed dependency, a worker that gave up).
    /// The chief or you decide: retry, fail or cancel.
    Blocked,
    Failed,
    Cancelled,
}

/// What a legal task transition means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskMove {
    /// A step forward, or to blocked, failed or cancelled.
    Step,
    /// Another attempt. Uses the task's retry.
    Retry,
}

impl TaskState {
    pub const ALL: [Self; 9] = [
        Self::Proposed,
        Self::Ready,
        Self::Running,
        Self::Checking,
        Self::Review,
        Self::Merged,
        Self::Blocked,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Checking => "checking",
            Self::Review => "review",
            Self::Merged => "merged",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Merged, failed or cancelled: nothing more will happen.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Merged | Self::Failed | Self::Cancelled)
    }

    /// Whether a task in this state may move to `to`, and what kind of move
    /// that is. `None` means the move is illegal.
    pub fn transition(self, to: Self) -> Option<TaskMove> {
        use TaskMove::{Retry, Step};
        use TaskState::*;
        match (self, to) {
            (from, Cancelled) if !from.is_finished() => Some(Step),
            (Proposed, Ready | Blocked)
            | (Ready, Running | Blocked)
            | (Running, Checking | Blocked | Failed)
            | (Checking, Review | Blocked | Failed)
            | (Review, Merged | Blocked | Failed)
            | (Blocked, Failed) => Some(Step),
            (Running, Ready) | (Checking | Review, Running) | (Blocked, Ready) => Some(Retry),
            _ => None,
        }
    }

    /// Whether moving here from `from` needs a reason: blocking, failing and
    /// retrying do, so the chief, the worker and you can be told why.
    pub fn needs_reason(from: Self, to: Self) -> bool {
        matches!(to, Self::Blocked | Self::Failed) || from.transition(to) == Some(TaskMove::Retry)
    }
}

macro_rules! text_enum {
    ($type:ty, $what:literal) => {
        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $type {
            type Err = String;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::ALL
                    .into_iter()
                    .find(|state| state.as_str() == text)
                    .ok_or_else(|| format!("unknown {} {text:?}", $what))
            }
        }
    };
}

text_enum!(RunState, "run state");
text_enum!(TaskState, "task state");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_only_move_forward() {
        use RunState::*;
        let legal = [
            (Planning, AwaitingPlanApproval),
            (AwaitingPlanApproval, Running),
            (Running, Reporting),
            (Reporting, Done),
        ];
        for from in RunState::ALL {
            for to in RunState::ALL {
                let expected =
                    legal.contains(&(from, to)) || (matches!(to, Failed | Cancelled) && !from.is_finished());
                assert_eq!(from.can_become(to), expected, "{from} → {to}");
            }
        }
    }

    #[test]
    fn finished_runs_stay_finished() {
        for from in [RunState::Done, RunState::Failed, RunState::Cancelled] {
            assert!(RunState::ALL.iter().all(|&to| !from.can_become(to)), "{from}");
        }
    }

    #[test]
    fn task_transitions() {
        use TaskMove::*;
        use TaskState::*;
        let cases = [
            (Proposed, Ready, Some(Step)),
            (Proposed, Running, None),
            (Ready, Running, Some(Step)),
            (Running, Checking, Some(Step)),
            (Running, Ready, Some(Retry)),
            (Running, Merged, None),
            (Checking, Review, Some(Step)),
            (Checking, Running, Some(Retry)),
            (Review, Merged, Some(Step)),
            (Review, Running, Some(Retry)),
            (Review, Blocked, Some(Step)),
            (Blocked, Ready, Some(Retry)),
            (Blocked, Running, None),
            (Ready, Failed, None),
            (Proposed, Cancelled, Some(Step)),
            (Merged, Cancelled, None),
            (Failed, Ready, None),
            (Cancelled, Proposed, None),
        ];
        for (from, to, expected) in cases {
            assert_eq!(from.transition(to), expected, "{from} → {to}");
        }
    }

    #[test]
    fn finished_tasks_stay_finished() {
        for from in [TaskState::Merged, TaskState::Failed, TaskState::Cancelled] {
            assert!(
                TaskState::ALL.iter().all(|&to| from.transition(to).is_none()),
                "{from}"
            );
        }
    }

    #[test]
    fn blocking_failing_and_retrying_need_a_reason() {
        use TaskState::*;
        assert!(TaskState::needs_reason(Review, Blocked));
        assert!(TaskState::needs_reason(Checking, Failed));
        assert!(TaskState::needs_reason(Checking, Running));
        assert!(!TaskState::needs_reason(Ready, Running));
        assert!(!TaskState::needs_reason(Running, Cancelled));
    }

    #[test]
    fn states_round_trip_as_text_and_json() {
        for state in TaskState::ALL {
            assert_eq!(state.as_str().parse::<TaskState>(), Ok(state));
            assert_eq!(serde_json::to_value(state).unwrap(), state.as_str());
        }
        for state in RunState::ALL {
            assert_eq!(state.as_str().parse::<RunState>(), Ok(state));
            assert_eq!(serde_json::to_value(state).unwrap(), state.as_str());
        }
        assert!("nope".parse::<TaskState>().is_err());
    }
}
