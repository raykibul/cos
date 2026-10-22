# Agents API (contract)

How the daemon runs agents in terminals and how the apps show them. The daemon side (Rust) and the apps (CosKit and the Mac app) are built against this document in parallel. Change it only together with both sides.

Everything is on the existing HTTP API: the Unix socket `~/.cos/cos.sock`, and the tailnet listener when remote access is on (see "Remote access" in [daemon.md](daemon.md)). JSON errors look like `{"error": "…"}`, with 400, 404 and 409 as elsewhere in [daemon.md](daemon.md). Times are Unix milliseconds.

## Concepts

- **Agent:** one Claude Code session that the daemon started, running Claude's own UI in a PTY the daemon owns, in a workspace's folder. Each agent has a `role`:
  - `user`: you started it from an app or the CLI.
  - `chief`, `worker`, `reviewer`: started by a run (later).
- **Several agents per workspace:** a workspace can hold several agents at once, sharing its folder.
- **Followed through hooks:** the daemon follows each agent through hooks (see [claude-protocol.md](claude-protocol.md)), never by reading the screen.
- **Approval:** a permission request from an agent, waiting for an answer. It can be answered:
  - in the app or CLI (`allow` or `deny`),
  - by someone typing in the agent's terminal (then it's `settled`),
  - for agents a run starts: by Rust rules straight away, or by the run's chief within your limits (see [approvals.md](approvals.md#daemon-wiring)). Agents you start always ask you.

## Agent JSON

```json
{
  "id": 7,
  "workspace_id": 3,
  "run_id": null,
  "task_id": null,
  "role": "user",
  "title": "fix login bug",
  "session_id": "5504b543-4897-4751-add5-f81f24a6316c",
  "pid": 25615,
  "state": "working",
  "model": "sonnet",
  "permission_mode": "default",
  "user_settings": true,
  "rows": 40,
  "cols": 120,
  "started_ms": 1792611174811,
  "last_activity_ms": 1792611180000,
  "exited_ms": null,
  "exit_code": null,
  "last_message": "Done. I created hello.txt …",
  "pending_approval_id": null,
  "spawned_by": null,
  "transcript_path": "/Users/me/.claude/projects/-Users-me-.cos-worktrees-app-1-lisbon/5504b543-….jsonl"
}
```

`state` is one of:

| State | Meaning |
|---|---|
| `starting` | spawned; no hook yet |
| `needs_trust` | Claude is showing its trust dialog (the folder isn't trusted) |
| `working` | a prompt is running (after `UserPromptSubmit` or `PreToolUse`) |
| `waiting_approval` | a `PermissionRequest` is open |
| `idle` | the turn ended (`Stop`), waiting for input |
| `exited` | the process ended. `exit_code` is null if it was killed. |
| `interrupted` | it was running when the daemon stopped. `POST …/resume` brings it back. |

`last_message` is the `Stop` hook's `last_assistant_message`, cut to 2,000 characters.

- **`role`:** `user` (you, or a Chief of Staff for you, started it), `chief`, `worker`, `reviewer`, or `staff` (a project's Chief of Staff).
- **`run_id`:** the run's id (an integer), or null for agents you start.
- **`spawned_by`:** the Chief of Staff agent that started it, else null.
- **`transcript_path`:** Claude's transcript file, from the hooks; null until the first hook.
- **`task_id`:** the task a worker or reviewer works on, else null.
- **`cwd`:** only present when the agent runs somewhere other than the workspace folder: a worker or reviewer runs in its task's worktree.
- **`permission_mode`:** `default`, `acceptEdits`, `plan` or `dontAsk`. `default` is started as `--permission-mode manual`, which is what the CLI now calls it.
- **`user_settings`:** whether Claude loads your own settings (`~/.claude/settings.json`: allow rules, MCP servers, theme). True by default for agents you start, false for agents a run starts. With false, only the repo's settings apply, so your allow rules don't approve anything and more requests come to cos as approvals.

## Routes

| Method and path | Body | Result |
|---|---|---|
| `GET /v1/agents?workspace_id=&state=` | | `{"agents": [Agent]}`, newest first |
| `GET /v1/agents/{id}` | | Agent |
| `POST /v1/workspaces/{id}/agents` | `{"prompt"?, "title"?, "model"?, "permission_mode"?, "user_settings"?, "rows"?, "cols"?}` | 201 Agent (role `user`). A missing `title` becomes the first 60 characters of the prompt, else "Agent N". |
| `POST /v1/agents/{id}/input` | `{"text": "…"}` | 204. Types the text and presses Return. 409 if exited. |
| `POST /v1/agents/{id}/keys` | `{"data": "<base64>"}` | 204. Raw keystrokes, exactly as typed (Esc, arrows, Ctrl-C…). |
| `POST /v1/agents/{id}/resize` | `{"rows": 40, "cols": 120}` | 204 |
| `POST /v1/agents/{id}/trust` | | 204. Answers the trust dialog with yes. 409 unless `needs_trust`. |
| `POST /v1/agents/{id}/stop` | | 202. Sends `/exit`, and kills the process after 10 s. If the agent is `working` or `waiting_approval` it presses Esc first, so `/exit` isn't queued behind the turn; if it's `needs_trust` it kills it straight away. |
| `POST /v1/agents/{id}/resume` | | 201 Agent. Only for `exited` or `interrupted`: starts `claude --resume <session_id>` as the same agent id. |
| `DELETE /v1/agents/{id}` | | 204. Forgets an exited agent. 409 if still running. |
| `GET /v1/agents/{id}/screen` | | `{"rows", "cols", "text", "cursor": [row, col]}`: the current screen as plain text |
| `GET /v1/agents/{id}/terminal` | | SSE stream of the terminal (below) |
| `GET /v1/agents/{id}/transcript?after=N` | | `{"messages": [Message], "next": N}`: the chat read from Claude's transcript (below) |
| `GET /v1/approvals?state=pending` | | `{"approvals": [Approval]}`, newest first. Without `state`, every approval. |
| `GET /v1/approvals/{id}` | | Approval |
| `POST /v1/approvals/{id}` | `{"decision": "allow" \| "deny", "message"?}` | 200 Approval. 409 if it's no longer pending. You can answer any pending approval, including one routed to the chief. |
| `POST /v1/approvals/{id}/chief` | `{"decision": "allow" \| "deny", "reason"}` | 200 Approval: the chief's `approval_decide`. 400 an allow without a reason, 403 when the run's limits refuse it (the approval then waits for you), 409 no longer pending. See [daemon.md](daemon.md#approvals-policy). |

### Terminal stream

`GET /v1/agents/{id}/terminal` is a Server-Sent Events stream:

```
event: snapshot
data: {"rows":40,"cols":120,"data":"<base64>"}

event: output
data: {"data":"<base64>"}

event: resize
data: {"rows":50,"cols":160}

event: exit
data: {"exit_code":0}
```

- **First event:** always one `snapshot`. Its `data` is escape sequences that redraw the current screen, from vt100's `state_formatted()`: the contents, the cursor's position and visibility, and the input modes (application keypad and cursor keys, bracketed paste, mouse reporting, cursor style), so pastes and arrow keys typed after it reach Claude the right way. If Claude is on the alternate screen, the snapshot switches to it first (`\e[?1049h`). A terminal emulator such as SwiftTerm feeds it as-is, then every `output` chunk in order. The snapshot and the chunks after it come from the same lock, so nothing is missed or repeated between them.
- **Output:** chunks are raw PTY bytes. They're never split mid-escape on purpose, but a client must handle splits anyway.
- **Resize:** sent when anyone resizes the agent. The PTY has one size, and the last resize wins.
- **Exit:** after `exit` the stream ends.
- **Reconnecting:** a stream on an agent that has already exited sends `snapshot`, then `exit`. A client that reconnects just opens a new stream and gets a fresh snapshot; there is no `Last-Event-ID` resume. After a daemon restart the screen is gone, so an exited or interrupted agent's snapshot is blank.
- **Slow clients:** the daemon buffers 1,024 chunks per stream. A client that falls further behind has its stream ended (without `exit`), and reconnects for a fresh snapshot.
- **Typing and resizing:** go through `POST …/keys` and `POST …/resize`.

## Approval JSON

```json
{
  "id": 12,
  "agent_id": 7,
  "workspace_id": 3,
  "tool_name": "Bash",
  "tool_input": {"command": "npm install left-pad"},
  "tool_use_id": "toolu_01…",
  "summary": "Bash npm install left-pad",
  "state": "pending",
  "decided_by": null,
  "message": null,
  "created_ms": 1792611174811,
  "decided_ms": null,
  "tier": "chief",
  "route": "user",
  "reason": "changes or downloads dependencies (npm install); installing packages needs the Trusted preset"
}
```

- **`state`:** one of
  - `pending`,
  - `allowed`, `denied`: answered through cos,
  - `settled`: answered in the terminal, or the tool went ahead some other way,
  - `expired`: nobody answered in 5 minutes, so it was denied and the agent told to carry on without that step. Only for agents a run starts; yours wait for you.
- **`decided_by`:** `user`, `terminal`, `rule` or `chief`, or null when nobody decided: `expired`, or `settled` because the agent exited or the daemon restarted while it was pending.
- **`tier`:** how the approvals policy classified the request: `allowed`, `chief`, `ask_user` or `never_delegated` (see [approvals.md](approvals.md#tiers)). Null on approvals from before the policy.
- **`route`:** who answers it: `auto` (Rust rules allowed it at once, `state: allowed, decided_by: rule`; nobody is notified), `chief` (waiting for the run's chief) or `user` (waiting for you). A chief answer the limits refuse moves it from `chief` to `user`. Agents you start are always `user`.
- **`reason`:** the policy's one-line reason; for a chief-tier request that came to you, also why it didn't go to the chief.
- **`message`:** the message sent with a denial, or the chief's reason when the chief answered.
- **`settled` by `terminal`:** the request's hook was withdrawn (Claude stopped waiting, which is what happens when someone answers in the terminal), or the agent's next hook arrived first. The real CLI doesn't always kill a waiting hook promptly when the terminal answers, so the next hook is the reliable signal.
- **`tool_use_id`:** taken from the `PreToolUse` just before the request, since `PermissionRequest` has none.
- **`summary`:** the tool and its main argument, such as `Bash npm install left-pad` or `Write /path/to/file`.


### Transcript

`GET /v1/agents/{id}/transcript?after=N` reads the agent's transcript file and returns the messages numbered above `after` (default 0), oldest first, and `next`.

```jsonc
// Message
{"n": 12, "role": "user" | "assistant", "kind": "text" | "tool" | "update",
 "text": "markdown…",                                                  // text and update
 "tool": {"name": "Bash", "summary": "npm test", "is_error": false},   // tool
 "ts_ms": 1792611174811}
```

- **Numbers:** messages are numbered 1, 2, 3… in the order of the file, so the same file always gives the same numbers.
- **`next`:** every message up to `next` is final. A tool call still waiting for its result comes after `next`, so asking again with `after=next` returns it again, with its result once it's in. Poll with `after=next` when `agent.transcript` arrives.
- **Tool calls:** one message per call, with `summary` its main argument on one line (Bash's command, a file tool's path, a search's pattern, else its input as JSON, cut to 160 characters) and `is_error` from its result. `ToolSearch` (Claude loading deferred tools) is left out.
- **`update`:** a user turn that starts with `[cos ·`, a Chief of Staff's inbox wakeup.
- **Left out:** thinking, system and meta entries, subagents (sidechains), bookkeeping entries, local command echoes (`<command-…>`) and "[Request interrupted…]" markers. A bracketed paste is shown as the text you pasted. A line that isn't JSON, or isn't finished yet, is skipped.
- **No file yet:** `{"messages": [], "next": 0}`.

## Events

These go in the event log, so apps get them over `/v1/events/stream`. `data` holds the ids:

| Kind | `data` |
|---|---|
| `agent.started` | `{agent_id, workspace_id, role, run_id, session_id}` (`run_id` is null for your own agents) |
| `agent.state_changed` | `{agent_id, state, from}` |
| `agent.exited` | `{agent_id, exit_code}` |
| `agent.removed` | `{agent_id}` |
| `approval.requested` | `{approval_id, agent_id, summary, tier, route}`. Not logged for `auto` approvals. |
| `approval.decided` | `{approval_id, state, decided_by}`, plus `route: "auto"` for approvals Rust rules allowed |
| `approval.routed` | `{approval_id, route, tier, run_id, agent_id, summary, reason}`: sent to the chief, or moved to you. The event's `run_id` is set. |
| `agent.transcript` | `{agent_id, next}`: the transcript grew. At most once a second per agent. |

Terminal output is never in the event log; it's too much data.

## CLI

| Command | What it does |
|---|---|
| `cos agent new <workspace> ["prompt"] [--title …] [--model …] [--permission-mode …] [--project-settings]` | Start an agent in a workspace. `--project-settings` sets `user_settings` to false. |
| `cos agents [--workspace …]` | List agents. |
| `cos agent stop` / `resume` / `rm <id>` | Stop, resume or forget an agent. |
| `cos agent say <id> "text"` | Type a message into an agent. |
| `cos agent screen <id>` | Print an agent's screen. |
| `cos agent trust <id>` | Answer the trust dialog with yes. |
| `cos agent attach <id>` | Your terminal becomes the agent's terminal: raw mode, keys in, output out, Ctrl-] to detach. |
| `cos approvals [--all]` | List pending approvals, or every approval. |
| `cos allow <id>` / `cos deny <id> [message]` | Answer an approval. |
| `cos sessions` | Lists the Claude sessions running on this Mac, from Claude's own files. Local only; works without the daemon. |

## How the daemon does it

- **One hook socket:** every agent's hooks go to one listener at `<home>/hooks.sock`, routed by `session_id`. The daemon picks the session id (`--session-id`) and registers it before Claude starts, so even the first hook finds its agent.
- **The claude binary:** `COS_CLAUDE_BIN`, else `claude` on the daemon's `PATH`. Agents get a clean environment: the daemon's own Claude variables (`CLAUDE_CODE_*`, `CLAUDECODE`) are removed, `CLAUDE_CONFIG_DIR` is kept.
- **Trust:** the daemon watches the screen for Claude's trust dialog only to set `needs_trust`; everything else comes from hooks.
- **Pending approvals and decisions:** each `PermissionRequest` is classified by the approvals policy (`cos-policy`, `daemon::agents::approvals`) first; see [approvals.md](approvals.md#daemon-wiring).
- **Daemon restarts:** agents still running when the daemon starts again (after a crash) become `interrupted` and their pending approvals `settled`. A daemon that stops normally ends its agents first (`/exit`, then kill), and marks them `interrupted` so `resume` brings them back.
- **Workspaces:** `DELETE /v1/workspaces/{id}` is 409 while an agent runs in it. Removing a workspace forgets its agents and their approvals.
