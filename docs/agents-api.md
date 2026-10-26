# Agents API (contract)

How the daemon runs agents in terminals and how the apps show them. The daemon side (Rust) and the apps (CosKit and the Mac app) are built against this document in parallel. Change it only together with both sides.

Everything is on the existing HTTP API: the Unix socket `~/.cos/cos.sock`, and the tailnet listener when remote access is on (see "Remote access" in [daemon.md](daemon.md)). JSON errors look like `{"error": "…"}`, with 400, 404 and 409 as elsewhere in [daemon.md](daemon.md). Times are Unix milliseconds.

## Concepts

- **Agent:** one Claude Code session that the daemon started, running Claude's own UI in a PTY the daemon owns, in a workspace's folder. Each agent has a `role`:
  - `user`: you started it from an app or the CLI, or a Chief of Staff started it for you.
  - `staff`: a project's Chief of Staff (see [chief-of-staff.md](chief-of-staff.md)).
- **Several agents per workspace:** a workspace can hold several agents at once, sharing its folder.
- **Followed through hooks:** the daemon follows each agent through hooks (see [claude-protocol.md](claude-protocol.md)), never by reading the screen.
- **Approval:** a permission request from an agent, waiting for an answer. It can be answered:
  - in the app or CLI (`allow` or `deny`),
  - by someone typing in the agent's terminal (then it's `settled`).

  Every approval waits for you; nothing answers on your behalf (see [approvals.md](approvals.md)).

## Agent JSON

```json
{
  "id": 7,
  "workspace_id": 3,
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
  "transcript_path": "/Users/me/.claude/projects/-Users-me-.cos-worktrees-app-1-lisbon/5504b543-….jsonl",
  "env_names": ["CLAUDE_CONFIG_DIR"]
}
```

`state` is one of:

