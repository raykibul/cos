//! The approvals policy in the hook path: every request is classified, with
//! its tier and reason stored, and waits for you, even what Rust rules would
//! allow. The fake's `ask:<Tool> <arg>` prompts make the requests.

use cos_core::events::kind;
use cos_core::{ApprovalState, ApprovalTier, DecidedBy};

use super::{Fixture, status};

#[tokio::test]
async fn every_request_is_classified_and_waits_for_you() {
    let fixture = Fixture::start().await;
    let client = &fixture.client;
    let root = fixture.projects.workspace(fixture.workspace_id).unwrap().path;

    // A read inside the workspace: rules would allow it, but it's yours.
    let read = format!("ask:Read {}", root.join("README").display());
    let agent = fixture.start_agent(&read).await;
    let approval = fixture.pending_approval(agent.id).await;
    assert_eq!(approval.tier, Some(ApprovalTier::Allowed));
    assert!(approval.reason.is_some(), "{approval:?}");
    let json = serde_json::to_value(&approval).unwrap();
    assert!(json.get("route").is_none(), "{json}");
    client.answer_approval(approval.id, true, None).await.unwrap();
    fixture.until_idle_with(agent.id, "allowed by hook").await;

    // A push is never delegated, and still comes to you the same way.
    client.say(agent.id, "ask:Bash git push".into()).await.unwrap();
    let push = fixture.pending_approval(agent.id).await;
    assert_eq!(push.tier, Some(ApprovalTier::NeverDelegated));
    assert!(push.reason.as_deref().unwrap().contains("push"), "{push:?}");
    let denied = client
        .answer_approval(push.id, false, Some("not yet".into()))
        .await
        .unwrap();
    assert_eq!(
        (denied.state, denied.decided_by),
        (ApprovalState::Denied, Some(DecidedBy::User))
    );
    assert_eq!(
        status(client.answer_approval(push.id, true, None).await.unwrap_err()),
        409
    );

    let kinds = fixture.kinds();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| *kind == kind::APPROVAL_REQUESTED)
            .count(),
        2
    );
}
