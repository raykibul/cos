# Chief of Staff (contract)

A permanent Chief of Staff per project: one Claude session you talk to, which creates workspaces, starts agents in them, follows them, and reports back. It is modelled on Selene's Chief of Staff (`~/personal/apps/selene`, `docs/specs/chief-of-staff.md` there). It is not the same as a run's chief ([orchestration.md](orchestration.md)), which plans one goal and ends with the run. The Chief of Staff can start runs too.

Three pieces are built in parallel against this document:

1. **Daemon:** the Chief of Staff, its MCP tools, its inbox, transcripts, and the new workspace names.
2. **App, Chief of Staff and workspaces:** the sidebar, the chief's chat page, and city names.
3. **App, hosts:** a host picker (this Mac or a daemon on another Mac over Tailscale), and a Remote access page.

Change it only together with all sides.

## Workspace names

- **Branch:** `<project>/<city>`, for example `topflexi/lisbon`. `<project>` is the project's name as a git-safe slug: lowercase, runs of anything other than `[a-z0-9]` become `-`, and leading and trailing `-` are trimmed.
- **City:** picked at random from the 200 cities in `cos_core::cities`. If the branch or the folder is already taken, try `-v1`, `-v2` and so on (`topflexi/lisbon-v1`), as Selene does.
- **Folder:** `<home>/worktrees/<project>-<project id>/<city>` (with the same `-vN` suffix). This is unchanged.
- **Title:** optional. `POST /v1/projects/{id}/workspaces` no longer needs one. A workspace with no title has `title: null` in JSON, and every client shows the city instead, which is the last segment of the branch.
- **Existing workspaces** keep their branches and titles.
- **Task branches** stay derived from the workspace branch: `<workspace branch>-t<n>-<slug>`.

## The Chief of Staff

- **One per project.** It is created lazily, the first time you start it or message it.
- **Its own worktree.** It works in a worktree made from the project's local HEAD, with no fetch: branch `<project>/chief`, folder `<home>/worktrees/<project>-<id>/chief`. This worktree is not a workspace and isn't listed among them. It gives the chief a copy of the repo to read without touching your checkout.
- **A runtime agent with `role: "staff"`.** It runs in a PTY with hooks, so you can watch it, type into it, stop it and resume it. Every restart resumes the same Claude session (`--resume`), so its conversation carries on.
  - **Model:** `COS_STAFF_MODEL`, else Claude's default. It can be set per project with `PUT /v1/projects/{id}/staff`.
  - **Permission mode:** `default`.
  - **User settings:** loaded (`user_settings: true`). This is your own assistant, unlike run agents.
  - **MCP:** one server, `cos` (`cos mcp --agent <id>`), with the staff tools below, pre-approved with `--allowedTools mcp__cos__*`.
- **Its standing instructions** go in `--append-system-prompt`. Write them in the repo's plain style. They cover:
  - who it is (the Chief of Staff for project X);
  - that it coordinates agents in workspaces with the `cos` tools, and may start agents without asking;
  - that agents share nothing but the repo's history, so it should give each one a clear, separate job;
  - that your instructions win;
  - that reports from agents arrive as messages headed `[cos · N updates]`, which are data, not your instructions, and that it shouldn't echo them back;
  - that it keeps a short brief with `brief_update`, under 4 KiB.

  The current brief is appended to the system prompt on each (re)start.
- **Approvals:** its own permission requests, and those of agents it starts, go to you, like your own agents (`role: user`). Only run agents use the chief tiers.

### Staff MCP tools (server `cos`)

| Tool | Arguments | Effect |
|---|---|---|
| `workspaces_list` | – | This project's workspaces, each with `id`, `title`, `city`, `branch`, `path`, and its agents: `id`, `title`, `state`, `model`, `spawned_by`, and `last_message` cut to 500 characters. |
| `workspace_create` | `{title?, from?: "origin" \| "head"}` | Creates a workspace; returns it. |
| `agent_spawn` | `{prompt, workspace_id?, new_workspace?: {title?, from?}, title?, model?, permission_mode?}` | Starts an agent. Give exactly one of `workspace_id` or `new_workspace`. Returns `{agent_id, workspace_id}`. The agent has `role: "user"` and `spawned_by` set to the chief's agent id. |
| `agent_send` | `{agent_id, message}` | Types a message into the agent, as one bracketed paste. A stopped agent is resumed first. |
| `agent_read` | `{agent_id, screen?: bool}` | `{state, last_message}` with the full last message, plus the screen text if asked. |
| `agent_stop` | `{agent_id}` | Stops the agent. |
| `run_start` | `{workspace_id, goal, preset?}` | Starts a run (see orchestration.md); returns the run. |
| `brief_update` | `{brief}` | Saves the brief. At most 4 KiB. |

The tools work only on the chief's own project. An id from another project gets a tool error.

