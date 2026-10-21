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

## Interactive sessions in a terminal (PTY)

Every agent cos launches runs Claude's own interactive UI in a pseudo-terminal that cos owns (`ClaudeTerminal` in `cos-harness`), so a person can watch it or type into it from the apps. cos doesn't read the screen to follow the session. It uses hooks instead. Checked against Claude Code 2.1.306 with `cos try --pty`.

**The command:** `claude --session-id <uuid> --permission-mode <mode> [--setting-sources project --strict-mcp-config] --settings '<hooks json>' [--model …] -- "<first prompt>"`. There's no `-p` and no stream-json. The process gets `TERM=xterm-256color`, plus `COS_HOOK_SOCKET` for the relay.

**Hooks:** `--settings` takes inline JSON, and its hooks apply to that one session only, even with `--setting-sources project`. Each of `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `Notification`, `Stop` and `SessionEnd` runs `cos hook`. The relay passes the hook's JSON input to the hook socket and prints what cos answers. Only `PermissionRequest` waits for an answer; the others are answered empty at once. If cos isn't listening, the relay prints nothing and Claude behaves as if there were no hook.

**Hook inputs:** all of them carry `session_id`, `cwd` and `transcript_path`.
- `PreToolUse` and `PostToolUse` have `tool_name`, `tool_input` and `tool_use_id`. `PostToolUse` also has `tool_response` and `duration_ms`.
- `PermissionRequest` has `tool_name`, `tool_input` and `permission_suggestions`, but **no `tool_use_id`**. Pair it with the `PreToolUse` just before it in the same session.
- `Stop` has `last_assistant_message`.
- `SessionEnd` has `reason`, for example `prompt_input_exit`.

**Approvals:** the `PermissionRequest` hook's output is `{"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}}}` or `{… "decision": {"behavior": "deny", "message": "…"}}`. Claude shows its own dialog in the terminal while the hook waits, and whichever answers first wins. The screen then shows "Allowed by PermissionRequest hook".
- **Answered in the terminal first:** Claude doesn't kill the waiting hook straight away. cos saw the relay close only after the turn ended.
- **What cos does about it:** the daemon must treat a pending request as settled once a later hook for that tool arrives (`PostToolUse` with the same `tool_use_id`, the next `PreToolUse`, or `Stop`), not only when the relay closes.
- **Timeout:** cos sets the permission hook's timeout to 24 hours. Claude's actual cap hasn't been checked yet.

**Trust dialog:** in a folder nobody has trusted, Claude first asks "Do you trust the files in this folder?". Nothing runs until it's answered, not even hooks.
- **How Claude decides:** it looks up trust from the folder up to the git root, so a trusted parent folder doesn't cover a separate repo inside it.
- **Worktrees:** a git worktree of a trusted repo is trusted. That covers cos workspaces of any project you've opened in Claude.
- **Who answers:** cos answers the dialog only when told to (`cos try --pty --trust`). Otherwise the person at the terminal does. Detecting the dialog (`asks_for_trust`) is the one place cos reads the screen.

**Terminal queries:** the PTY reader answers cursor-position, status, device-attribute and colour queries, as a terminal would. Without answers some programs pause or guess.

**Inherited environment:** a Claude session sets markers such as `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION` and `CLAUDE_CODE_SESSION_ID` for its children. When cos itself was started from inside Claude, passing them on made the new Claude believe it was a child session and stop saving its transcript. cos removes those markers (`SESSION_MARKERS`) and keeps settings such as `CLAUDE_CONFIG_DIR`.

**Ending:** typing `/exit` and pressing Return ends the session with exit code 0 (`SessionEnd` reason `prompt_input_exit`). Claude clears the screen when it exits.

## Sessions cos didn't start

Each interactive `claude` keeps `$CLAUDE_CONFIG_DIR/sessions/<pid>.json` (the default config dir is `~/.claude`). It holds `pid`, `sessionId`, `cwd`, `kind`, `name`, `status` (`busy` or `idle`), `version`, `statusUpdatedAt` and `procStart`.
- **`procStart`:** the process start time, which Claude writes as `TZ=UTC ps -o lstart` prints it. Files outlive their process, so `cos sessions` lists only entries whose pid is running and started at that time.
- **Transcript:** `<config dir>/projects/<cwd with non-alphanumerics as ->/<sessionId>.jsonl`.
- **Files never read:** the `.key` files next to the session files.

This is read-only: cos doesn't own those terminals, so it can't show their screen or answer their prompts.

## Not verified yet

- `--resume` after a restart.
- Long sessions: queued messages, compaction, subagent output (`parent_tool_use_id`).
- The longest timeout Claude allows a `PermissionRequest` hook, and what it does when the timeout runs out.
- Whether `Notification` hooks fire in a PTY session; none did in the short runs.
