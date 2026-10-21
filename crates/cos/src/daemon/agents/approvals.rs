//! The approvals policy (`cos-policy`) in the hook path. Every permission
//! request is classified with what the daemon knows about the agent (its
//! role, its run's limits, its task's worktree, scope and test command), then:
//!
//! - **auto:** Rust rules allow it. Answered at once and recorded as
//!   `allowed` by `rule`; nobody is notified.
//! - **chief:** left pending with `route: chief` and `approval.routed`; the
//!   chief answers through `POST /v1/approvals/{id}/chief`
//!   ([`Inner::chief_answer`]), checked against the run's limits.
//! - **user:** left pending with `route: user`, as before the policy.
//!
//! Agents you start (`role: user`) always go to you; their requests are still
//! classified so the apps can show the tier and reason. Every answer to a run
//! agent's request that the rules, the chief, a timeout or you gave through
//! cos lands in the run's audit trail, which feeds the report section
//! "Approved on your behalf" ([`cos_core::Runs::approvals_report_section`]).

use std::path::PathBuf;
use std::sync::PoisonError;

use cos_core::{Agent, Approval, ApprovalRoute, ApprovalState, DecidedBy, Role, RunError, Task, Workspace};
use cos_harness::Decision;
use cos_policy::{
    ApprovalRequest, AuditRecord, ChiefAnswer, ChiefDecision, Decider, Filesystem, Limits, Outcome,
    Role as PolicyRole, Route, RunLedger, Tier, Verdict, classify_with, summarize,
};
use jiff::Timestamp;
use serde_json::Value;

use super::{AgentError, Inner};

/// Finds the task a worker or reviewer works on, for its worktree, scope and
/// test command. The pipeline knows; once agents record their task
/// (`agents.task_id`), [`Inner::task_of`] reads that instead.
pub(crate) trait TaskLookup: Send + Sync {
    fn task_of(&self, agent: &Agent) -> Option<i64>;
}

/// Knows no tasks.
pub(crate) struct NoTasks;

impl TaskLookup for NoTasks {
    fn task_of(&self, _agent: &Agent) -> Option<i64> {
        None
    }
}

/// How the policy judged one request; kept with it until it's answered.
#[derive(Debug, Clone)]
pub(super) struct Judgement {
    pub verdict: Verdict,
    pub role: PolicyRole,
    pub run_id: Option<i64>,
    pub route: ApprovalRoute,
    /// Stored as the approval's `reason`: the policy's reason, and why it
    /// didn't go to the chief when its tier would have let it.
    pub reason: String,
}

impl Inner {
    /// Classifies a permission request from `agent` and works out who
    /// answers it. `cwd` is the hook's working directory.
    pub(super) fn judge(&self, agent: &Agent, tool: &str, input: &Value, cwd: Option<&str>) -> Judgement {
        let role = policy_role(agent.role);
        let request = self.policy_request(agent, role, tool, input, cwd);
        let verdict = classify_with(&request, &Filesystem);
        let (route, why) = if agent.role == Role::User {
            (ApprovalRoute::User, None)
        } else {
            self.route(agent, role, &verdict)
        };
        let reason = match why {
            Some(why) if matches!(verdict.tier, Tier::Chief { .. }) => format!("{}; {why}", verdict.reason),
            _ => verdict.reason.clone(),
        };
        Judgement {
            verdict,
            role,
            run_id: agent.run_id,
            route,
            reason,
        }
    }