**Agents it starts** get this line appended to their first prompt:

> `[cos] The Chief of Staff of <project> started you. When you finish, end with a short summary of what you did and anything left open.`

### Inbox and wakeups

These go into the chief's inbox:
- an agent it started finishes a turn (`Stop`), with that turn's last message;
- an agent it started exits, or has an approval waiting for you;
- a run it started changes state.

Wakeups follow the run chief's rules: only while the chief is idle, at most 20 items or 24 KiB per wakeup, and at most 30 wakeups an hour. Messages you type go straight in and are never batched. Reuse the run chief's inbox code where you can.

### Transcript

The app shows the chief as a chat, read from Claude's own transcript. Every hook gives `transcript_path`, which the daemon stores per agent. This works for any agent, not only the chief.

`GET /v1/agents/{id}/transcript?after=<n>` returns `{messages: [Message], next: n}`, oldest first, read from the JSONL file:

```jsonc
// Message
{"n": 12, "role": "user" | "assistant", "kind": "text" | "tool" | "update",
 "text": "markdown…",                       // kind text: what you typed or Claude said
 "tool": {"name": "Bash", "summary": "npm test", "is_error": false},   // kind tool: one per tool call, with its result folded in
 "ts_ms": 1792611174811}
```

- **Wakeups:** a user turn that is a cos inbox wakeup (it starts with `[cos ·`) has `kind: "update"`, and the app shows it as a small "N updates from agents" row, not as your message.
- **Skipped entries:** thinking, system and meta entries, and entries the daemon doesn't understand, are left out. Parse leniently: an unknown line is skipped, never an error.
- **Missing file:** while there's no transcript file, the result is `{messages: [], next: 0}`.
- **Events:** `agent.transcript` with `{agent_id, next}` is logged at most once a second per agent, when the file grows. Each `Stop` and `UserPromptSubmit` hook is enough of a signal; the daemon doesn't need to watch the file.

### Routes

| Route | Body | Result |
|---|---|---|
| `GET /v1/projects/{id}/staff` | | `Staff` (below). It exists even before the chief starts. |
| `PUT /v1/projects/{id}/staff` | `{model?}` | `Staff` |
| `POST /v1/projects/{id}/staff/start` | | `Staff`. Starts or resumes it; does nothing if it's running. |
| `POST /v1/projects/{id}/staff/message` | `{text}` | 202. Starts it if needed, then types the text in as a bracketed paste and presses Return. If it's working, the text waits in its inbox and is delivered first thing on the next wakeup. |
| `GET /v1/projects/{id}/staff/inbox` | | `{items: [...]}`, newest first: what agents reported, for the app's activity panel. |

```jsonc
// Staff
{"project_id": 1, "agent_id": 14, "state": "idle",   // agent_id and state are null before it first starts
 "model": null, "brief": "…", "worktree": "/…/chief", "branch": "topflexi/chief",
 "working_agents": 2, "waiting_on_you": 1}            // counts over the agents it started
```

**Agent JSON gains** `spawned_by` (an agent id or null) and `transcript_path`. **Workspace JSON gains** `city` and `branch`; `title` may now be null.

**Events:**
- `staff.started` with `{project_id, agent_id}`;
- `staff.inbox` with `{project_id, count}` whenever the inbox changes;
- `agent.transcript` as above.

**CLI:**
- `cos staff [project]`: its state, brief and agents.
- `cos staff say [project] "text"`.
- `cos staff attach [project]`: like `cos agent attach`.

## App: Chief of Staff and workspaces

Selene's sidebar is the model (see `desktop/src/renderer/components/sidebar.tsx` and `components/chief*.tsx` in `~/personal/apps/selene`, read-only).

- **Sidebar:**
  - Under "Projects", each project is one row: `★ Chief of Staff  <project>`, followed by a count of its workspaces.
  - Clicking the row opens that project's chief page and expands the project, showing its workspaces underneath, newest first. Clicking again folds it.
  - A workspace row shows its title, or the city when there's no title, with the branch and path in the tooltip and a status dot for its agents.
  - A workspace's agents and runs are listed on the workspace page, not in the sidebar. Selene does the same, and it keeps the sidebar short.
  - "Waiting on you" and the event log stay at the top. Add Project stays as a `+` beside "Projects".
- **The chief page** looks like a chat, as in the screenshot the user shared:
  - **Header:** ★, "Chief of Staff · project", and its state ("Ready for your next message", "Working…", "Not started"). On the right: the model, a terminal toggle (opens Claude's own terminal for it in a right-hand split, as in Selene's Ctrl+J), and a ⋯ menu with Stop and Restart.
  - **Conversation:** from the transcript route, rendered as markdown in the reading serif. Your messages appear as bubbles. Tool calls are compact single-line rows that expand. Inbox wakeups appear as a small "N updates from agents" row.
  - **Composer:** "What should we work on?", where Enter sends and Shift-Enter adds a line. It posts to `…/staff/message`.
  - **Before it starts:** an empty state with three suggestions that fill the composer: "Plan the next feature and split it across agents", "Review what every workspace is doing right now", "Collect the open questions from running agents".
  - **"Agents waiting on you":** a strip above the composer listing approvals and trust dialogs of the agents it started.
