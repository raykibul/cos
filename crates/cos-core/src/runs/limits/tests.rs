//! Limits, chief decisions and the audit trail against an in-memory database.

use std::sync::Arc;

use cos_policy::{Decider, Needs, Outcome, Tier};
use jiff::Timestamp;

use super::*;
use crate::{EventLog, NewRun, Store};

fn runs() -> (Runs, EventLog, i64) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let log = EventLog::new(store.clone());
    let workspace = {
        let conn = store.lock();
        conn.execute(
            "INSERT INTO projects (root, name, base_branch, created_ms) VALUES ('/repos/x', 'repo', 'main', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
             VALUES (1, 'oslo', 'oslo', 'cos/oslo', '/wt/oslo', 'head', 'main', 'abc', 0)",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let runs = Runs::new(store, log.clone());
    let run = runs
        .create(NewRun {
            workspace_id: workspace,
            goal: "add limits".into(),
            preset: ApprovalPreset::Trusted,
            approval_cap: Some(1),
        })
        .unwrap();
    (runs, log, run.id)
}

fn install() -> Verdict {
    Verdict::chief(Needs::INSTALL, "installs zod (npm install)")
}

fn allow(reason: &str) -> ChiefDecision {
    ChiefDecision {
        answer: cos_policy::ChiefAnswer::Allow,
        reason: reason.into(),
    }
}

#[test]
fn limits_change_while_the_run_is_open() {
    let (runs, log, id) = runs();
    assert_eq!(
        runs.limits(id).unwrap(),
        RunLimits {
            preset: ApprovalPreset::Trusted,
            approval_cap: 1,
            chief_approvals: 0
        }
    );

    let changed = runs
        .set_limits(
            id,
            &LimitsChange {
                preset: Some(ApprovalPreset::Strict),
                approval_cap: None,
            },
        )
        .unwrap();
    assert_eq!(
        (changed.preset, changed.approval_cap),
        (ApprovalPreset::Strict, 1)
    );
    assert_eq!(runs.run(id).unwrap().preset, ApprovalPreset::Strict);
    let event = log.after(0, 100).unwrap().pop().unwrap();
    assert_eq!(event.kind, kind::RUN_LIMITS_CHANGED);
    assert_eq!(event.run_id.as_deref(), Some(id.to_string().as_str()));

    let too_big = LimitsChange {
        preset: None,
        approval_cap: Some(MAX_APPROVAL_CAP + 1),
    };
    assert!(matches!(runs.set_limits(id, &too_big), Err(RunError::Invalid(_))));
    assert!(matches!(runs.limits(99), Err(RunError::NotFound(_))));
    runs.cancel(id, None).unwrap();
    assert!(matches!(
        runs.set_limits(id, &LimitsChange::default()),
        Err(RunError::Conflict(_))
    ));
}

#[test]
fn chief_approvals_count_against_the_cap() {
    let (runs, _, id) = runs();

    let first = runs
        .apply_chief_decision(id, &install(), PolicyRole::Worker, &allow("adds zod"))
        .unwrap()
        .unwrap();
    assert_eq!(first.chief_approvals, 1);
    let refused = runs
        .apply_chief_decision(id, &install(), PolicyRole::Worker, &allow("again"))
        .unwrap()
        .unwrap_err();
    assert!(refused.why.contains("used all its approvals"), "{refused}");
    // A deny is always fine and costs nothing.
    let deny = ChiefDecision {
        answer: cos_policy::ChiefAnswer::Deny,
        reason: String::new(),
    };
    assert!(
        runs.apply_chief_decision(id, &install(), PolicyRole::Worker, &deny)
            .unwrap()
            .is_ok()
    );
    assert_eq!(runs.limits(id).unwrap().chief_approvals, 1);

    // A preset change holds for answers still to come.
    runs.set_limits(
        id,
        &LimitsChange {
            preset: Some(ApprovalPreset::Normal),
            approval_cap: Some(5),
        },
    )
    .unwrap();
    let refused = runs
        .apply_chief_decision(id, &install(), PolicyRole::Worker, &allow("adds zod"))
        .unwrap()
        .unwrap_err();
    assert!(refused.why.contains("Trusted"), "{refused}");
    runs.cancel(id, None).unwrap();
    let over = runs
        .apply_chief_decision(id, &install(), PolicyRole::Worker, &deny)
        .unwrap()
        .unwrap_err();
    assert!(over.why.contains("cancelled"), "{over}");
}

#[test]
fn the_audit_feeds_the_report_section() {
    let (runs, _, id) = runs();
    let record = |decided_by, outcome, summary: &str| AuditRecord {
        request_id: "1".into(),
        at: Timestamp::now(),
        role: PolicyRole::Worker,
        tool: "Bash".into(),
        summary: summary.into(),
        tier: Tier::Chief {
            needs: Needs::INSTALL,
        },
        rule: "installs zod".into(),
        decided_by,
        outcome,
        reason: "the task adds zod".into(),
    };
    runs.record_audit(
        id,
        Some(1),
        &record(Decider::Chief, Outcome::Allowed, "npm install zod"),
    )
    .unwrap();
    runs.record_audit(
        id,
        Some(2),
        &record(Decider::Rules, Outcome::Allowed, "cargo test"),
    )
    .unwrap();
    runs.record_audit(id, None, &record(Decider::Timeout, Outcome::Denied, "git push"))
        .unwrap();

    let audit = runs.approval_audit(id).unwrap();
    assert_eq!(audit.len(), 3);
    assert_eq!(audit[0].summary, "npm install zod");
    let section = runs.approvals_report_section(id).unwrap();
    assert!(
        section.starts_with("## Approved on your behalf (1)\n"),
        "{section}"
    );
    assert!(
        section.contains("`npm install zod` — Bash, approved by the chief (install)"),
        "{section}"
    );
    assert!(
        section.contains("Rust rules allowed 1 routine requests, 1 went unanswered and were refused."),
        "{section}"
    );
    assert!(runs.approval_audit(99).unwrap().is_empty());
    assert!(matches!(
        runs.approvals_report_section(99),
        Err(RunError::NotFound(_))
    ));
}
