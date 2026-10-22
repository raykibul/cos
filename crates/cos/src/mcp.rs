//! `cos mcp --agent <id>`: the chief's MCP server, `cos`. Claude starts it
//! over stdio (from the chief's `--mcp-config`), and it forwards every tool
//! call to the daemon on the socket as that agent, so the daemon knows which
//! chief and run is calling. The daemon does the work and the checks; this
//! side only holds the tool list, whose descriptions and schemas are what
//! the chief sees. See "Chief tools" in `docs/orchestration.md`.
//!
//! Built on the official `rmcp` crate. Logging goes to stderr only, since
//! stdout carries the protocol.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, bail};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};

use crate::client::Client;
use crate::home::CosHome;

#[derive(clap::Args)]
pub(crate) struct McpArgs {
    /// The chief agent this server speaks for.
    #[arg(long)]
    agent: i64,
}

const INSTRUCTIONS: &str = "Tools for the chief of a cos run. cos runs the workers, checks, reviews and merges; \
you plan, decide and report through these tools. Every call acts for this chief's own run.";

pub(crate) async fn run(home: &CosHome, args: McpArgs) -> anyhow::Result<ExitCode> {
    let bridge = Bridge {
        client: Arc::new(Client::new(home.socket())),
        agent: args.agent,
    };
    let service = bridge
        .serve(rmcp::transport::stdio())
        .await
        .context("the MCP client didn't initialize")?;
    service.waiting().await?;
    Ok(ExitCode::SUCCESS)
}

/// Forwards tool calls to the daemon as one agent.
#[derive(Clone)]
pub(crate) struct Bridge {
    pub client: Arc<Client>,
    pub agent: i64,
}

impl Bridge {
    /// Runs one tool and returns the text for the chief, or an error the
    /// chief should read.
    pub(crate) async fn call(&self, name: &str, arguments: Value) -> anyhow::Result<String> {
        if name == "approval_decide" {
            return self.approval_decide(&arguments).await;
        }
        self.client.chief_tool(self.agent, name, &arguments).await
    }

