# Driving Claude Code from cos

How `cos-harness` runs Claude Code and answers its permission prompts. Everything here was checked against Claude Code 2.1.306 on 21 Oct 2026. The protocol is the one the Agent SDKs use, but Anthropic doesn't document it as a public contract, so recheck this page when Claude Code updates. Real captures are in `crates/cos-harness/tests/fixtures/`, and the unit tests parse them.

## The command

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --permission-prompts host --permission-prompt-tool stdio \
  --setting-sources project --strict-mcp-config \
  --permission-mode manual --session-id <uuid>
```

`ClaudeOptions::args()` builds this. Three of these flags were found by trial during the spike:

| Flag | Without it |
|---|---|
| `--permission-prompt-tool stdio` | Claude refuses anything that needs approval ("needs approval", `source: config`) and never asks cos. The flag is hidden from `--help`; `--permission-prompts host` alone is not enough. Found in the Agent SDK source, which always passes it when a `canUseTool` callback is set. |
| `--setting-sources project` | Your user settings apply. Your personal allow rules approved `touch` (`source: user_permanent`), and your default `auto` mode let its classifier approve the command. cos never saw either request. |
| `--strict-mcp-config` | Your personal MCP servers load into every worker. |

Also: Claude treats paths under `~/.claude/` as sensitive and refuses writes there whatever cos answers. Use a normal directory such as `.sandbox/` for experiments.

## Messages

One JSON object per line in each direction.

**cos → Claude**

```jsonc
// Handshake, sent first; Claude replies with a control_response.
{"type":"control_request","request_id":"<id>","request":{"subtype":"initialize"}}
// A prompt.
{"type":"user","message":{"role":"user","content":"..."}}
// Stop the current turn. Claude replies with a control_response.
{"type":"control_request","request_id":"<id>","request":{"subtype":"interrupt"}}
// Answer to a permission request. `updatedInput` is required on allow.
{"type":"control_response","response":{"subtype":"success","request_id":"<id>",
  "response":{"behavior":"allow","updatedInput":{...},"toolUseID":"toolu_..."}}}
{"type":"control_response","response":{"subtype":"success","request_id":"<id>",
  "response":{"behavior":"deny","message":"shown to the model as the tool error","toolUseID":"toolu_..."}}}
```

**Claude → cos**

| `type` | Meaning | Becomes |
|---|---|---|
| `system` / `init` | Session is up: id, model, permission mode, tools | `Event::Started` |
| `assistant` | Text and `tool_use` blocks | `Event::Text`, `Event::ToolUse` |
| `control_request` / `can_use_tool` | Needs approval: `tool_name`, `input`, `tool_use_id`, `description`, `blocked_path`, `permission_suggestions` | `Event::PermissionRequest` |
| `user` | `tool_result` blocks, including refusals | `Event::ToolResult` |
| `result` | End of turn: `subtype`, `is_error`, `result`, `total_cost_usd`, `duration_ms`, `permission_denials` | `Event::TurnFinished` |
| `control_response` | Reply to cos's `initialize` or `interrupt` | resolves the waiting call |
| `control_cancel_request` | Claude withdrew a permission request | `Event::PermissionCancelled` |
| `keep_alive` | Nothing | dropped |
| anything else (`rate_limit_event`, `system`/`thinking_tokens`, ...) | Not used yet | `Event::Other` |

Other `control_request` subtypes (hook callbacks, SDK MCP messages) get an error reply, because cos registers none of them.

## Order of a turn that needs approval

```
→ initialize                ← control_response (success)
→ user "..."                ← system/init
                            ← assistant  tool_use Bash {...}
                            ← control_request can_use_tool   (Claude waits here)
→ control_response allow
                            ← user  tool_result "done"
                            ← assistant  text
                            ← result success
→ (close stdin)             process exits 0
```

The tool result carries `tool_result_meta[].permission_decision`. Its `source` (`user_temporary`, `user_reject`, `config`, `user_permanent`, `classifier`) tells you who decided. That's how the spike spotted settings approving things before cos saw them.

## Interrupting

An `interrupt` control request during a running tool ends the turn within about a second. Claude kills the tool's process, reports the tool as rejected, and ends the turn with `result` subtype `error_during_execution`. If cos then closes stdin, Claude exits with code 1. This was checked against the real CLI with a 30-second `ping`.

Claude also refuses some commands on its own before asking cos. For example, it blocks `sleep N && …` and suggests its Monitor tool instead.

## Not verified yet

- `--resume` after a restart.
- Long sessions: queued messages, compaction, subagent output (`parent_tool_use_id`).
