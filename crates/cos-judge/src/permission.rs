//! Decision point 1: sorting permission requests. For a request cos-policy
//! sends to you, jev answers `action_kind`, `serves_task` and
//! `leaves_worktree` in one call, and [`would_have`] turns the answers into
//! what jev would do: approve, or ask you.
//!
//! jev may only ever approve actions Rust already knows stay inside the
//! worktree ([`Locality::Worktree`]), of a kind in `auto_approve`, with
//! `action_kind` more confident than 0.9 and `serves_task` above 0.8. That
//! check is code, so text in the request can't talk its way past it.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::answers::{Choice, RawAnswers, YesNo};
use crate::questions::{PermissionPoint, questions};
use crate::trim::{cut, relative_paths};
use crate::{JudgeError, choice_options};

choice_options! {
    /// What a requested action does, as jev sees it.
    pub enum ActionKind {
        ReadOnly = "read_only",
        BuildOrTest = "build_or_test",
        EditInWorktree = "edit_in_worktree",
        InstallDep = "install_dep",
        External = "external",
        Destructive = "destructive",
    }
}

/// Most characters of a Bash command sent.
const MAX_COMMAND: usize = 1_000;
/// Most characters of any other string field sent.
const MAX_FIELD: usize = 300;
/// Most characters of the task's instructions sent.
const MAX_INSTRUCTIONS: usize = 600;
const MAX_TITLE: usize = 200;
/// Fields that carry file contents. Only their size is sent.
const CONTENT_FIELDS: &[&str] = &["content", "new_string", "old_string", "new_source", "edits"];

/// What jev reads about one permission request: structured fields only,
/// trimmed, paths made relative to the worktree (`./…`) or home (`~/…`).
/// File contents never go; only their size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionInput {
    pub request: RequestBrief,
    /// `None` for an agent that isn't on a task (your own agents, the chief).
    pub task: Option<TaskBrief>,
    /// What cos-policy made of it, for context.
    pub policy: PolicyBrief,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestBrief {
    pub tool: String,
    pub input: Value,
    pub cwd: String,
    /// `worker`, `reviewer`, `chief` or `user-session`.
    pub agent_role: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskBrief {
    pub title: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyBrief {
    pub tier: String,
    pub reason: String,
}

/// Where Rust rules say an action stays. Only `Worktree` may ever be
/// approved by jev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// cos-policy checked every path and found nothing outside the worktree,
    /// no network and no install.
    Worktree,
    /// Anything else, including what cos-policy couldn't classify.
    Beyond,
}

/// The raw facts the daemon has about a request, before trimming.
pub struct RawRequest<'a> {
    pub tool: &'a str,
    pub input: &'a Value,
    pub cwd: Option<&'a Path>,
    pub worktree_root: &'a Path,
    pub home: Option<&'a Path>,
    pub agent_role: &'a str,
    pub task_title: Option<&'a str>,
    pub task_instructions: Option<&'a str>,
    pub policy_tier: &'a str,
    pub policy_reason: &'a str,
}

impl PermissionInput {
    /// Trims a request down to what jev needs.
    pub fn from_request(raw: &RawRequest<'_>) -> Self {
        let paths = |text: &str| relative_paths(text, raw.worktree_root, raw.home);
        let input = trim_tool_input(raw.tool, raw.input, &paths);
        let cwd = raw
            .cwd
            .map(|cwd| paths(&cwd.to_string_lossy()))
            .filter(|cwd| !cwd.is_empty())
            .unwrap_or_else(|| ".".to_owned());
        Self {
            request: RequestBrief {
                tool: cut(raw.tool, MAX_FIELD),
                input,
                cwd,
                agent_role: raw.agent_role.to_owned(),
            },
            task: raw.task_title.map(|title| TaskBrief {
                title: cut(title, MAX_TITLE),
                instructions: cut(
                    &paths(raw.task_instructions.unwrap_or_default()),
                    MAX_INSTRUCTIONS,
                ),
            }),
            policy: PolicyBrief {
                tier: raw.policy_tier.to_owned(),
                reason: cut(&paths(raw.policy_reason), MAX_FIELD),
            },
        }
    }
}

