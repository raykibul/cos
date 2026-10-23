//! `cos mcp --agent <id>`: the MCP server `cos` of a project's Chief of
//! Staff. Claude starts it over stdio (from the agent's `--mcp-config`), and
//! it forwards every tool call to the daemon on the socket as that agent, so
//! the daemon knows who is calling. The daemon does the work and the checks;
//! this side only holds the tool list, whose descriptions and schemas are
//! what the chief sees. See "Staff MCP tools" in `docs/chief-of-staff.md`.
//!
//! Built on the official `rmcp` crate. Logging goes to stderr only, since
//! stdout carries the protocol.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
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
    /// The Chief of Staff agent this server speaks for.
    #[arg(long)]
    agent: i64,
}

const INSTRUCTIONS: &str = "Tools for a project's Chief of Staff in cos: list and create workspaces, start agents \
in them, talk to them, read what they said, stop them, and keep your brief. Every call acts on your own project only.";

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
        self.client.staff_tool(self.agent, name, &arguments).await
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

/// The Chief of Staff's tools. The descriptions are its manual.
pub(crate) fn tools() -> Vec<Tool> {
    let base = json!({"type": "string", "enum": ["origin", "head"], "description": "What it starts from: \"origin\" (the freshly fetched base branch) or \"head\" (the main checkout's HEAD, with unpushed commits). Defaults to origin when the project has one."});
    let new_workspace = json!({
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "What to call it. Optional: it is named after a city either way."},
            "from": base
        },
        "additionalProperties": false
    });
    vec![
        tool(
            "workspaces_list",
            "List this project's workspaces (id, title, city, branch, path) and the agents in each: id, title, state, \
             model, spawned_by (your agent id if you started it) and the start of its last message.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            true,
        ),
        tool(
            "workspace_create",
            "Create a workspace: a new git worktree of the project on its own branch. Returns it, with its id. You \
             can also create one while starting an agent (agent_spawn with new_workspace).",
            new_workspace.clone(),
            false,
        ),
        tool(
            "agent_spawn",
            "Start a Claude agent on a job. Give exactly one of workspace_id (an existing workspace) or new_workspace. \
             The prompt is all it knows, so make it complete: what to do, where, and how to know it's done. It reports \
             to you when it finishes a turn. Returns {agent_id, workspace_id}.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "The job, complete enough to do without asking."},
                    "workspace_id": {"type": "integer"},
                    "new_workspace": new_workspace,
                    "title": {"type": "string", "description": "A short name for the agent; defaults to the start of the prompt."},
                    "model": {"type": "string", "description": "A Claude model such as \"sonnet\"; defaults to Claude's default."},
                    "permission_mode": {"type": "string", "enum": ["default", "acceptEdits", "plan", "bypassPermissions"]}
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "agent_send",
            "Type a message into an agent, e.g. a follow-up job, an answer or a correction. A stopped agent is resumed \
             first.",
            json!({
                "type": "object",
                "properties": {"agent_id": {"type": "integer"}, "message": {"type": "string"}},
                "required": ["agent_id", "message"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "agent_read",
            "Read an agent's state and its full last message, and its terminal screen if you ask for it.",
            json!({
                "type": "object",
                "properties": {"agent_id": {"type": "integer"}, "screen": {"type": "boolean", "description": "Also return the terminal's text."}},
                "required": ["agent_id"],
                "additionalProperties": false
            }),
            true,
        ),
        tool(
            "agent_stop",
            "Stop an agent. Its workspace and branch stay; agent_send resumes it later.",
            json!({
                "type": "object",
                "properties": {"agent_id": {"type": "integer"}},
                "required": ["agent_id"],
                "additionalProperties": false
            }),
            false,
        ),
        tool(
            "brief_update",
            "Save your brief, replacing the old one: what you're working on, for whom, and what's open. At most 4 KiB. \
             It is appended to your instructions whenever you restart, so it is all you keep.",
            json!({
                "type": "object",
                "properties": {"brief": {"type": "string"}},
                "required": ["brief"],
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