    /// Goes to the approvals component, which checks the decision against
    /// the run's limits. First makes sure the approval is from this chief's run.
    async fn approval_decide(&self, arguments: &Value) -> anyhow::Result<String> {
        let approval_id = arguments["approval_id"]
            .as_i64()
            .context("approval_id must be a number")?;
        let decision = match arguments["decision"].as_str() {
            Some(decision @ ("allow" | "deny")) => decision,
            _ => bail!("decision must be \"allow\" or \"deny\""),
        };
        let reason = arguments["reason"].as_str().unwrap_or_default().trim();
        if reason.is_empty() {
            bail!("give a one-line reason");
        }
        let chief = self.client.agent(self.agent).await?;
        let approval = self.client.approval(approval_id).await?;
        let asker = self.client.agent(approval.agent_id).await?;
        if chief.run_id.is_none() || asker.run_id != chief.run_id {
            bail!("approval {approval_id} is not from your run");
        }
        let decided = self
            .client
            .chief_answer(approval_id, decision == "allow", reason)
            .await?;
        let state = serde_json::to_value(decided.state)
            .ok()
            .and_then(|state| state.as_str().map(str::to_owned))
            .unwrap_or_default();
        Ok(format!(
            "Approval {approval_id} ({}) is now {state}.",
            decided.summary
        ))
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("cos", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.as_ref();
        if !tools().iter().any(|tool| tool.name == name) {
            return Err(ErrorData::invalid_params(
                format!("there is no tool {name}"),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = match self.call(name, arguments).await {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
        };
        Ok(result.into())
    }
}

/// The chief's tools. The descriptions are the chief's manual.
pub(crate) fn tools() -> Vec<Tool> {
    let task_schema = json!({
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "Short and unique within the plan, e.g. \"Add rate limiter\". Other tasks name it in depends_on."},
            "instructions": {"type": "string", "description": "What the worker must do, in enough detail to do it without asking: what to change, where, and how to know it's done. The worker sees only this and the repo."},
            "files": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "The files the worker may change: paths or globs relative to the repo root (src/limit.rs, src/limit/**). A pattern for a folder covers everything in it. cos rejects changes outside this list."},
            "depends_on": {"type": "array", "items": {"type": "string"}, "description": "Titles of tasks in this plan that must be merged first."},
            "test_command": {"type": "string", "description": "A quick shell command run in the task's worktree after the worker finishes, e.g. \"cargo test -p limiter\". The task only goes to review if it passes. Leave out if there is none."}
        },
        "required": ["title", "instructions", "files"],
        "additionalProperties": false
    });
    vec![
        tool(
            "board_get",
            "Read the run's board: the goal, the run's state, and every task with its state, files, branch, \
             dependencies and last failure. Markdown first, then the full JSON. Call it when a wakeup needs details.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            true,
        ),
        tool(
            "board_propose",
            "Propose the plan for the run: 1 to 12 small tasks, each one change that can be reviewed and merged \
             on its own, with the files it may change. Tasks that can run at the same time must not share a file; \
             order them with depends_on instead. Proposing again before the user approves replaces the plan. If \
             the plan has problems, every one is listed in the error: fix them all and propose again. After a \
             plan is accepted, end your turn: the user approves it and cos runs the tasks.",
            json!({
                "type": "object",
                "properties": {"tasks": {"type": "array", "items": task_schema, "minItems": 1, "maxItems": 12}},
                "required": ["tasks"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "task_update",
            "Decide on a task, usually a blocked one: \"retry\" sends it back to a worker (each task gets one \
             retry; say in reason what to do differently, the worker reads it), \"fail\" gives up on it, \"cancel\" \
             drops it. Tasks that depend on a failed or cancelled task are blocked too.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "integer", "description": "The task's id, from the board."},
                    "action": {"type": "string", "enum": ["retry", "fail", "cancel"]},
                    "reason": {"type": "string", "description": "Why, in a sentence or two. For a retry, what the worker should do differently."}
                },
                "required": ["task_id", "action", "reason"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "task_send",
            "Type a message into a running task's worker, e.g. to answer its question or correct its course. Only \
             works while the task is running.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "integer"},
                    "message": {"type": "string", "description": "What the worker should read."}
                },
                "required": ["task_id", "message"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "ask_user",
            "Ask the user a question when a decision needs their judgment (scope, a trade-off, a risky step). \
             Returns at once; the answer arrives in a later wakeup. Don't wait for it in a loop.",
            json!({
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "One clear question, with the context needed to answer it."},
                    "choices": {"type": "array", "items": {"type": "string"}, "maxItems": 10, "description": "Suggested answers, if the choice is between a few options. The user may still answer freely."}
                },
                "required": ["question"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "approval_decide",
            "Allow or deny a worker's permission request that cos routed to you (a wakeup names it). Allow only \
             what the task needs and what stays inside its worktree; deny anything else. cos checks your decision \
             against the user's limits and may refuse it, in which case the user decides.",
            json!({
                "type": "object",
                "properties": {
                    "approval_id": {"type": "integer"},
                    "decision": {"type": "string", "enum": ["allow", "deny"]},
                    "reason": {"type": "string", "description": "One line; it goes in the user's report."}
                },
                "required": ["approval_id", "decision", "reason"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "report_write",
            "Write the run's final report once every task is merged, failed or cancelled (a wakeup says when). \
             Markdown: what was done task by task, what wasn't and why, and anything the user should check or \
             do next. Saving it finishes the run.",
            json!({
                "type": "object",
                "properties": {"markdown": {"type": "string"}},
                "required": ["markdown"],
                "additionalProperties": false
            }),
            false,
        ),
    ]
}

fn tool(name: &'static str, description: &'static str, schema: Value, read_only: bool) -> Tool {
    let Value::Object(schema) = schema else {
        unreachable!("tool schemas are objects")
    };
    let schema: Map<String, Value> = schema;
    let mut tool = Tool::new(name, description, Arc::new(schema));
    tool.annotations = Some(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(false)
            .open_world(false),
    );
    tool
}

#[cfg(test)]
mod tests;