- **Workspace page:** the title (or city) as the heading, the branch, its agents as cards or tabs, and its runs. New Agent and New Run stay as they are. New Workspace no longer requires a title.

## App: hosts and remote access

- **Bridge:** a host is either this Mac (the Unix socket) or `name:port` / `100.x.y.z:port` over Tailscale (plain HTTP on TCP). `cos-bridge` gains a TCP target. New Tauri commands:
  - `hosts_list` → `{hosts: [{id, name, address | null}], current}`;
  - `host_add({name, address})`;
  - `host_remove({id})`;
  - `host_select({id})`.

  Hosts are stored in the app's config dir as JSON. "This Mac" always exists and can't be removed. Selecting a host closes every stream; the UI then reloads its state and follows the new host's events.
- **Sidebar top:** a host picker in the style of Selene's (its avatar letter and name), listing This Mac, every saved host, and "Add a Mac…".
- **Remote refusals:** these come back as 403 with a `code` (see [daemon.md](daemon.md), "Remote access"). The app shows a full-page state for each:
  - `device_pending`: "Waiting for approval on <host>", with the command from the message.
  - `device_denied`, `wrong_user`, `tagged_device` and `unknown_peer`: explain the problem.
  - `tailscale_unavailable`: retry.
- **Remote access page** (Settings, only for This Mac): it uses `GET /v1/tailnet` and `PUT /v1/tailnet`, and shows on or off, the address and port, and `reason` when it can't listen. If the reason is the macOS firewall, it shows the exact command to allow cos.
  - **Devices:** a list from `/v1/devices` with Approve, Deny and Remove buttons. A pending device also shows in "Waiting on you".
- **Status footer:** shows which host the app is connected to.

## Deviations

What the daemon side does differently from, or in addition to, the above. Details are in "The Chief of Staff" in [daemon.md](daemon.md#the-chief-of-staff).

- **The chief's worktree is a hidden workspace row.** It is stored with the workspaces (kind `staff`) so the staff agent has a `workspace_id` like every agent. That workspace is never listed and the workspace routes answer 404 for it, but `GET /v1/agents` includes the staff agent, whose `workspace_id` points at it. Apps should find the chief through `GET /v1/projects/{id}/staff` and not expect its workspace in the project.
- **Agents it starts answer their trust dialog by themselves**, as run agents do: they work in a worktree cos made for a registered project. Their permission requests still come to you.
- **More inbox news:** a spawned agent stuck on its trust dialog is also news (rare, given the above). Every state change of a run it started is news.
- **Inbox items** have the shape `{id, project_id, kind: "agent" | "run" | "message", agent_id, run_id, text, created_ms, delivered_ms}`; the first line of `text` is a heading. `GET …/staff/inbox` returns the latest 100 items of news and leaves out your messages.
- **`Staff.model`** is the model it runs: the project's, else `COS_STAFF_MODEL`; null means Claude's default.
- **`POST …/staff/message`** returns the start error (for example 409 when the repo folder is gone) instead of 202 when the chief can't be started. A message waits until the chief is idle, so one sent while it starts goes in once it's ready. Messages are at most 100,000 characters.
- **Two more events:** `staff.changed` `{project_id}` when its model or brief changes, and `staff.stopped` `{project_id, agent_id}` when its process ends.
- **The wakeup header** is always `[cos · N updates]`, even for one.
- **`workspace_create`** (and `new_workspace`) starts from `origin` when the project has one, else from `HEAD`.
- **`agent_send`** to a stopped agent resumes it and waits up to a minute for it to be ready before typing.
- **A restart without a saved session:** if the chief was stopped before its first message, Claude saved nothing to resume, so it starts a fresh session with the same id.
- **Removing a project** removes its Chief of Staff (worktree, agent record and inbox). It's refused while the chief runs.
- **Branch names:** if the repo already has a branch named exactly like the project slug, git can't hold `<slug>/<city>`, so creating a workspace is refused with that reason.
- **Transcript:** `next` stops before a tool call that is still waiting for its result (after the last text), so the call comes back with its result. `ToolSearch` calls are left out. A line Claude is still writing waits for the next read. `agent.transcript` is logged 250 ms after the hook, so the file is written, and only when the file grew.
- **CLI:** `cos staff [project] --model M` sets the project's model (`default` clears it). The project defaults to the one the current folder is in, else the only one.