    /// Who answers a request from a run-started agent, and why not the chief.
    fn route(&self, agent: &Agent, role: PolicyRole, verdict: &Verdict) -> (ApprovalRoute, Option<String>) {
        let (limits, ledger) = match agent.run_id.map(|run_id| self.runs.limits(run_id)) {
            Some(Ok(limits)) => (limits.policy(), limits.ledger()),
            Some(Err(error)) => {
                tracing::error!(%error, agent = agent.id, "could not read the run's limits");
                let why = "cos couldn't read the run's limits".to_owned();
                return (ApprovalRoute::User, Some(why));
            }
            None => (Limits::default(), RunLedger::default()),
        };
        match limits.route(verdict, role, &ledger) {
            Route::Allow => (ApprovalRoute::Auto, None),
            Route::Chief if agent.run_id.is_none() => (
                ApprovalRoute::User,
                Some("the agent isn't part of a run, so there's no chief to ask".into()),
            ),
            Route::Chief => (ApprovalRoute::Chief, None),
            Route::User { why } => (ApprovalRoute::User, Some(why)),
        }
    }

    /// The request as `cos-policy` takes it. The worktree is the task's for
    /// workers and reviewers, else the workspace, where agents start. Only a
    /// task opens a scope: the chief and workers without a known task have
    /// nothing in scope, so their edits are never allowed by rule alone.
    fn policy_request(
        &self,
        agent: &Agent,
        role: PolicyRole,
        tool: &str,
        input: &Value,
        cwd: Option<&str>,
    ) -> ApprovalRequest {
        let workspace = self.projects.workspace(agent.workspace_id).ok();
        let task = match agent.role {
            Role::Worker | Role::Reviewer => self.task_of(agent),
            Role::User | Role::Chief => None,
        };
        let (root, scope, test_command) = match (task, workspace) {
            (Some(task), _) if task.worktree.is_some() => (
                task.worktree.clone().unwrap_or_default(),
                Some(task.files),
                task.test_command,
            ),
            (_, Some(Workspace { path, .. })) => {
                let scope = (agent.role != Role::User).then(Vec::new);
                (path, scope, None)
            }
            // No folder to judge against: every path counts as unknown.
            (_, None) => (PathBuf::new(), Some(Vec::new()), None),
        };
        let cwd = cwd
            .filter(|cwd| !cwd.is_empty())
            .map_or_else(|| root.clone(), PathBuf::from);
        let mut request = ApprovalRequest::new(tool, input.clone(), cwd, root);
        request.scope = scope;
        request.role = role;
        request.test_command = test_command;
        request.home = std::env::var_os("HOME").map(PathBuf::from);
        request
    }

    /// The task a worker or reviewer works on, if it belongs to its run.
    fn task_of(&self, agent: &Agent) -> Option<Task> {
        let lookup = self
            .task_lookup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let task = self.runs.task(lookup.task_of(agent)?).ok()?;
        (Some(task.run_id) == agent.run_id).then_some(task)
    }

    /// Adds an answer to the run's audit trail. Agents outside a run have none.
    pub(super) fn audit(
        &self,
        judgement: &Judgement,
        approval: &Approval,
        decided_by: Decider,
        outcome: Outcome,
        reason: &str,
    ) {
        let Some(run_id) = judgement.run_id else {
            return;
        };
        let record = AuditRecord {
            request_id: approval.id.to_string(),
            at: Timestamp::now(),
            role: judgement.role,
            tool: approval.tool_name.clone(),
            summary: summarize(&approval.tool_name, &approval.tool_input),
            tier: judgement.verdict.tier,
            rule: judgement.verdict.reason.clone(),
            decided_by,
            outcome,
            reason: reason.to_owned(),
        };
        if let Err(error) = self.runs.record_audit(run_id, Some(approval.id), &record) {
            tracing::error!(%error, approval = approval.id, "could not record an approval in the audit");
        }
    }

