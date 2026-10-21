# Agents API (contract)

How the daemon runs agents in terminals and how the apps show them. The daemon side (Rust) and the apps (CosKit and the Mac app) are built against this document in parallel. Change it only together with both sides.

Everything is on the existing HTTP API: the Unix socket `~/.cos/cos.sock`, and later the tailnet listener. JSON errors look like `{"error": "…"}`, with 400, 404 and 409 as elsewhere in [daemon.md](daemon.md). Times are Unix milliseconds.

## Concepts

- **Agent:** one Claude Code session that the daemon started, running Claude's own UI in a PTY the daemon owns, in a workspace's folder. Each agent has a `role`:
  - `user`: you started it from an app or the CLI.
  - `chief`, `worker`, `reviewer`: started by a run (later).
- **Several agents per workspace:** a workspace can hold several agents at once, sharing its folder.
- **Followed through hooks:** the daemon follows each agent through hooks (see [claude-protocol.md](claude-protocol.md)), never by reading the screen.
- **Approval:** a permission request from an agent, waiting for an answer. It can be answered:
  - in the app or CLI (`allow` or `deny`),
  - by someone typing in the agent's terminal (then it's `settled`),
  - by a rule later (the approvals engine).
- **External session:** a Claude session the daemon didn't start, read from Claude's own `sessions/<pid>.json` files. Read-only.

## Agent JSON

```json
{
  "id": 7,
  "workspace_id": 3,
  "run_id": null,
  "role": "user",
  "title": "fix login bug",
  "session_id": "5504b543-4897-4751-add5-f81f24a6316c",
  "pid": 25615,
  "state": "working",
  "model": "sonnet",
  "permission_mode": "default",
  "rows": 40,
  "cols": 120,
  "started_ms": 1792611174811,
  "last_activity_ms": 1792611180000,
  "exited_ms": null,
  "exit_code": null,
  "last_message": "Done. I created hello.txt …",
  "pending_approval_id": null
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

## Routes

| Method and path | Body | Result |
|---|---|---|
| `GET /v1/agents?workspace_id=&state=` | | `{"agents": [Agent]}`, newest first |
| `GET /v1/agents/{id}` | | Agent |
| `POST /v1/workspaces/{id}/agents` | `{"prompt"?, "title"?, "model"?, "permission_mode"?, "rows"?, "cols"?}` | 201 Agent (role `user`). A missing `title` becomes the first 60 characters of the prompt, else "Agent N". |
| `POST /v1/agents/{id}/input` | `{"text": "…"}` | 204. Types the text and presses Return. 409 if exited. |
| `POST /v1/agents/{id}/keys` | `{"data": "<base64>"}` | 204. Raw keystrokes, exactly as typed (Esc, arrows, Ctrl-C…). |
| `POST /v1/agents/{id}/resize` | `{"rows": 40, "cols": 120}` | 204 |
| `POST /v1/agents/{id}/trust` | | 204. Answers the trust dialog with yes. 409 unless `needs_trust`. |
| `POST /v1/agents/{id}/stop` | | 202. Sends `/exit`, and kills the process after 10 s. |
| `POST /v1/agents/{id}/resume` | | 201 Agent. Only for `exited` or `interrupted`: starts `claude --resume <session_id>` as the same agent id. |
| `DELETE /v1/agents/{id}` | | 204. Forgets an exited agent. 409 if still running. |
| `GET /v1/agents/{id}/screen` | | `{"rows", "cols", "text", "cursor": [row, col]}`: the current screen as plain text |
| `GET /v1/agents/{id}/terminal` | | SSE stream of the terminal (below) |
| `GET /v1/approvals?state=pending` | | `{"approvals": [Approval]}` |
| `POST /v1/approvals/{id}` | `{"decision": "allow" \| "deny", "message"?}` | 200 Approval. 409 if it's no longer pending. |
| `GET /v1/sessions` | | `{"sessions": [ExternalSession]}`: live Claude sessions the daemon didn't start |

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

- **First event:** always one `snapshot`. Its `data` is escape sequences that redraw the current screen, from vt100's `contents_formatted()`. A terminal emulator such as SwiftTerm feeds it as-is, then every `output` chunk in order.
- **Output:** chunks are raw PTY bytes. They're never split mid-escape on purpose, but a client must handle splits anyway.
- **Resize:** sent when anyone resizes the agent. The PTY has one size, and the last resize wins.
- **Exit:** after `exit` the stream ends.
- **Reconnecting:** a stream on an agent that has already exited sends `snapshot`, then `exit`. A client that reconnects just opens a new stream and gets a fresh snapshot; there is no `Last-Event-ID` resume.
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
  "decided_ms": null
}
```

- **`state`:** one of
  - `pending`,
  - `allowed`, `denied`: answered through cos,
  - `settled`: answered in the terminal, or the tool went ahead some other way,
  - `expired`.
- **`decided_by`:** `user`, `terminal`, `rule` or `chief`.
- **`tool_use_id`:** taken from the `PreToolUse` just before the request, since `PermissionRequest` has none.

## ExternalSession JSON

```json
{"pid": 2269, "session_id": "e9ad…", "cwd": "/Users/me/apps/cos", "name": "cos-6e", "status": "busy",
 "version": "2.1.306", "status_updated_ms": 1792611174811, "project_id": 1, "workspace_id": null}
```

## Events

These go in the event log, so apps get them over `/v1/events/stream`. `data` holds the ids:

| Kind | `data` |
|---|---|
| `agent.started` | `{agent_id, workspace_id, role, session_id}` |
| `agent.state_changed` | `{agent_id, state, from}` |
| `agent.exited` | `{agent_id, exit_code}` |
| `agent.removed` | `{agent_id}` |
| `approval.requested` | `{approval_id, agent_id, summary}` |
| `approval.decided` | `{approval_id, state, decided_by}` |
| `session.changed` | `{}`: the external session list changed. Refetch `/v1/sessions`. Sent at most every 2 s. |

Terminal output is never in the event log; it's too much data.

## CLI

| Command | What it does |
|---|---|
| `cos agent new <workspace> ["prompt"] [--title …] [--model …]` | Start an agent in a workspace. |
| `cos agents [--workspace …]` | List agents. |
| `cos agent stop` / `resume` / `rm <id>` | Stop, resume or forget an agent. |
| `cos agent say <id> "text"` | Type a message into an agent. |
| `cos agent screen <id>` | Print an agent's screen. |
| `cos agent attach <id>` | Your terminal becomes the agent's terminal: raw mode, keys in, output out, Ctrl-] to detach. |
| `cos approvals` | List pending approvals. |
| `cos allow <id>` / `cos deny <id> [message]` | Answer an approval. |
