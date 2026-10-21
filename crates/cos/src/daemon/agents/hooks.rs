//! What each hook means for its agent: state changes, and permission
//! requests turned into approvals.
//!
//! - `SessionStart` → `idle` (from `starting` or `needs_trust`)
//! - `UserPromptSubmit`, `PreToolUse`, `PostToolUse` → `working`
//! - `PermissionRequest` → an approval and `waiting_approval`, unless a rule
//!   answers it at once ([`decide`])
//! - `Stop` → `idle`, with `last_message`
//!
//! A permission request can also be answered in the terminal. Claude doesn't
//! kill the waiting hook promptly then, so a pending approval counts as
//! `settled` once its hook is withdrawn **or** a later hook shows the session
//! moved on: `PostToolUse` of the same tool, the next `PreToolUse`, or `Stop`.

use std::sync::Arc;
use std::time::Duration;

use cos_core::{AgentState, Approval, ApprovalState, DecidedBy, NewApproval, Role, StoreError};
use cos_harness::Decision;
use cos_harness::claude::hooks::{Hook, HookEvent};
use serde_json::Value;

use super::{AgentError, Inner, Live};

/// Sent to Claude when nobody answered an approval in time.
pub(crate) const EXPIRED_MESSAGE: &str = "Nobody answered in 5 minutes; carry on without this step.";
/// Sent to Claude with a denial that came without a message.
const DEFAULT_DENIAL: &str = "The user denied this.";
/// Tools remembered per agent for pairing permission requests.
const MAX_OPEN_TOOLS: usize = 32;

/// A permission hook waiting for an answer.
pub(super) struct Waiting {
    pub hook: Hook,
    pub agent_id: i64,
    pub tool_use_id: Option<String>,
}

/// A tool that started and hasn't finished.
pub(super) struct OpenTool {
    tool_use_id: String,
    tool_name: String,
    tool_input: Value,
}

/// What a rule gets to judge a permission request by.
#[allow(
    dead_code,
    reason = "read by the approvals engine once it plugs into `decide`"
)]
pub(crate) struct ApprovalRequest<'a> {
    pub approval: &'a Approval,
    pub role: Role,
}

/// The approvals engine's seam: a decision on a permission request without
/// asking anyone, and who made it. Every request goes to the user for now;
/// the policy engine plugs in here.
pub(crate) fn decide(_request: &ApprovalRequest<'_>) -> Option<(Decision, DecidedBy)> {
    None
}

impl Inner {
    pub(super) fn on_hook(self: &Arc<Self>, event: HookEvent) {
        let result = match event {
            HookEvent::Fired(hook) => self.on_fired(hook),
            HookEvent::Withdrawn { id } => self.on_withdrawn(id),
        };
        if let Err(error) = result {
            tracing::error!(%error, "could not record a hook");
        }
    }

    fn on_fired(self: &Arc<Self>, hook: Hook) -> Result<(), StoreError> {
        let mut live = self.live();
        let session = hook.session_id.as_deref().unwrap_or_default();
        let Some(&id) = live.by_session.get(session) else {
            // Not one of ours. Dropping a permission hook unanswered leaves
            // the decision to Claude's own prompt in the terminal.
            return Ok(());
        };
        if let Some(running) = live.terminals.get_mut(&id) {
            running.heard = true;
        }
        match hook.event.as_str() {
            "SessionStart" => {
                let state = self.agents.get(id)?.map(|agent| agent.state);
                if matches!(state, Some(AgentState::Starting | AgentState::NeedsTrust)) {
                    self.agents.set_state(id, AgentState::Idle)?;
                } else {
                    self.agents.touch(id)?;
                }
            }
            "UserPromptSubmit" => {
                self.agents.set_state(id, AgentState::Working)?;
            }
            "PreToolUse" => {
                // Claude moved on, so whatever it asked before was answered.
                self.settle_all(&mut live, id, Some(DecidedBy::Terminal))?;
                if let (Some(running), Some(tool)) = (live.terminals.get_mut(&id), OpenTool::from_hook(&hook))
                {
                    if running.open_tools.len() == MAX_OPEN_TOOLS {
                        running.open_tools.remove(0);
                    }
                    running.open_tools.push(tool);
                }
                self.agents.set_state(id, AgentState::Working)?;
            }
            "PermissionRequest" => self.request_approval(&mut live, id, hook)?,
            "PostToolUse" => {
                if let Some(tool_use_id) = hook.input.get("tool_use_id").and_then(Value::as_str) {
                    let settled: Vec<i64> = live
                        .waiting
                        .iter()
                        .filter(|(_, waiting)| {
                            waiting.agent_id == id && waiting.tool_use_id.as_deref() == Some(tool_use_id)
                        })
                        .map(|(approval_id, _)| *approval_id)
                        .collect();
                    for approval_id in settled {
                        self.settle(&mut live, approval_id, Some(DecidedBy::Terminal))?;
                    }
                    if let Some(running) = live.terminals.get_mut(&id) {
                        running.open_tools.retain(|tool| tool.tool_use_id != tool_use_id);
                    }
                }
                self.refresh_state(&live, id, AgentState::Working)?;
            }
            "Stop" => {
                self.settle_all(&mut live, id, Some(DecidedBy::Terminal))?;
                if let Some(running) = live.terminals.get_mut(&id) {
                    running.open_tools.clear();
                }
                if let Some(message) = hook.input.get("last_assistant_message").and_then(Value::as_str) {
                    self.agents.set_last_message(id, message)?;
                }
                self.agents.set_state(id, AgentState::Idle)?;
            }
            _ => self.agents.touch(id)?,
        }
        Ok(())
    }