/// The tool input jev sees: a Bash command and its description, a file
/// tool's path without the contents, or any other tool's scalar fields.
fn trim_tool_input(tool: &str, input: &Value, paths: &dyn Fn(&str) -> String) -> Value {
    let Some(fields) = input.as_object() else {
        return Value::String(cut(&paths(&input.to_string()), MAX_FIELD));
    };
    let mut out = Map::new();
    for (name, value) in fields {
        let limit = if tool == "Bash" && name == "command" {
            MAX_COMMAND
        } else {
            MAX_FIELD
        };
        let trimmed = if CONTENT_FIELDS.contains(&name.as_str()) {
            json!(format!("<{} characters, not sent>", size_of(value)))
        } else {
            match value {
                Value::String(text) => Value::String(cut(&paths(text), limit)),
                Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
                Value::Array(items) => json!(format!("<{} items, not sent>", items.len())),
                Value::Object(map) => json!(format!("<{} fields, not sent>", map.len())),
            }
        };
        out.insert(cut(name, 64), trimmed);
    }
    Value::Object(out)
}

fn size_of(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        other => other.to_string().chars().count(),
    }
}

/// jev's answers for one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionAnswers {
    pub action_kind: Choice<ActionKind>,
    pub serves_task: YesNo,
    pub leaves_worktree: YesNo,
}

impl PermissionAnswers {
    pub fn decode(raw: &RawAnswers) -> Result<Self, JudgeError> {
        Ok(Self {
            action_kind: raw.choice("action_kind")?,
            serves_task: raw.yes_no("serves_task")?,
            leaves_worktree: raw.yes_no("leaves_worktree")?,
        })
    }
}

/// What jev would do with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Approve,
    AskYou,
}

impl PermissionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::AskYou => "ask_you",
        }
    }
}

/// jev's would-have, with a one-line reason for people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionVerdict {
    pub action: PermissionAction,
    /// True when jev had no usable opinion (error, timeout, confidence below
    /// the minimum), so it falls back to asking you.
    pub no_opinion: bool,
    pub why: String,
}

/// What jev would do, given where Rust says the action stays and jev's
/// answers (or why there are none). The thresholds come from
/// `judge/questions.json`.
pub fn would_have(locality: Locality, answers: Result<&PermissionAnswers, &JudgeError>) -> PermissionVerdict {
    would_have_with(
        &questions().points.permission,
        questions().min_confidence,
        locality,
        answers,
    )
}

