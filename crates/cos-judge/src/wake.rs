//! Decision point 2: when to wake a chief. For an item added to a run
//! chief's or a Chief of Staff's inbox, jev answers `wake` (now, batch or
//! none) and `is_question_for_chief` in one call.

use serde::{Deserialize, Serialize};

use crate::answers::{Choice, RawAnswers, YesNo};
use crate::questions::questions;
use crate::trim::cut;
use crate::{JudgeError, choice_options};

choice_options! {
    /// How soon the chief should read an inbox item.
    pub enum Wake {
        Now = "now",
        Batch = "batch",
        None = "none",
    }
}

/// Most characters of an item's text sent.
const MAX_TEXT: usize = 2_000;
const MAX_GOAL: usize = 300;

/// Which chief an item is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChiefKind {
    /// The chief of one run.
    RunChief,
    /// A project's Chief of Staff.
    StaffChief,
}

/// What jev reads about one inbox item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeInput {
    pub source: ChiefKind,
    pub update: Update,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    /// The inbox item's kind: `task`, `approval`, `reporting`, `agent`, `run`.
    pub kind: String,
    pub text: String,
    /// The run's goal, for a run chief's item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_goal: Option<String>,
}

impl WakeInput {
    pub fn new(source: ChiefKind, kind: &str, text: &str, run_goal: Option<&str>) -> Self {
        Self {
            source,
            update: Update {
                kind: kind.to_owned(),
                text: cut(text, MAX_TEXT),
                run_goal: run_goal.map(|goal| cut(goal, MAX_GOAL)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeAnswers {
    pub wake: Choice<Wake>,
    pub is_question_for_chief: YesNo,
}

impl WakeAnswers {
    pub fn decode(raw: &RawAnswers) -> Result<Self, JudgeError> {
        Ok(Self {
            wake: raw.choice("wake")?,
            is_question_for_chief: raw.yes_no("is_question_for_chief")?,
        })
    }
}

/// jev's would-have for an inbox item: a wake choice, or no opinion (the
/// item is delivered exactly as without jev).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeVerdict {
    /// `None` when jev has no opinion.
    pub wake: Option<Wake>,
    pub why: String,
}

impl WakeVerdict {
    /// `now`, `batch`, `none` or `no_opinion`.
    pub fn as_str(&self) -> &'static str {
        match self.wake {
            Some(Wake::Now) => "now",
            Some(Wake::Batch) => "batch",
            Some(Wake::None) => "none",
            None => "no_opinion",
        }
    }
}

pub fn would_have(answers: Result<&WakeAnswers, &JudgeError>) -> WakeVerdict {
    let min_confidence = questions().min_confidence;
    match answers {
        Err(error) => WakeVerdict {
            wake: None,
            why: format!("no opinion: {error}"),
        },
        Ok(answers) if answers.wake.confidence < min_confidence => WakeVerdict {
            wake: None,
            why: format!(
                "no opinion: wake {} at confidence {:.2}",
                answers.wake.picked, answers.wake.confidence
            ),
        },
        Ok(answers) => WakeVerdict {
            wake: Some(answers.wake.picked),
            why: format!(
                "{} ({:.2}), is_question_for_chief {:.2}",
                answers.wake.picked, answers.wake.confidence, answers.is_question_for_chief.probability
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn answers(wake: Wake, confidence: f64) -> WakeAnswers {
        WakeAnswers {
            wake: Choice {
                picked: wake,
                probabilities: BTreeMap::new(),
                confidence,
            },
            is_question_for_chief: YesNo { probability: 0.1 },
        }
    }

    #[test]
    fn a_confident_answer_is_the_would_have() {
        let verdict = would_have(Ok(&answers(Wake::Batch, 0.8)));

        assert_eq!(verdict.wake, Some(Wake::Batch));
        assert_eq!(verdict.as_str(), "batch");
    }

    #[test]
    fn low_confidence_or_an_error_is_no_opinion() {
        assert_eq!(would_have(Ok(&answers(Wake::Now, 0.3))).as_str(), "no_opinion");
        assert_eq!(would_have(Err(&JudgeError::Timeout)).as_str(), "no_opinion");
    }

    #[test]
    fn the_text_is_cut() {
        let input = WakeInput::new(ChiefKind::RunChief, "task", &"y".repeat(10_000), Some("goal"));

        assert!(input.update.text.chars().count() <= MAX_TEXT + 1);
        assert_eq!(input.update.run_goal.as_deref(), Some("goal"));
    }
}
