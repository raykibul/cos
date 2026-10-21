//! The approvals policy in the hook path: run agents' requests answered by
//! Rust rules, routed to the chief within the run's limits, or sent to you.
//! The fake's `ask:<Tool> <arg>` prompts make the requests.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cos_core::events::kind;
use cos_core::{
    Agent, ApprovalPreset, ApprovalRoute, ApprovalState, ApprovalTier, DecidedBy, LimitsChange, NewRun,
    PlanProposal, ProposedTask, Role, Runs,
};
use cos_policy::{Decider, Outcome};

use super::super::{StartAgent, TaskLookup};
use super::{Fixture, status, until};

/// Says every worker and reviewer works on one task.
struct OneTask(i64);

impl TaskLookup for OneTask {
    fn task_of(&self, agent: &Agent) -> Option<i64> {
        matches!(agent.role, Role::Worker | Role::Reviewer).then_some(self.0)
    }
}

/// A fixture with a running run whose one task (`src/**`, tested with
/// `cargo test`) works in the workspace folder, where agents start.
struct RunFixture {
    fixture: Fixture,
    runs: Runs,
    run_id: i64,
    root: PathBuf,
}

impl RunFixture {
    async fn start(preset: ApprovalPreset, cap: u32) -> Self {
        Self::start_with(preset, cap, Duration::from_secs(300)).await
    }

    async fn start_with(preset: ApprovalPreset, cap: u32, timeout: Duration) -> Self {
        let fixture = Fixture::start_with(|config| config.approval_timeout = timeout).await;
        let runs = Runs::new(fixture.store.clone(), fixture.log.clone());
        let run = runs
            .create(NewRun {
                workspace_id: fixture.workspace_id,
                goal: "add limits".into(),
                preset,
                approval_cap: Some(cap),
            })
            .unwrap();
        let task = ProposedTask {
            title: "limits".into(),
            instructions: "add limits".into(),
            files: vec!["src/**".into()],
            test_command: Some("cargo test".into()),
            depends_on: Vec::new(),
        };
        runs.propose_plan(run.id, &PlanProposal { tasks: vec![task] })
            .unwrap();
        let task_id = runs.approve_plan(run.id).unwrap().tasks[0].id;
        let root = fixture.projects.workspace(fixture.workspace_id).unwrap().path;
        runs.assign_worktree(task_id, "cos/oslo-1", &root).unwrap();
        fixture.manager.set_task_lookup(Arc::new(OneTask(task_id)));
        Self {
            fixture,
            runs,
            run_id: run.id,
            root,
        }
    }

    fn agent(&self, role: Role, prompt: &str) -> Agent {
        let request = StartAgent {
            prompt: Some(prompt.into()),
            user_settings: Some(false),
            ..StartAgent::default()
        };
        self.fixture
            .manager
            .start_agent(self.fixture.workspace_id, role, Some(self.run_id), &request)
            .unwrap()
    }

    fn path(&self, rel: &str) -> String {
        self.root.join(rel).display().to_string()
    }

    fn audit(&self) -> Vec<(Decider, Outcome, String)> {
        self.runs
            .approval_audit(self.run_id)
            .unwrap()
            .into_iter()
            .map(|record| (record.decided_by, record.outcome, record.summary))
            .collect()
    }

    fn set_limits(&self, preset: Option<ApprovalPreset>, cap: Option<u32>) {
        self.runs
            .set_limits(
                self.run_id,
                &LimitsChange {
                    preset,
                    approval_cap: cap,
                },
            )
            .unwrap();
    }
}