| State | Meaning |
|---|---|
| `starting` | spawned; no hook yet |
| `needs_trust` | Claude is showing its trust dialog (the folder isn't trusted) |
| `needs_login` | Claude is showing its first-run or login screen: the agent's Claude config dir (`CLAUDE_CONFIG_DIR`) isn't logged in. Someone logs in in its terminal, then it carries on. |
| `working` | a prompt is running (after `UserPromptSubmit` or `PreToolUse`) |
| `waiting_approval` | a `PermissionRequest` is open |
| `idle` | the turn ended (`Stop`), waiting for input |
| `exited` | the process ended. `exit_code` is null if it was killed. |
| `interrupted` | it was running when the daemon stopped. `POST …/resume` brings it back. |

`last_message` is the `Stop` hook's `last_assistant_message`, cut to 2,000 characters.

- **`role`:** `user` (you, or a Chief of Staff for you, started it) or `staff` (a project's Chief of Staff).
- **`spawned_by`:** the Chief of Staff agent that started it, else null.
- **`transcript_path`:** Claude's transcript file, from the hooks; null until the first hook.
- **`env_names`:** the sorted names of the extra environment variables (global and project, see "Environment" in [daemon.md](daemon.md#environment)) it was last launched with, so an app can show "uses CLAUDE_CONFIG_DIR". Never the values. Empty for an agent launched before this existed, or with none.
- **`cwd`:** only present when the agent runs somewhere other than the workspace folder: the Chief of Staff runs in its own worktree.
- **`permission_mode`:** `default`, `acceptEdits`, `plan` or `dontAsk`. `default` is started as `--permission-mode manual`, which is what the CLI now calls it.
- **`user_settings`:** whether Claude loads your own settings (`~/.claude/settings.json`: allow rules, MCP servers, theme). True by default. With false, only the repo's settings apply, so your allow rules don't approve anything and more requests come to cos as approvals.

## Routes

| Method and path | Body | Result |
|---|---|---|
| `GET /v1/agents?workspace_id=&state=` | | `{"agents": [Agent]}`, newest first |
| `GET /v1/agents/{id}` | | Agent |
| `POST /v1/workspaces/{id}/agents` | `{"prompt"?, "title"?, "model"?, "permission_mode"?, "user_settings"?, "rows"?, "cols"?, "attachments"?}` | 201 Agent (role `user`). A missing `title` becomes the first 60 characters of the prompt, else the first attachment's name, else "Agent N". `attachments` name files uploaded to the workspace (below). |
| `POST /v1/agents/{id}/input` | `{"text": "…", "attachments"?: ["name"]}` | 204. Types the text and presses Return; with attachments, as one paste with their paths at the end. The text may be empty when there are attachments. 409 if exited. |
| `POST /v1/agents/{id}/attachments?name=` | the file's bytes | 201 Attachment (below) |
| `POST /v1/workspaces/{id}/attachments?name=` | the file's bytes | 201 Attachment, for an agent you're about to start there |
| `GET /v1/agents/{id}/attachments/{name}` | | The file, with its content type |
| `POST /v1/agents/{id}/keys` | `{"data": "<base64>"}` | 204. Raw keystrokes, exactly as typed (Esc, arrows, Ctrl-C…). |
| `POST /v1/agents/{id}/resize` | `{"rows": 40, "cols": 120}` | 204 |
| `POST /v1/agents/{id}/trust` | | 204. Answers the trust dialog with yes. 409 unless `needs_trust`. |
| `POST /v1/agents/{id}/stop` | | 202. Sends `/exit`, and kills the process after 10 s. If the agent is `working` or `waiting_approval` it presses Esc first, so `/exit` isn't queued behind the turn; if it's `needs_trust` it kills it straight away. |
| `POST /v1/agents/{id}/resume` | | 201 Agent. Only for `exited` or `interrupted`: starts `claude --resume <session_id>` as the same agent id. |
| `DELETE /v1/agents/{id}` | | 204. Forgets an exited agent. 409 if still running. |
| `GET /v1/agents/{id}/screen` | | `{"rows", "cols", "text", "cursor": [row, col]}`: the current screen as plain text |
| `GET /v1/agents/{id}/terminal` | | SSE stream of the terminal (below) |
| `GET /v1/agents/{id}/transcript?after=N` | | `{"messages": [Message], "next": N, "context": Context \| null}`: the chat read from Claude's transcript, and how full its context is (below) |
| `GET /v1/approvals?state=pending` | | `{"approvals": [Approval]}`, newest first. Without `state`, every approval. |
| `GET /v1/approvals/{id}` | | Approval |
| `POST /v1/approvals/{id}` | `{"decision": "allow" \| "deny", "message"?}` | 200 Approval. 409 if it's no longer pending. |

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

- **First event:** always one `snapshot`. Its `data` is escape sequences that redraw the current screen, from vt100's `state_formatted()`: the contents, the cursor's position and visibility, and the input modes (application keypad and cursor keys, bracketed paste, mouse reporting, cursor style), so pastes and arrow keys typed after it reach Claude the right way. If Claude is on the alternate screen, the snapshot switches to it first (`\e[?1049h`). A terminal emulator such as Ghostty feeds it as-is, then every `output` chunk in order. The snapshot and the chunks after it come from the same lock, so nothing is missed or repeated between them.
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
  "reason": "changes or downloads dependencies (npm install)"
}
```

- **`state`:** one of
  - `pending`,
  - `allowed`, `denied`: answered through cos,
  - `settled`: answered in the terminal, or the tool went ahead some other way,
  - `expired`: only on old approvals, from when some agents' requests were refused after 5 minutes. Nothing expires now.
- **`decided_by`:** `user` or `terminal`, or null when nobody decided: `expired`, or `settled` because the agent exited or the daemon restarted while it was pending.
- **`tier`:** how the approvals policy classified the request: `allowed`, `chief`, `ask_user` or `never_delegated` (see [approvals.md](approvals.md#tiers)). It's shown with the request; you answer every tier. Null on approvals from before the policy.
- **`reason`:** the policy's one-line reason.
- **`message`:** the message sent with a denial.
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
 "attachments": [{"name": "shot.png", "path": "/…/.cos/attachments/shot.png",
                  "content_type": "image/png", "size_label": "245 KB"}], // your text messages, when they name files
 "ts_ms": 1792611174811}
```

- **Numbers:** messages are numbered 1, 2, 3… in the order of the file, so the same file always gives the same numbers.
- **`context`:** `{"used_tokens": 41203, "window_tokens": 200000}`, for the whole file whatever `after` is. `used_tokens` is the newest reply's input plus cache-read plus cache-creation tokens (subagents' replies and Claude's `<synthetic>` notices don't count). `window_tokens` is the model's context window when cos knows it: 1M for a model started or named with `[1m]` and for Opus and Sonnet from 4.6, Haiku from 5 and Fable; 200K for older Claude models; else null, and null too when the conversation is already past the guess. `context` is null before the first reply.
- **`next`:** every message up to `next` is final. A tool call still waiting for its result comes after `next`, so asking again with `after=next` returns it again, with its result once it's in. Poll with `after=next` when `agent.transcript` arrives.
- **Tool calls:** one message per call, with `summary` its main argument on one line (Bash's command, a file tool's path, a search's pattern, else its input as JSON, cut to 160 characters) and `is_error` from its result. `ToolSearch` (Claude loading deferred tools) is left out.
- **`update`:** a user turn that starts with `[cos ·`, a Chief of Staff's inbox wakeup.
- **Left out:** thinking, system and meta entries, subagents (sidechains), bookkeeping entries, local command echoes (`<command-…>`) and "[Request interrupted…]" markers. A bracketed paste is shown as the text you pasted. A line that isn't JSON, or isn't finished yet, is skipped.
- **`attachments`:** on a user `text` message that ends with cos's "Attached files" block (below), the block is taken out of `text` and its files listed here. Left out when there are none.
- **No file yet:** `{"messages": [], "next": 0}`.

## Attachments

Files you send with a message, from any device: the Mac app, the iPhone, or `cos --host … --file`. A file is uploaded first, then a message names it.

- **Upload:** `POST …/attachments?name=<file name>` with the file's bytes as the body and its `Content-Type`. Routes: an agent's folder (`/v1/agents/{id}/attachments`), a workspace for an agent about to start (`/v1/workspaces/{id}/attachments`), and a Chief of Staff's worktree (`/v1/projects/{id}/staff/attachments`, see [chief-of-staff.md](chief-of-staff.md#routes)), which is made if needed without starting the chief. Over the tailnet too, for approved devices.
- **Answer:** 201 `{"name": "shot.png", "path": "/abs/…/.cos/attachments/shot.png", "size": 250880, "content_type": "image/png"}`. Use the answer's `name`: it's made safe (the last path segment; letters, digits, `.`, `-` and `_`; no leading dot; at most 100 characters; an extension from the content type when it has none) and unique in the folder (`shot-2.png`).
- **Where files go:** `<agent's folder>/.cos/attachments/`, inside the worktree, so Claude reads them without asking to leave it. The folder holds a `.gitignore` of `*`, so they never show in `git status` or get committed. A `.cos` or `attachments` that is a symlink is refused.
- **Limits:** 32 MB a file (413 over it), 20 files a message (400).
- **Naming them:** `attachments` on `POST /v1/agents/{id}/input`, `POST /v1/workspaces/{id}/agents` and the staff message takes names, or absolute paths, of files in that agent's folder; an unknown one is 400. cos adds a block at the end of the text, and types it as one paste:

  ```
  <your text>

  Attached files (saved by cos; read them with the Read tool):
  - /abs/…/.cos/attachments/shot.png (image/png, 245 KB)
  ```

- **Download:** `GET /v1/agents/{id}/attachments/{name}` gives the file with its content type, `Cache-Control: private, max-age=86400` (names never get reused) and `nosniff`. 404 for anything else in the folder.
- **Between agents:** a Chief of Staff passes files on with `attachments` on `agent_spawn` and `agent_send` (see [chief-of-staff.md](chief-of-staff.md#staff-mcp-tools-server-cos)); cos copies them into the target's folder, reusing an identical copy already there.

## Shells

A shell is a login shell (`$SHELL -l`, else the OS's usual one) the daemon runs in a folder, for you to type in: the Mac app opens one with Ctrl-J. It runs in a terminal like an agent's and streams the same way, but it isn't an agent: no hooks, no approvals, no events, and nothing in the database. Shells live in the daemon's memory, and stop when it stops.

| Route | Body | Answer |
| --- | --- | --- |
| `GET /v1/shells` | | `{"shells": [Shell]}` |
| `POST /v1/shells` | `{"cwd": "/abs/folder", "project_id"?: 1, "rows"?: 40, "cols"?: 120}` | 201 `Shell`. `cwd` must be an absolute path to a folder (400 otherwise). With `project_id` (404 if unknown) the shell gets the project's environment (global, then the project's own), as its agents do, so `claude` there uses the project's account. At most 32 run at once (409); exited ones are forgotten when the next starts. |
| `POST /v1/shells/{id}/keys` | `{"data": "<base64>"}` | 204 |
| `POST /v1/shells/{id}/resize` | `{"rows": 40, "cols": 120}` | 204 |
| `GET /v1/shells/{id}/terminal` | | SSE, as [Terminal stream](#terminal-stream) |
| `DELETE /v1/shells/{id}` | | 204. Kills it if it still runs, and forgets it. |

`Shell` is `{"id": 1, "project_id": 1, "cwd": "/abs/folder", "pid": 4242, "started_ms": 1761330000000, "exited": false, "exit_code": null}`. An unknown id is 404 "there is no such terminal".

**Over the tailnet:** approved devices open and type into shells too, as they already type into agents' terminals (where Claude's `!` runs any command). The guard admits only devices signed in as this machine's Tailscale user, not tagged, and approved on the host, so a shell there is you on your own machine. The daemon logs each shell a device starts, with the device's name, node ID and address. Older daemons answer every `/v1/shells` route 403 `host_only` over the tailnet.

## Events

These go in the event log, so apps get them over `/v1/events/stream`. `data` holds the ids:

| Kind | `data` |
|---|---|
| `agent.started` | `{agent_id, workspace_id, role, session_id}` |
| `agent.state_changed` | `{agent_id, state, from}` |
| `agent.exited` | `{agent_id, exit_code}` |
| `agent.removed` | `{agent_id}` |
| `approval.requested` | `{approval_id, agent_id, summary, tier}` |
| `approval.decided` | `{approval_id, state, decided_by}` |
| `agent.transcript` | `{agent_id, next}`: the transcript grew. At most once a second per agent. |
| `attachment.added` | `{workspace_id, name, size, content_type}`: a file was uploaded. |

Terminal output is never in the event log; it's too much data.

## CLI

| Command | What it does |
|---|---|
| `cos agent new <workspace> ["prompt"] [--title …] [--model …] [--permission-mode …] [--project-settings] [--file PATH]…` | Start an agent in a workspace. `--project-settings` sets `user_settings` to false. `--file` uploads a file and names it in the prompt; repeat it for more. |
| `cos agents [--workspace …]` | List agents. |
| `cos agent stop` / `resume` / `rm <id>` | Stop, resume or forget an agent. |
| `cos agent say <id> "text" [--file PATH]…` | Type a message into an agent, with files. |
| `cos agent screen <id>` | Print an agent's screen. |
| `cos agent trust <id>` | Answer the trust dialog with yes. |
| `cos agent attach <id>` | Your terminal becomes the agent's terminal: raw mode, keys in, output out, Ctrl-] to detach. |
| `cos approvals [--all]` | List pending approvals, or every approval. |
| `cos allow <id>` / `cos deny <id> [message]` | Answer an approval. |
| `cos sessions` | Lists the Claude sessions running on this Mac, from Claude's own files. Local only; works without the daemon. With the daemon up it also looks in each `CLAUDE_CONFIG_DIR` set in cos. |
| `cos env [set \| unset] [--project P]` | The environment variables agents get. See "Environment" in [daemon.md](daemon.md#environment). |

## How the daemon does it

- **One hook socket:** every agent's hooks go to one listener at `<home>/hooks.sock`, routed by `session_id`. The daemon picks the session id (`--session-id`) and registers it before Claude starts, so even the first hook finds its agent. Each agent's hooks run `cos hook --agent <id>`, which adds `cos_agent_id` to the hook. When Claude starts a new session in the same terminal (`/clear`, or `/resume` of another session), its `SessionStart` comes from an unknown session id but names the agent, so the agent moves to the new session: its `session_id` (and so its next `--resume`) and `transcript_path` change, approvals of the old session are settled, and `agent.transcript` is logged with `next: 0` so apps read the transcript from the top.
- **The claude binary:** `COS_CLAUDE_BIN`, else `claude` on the daemon's `PATH`.
- **Environment:** the daemon's own, then the global and project variables set in cos (see "Environment" in [daemon.md](daemon.md#environment)), read at every launch and resume, then what cos sets itself (`TERM`, `COLORTERM`, the hook socket). Claude's session markers (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, …) are removed; `CLAUDE_CONFIG_DIR` is kept unless a project sets its own. A running agent keeps the environment it started with.
- **Trust and login:** until the first hook, the daemon watches the screen for Claude's trust dialog, to set `needs_trust`, and for its first-run or login screen, to set `needs_login`. Everything else comes from hooks.
- **Pending approvals and decisions:** each `PermissionRequest` is classified by the approvals policy (`cos-policy`, `daemon::agents::approvals`) first; see [approvals.md](approvals.md#daemon-wiring).
- **Daemon restarts:** agents still running when the daemon starts again (after a crash) become `interrupted` and their pending approvals `settled`. A daemon that stops normally ends its agents first (`/exit`, then kill), and marks them `interrupted` so `resume` brings them back.
- **Workspaces:** `DELETE /v1/workspaces/{id}` is 409 while an agent runs in it. Removing a workspace forgets its agents and their approvals.
