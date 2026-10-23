//! The approvals policy (`cos-policy`) in the hook path. Every permission
//! request is classified against the agent's workspace, and the tier and
//! reason are stored with the approval so the apps can show them. You answer
//! every request: the policy never answers one by itself, and jev
//! (`cos-judge`) only gives its opinion, in shadow mode.

use std::path::PathBuf;

use cos_core::{Agent, Workspace};
use cos_policy::{ApprovalRequest, Filesystem, Role as PolicyRole, Verdict, classify_with};
use serde_json::Value;

use super::{AgentError, Inner};

/// How the policy judged one request; kept with it until it's answered.
#[derive(Debug, Clone)]
pub(super) struct Judgement {
    pub verdict: Verdict,
    /// The request as the policy classified it, for jev.
    pub request: ApprovalRequest,
}

impl Inner {
    /// Classifies a permission request from `agent`. `cwd` is the hook's
    /// working directory.
    pub(super) fn judge(&self, agent: &Agent, tool: &str, input: &Value, cwd: Option<&str>) -> Judgement {
        let request = self.policy_request(agent, tool, input, cwd);
        let verdict = classify_with(&request, &Filesystem);
        Judgement { verdict, request }
    }

    /// The request as `cos-policy` takes it: a session you watch, judged
    /// against its workspace, where agents start, with the whole worktree in
    /// scope.
    fn policy_request(&self, agent: &Agent, tool: &str, input: &Value, cwd: Option<&str>) -> ApprovalRequest {
        let root = match self.projects.workspace(agent.workspace_id) {
            Ok(Workspace { path, .. }) => path,
            // No folder to judge against: every path counts as unknown.
            Err(_) => PathBuf::new(),
        };
        let cwd = cwd
            .filter(|cwd| !cwd.is_empty())
            .map_or_else(|| root.clone(), PathBuf::from);
        let mut request = ApprovalRequest::new(tool, input.clone(), cwd, root);
        request.role = PolicyRole::UserSession;
        request.home = std::env::var_os("HOME").map(PathBuf::from);
        request
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