    /// A permission hook stopped waiting: answered in the terminal, timed
    /// out, or the session ended.
    fn on_withdrawn(&self, hook_id: u64) -> Result<(), StoreError> {
        let mut live = self.live();
        let Some(approval_id) = live.hook_approvals.get(&hook_id).copied() else {
            return Ok(());
        };
        let agent_id = live.waiting.get(&approval_id).map(|waiting| waiting.agent_id);
        self.settle(&mut live, approval_id, Some(DecidedBy::Terminal))?;
        if let Some(agent_id) = agent_id {
            self.refresh_state(&live, agent_id, AgentState::Working)?;
        }
        Ok(())
    }

    fn request_approval(self: &Arc<Self>, live: &mut Live, id: i64, hook: Hook) -> Result<(), StoreError> {
        let Some(running) = live.terminals.get(&id) else {
            return Ok(());
        };
        let (role, workspace_id) = (running.role, running.workspace_id);
        let tool_name = hook.tool_name().unwrap_or("?").to_owned();
        let tool_input = hook.tool_input().cloned().unwrap_or(Value::Null);
        let tool_use_id = pair(&running.open_tools, &tool_name, &tool_input);
        let summary = format!(
            "{tool_name} {}",
            crate::try_cmd::summarize(&tool_name, &tool_input)
        );
        let approval = self.agents.add_approval(NewApproval {
            agent_id: id,
            workspace_id,
            tool_name,
            tool_input,
            tool_use_id: tool_use_id.clone(),
            summary: summary.trim_end().to_owned(),
        })?;

        if let Some((decision, by)) = decide(&ApprovalRequest {
            approval: &approval,
            role,
        }) {
            hook.answer(&decision);
            let (state, message) = outcome(&decision);
            self.agents
                .decide_approval(approval.id, state, Some(by), message)?;
            self.agents.touch(id)?;
            return Ok(());
        }

        live.hook_approvals.insert(hook.id, approval.id);
        live.waiting.insert(
            approval.id,
            Waiting {
                hook,
                agent_id: id,
                tool_use_id,
            },
        );
        self.agents.set_state(id, AgentState::WaitingApproval)?;
        // A person can always answer in their own terminal; the agents runs
        // start can't wait forever.
        if role != Role::User {
            tokio::spawn(expire(
                Arc::downgrade(self),
                approval.id,
                self.config.approval_timeout,
            ));
        }
        Ok(())
    }

    /// Answers a pending approval through cos, for the user.
    pub(super) fn answer(
        &self,
        approval_id: i64,
        allow: bool,
        message: Option<String>,
    ) -> Result<Approval, AgentError> {
        let mut live = self.live();
        let Some(waiting) = live.waiting.remove(&approval_id) else {
            let approval = self
                .agents
                .approval(approval_id)?
                .ok_or_else(|| AgentError::NotFound(format!("there is no approval {approval_id}")))?;
            return Err(AgentError::Conflict(format!(
                "approval {approval_id} is no longer pending (it is {})",
                state_name(approval.state)
            )));
        };
        live.hook_approvals.remove(&waiting.hook.id);
        let agent_id = waiting.agent_id;
        let decision = if allow {
            Decision::Allow
        } else {
            Decision::Deny {
                message: message.clone().unwrap_or_else(|| DEFAULT_DENIAL.into()),
            }
        };
        waiting.hook.answer(&decision);
        let state = if allow {
            ApprovalState::Allowed
        } else {
            ApprovalState::Denied
        };
        let approval = self
            .agents
            .decide_approval(approval_id, state, Some(DecidedBy::User), message.as_deref())?
            .ok_or_else(|| AgentError::Conflict(format!("approval {approval_id} is no longer pending")))?;
        self.refresh_state(&live, agent_id, AgentState::Working)?;
        Ok(approval)
    }

