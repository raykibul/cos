use serde_json::Value;

/// Something that happened in an agent session, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The agent is ready. Comes before anything the model says.
    Started {
        session_id: String,
        model: String,
        permission_mode: String,
    },
    /// Text the model wrote.
    Text(String),
    /// The model called a tool. A [`Event::PermissionRequest`] follows if the
    /// call needs approval.
    ToolUse { id: String, name: String, input: Value },
    /// A tool finished, or was refused.
    ToolResult {
        tool_use_id: String,
        is_error: bool,
        content: String,
    },
    /// The agent is waiting for a decision before it runs a tool. Answer it
    /// with `ClaudeSession::answer`.
    PermissionRequest(PermissionRequest),
    /// The agent withdrew a permission request, for example after an
    /// interrupt. It can no longer be answered.
    PermissionCancelled { request_id: String },
    /// The model finished its turn and is waiting for the next message.
    TurnFinished(TurnResult),
    /// A line the agent wrote to stderr.
    Stderr(String),
    /// A message cos doesn't interpret yet, kept as it arrived.
    Other(Value),
    /// The agent process exited. Always the last event.
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    pub tool_use_id: Option<String>,
    /// The agent's one-line description of the action, when it gave one.
    pub description: Option<String>,
    /// The file path that made the action need approval, when there is one.
    pub blocked_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run the tool with the input it asked for.
    Allow,
    /// Don't run it. The model sees `message` as the tool's error.
    Deny { message: String },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnResult {
    pub is_error: bool,
    /// `success`, or the kind of failure, such as `error_max_turns`.
    pub subtype: String,
    /// The model's final message for the turn.
    pub text: Option<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    /// Names of the tools that were refused during the turn.
    pub denied_tools: Vec<String>,
}
