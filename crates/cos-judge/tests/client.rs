//! The client against a fake jev server on 127.0.0.1. No real calls.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use cos_judge::permission::{ActionKind, PermissionInput, RawRequest};
use cos_judge::wake::{ChiefKind, Wake, WakeInput};
use cos_judge::{ApiKey, JudgeClient, JudgeError};
use serde_json::{Value, json};

#[derive(Clone)]
struct Fake {
    /// What to answer: status and body.
    reply: Arc<Mutex<(StatusCode, Value)>>,
    delay: Duration,
    /// Every request: its Authorization header and body.
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn decide(State(fake): State<Fake>, headers: HeaderMap, body: String) -> (StatusCode, String) {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    fake.seen.lock().unwrap().push((auth, body));
    tokio::time::sleep(fake.delay).await;
    let (status, reply) = fake.reply.lock().unwrap().clone();
    (status, reply.to_string())
}

async fn serve(status: StatusCode, reply: Value, delay: Duration) -> (String, Fake) {
    let fake = Fake {
        reply: Arc::new(Mutex::new((status, reply))),
        delay,
        seen: Arc::default(),
    };
    let app = Router::new()
        .route("/v1/systemone", post(decide))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/v1/systemone"), fake)
}

fn client(endpoint: &str) -> JudgeClient {
    JudgeClient::with_endpoint(ApiKey::new("sk-test").unwrap(), endpoint)
}

fn permission_input() -> PermissionInput {
    let input = json!({ "command": "ls", "description": "List files" });
    PermissionInput::from_request(&RawRequest {
        tool: "Bash",
        input: &input,
        cwd: None,
        worktree_root: std::path::Path::new("/w"),
        home: None,
        agent_role: "worker",
        task_title: Some("List the files"),
        task_instructions: Some("Run ls."),
        policy_tier: "ask you",
        policy_reason: "synthetic",
    })
}

fn permission_reply(kind: &str) -> Value {
    json!({
        "model": "typesafe/jev-1.13-20260917",
        "answers": {
            "action_kind": {
                "type": "choice", "choice": kind, "confidence": 0.97,
                "probabilities": { "read_only": 0.97, "build_or_test": 0.01, "edit_in_worktree": 0.01,
                                   "install_dep": 0.0, "external": 0.01, "destructive": 0.0 }
            },
            "serves_task": { "type": "noul", "noul": 0.93 },
            "leaves_worktree": { "type": "noul", "noul": 0.02 }
        },
        "usage": { "input_tokens": 1500, "output_tokens": 30 }
    })
}

#[tokio::test]
async fn one_call_with_every_question_and_typed_answers() {
    let (endpoint, fake) = serve(StatusCode::OK, permission_reply("read_only"), Duration::ZERO).await;

    let judged = client(&endpoint).permission(&permission_input()).await.unwrap();

    assert_eq!(judged.answers.action_kind.picked, ActionKind::ReadOnly);
    assert_eq!(judged.answers.action_kind.confidence, 0.97);
    assert_eq!(judged.answers.serves_task.probability, 0.93);
    assert_eq!(judged.model.as_deref(), Some("typesafe/jev-1.13-20260917"));

    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one call per decision point");
    let (auth, body) = &seen[0];
    assert_eq!(auth, "Bearer sk-test");
    assert_eq!(body["model"], "jev-latest");
    let questions: Vec<&str> = body["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(questions, ["action_kind", "leaves_worktree", "serves_task"]);
    assert_eq!(body["state"]["request"]["input"]["command"], "ls");
    assert_eq!(body["state"]["task"]["title"], "List the files");
}

#[tokio::test]
async fn wake_answers_decode_into_the_enum() {
    let reply = json!({
        "answers": {
            "wake": { "type": "choice", "choice": "batch", "confidence": 0.81, "probabilities": { "now": 0.1, "batch": 0.81, "none": 0.09 } },
            "is_question_for_chief": { "type": "noul", "noul": 0.04 }
        }
    });
    let (endpoint, _fake) = serve(StatusCode::OK, reply, Duration::ZERO).await;
    let input = WakeInput::new(
        ChiefKind::RunChief,
        "task",
        "Task 2 \"Add subtract\" merged.",
        Some("calc"),
    );

    let judged = client(&endpoint).wake(&input).await.unwrap();

    assert_eq!(judged.answers.wake.picked, Wake::Batch);
    assert_eq!(judged.answers.is_question_for_chief.probability, 0.04);
}

#[tokio::test]
async fn an_unknown_option_is_an_error() {
    let (endpoint, _fake) = serve(StatusCode::OK, permission_reply("teleport"), Duration::ZERO).await;

    let error = client(&endpoint)
        .permission(&permission_input())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, JudgeError::UnknownOption { question, option } if question == "action_kind" && option == "teleport"),
        "{error:?}"
    );
}

#[tokio::test]
async fn slower_than_the_timeout_is_a_timeout() {
    let (endpoint, _fake) = serve(
        StatusCode::OK,
        permission_reply("read_only"),
        Duration::from_millis(600),
    )
    .await;

    let started = std::time::Instant::now();
    let error = client(&endpoint)
        .timeout(Duration::from_millis(200))
        .permission(&permission_input())
        .await
        .unwrap_err();

    assert!(matches!(error, JudgeError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(550));
}

#[tokio::test]
async fn the_default_timeout_is_one_second() {
    let (endpoint, _fake) = serve(
        StatusCode::OK,
        permission_reply("read_only"),
        Duration::from_millis(1_300),
    )
    .await;

    let error = client(&endpoint)
        .permission(&permission_input())
        .await
        .unwrap_err();

    assert!(matches!(error, JudgeError::Timeout), "{error:?}");
}

#[tokio::test]
async fn http_errors_keep_the_status_and_a_short_body() {
    let (endpoint, _fake) = serve(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "questions.action_kind.criteria: bad", "padding": "z".repeat(2_000) }),
        Duration::ZERO,
    )
    .await;

    let error = client(&endpoint)
        .permission(&permission_input())
        .await
        .unwrap_err();

    match error {
        JudgeError::Http { status, body } => {
            assert_eq!(status, 422);
            assert!(body.contains("questions.action_kind"));
            assert!(body.chars().count() <= 301);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_reply_without_answers_or_an_answer_is_a_decode_error() {
    let (endpoint, fake) = serve(StatusCode::OK, json!({ "nope": 1 }), Duration::ZERO).await;
    let first = client(&endpoint)
        .permission(&permission_input())
        .await
        .unwrap_err();
    *fake.reply.lock().unwrap() = (
        StatusCode::OK,
        json!({ "answers": { "action_kind": permission_reply("read_only")["answers"]["action_kind"] } }),
    );
    let second = client(&endpoint)
        .permission(&permission_input())
        .await
        .unwrap_err();

    assert!(matches!(first, JudgeError::Decode(_)), "{first:?}");
    assert!(
        matches!(second, JudgeError::Missing(ref name) if name == "serves_task"),
        "{second:?}"
    );
}

#[tokio::test]
async fn nobody_listening_is_a_transport_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = client(&format!("http://{address}/v1/systemone"))
        .permission(&permission_input())
        .await
        .unwrap_err();

    assert!(matches!(error, JudgeError::Transport(_)), "{error:?}");
    assert!(!error.to_string().contains("sk-test"));
}