#[tokio::test]
async fn in_scope_work_is_allowed_by_rule_and_the_rest_goes_to_the_chief() {
    let run = RunFixture::start(ApprovalPreset::Normal, 20).await;
    let fixture = &run.fixture;
    let client = &fixture.client;

    let worker = run.agent(Role::Worker, &format!("ask:Write {}", run.path("src/limits.rs")));
    fixture.until_idle_with(worker.id, "allowed by hook").await;
    let ruled = client.approvals(None).await.unwrap().remove(0);
    assert_eq!(
        (ruled.state, ruled.decided_by, ruled.route, ruled.tier),
        (
            ApprovalState::Allowed,
            Some(DecidedBy::Rule),
            Some(ApprovalRoute::Auto),
            Some(ApprovalTier::Allowed)
        )
    );
    assert!(
        !fixture
            .kinds()
            .iter()
            .any(|kind| kind == kind::APPROVAL_REQUESTED),
        "nobody is asked about an auto-allowed request"
    );

    // The task's test command, with extra arguments, too.
    client
        .say(worker.id, "ask:Bash cargo test -p cos".into())
        .await
        .unwrap();
    until("the test command to be allowed", || async {
        (client.approvals(None).await.ok()?.len() == 2).then_some(())
    })
    .await;
    fixture.until_idle_with(worker.id, "allowed by hook").await;

    // Outside the task's scope: the chief may approve it under Normal.
    client
        .say(worker.id, format!("ask:Write {}", run.path("docs/limits.md")))
        .await
        .unwrap();
    let routed = fixture.pending_approval(worker.id).await;
    assert_eq!(
        (routed.route, routed.tier),
        (Some(ApprovalRoute::Chief), Some(ApprovalTier::Chief))
    );
    assert!(
        routed
            .reason
            .as_deref()
            .unwrap()
            .contains("outside the task's scope"),
        "{routed:?}"
    );
    let event = fixture
        .log
        .after(0, 1000)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == kind::APPROVAL_ROUTED)
        .unwrap();
    assert_eq!(event.data["route"], "chief");
    assert_eq!(event.data["run_id"], run.run_id);
    assert_eq!(event.run_id, Some(run.run_id.to_string()));

    // An allow needs a reason.
    assert_eq!(
        status(client.chief_answer(routed.id, true, " ").await.unwrap_err()),
        400
    );
    let allowed = client
        .chief_answer(routed.id, true, "the docs describe the new limits")
        .await
        .unwrap();
    assert_eq!(
        (allowed.state, allowed.decided_by, allowed.message.as_deref()),
        (
            ApprovalState::Allowed,
            Some(DecidedBy::Chief),
            Some("the docs describe the new limits")
        )
    );
    fixture.until_idle_with(worker.id, "allowed by hook").await;
    assert_eq!(client.run_limits(run.run_id).await.unwrap().chief_approvals, 1);
    assert_eq!(
        status(client.chief_answer(routed.id, false, "").await.unwrap_err()),
        409
    );
    assert_eq!(
        status(client.chief_answer(999, false, "").await.unwrap_err()),
        404
    );

    let docs = run.path("docs/limits.md");
    assert_eq!(
        run.audit(),
        [
            (
                Decider::Rules,
                Outcome::Allowed,
                format!("Write {}", run.path("src/limits.rs"))
            ),
            (Decider::Rules, Outcome::Allowed, "cargo test -p cos".into()),
            (Decider::Chief, Outcome::Allowed, format!("Write {docs}")),
        ]
    );
    let section = run.runs.approvals_report_section(run.run_id).unwrap();
    assert!(section.starts_with("## Approved on your behalf (1)"), "{section}");
    assert!(
        section.contains(&format!(
            "`Write {docs}` — Write, approved by the chief (worktree)"
        )),
        "{section}"
    );
    assert!(
        section.contains("Reason: the docs describe the new limits"),
        "{section}"
    );
    assert!(
        section.contains("Rust rules allowed 2 routine requests."),
        "{section}"
    );
}

