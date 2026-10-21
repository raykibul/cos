//! Drives `ClaudeSession` against `cos-fake-claude`, which speaks the same
//! stdin/stdout protocol as the real CLI.

use std::time::Duration;

use cos_harness::{ClaudeOptions, ClaudeSession, Decision, Event, HarnessError, PermissionRequest};
use tokio::sync::mpsc::UnboundedReceiver;

fn fake() -> ClaudeOptions {
    let mut options = ClaudeOptions::new(std::env::temp_dir());
    options.binary = env!("CARGO_BIN_EXE_cos-fake-claude").into();
    options
}

/// Collects events up to and including the first one `stop` matches.
async fn until(events: &mut UnboundedReceiver<Event>, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out; events so far: {seen:#?}"))
            .unwrap_or_else(|| panic!("event stream ended; events so far: {seen:#?}"));
        let done = stop(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn is_turn_end(event: &Event) -> bool {
    matches!(event, Event::TurnFinished(_))
}

async fn permission_request(events: &mut UnboundedReceiver<Event>) -> PermissionRequest {
    let seen = until(events, |event| matches!(event, Event::PermissionRequest(_))).await;
    let Some(Event::PermissionRequest(request)) = seen.last() else {
        unreachable!()
    };
    request.clone()
}

async fn close_and_expect_clean_exit(session: &ClaudeSession, events: &mut UnboundedReceiver<Event>) {
    session.close_input().await;
    let seen = until(events, |event| matches!(event, Event::Exited { .. })).await;
    assert_eq!(seen.last(), Some(&Event::Exited { code: Some(0) }));
}

#[tokio::test]
async fn starts_and_answers_a_prompt() {
    let options = fake();
    let session_id = options.session_id.clone();
    let (session, mut events) = ClaudeSession::spawn(options).await.unwrap();

    session.send("hello").await.unwrap();
    let seen = until(&mut events, is_turn_end).await;

    assert_eq!(
        seen[0],
        Event::Started {
            session_id,
            model: "fake-model".into(),
            permission_mode: "default".into(),
        }
    );
    assert!(seen.contains(&Event::Text("echo: hello".into())));
    close_and_expect_clean_exit(&session, &mut events).await;
}

#[tokio::test]
async fn allowed_tool_runs() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.send("needs permission").await.unwrap();

    let request = permission_request(&mut events).await;
    assert_eq!(request.tool_name, "Bash");
    assert_eq!(request.input["command"], "touch x");
    assert_eq!(request.tool_use_id.as_deref(), Some("toolu_1"));
    assert_eq!(request.blocked_path.as_deref(), Some("/work/x"));
    session
        .answer(&request.request_id, Decision::Allow)
        .await
        .unwrap();

    let seen = until(&mut events, is_turn_end).await;
    assert!(seen.iter().any(|event| matches!(
        event,
        Event::ToolResult { is_error: false, content, .. } if content == "ran: touch x"
    )));
    assert!(seen.contains(&Event::Text("allowed".into())));
    close_and_expect_clean_exit(&session, &mut events).await;
}

#[tokio::test]
async fn denied_tool_shows_the_reason_to_the_model() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.send("needs permission").await.unwrap();

    let request = permission_request(&mut events).await;
    let deny = Decision::Deny {
        message: "not in this task".into(),
    };
    session.answer(&request.request_id, deny).await.unwrap();

    let seen = until(&mut events, is_turn_end).await;
    assert!(seen.iter().any(|event| matches!(
        event,
        Event::ToolResult { is_error: true, content, .. } if content == "not in this task"
    )));
    let Some(Event::TurnFinished(result)) = seen.last() else {
        unreachable!()
    };
    assert_eq!(result.denied_tools, vec!["Bash".to_owned()]);
    close_and_expect_clean_exit(&session, &mut events).await;
}

#[tokio::test]
async fn a_request_is_answered_only_once() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.send("needs permission").await.unwrap();

    let request = permission_request(&mut events).await;
    session
        .answer(&request.request_id, Decision::Allow)
        .await
        .unwrap();

    let again = session.answer(&request.request_id, Decision::Allow).await;
    assert!(matches!(again, Err(HarnessError::UnknownRequest(id)) if id == "perm-1"));
}

#[tokio::test]
async fn withdrawn_request_cannot_be_answered() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.send("cancel").await.unwrap();

    let seen = until(&mut events, is_turn_end).await;
    assert!(seen.contains(&Event::PermissionCancelled {
        request_id: "perm-2".into()
    }));

    let late = session.answer("perm-2", Decision::Allow).await;
    assert!(matches!(late, Err(HarnessError::UnknownRequest(_))));
}

#[tokio::test]
async fn unsupported_control_requests_are_refused() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.send("hook").await.unwrap();

    let seen = until(&mut events, is_turn_end).await;
    assert!(seen.contains(&Event::Text("hook refused".into())));
}

#[tokio::test]
async fn interrupt_is_acknowledged() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.interrupt().await.unwrap();
    close_and_expect_clean_exit(&session, &mut events).await;
}

#[tokio::test]
async fn startup_failure_includes_stderr() {
    let mut options = fake();
    options.env.push(("COS_FAKE_CLAUDE_FAIL".into(), "1".into()));

    let error = ClaudeSession::spawn(options)
        .await
        .err()
        .expect("spawn should fail");

    assert!(matches!(error, HarnessError::Exited { .. }), "{error:?}");
    assert!(error.to_string().contains("not logged in"), "{error}");
}

#[tokio::test]
async fn missing_binary_is_a_spawn_error() {
    let mut options = fake();
    options.binary = "/nonexistent/claude".into();

    let error = ClaudeSession::spawn(options)
        .await
        .err()
        .expect("spawn should fail");

    assert!(matches!(error, HarnessError::Spawn { .. }), "{error:?}");
}

#[tokio::test]
async fn kill_ends_the_session() {
    let (session, mut events) = ClaudeSession::spawn(fake()).await.unwrap();
    session.kill();

    let seen = until(&mut events, |event| matches!(event, Event::Exited { .. })).await;
    assert!(matches!(seen.last(), Some(Event::Exited { code: None })));
}