    /// Nobody answered in time: deny, so the agent carries on without it.
    fn expire_approval(&self, approval_id: i64) -> Result<(), StoreError> {
        let mut live = self.live();
        let Some(waiting) = live.waiting.remove(&approval_id) else {
            return Ok(());
        };
        live.hook_approvals.remove(&waiting.hook.id);
        let agent_id = waiting.agent_id;
        waiting.hook.answer(&Decision::Deny {
            message: EXPIRED_MESSAGE.into(),
        });
        self.agents
            .decide_approval(approval_id, ApprovalState::Expired, None, Some(EXPIRED_MESSAGE))?;
        self.refresh_state(&live, agent_id, AgentState::Working)
    }

    /// Marks every pending approval of an agent as settled.
    pub(super) fn settle_all(
        &self,
        live: &mut Live,
        agent_id: i64,
        by: Option<DecidedBy>,
    ) -> Result<(), StoreError> {
        let pending: Vec<i64> = live
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.agent_id == agent_id)
            .map(|(approval_id, _)| *approval_id)
            .collect();
        for approval_id in pending {
            self.settle(live, approval_id, by)?;
        }
        Ok(())
    }

    /// Someone answered without cos. Dropping the hook unanswered is safe:
    /// Claude has its answer already.
    fn settle(&self, live: &mut Live, approval_id: i64, by: Option<DecidedBy>) -> Result<(), StoreError> {
        if let Some(waiting) = live.waiting.remove(&approval_id) {
            live.hook_approvals.remove(&waiting.hook.id);
        }
        self.agents
            .decide_approval(approval_id, ApprovalState::Settled, by, None)?;
        Ok(())
    }

    /// `waiting_approval` while any approval of the agent is open, else `otherwise`.
    fn refresh_state(&self, live: &Live, agent_id: i64, otherwise: AgentState) -> Result<(), StoreError> {
        let waiting = live.waiting.values().any(|waiting| waiting.agent_id == agent_id);
        let state = if waiting {
            AgentState::WaitingApproval
        } else {
            otherwise
        };
        self.agents.set_state(agent_id, state)?;
        Ok(())
    }
}

impl OpenTool {
    fn from_hook(hook: &Hook) -> Option<Self> {
        Some(Self {
            tool_use_id: hook.input.get("tool_use_id")?.as_str()?.to_owned(),
            tool_name: hook.tool_name()?.to_owned(),
            tool_input: hook.tool_input().cloned().unwrap_or(Value::Null),
        })
    }
}

/// The `tool_use_id` of the request's tool: the newest open tool with the
/// same name and input, else the newest with the same name.
fn pair(open: &[OpenTool], tool_name: &str, tool_input: &Value) -> Option<String> {
    let same_name = || open.iter().rev().filter(|tool| tool.tool_name == tool_name);
    same_name()
        .find(|tool| &tool.tool_input == tool_input)
        .or_else(|| same_name().next())
        .map(|tool| tool.tool_use_id.clone())
}

fn outcome(decision: &Decision) -> (ApprovalState, Option<&str>) {
    match decision {
        Decision::Allow => (ApprovalState::Allowed, None),
        Decision::Deny { message } => (ApprovalState::Denied, Some(message)),
    }
}

fn state_name(state: ApprovalState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

async fn expire(inner: std::sync::Weak<Inner>, approval_id: i64, after: Duration) {
    tokio::time::sleep(after).await;
    if let Some(inner) = inner.upgrade()
        && let Err(error) = inner.expire_approval(approval_id)
    {
        tracing::error!(%error, approval = approval_id, "could not expire an approval");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(id: &str, name: &str, input: Value) -> OpenTool {
        OpenTool {
            tool_use_id: id.into(),
            tool_name: name.into(),
            tool_input: input,
        }
    }

    #[test]
    fn pairs_a_request_with_its_tool() {
        let open = [
            tool("a", "Bash", json!({ "command": "ls" })),
            tool("b", "Bash", json!({ "command": "touch x" })),
            tool("c", "Read", json!({ "file_path": "/x" })),
        ];

        assert_eq!(
            pair(&open, "Bash", &json!({ "command": "ls" })).as_deref(),
            Some("a")
        );
        assert_eq!(
            pair(&open, "Bash", &json!({ "command": "other" })).as_deref(),
            Some("b")
        );
        assert_eq!(pair(&open, "Write", &json!({})), None);
    }
}
