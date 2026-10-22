//! The HTTP client: one `POST` per decision point with all its questions
//! (batching is ~12x cheaper than one call per question, with the same
//! answers), a 1 s timeout, and typed answers.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::answers::RawAnswers;
use crate::key::ApiKey;
use crate::permission::{PermissionAnswers, PermissionInput};
use crate::questions::questions;
use crate::trim::cut;
use crate::wake::{WakeAnswers, WakeInput};

/// TypeSafe's System One endpoint.
pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-latest";
/// Past this, jev has no opinion and things go on as without it.
pub const TIMEOUT: Duration = Duration::from_secs(1);
/// The most characters of an error body kept.
const MAX_ERROR_BODY: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum JudgeError {
    #[error("jev took longer than {}ms", TIMEOUT.as_millis())]
    Timeout,
    #[error("jev answered HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("could not reach jev: {0}")]
    Transport(String),
    #[error("jev's answer is unreadable: {0}")]
    Decode(String),
    #[error("jev answered no `{0}`")]
    Missing(String),
    #[error("jev picked `{option}` for `{question}`, which isn't one of its options")]
    UnknownOption { question: String, option: String },
}

/// One call's typed answers, with what it cost.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Judged<T> {
    pub answers: T,
    /// The model that answered, such as `typesafe/jev-1.13-20260917`.
    pub model: Option<String>,
    pub latency_ms: u64,
}

/// A jev client. Cheap to clone.
#[derive(Clone)]
pub struct JudgeClient {
    http: reqwest::Client,
    endpoint: String,
    model: String,
    key: ApiKey,
    timeout: Duration,
}

impl std::fmt::Debug for JudgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgeClient")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl JudgeClient {
    /// A client for TypeSafe's endpoint.
    pub fn new(key: ApiKey) -> Self {
        Self::with_endpoint(key, ENDPOINT)
    }

    /// A client for another endpoint with the same API (a fake in tests).
    pub fn with_endpoint(key: ApiKey, endpoint: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("cos/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("the TLS backend initialises");
        Self {
            http,
            endpoint: endpoint.into(),
            model: MODEL.to_owned(),
            key,
            timeout: TIMEOUT,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Decision point 1: `action_kind`, `serves_task` and `leaves_worktree`.
    pub async fn permission(&self, input: &PermissionInput) -> Result<Judged<PermissionAnswers>, JudgeError> {
        let point = &questions().points.permission.questions;
        self.ask(input, point, PermissionAnswers::decode).await
    }

    /// Decision point 2: `wake` and `is_question_for_chief`.
    pub async fn wake(&self, input: &WakeInput) -> Result<Judged<WakeAnswers>, JudgeError> {
        let point = &questions().points.wake.questions;
        self.ask(input, point, WakeAnswers::decode).await
    }

    /// One call with every question of a decision point, decoded by
    /// `decode`. Times out after the client's timeout.
    pub async fn ask<S: Serialize, T>(
        &self,
        state: &S,
        questions: &Map<String, Value>,
        decode: impl FnOnce(&RawAnswers) -> Result<T, JudgeError>,
    ) -> Result<Judged<T>, JudgeError> {
        let started = Instant::now();
        let body = json!({ "model": self.model, "state": state, "questions": questions });
        let call = async {
            let response = self
                .http
                .post(&self.endpoint)
                .bearer_auth(self.key.expose())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .send()
                .await
                .map_err(transport)?;
            let status = response.status();
            let text = response.text().await.map_err(transport)?;
            if !status.is_success() {
                return Err(JudgeError::Http {
                    status: status.as_u16(),
                    body: cut(&text, MAX_ERROR_BODY),
                });
            }
            Ok(text)
        };
        let text = tokio::time::timeout(self.timeout, call)
            .await
            .map_err(|_| JudgeError::Timeout)??;
        let response: Value =
            serde_json::from_str(&text).map_err(|error| JudgeError::Decode(error.to_string()))?;
        let answers = response
            .get("answers")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| JudgeError::Decode("the response has no `answers`".into()))?;
        let answers = decode(&RawAnswers { answers })?;
        Ok(Judged {
            answers,
            model: response.get("model").and_then(Value::as_str).map(str::to_owned),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn transport(error: reqwest::Error) -> JudgeError {
    if error.is_timeout() {
        return JudgeError::Timeout;
    }
    // Without the URL: it's fixed, and the message stays short.
    JudgeError::Transport(error.without_url().to_string())
}