pub fn would_have_with(
    point: &PermissionPoint,
    min_confidence: f64,
    locality: Locality,
    answers: Result<&PermissionAnswers, &JudgeError>,
) -> PermissionVerdict {
    let ask = |why: String| PermissionVerdict {
        action: PermissionAction::AskYou,
        no_opinion: false,
        why,
    };
    let answers = match answers {
        Ok(answers) => answers,
        Err(error) => {
            return PermissionVerdict {
                no_opinion: true,
                ..ask(format!("no opinion: {error}"))
            };
        }
    };
    let kind = &answers.action_kind;
    if kind.confidence < min_confidence {
        return PermissionVerdict {
            no_opinion: true,
            ..ask(format!(
                "no opinion: action_kind {} at confidence {:.2}",
                kind.picked, kind.confidence
            ))
        };
    }
    let thresholds = point.thresholds;
    let summary = format!(
        "{} ({:.2}), serves_task {:.2}, leaves_worktree {:.2}",
        kind.picked, kind.confidence, answers.serves_task.probability, answers.leaves_worktree.probability
    );
    let refusal = if locality != Locality::Worktree {
        Some("cos-policy can't confirm it stays in the worktree")
    } else if !point.auto_approve.contains(&kind.picked) {
        Some("not a kind jev may approve")
    } else if kind.confidence <= thresholds.action_kind_confidence {
        Some("action_kind isn't confident enough")
    } else if answers.serves_task.probability <= thresholds.serves_task {
        Some("not clearly needed for the task")
    } else if answers.leaves_worktree.probability >= thresholds.leaves_worktree {
        Some("looks like it leaves the worktree")
    } else {
        None
    };
    match refusal {
        Some(why) => ask(format!("{why}: {summary}")),
        None => PermissionVerdict {
            action: PermissionAction::Approve,
            no_opinion: false,
            why: summary,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn answers(kind: ActionKind, confidence: f64, serves: f64, leaves: f64) -> PermissionAnswers {
        PermissionAnswers {
            action_kind: Choice {
                picked: kind,
                probabilities: BTreeMap::new(),
                confidence,
            },
            serves_task: YesNo { probability: serves },
            leaves_worktree: YesNo { probability: leaves },
        }
    }

    fn action(locality: Locality, answers: &PermissionAnswers) -> PermissionAction {
        would_have(locality, Ok(answers)).action
    }

    #[test]
    fn approves_only_confident_worktree_kinds_that_serve_the_task() {
        for kind in [
            ActionKind::ReadOnly,
            ActionKind::BuildOrTest,
            ActionKind::EditInWorktree,
        ] {
            assert_eq!(
                action(Locality::Worktree, &answers(kind, 0.95, 0.9, 0.05)),
                PermissionAction::Approve,
                "{kind}"
            );
        }
        for kind in [
            ActionKind::InstallDep,
            ActionKind::External,
            ActionKind::Destructive,
        ] {
            assert_eq!(
                action(Locality::Worktree, &answers(kind, 0.99, 0.99, 0.0)),
                PermissionAction::AskYou,
                "{kind}"
            );
        }
    }

    #[test]
    fn thresholds_are_strict() {
        let ask = PermissionAction::AskYou;
        assert_eq!(
            action(Locality::Worktree, &answers(ActionKind::ReadOnly, 0.9, 0.95, 0.0)),
            ask
        );
        assert_eq!(
            action(Locality::Worktree, &answers(ActionKind::ReadOnly, 0.95, 0.8, 0.0)),
            ask
        );
        assert_eq!(
            action(
                Locality::Worktree,
                &answers(ActionKind::ReadOnly, 0.95, 0.95, 0.5)
            ),
            ask
        );
    }

    #[test]
    fn never_approves_beyond_the_worktree() {
        let verdict = would_have(
            Locality::Beyond,
            Ok(&answers(ActionKind::ReadOnly, 1.0, 1.0, 0.0)),
        );

        assert_eq!(verdict.action, PermissionAction::AskYou);
        assert!(!verdict.no_opinion);
        assert!(verdict.why.contains("worktree"), "{}", verdict.why);
    }

    #[test]
    fn low_confidence_and_errors_are_no_opinion() {
        let low = would_have(
            Locality::Worktree,
            Ok(&answers(ActionKind::ReadOnly, 0.4, 1.0, 0.0)),
        );
        let failed = would_have(Locality::Worktree, Err(&JudgeError::Timeout));

        for verdict in [low, failed] {
            assert_eq!(verdict.action, PermissionAction::AskYou);
            assert!(verdict.no_opinion, "{}", verdict.why);
        }
    }

    #[test]
    fn the_input_is_trimmed_and_relative() {
        let content = "x".repeat(5_000);
        let input = json!({ "file_path": "/w/tree/src/a.rs", "content": content, "extra": [1, 2] });
        let raw = RawRequest {
            tool: "Write",
            input: &input,
            cwd: Some(Path::new("/w/tree/src")),
            worktree_root: Path::new("/w/tree"),
            home: Some(Path::new("/Users/me")),
            agent_role: "worker",
            task_title: Some("Add a"),
            task_instructions: Some(&"do it in /w/tree/src and ~ ".repeat(100)),
            policy_tier: "chief (worktree)",
            policy_reason: "edits /Users/me/notes outside the scope",
        };

        let trimmed = PermissionInput::from_request(&raw);

        assert_eq!(trimmed.request.input["file_path"], "./src/a.rs");
        assert_eq!(trimmed.request.input["content"], "<5000 characters, not sent>");
        assert_eq!(trimmed.request.input["extra"], "<2 items, not sent>");
        assert_eq!(trimmed.request.cwd, "./src");
        assert_eq!(trimmed.policy.reason, "edits ~/notes outside the scope");
        let task = trimmed.task.unwrap();
        assert!(task.instructions.chars().count() <= MAX_INSTRUCTIONS + 1);
        assert!(!task.instructions.contains("/w/tree"));
        assert!(
            !serde_json::to_string(&trimmed.request)
                .unwrap()
                .contains(&"x".repeat(100))
        );
    }

    #[test]
    fn a_long_command_is_cut() {
        let command = format!("echo {}", "a".repeat(5_000));
        let input = json!({ "command": command, "description": "say a lot" });
        let raw = RawRequest {
            tool: "Bash",
            input: &input,
            cwd: None,
            worktree_root: Path::new("/w"),
            home: None,
            agent_role: "user-session",
            task_title: None,
            task_instructions: None,
            policy_tier: "ask you",
            policy_reason: "",
        };

        let trimmed = PermissionInput::from_request(&raw);

        assert!(trimmed.request.input["command"].as_str().unwrap().chars().count() <= MAX_COMMAND + 1);
        assert_eq!(trimmed.request.input["description"], "say a lot");
        assert_eq!(trimmed.request.cwd, ".");
        assert_eq!(trimmed.task, None);
    }
}