#[tokio::test]
async fn pushes_and_installs_need_you_unless_the_preset_lets_the_chief() {
    let run = RunFixture::start(ApprovalPreset::Normal, 20).await;
    let fixture = &run.fixture;
    let client = &fixture.client;

    let worker = run.agent(Role::Worker, "ask:Bash git push origin HEAD");
    let push = fixture.pending_approval(worker.id).await;
    assert_eq!(
        (push.route, push.tier),
        (Some(ApprovalRoute::User), Some(ApprovalTier::NeverDelegated))
    );
    // The chief may not allow it, but a deny is always fine.
    assert_eq!(
        status(client.chief_answer(push.id, true, "ship it").await.unwrap_err()),
        403
    );
    let denied = client
        .chief_answer(push.id, false, "pushing isn't part of the task")
        .await
        .unwrap();
    assert_eq!(
        (denied.state, denied.decided_by),
        (ApprovalState::Denied, Some(DecidedBy::Chief))
    );
    fixture
        .until_idle_with(
            worker.id,
            "denied by hook: The chief denied this: pushing isn't part of the task",
        )
        .await;

    // Normal: installs need you, and the reason says why.
    client
        .say(worker.id, "ask:Bash npm install zod".into())
        .await
        .unwrap();
    let install = fixture.pending_approval(worker.id).await;
    assert_eq!(
        (install.route, install.tier),
        (Some(ApprovalRoute::User), Some(ApprovalTier::Chief))
    );
    assert!(
        install.reason.as_deref().unwrap().contains("Trusted preset"),
        "{install:?}"
    );
    client
        .answer_approval(install.id, false, Some("not yet".into()))
        .await
        .unwrap();
    fixture
        .until_idle_with(worker.id, "denied by hook: not yet")
        .await;

    // Trusted: the chief may answer installs.
    run.set_limits(Some(ApprovalPreset::Trusted), None);
    client
        .say(worker.id, "ask:Bash npm install zod".into())
        .await
        .unwrap();
    let install = fixture.pending_approval(worker.id).await;
    assert_eq!(install.route, Some(ApprovalRoute::Chief));
    client
        .chief_answer(install.id, true, "the task adds zod")
        .await
        .unwrap();
    fixture.until_idle_with(worker.id, "allowed by hook").await;

    let audit: Vec<(Decider, Outcome)> = run
        .audit()
        .into_iter()
        .map(|(by, outcome, _)| (by, outcome))
        .collect();
    assert_eq!(
        audit,
        [
            (Decider::Chief, Outcome::Denied),
            (Decider::User, Outcome::Denied),
            (Decider::Chief, Outcome::Allowed),
        ]
    );
    let section = run.runs.approvals_report_section(run.run_id).unwrap();
    assert!(
        section.contains("`npm install zod` — Bash, approved by the chief (install)"),
        "{section}"
    );
    assert!(
        section.contains("The chief refused 1, you answered 1."),
        "{section}"
    );
}

