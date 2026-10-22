//! `judge/questions.json`, embedded in the binary: every question jev is
//! asked and every threshold applied to its answers, in one file to review
//! and tune.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::permission::ActionKind;

/// The file as shipped.
pub const QUESTIONS_JSON: &str = include_str!("../judge/questions.json");

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionsFile {
    /// Below this confidence a choice or score answer is "no opinion".
    pub min_confidence: f64,
    pub points: Points,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Points {
    pub permission: PermissionPoint,
    pub wake: WakePoint,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionPoint {
    pub thresholds: PermissionThresholds,
    /// The action kinds jev may approve, and only inside the worktree.
    pub auto_approve: Vec<ActionKind>,
    pub questions: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PermissionThresholds {
    /// `action_kind` must be more confident than this.
    pub action_kind_confidence: f64,
    /// `serves_task` must be above this.
    pub serves_task: f64,
    /// `leaves_worktree` must be below this.
    pub leaves_worktree: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WakePoint {
    pub questions: Map<String, Value>,
}

/// The embedded questions file, parsed once.
pub fn questions() -> &'static QuestionsFile {
    static FILE: OnceLock<QuestionsFile> = OnceLock::new();
    FILE.get_or_init(|| {
        serde_json::from_str(QUESTIONS_JSON).expect("judge/questions.json is valid (checked by tests)")
    })
}

/// Problems with a `questions` object, as the jev skill's `check` finds
/// them: unknown types, missing instructions, malformed criteria, bare
/// numeric score levels. Empty when it's well formed.
pub fn check(questions: &Map<String, Value>) -> Vec<String> {
    let mut problems = Vec::new();
    if questions.is_empty() {
        problems.push("questions must be a non-empty object".to_owned());
    }
    for (key, question) in questions {
        let place = format!("questions.{key}");
        let kind = question.get("type").and_then(Value::as_str);
        if question.get("instructions").is_none_or(Value::is_null) {
            problems.push(format!("{place}: instructions is required"));
        }
        let criteria = question.get("criteria");
        match kind {
            Some("noul") => {
                if let Some(criteria) = criteria
                    && !criteria
                        .as_object()
                        .is_some_and(|map| map.keys().all(|key| key == "true" || key == "false"))
                {
                    problems.push(format!("{place}: noul criteria may only have `true` and `false`"));
                }
            }
            Some("choice") => match criteria.and_then(Value::as_object) {
                Some(options) if (2..=255).contains(&options.len()) => {}
                _ => problems.push(format!(
                    "{place}: choice criteria must be an object of 2 to 255 options"
                )),
            },
            Some("score") => match criteria.and_then(Value::as_array) {
                Some(levels) if (2..=10).contains(&levels.len()) => {
                    if levels.iter().all(|level| {
                        level
                            .as_str()
                            .is_some_and(|text| text.trim().parse::<u32>().is_ok())
                    }) {
                        problems.push(format!(
                            "{place}: bare numeric levels score badly; describe each level"
                        ));
                    }
                }
                _ => problems.push(format!(
                    "{place}: score criteria must be an array of 2 to 10 levels"
                )),
            },
            other => problems.push(format!(
                "{place}: type must be noul, choice or score, not {}",
                other.unwrap_or("nothing")
            )),
        }
    }
    problems
}

/// A choice question's option names, in file order.
pub fn options(questions: &Map<String, Value>, question: &str) -> Vec<String> {
    questions
        .get(question)
        .and_then(|question| question.get("criteria"))
        .and_then(Value::as_object)
        .map(|criteria| criteria.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::ChoiceOption;
    use crate::wake::Wake;

    fn names<T: ChoiceOption>() -> BTreeSet<String> {
        T::OPTIONS.iter().map(|(name, _)| (*name).to_owned()).collect()
    }

    #[test]
    fn the_shipped_file_is_well_formed() {
        let file = questions();

        assert_eq!(check(&file.points.permission.questions), Vec::<String>::new());
        assert_eq!(check(&file.points.wake.questions), Vec::<String>::new());
        assert!(file.min_confidence > 0.0 && file.min_confidence < 1.0);
    }

    #[test]
    fn choice_questions_match_their_enums() {
        let file = questions();

        let action_kinds: BTreeSet<String> = options(&file.points.permission.questions, "action_kind")
            .into_iter()
            .collect();
        let wakes: BTreeSet<String> = options(&file.points.wake.questions, "wake").into_iter().collect();

        assert_eq!(action_kinds, names::<ActionKind>());
        assert_eq!(wakes, names::<Wake>());
    }

    #[test]
    fn the_proposal_thresholds_are_in_the_file() {
        let permission = &questions().points.permission;

        assert_eq!(permission.thresholds.action_kind_confidence, 0.9);
        assert_eq!(permission.thresholds.serves_task, 0.8);
        assert_eq!(
            permission.auto_approve,
            [
                ActionKind::ReadOnly,
                ActionKind::BuildOrTest,
                ActionKind::EditInWorktree
            ]
        );
        assert_eq!(questions().min_confidence, 0.5);
    }

    #[test]
    fn check_finds_what_the_skill_warns_about() {
        let bad = json!({
            "a": { "type": "maybe", "instructions": "x" },
            "b": { "type": "choice", "instructions": "x", "criteria": { "only": null } },
            "c": { "type": "score", "instructions": "x", "criteria": ["0", "1", "2"] },
            "d": { "type": "noul", "criteria": { "yes": "x" } },
        });

        let problems = check(bad.as_object().unwrap());

        assert_eq!(problems.len(), 5, "{problems:#?}");
    }
}