    /// The chief's answer (`approval_decide`). A deny is always accepted. An
    /// allow needs a reason, an approval routed to the chief, and the run's
    /// limits as they are now; if the limits refuse it, the approval goes to
    /// the user and the refusal comes back as [`AgentError::Forbidden`].
    pub(super) fn chief_answer(
        &self,
        approval_id: i64,
        allow: bool,
        reason: &str,
    ) -> Result<Approval, AgentError> {
        let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
        if allow && reason.is_empty() {
            return Err(AgentError::Invalid(
                "say why you allow it; the reason goes in the report".into(),
            ));
        }
        let mut live = self.live();
        let Some(waiting) = live.waiting.get(&approval_id) else {
            return Err(self.not_pending(approval_id));
        };
        let judgement = waiting.judgement.clone();
        let Some(run_id) = judgement.run_id else {
            return Err(AgentError::Forbidden(format!(
                "approval {approval_id} isn't from a run's agent; only the user can answer it"
            )));
        };
        if judgement.role == PolicyRole::UserSession {
            return Err(AgentError::Forbidden(format!(
                "approval {approval_id} is from a session the user started; only they can answer it"
            )));
        }
        let routed = self
            .agents
            .approval(approval_id)?
            .and_then(|approval| approval.route);
        if allow && routed != Some(ApprovalRoute::Chief) {
            return Err(AgentError::Forbidden(format!(
                "approval {approval_id} is waiting for the user, not the chief"
            )));
        }
        let decision = ChiefDecision {
            answer: if allow {
                ChiefAnswer::Allow
            } else {
                ChiefAnswer::Deny
            },
            reason: reason.clone(),
        };
        let checked = self
            .runs
            .apply_chief_decision(run_id, &judgement.verdict, judgement.role, &decision)
            .map_err(run_error)?;
        if let Err(refusal) = checked {
            let why = format!(
                "{}; the chief's approval was refused: {}",
                judgement.verdict.reason, refusal.why
            );
            self.agents
                .route_approval(approval_id, ApprovalRoute::User, Some(&why))?;
            return Err(AgentError::Forbidden(format!(
                "{}; approval {approval_id} now waits for the user",
                refusal.why
            )));
        }

        let waiting = live
            .waiting
            .remove(&approval_id)
            .expect("checked above under the same lock");
        live.hook_approvals.remove(&waiting.hook.id);
        let agent_id = waiting.agent_id;
        let (answer, state, outcome) = if allow {
            (Decision::Allow, ApprovalState::Allowed, Outcome::Allowed)
        } else {
            let message = if reason.is_empty() {
                "The chief denied this.".to_owned()
            } else {
                format!("The chief denied this: {reason}")
            };
            (Decision::Deny { message }, ApprovalState::Denied, Outcome::Denied)
        };
        waiting.hook.answer(&answer);
        let message = (!reason.is_empty()).then_some(reason.as_str());
        let approval = self
            .agents
            .decide_approval(approval_id, state, Some(DecidedBy::Chief), message)?
            .ok_or_else(|| AgentError::Conflict(format!("approval {approval_id} is no longer pending")))?;
        self.audit(&judgement, &approval, Decider::Chief, outcome, &reason);
        self.refresh_state(&live, agent_id, cos_core::AgentState::Working)?;
        Ok(approval)
    }

    /// Why an approval can't be answered: it doesn't exist, or it already
    /// was.
    pub(super) fn not_pending(&self, approval_id: i64) -> AgentError {
        match self.agents.approval(approval_id) {
            Ok(Some(approval)) => AgentError::Conflict(format!(
                "approval {approval_id} is no longer pending (it is {})",
                super::hooks::state_name(approval.state)
            )),
            Ok(None) => AgentError::NotFound(format!("there is no approval {approval_id}")),
            Err(error) => AgentError::Store(error),
        }
    }
}

/// The agent's role as `cos-policy` knows it.
fn policy_role(role: Role) -> PolicyRole {
    match role {
        Role::User => PolicyRole::UserSession,
        Role::Chief => PolicyRole::Chief,
        Role::Worker => PolicyRole::Worker,
        Role::Reviewer => PolicyRole::Reviewer,
    }
}

fn run_error(error: RunError) -> AgentError {
    match error {
        RunError::Invalid(message) => AgentError::Invalid(message),
        RunError::NotFound(message) => AgentError::NotFound(message),
        RunError::Conflict(message) => AgentError::Conflict(message),
        RunError::Store(error) => AgentError::Store(error),
    }
}