#[tokio::test]
async fn a_refused_chief_answer_goes_to_you() {
    let run = RunFixture::start(ApprovalPreset::Trusted, 1).await;
    let fixture = &run.fixture;
    let client = &fixture.client;
    let first = run.agent(Role::Worker, "ask:Bash npm install zod");
    let second = run.agent(Role::Worker, "ask:Bash npm install left-pad");
    let a = fixture.pending_approval(first.id).await;
    let b = fixture.pending_approval(second.id).await;
    assert_eq!(
        (a.route, b.route),
        (Some(ApprovalRoute::Chief), Some(ApprovalRoute::Chief))
    );

    client.chief_answer(a.id, true, "adds zod").await.unwrap();
    // The cap is reached in the meantime.
    let refused = client
        .chief_answer(b.id, true, "adds left-pad")
        .await
        .unwrap_err();
    assert_eq!(status(refused), 403);
    let b = client
        .approvals(Some("pending"))
        .await
        .unwrap()
        .into_iter()
        .find(|approval| approval.id == b.id)
        .unwrap();
    assert_eq!(b.route, Some(ApprovalRoute::User));
    assert!(
        b.reason.as_deref().unwrap().contains("used all its approvals"),
        "{b:?}"
    );
    let routed: Vec<String> = fixture
        .log
        .after(0, 1000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind::APPROVAL_ROUTED)
        .map(|event| event.data["route"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(routed, ["chief", "chief", "user"]);
    // Now it's yours: the chief can't allow it any more.
    assert_eq!(
        status(client.chief_answer(b.id, true, "please").await.unwrap_err()),
        403
    );
    client.answer_approval(b.id, true, None).await.unwrap();
    fixture.until_idle_with(second.id, "allowed by hook").await;

    // A preset change applies to requests already routed to the chief.
    run.set_limits(None, Some(10));
    client
        .say(first.id, "ask:Bash npm install zod".into())
        .await
        .unwrap();
    let c = fixture.pending_approval(first.id).await;
    assert_eq!(c.route, Some(ApprovalRoute::Chief));
    run.set_limits(Some(ApprovalPreset::Normal), None);
    let refused = client.chief_answer(c.id, true, "adds zod").await.unwrap_err();
    assert!(refused.to_string().contains("Trusted preset"), "{refused}");
    let limits = client.run_limits(run.run_id).await.unwrap();
    assert_eq!(
        (limits.preset, limits.approval_cap, limits.chief_approvals),
        (ApprovalPreset::Normal, 10, 1)
    );
}

#[tokio::test]
async fn unanswered_requests_of_run_agents_expire_into_the_audit() {
    let run = RunFixture::start_with(ApprovalPreset::Normal, 20, Duration::from_millis(300)).await;
    let fixture = &run.fixture;
    let worker = run.agent(Role::Worker, "ask:Bash git push");

    fixture
        .until_idle_with(
            worker.id,
            &format!("denied by hook: {}", cos_policy::timeout::UNANSWERED_MESSAGE),
        )
        .await;
    let expired = fixture.client.approvals(None).await.unwrap().remove(0);
    assert_eq!(
        (expired.state, expired.decided_by),
        (ApprovalState::Expired, None)
    );
    assert_eq!(
        run.audit(),
        [(Decider::Timeout, Outcome::Denied, "git push".into())]
    );
    let section = run.runs.approvals_report_section(run.run_id).unwrap();
    assert!(
        section.contains("1 went unanswered and were refused"),
        "{section}"
    );
}

#[tokio::test]
async fn the_chief_and_your_own_agents_ask_you() {
    let run = RunFixture::start(ApprovalPreset::Trusted, 20).await;
    let fixture = &run.fixture;
    let client = &fixture.client;

    // The chief reads freely but can't approve its own requests.
    let chief = run.agent(Role::Chief, &format!("ask:Read {}", run.path("README")));
    fixture.until_idle_with(chief.id, "allowed by hook").await;
    client.say(chief.id, "ask:Bash cargo build".into()).await.unwrap();
    let build = fixture.pending_approval(chief.id).await;
    assert_eq!(
        (build.route, build.tier),
        (Some(ApprovalRoute::User), Some(ApprovalTier::Chief))
    );
    assert!(
        build.reason.as_deref().unwrap().contains("its own requests"),
        "{build:?}"
    );
    client.answer_approval(build.id, true, None).await.unwrap();

    // A reviewer's edits ask you.
    let reviewer = run.agent(Role::Reviewer, &format!("ask:Write {}", run.path("src/x.rs")));
    let edit = fixture.pending_approval(reviewer.id).await;
    assert_eq!(
        (edit.route, edit.tier),
        (Some(ApprovalRoute::User), Some(ApprovalTier::AskUser))
    );
    client.answer_approval(edit.id, false, None).await.unwrap();

    // Your own agent: always you, even for what rules would allow, and the
    // chief can't answer for you.
    let mine = fixture
        .start_agent(&format!("ask:Read {}", run.path("README")))
        .await;
    let read = fixture.pending_approval(mine.id).await;
    assert_eq!(
        (read.route, read.tier),
        (Some(ApprovalRoute::User), Some(ApprovalTier::Allowed))
    );
    assert_eq!(
        status(client.chief_answer(read.id, false, "no").await.unwrap_err()),
        403
    );
    client.answer_approval(read.id, true, None).await.unwrap();
    fixture.until_idle_with(mine.id, "allowed by hook").await;
}

#[tokio::test]
async fn limits_are_read_and_changed_per_run() {
    let run = RunFixture::start(ApprovalPreset::Normal, 20).await;
    let client = &run.fixture.client;

    let limits = client.run_limits(run.run_id).await.unwrap();
    assert_eq!(
        (limits.preset, limits.approval_cap, limits.chief_approvals),
        (ApprovalPreset::Normal, 20, 0)
    );
    let change = LimitsChange {
        preset: Some(ApprovalPreset::Strict),
        approval_cap: Some(5),
    };
    let changed = client.set_run_limits(run.run_id, &change).await.unwrap();
    assert_eq!(
        (changed.preset, changed.approval_cap),
        (ApprovalPreset::Strict, 5)
    );
    assert_eq!(
        client.board(run.run_id).await.unwrap().run.preset,
        ApprovalPreset::Strict
    );
    assert!(
        run.fixture
            .kinds()
            .iter()
            .any(|kind| kind == kind::RUN_LIMITS_CHANGED)
    );

    let too_big = LimitsChange {
        preset: None,
        approval_cap: Some(5_000),
    };
    assert_eq!(
        status(client.set_run_limits(run.run_id, &too_big).await.unwrap_err()),
        400
    );
    assert_eq!(status(client.run_limits(999).await.unwrap_err()), 404);
    client.cancel_run(run.run_id, None).await.unwrap();
    assert_eq!(
        status(client.set_run_limits(run.run_id, &change).await.unwrap_err()),
        409
    );
}
