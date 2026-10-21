# The cos daemon

`cos daemon` is the long-running process that will own runs, workers and approvals. As of milestone 3 it owns the database and the event log, and serves an HTTP API on a Unix socket. It also keeps the books for runs and their tasks (see "Runs, tasks and the board" below); the workers and the chief that drive them arrive in milestones 4 and 5. It runs agents (Claude Code in terminals it owns) and holds their permission requests as approvals (see "Agents" below).

## Commands

| Command | What it does |
|---|---|
| `cos daemon` | Run in the foreground. Ctrl-C or SIGTERM stops it cleanly, asking its agents to exit first. |
| `cos daemon install` | Install a launchd agent (`local.cos.daemon`): starts now, at every login, and again about 10 s after a crash. |
| `cos daemon uninstall` | Stop the agent and remove its plist. |
| `cos status` | Whether the daemon is running, and its pid, uptime, event count and launchd state. Exits 1 when it isn't running. |
| `cos events` | The last 20 events. `-n 50` for more, `--after N` to start after event N, `--json` for one JSON object per line. |
| `cos events -f` | Keep printing new events. If the daemon restarts, it waits and carries on from the last event it printed. |
| `cos remote [on [--port N] \| off]` | Turn remote access over Tailscale on or off, or show where it listens. See [Remote access](#remote-access). |
| `cos devices [list \| approve \| deny \| remove <device>]` | The tailnet devices that may use this daemon. |
| `cos --host <ip or name>[:port] …` | Talk to another machine's daemon over its tailnet listener instead of the local socket (also `COS_HOST`). The port defaults to 7700. |

## Files

Everything lives in `~/.cos`, or in `$COS_HOME` if that's set. The directory is readable by you only.

| Path | |
|---|---|
| `cos.sock` | The API socket (mode 0600). Removed on a clean stop, and replaced on start if a crash left it behind. |
| `hooks.sock` | Where the agents' hooks (relayed by `cos hook`) reach the daemon (mode 0600). One socket for every agent; hooks are routed by session id. |
| `cos.db` | SQLite in WAL mode. The schema version is in `PRAGMA user_version`; migrations are in `crates/cos-core/src/store.rs`. |
| `daemon.lock` | Holds the running daemon's pid. Its lock means only one daemon runs per home. |
| `logs/daemon.log` | The daemon's stderr when launchd runs it. |

The launchd plist records the binary's path, `COS_HOME` and your `PATH` at install time, and also `CLAUDE_CONFIG_DIR` and `COS_CLAUDE_BIN` if they're set, so agents use the same Claude config and binary as your shell. Run `cos daemon install` again after moving the binary or changing any of them.

## Event log

Every event has a sequence number (`seq`) starting at 1. Numbers only go up and are never reused, so a client can always resume from the last one it saw.

```json
{"seq":3,"ts":"2026-10-21T18:06:01.843Z","kind":"daemon.started","data":{"pid":58448,"version":"0.1.0"}}
```

`run_id` is set on events that belong to a run: the run's id, as a string. The only kinds so far are `daemon.started` and `daemon.stopped` (with the reason). A crash leaves no `daemon.stopped`, so a `started` straight after a `started` means the daemon crashed.

A live subscriber first reads stored events from the database, then switches to an in-memory channel. If it falls more than 1024 events behind, it catches up from the database again, so a slow client never misses events and never sees one twice.

## API

HTTP/1.1 over `~/.cos/cos.sock`, and over the tailnet when remote access is on (see [Remote access](#remote-access)). The paths are versioned because the iPhone uses the same routes over the tailnet.

| Route | Returns |
|---|---|
| `GET /v1/status` | `{version, pid, started_at, home, events, last_seq, tailnet}`. `tailnet` is a `TailnetStatus` (below). |
| `GET /v1/events?after=N&limit=M` | `{events: [...]}`, oldest first. `limit` defaults to 100, max 1000. |
| `GET /v1/events/stream?after=N` | Server-sent events, one per log event, with `id: <seq>` and the event as JSON `data`. A `Last-Event-ID` header overrides `after`. Keep-alive comments every 15 s. |

Try it with curl:

```sh
curl --unix-socket ~/.cos/cos.sock http://cos/v1/status
curl -N --unix-socket ~/.cos/cos.sock 'http://cos/v1/events/stream?after=0'
```

## Projects and workspaces

A **project** is a registered git repo. A **workspace** is a git worktree of a project that cos creates and owns. Agents run inside workspaces: the chief, agents you start, and reviewers all share the workspace's folder. When the chief runs tasks in parallel, each of those workers gets its own short-lived worktree branched from the workspace, merged back after review. Those task worktrees are internal and aren't listed as workspaces.

- Registering a subfolder or a linked worktree of a repo registers its main checkout, so a repo can't be added twice.
- A workspace's folder is `<cos home>/worktrees/<project name>-<project id>/<name>`. Its branch is `cos/<name>`, where `<name>` is a city (`lisbon`, then `lisbon-v1`, `lisbon-v2`, … if taken).
- A workspace starts from either:
  - `origin` (the default): cos fetches `origin/<base branch>` and starts from exactly that commit. If the fetch fails, nothing is created; cos never falls back to a stale branch.
  - `head`: the main checkout's current `HEAD`, so agents see commits you haven't pushed. Uncommitted changes are not copied.
- Removing a workspace deletes its folder only if it has no uncommitted changes, and always keeps its branch, so commits are never lost. Removing a project only unregisters it; your repo is never touched. A project with workspaces can't be removed.

### Routes

| Route | Body | Returns |
|---|---|---|
| `GET /v1/projects` | | `{projects: [Project]}`, in the order they were added, each with its workspaces |
| `POST /v1/projects` | `{"path": "/abs/path"}` | `Project`: 201 if new, 200 if already registered |
| `DELETE /v1/projects/{id}` | | 204. 409 if it still has workspaces. |
| `POST /v1/projects/{id}/workspaces` | `{"base": "origin" or "head", "title": "optional"}` | 201 `Workspace` |
| `PATCH /v1/workspaces/{id}` | `{"title": "..."}` | `Workspace` |
| `DELETE /v1/workspaces/{id}` | | 204. 409 if the worktree has uncommitted changes or an agent is running in it. Its exited agents and their approvals are forgotten with it. |

Errors are `{"error": "message"}`, and the message is written to be shown to the user as-is. Codes: 400 bad input, or git refused (not a repo, no `origin` remote, fetch failed); 404 unknown id; 409 conflict.

```jsonc
// Project
{"id": 1, "name": "cos", "root": "/Users/me/personal/apps/cos", "base_branch": "main",
 "has_origin": true, "created_at": "2026-10-22T09:00:00Z", "workspaces": [/* Workspace */]}
// Workspace
{"id": 3, "project_id": 1, "name": "lisbon", "title": "lisbon", "branch": "cos/lisbon",
 "path": "/Users/me/.cos/worktrees/cos-1/lisbon", "base": "origin", "base_ref": "origin/main",
 "base_commit": "3f2c…40 hex", "created_at": "2026-10-22T09:01:00Z"}
```

`base_branch` comes from `origin/HEAD` when there's a remote, otherwise from the main checkout's current branch.

### Events

Every change is logged, so clients that follow the stream can refresh their project list when one arrives:

| Kind | Data |
|---|---|
| `project.added` | `{project_id, name, root}` |
| `project.removed` | `{project_id, name}` |
| `workspace.created` | `{workspace_id, project_id, name, branch, path, base}` |
| `workspace.renamed` | `{workspace_id, project_id, title}` |
| `workspace.removed` | `{workspace_id, project_id, name, branch}` |

## Task git lifecycle

When the chief runs tasks in parallel, each worker gets its own worktree. The git side of that is `cos_core::taskgit`. It has no database tables and no routes: the orchestrator stores the `TaskWorktree` it gets back and calls these steps in order.

| Step | Call | Result |
|---|---|---|
| Wait for a slot | `WorkerLimiter::acquire(workspace_id)` | A `WorkerPermit`; hold it while the worker runs |
| Create | `TaskWorktrees::new(<cos home>/tasks).create(&workspace, task_id, title)` | `TaskWorktree {task_id, workspace, workspace_branch, path, branch, base_commit}` |
| Check scope | `task.check_scope(&Scope::new(globs)?)` | `ScopeReport`; `is_ok()` or the violations |
| Test | `task.run_tests(command, &TestOptions::default())` | `TestOutcome {status: passed/failed/timed_out, exit_code, signal, output, truncated, duration_ms}` |
| Review | `task.diff_summary()`, `task.commits_ahead()` | Files changed, insertions, deletions, `git diff --stat` text |
| Merge | `task.merge(title)` | `Merged {commit}`, `AlreadyMerged`, or `Conflict {paths}` |
| Clean up | `task.remove(force)`, then `task.delete_branch()` | |

**Naming.** A task worktree starts at the tip of the workspace branch. Its branch is `cos/<workspace>-t<task id>-<slug of title>`, for example `cos/lisbon-t7-add-login-page`, and its folder is `<dir>/<workspace>-<workspace id>/t7-add-login-page`. If the branch or folder exists, `-2`, `-3`, … is added. It can't be `cos/lisbon/t7-…`, because git can't hold a branch `cos/lisbon` and branches under `cos/lisbon/` at once. Uncommitted changes in the workspace aren't copied into the task worktree.

**Scope.** A task's changes are its commits, `git diff <base commit>...<task branch>`, which is what the merge brings in. The report fails if any of these is true:

- A changed path isn't on the allowed list (`out_of_scope`). Deleted files count, and a rename counts as both its old and new path. A copy counts only its new path.
- A changed path is protected (`protected`), even if the list allows it: anything inside `.git/`, `.claude/`, `.codex/` or `.cos/`, any file or folder starting with `.env`, and `.mcp.json`, at any depth and in any letter case. These give agents permissions or hold secrets, so only you change them. A protected path is listed only under `protected`.
- The worker left uncommitted or untracked files (`uncommitted`). Workers must commit. Uncommitted work would never be merged, so cos rejects it instead of guessing. Gitignored files don't count, and they're deleted with the worktree.
- The worktree isn't on the task branch any more (`wrong_branch`).

Allowed-file globs are relative to the repo root and anchored there. `*` and `?` stay within one path segment, a `**` segment matches any number of folders, and everything else is literal, so `app/[id].tsx` means that file. A pattern that matches a folder covers everything inside it: `src/auth`, `src/auth/` and `src/auth/**` all mean the same. `*.md` matches only top-level files; use `**/*.md` for all of them. Patterns with `..` are refused.

**Tests.** The command runs as `sh -c` in the task worktree, in its own process group, with stdin closed. The default timeout is 15 minutes. On a timeout the whole group gets SIGTERM, then SIGKILL 2 s later. Anything the command leaves running in its group when it exits, like a background server, is killed too. cos keeps the last 64 KiB of stdout and stderr together, interleaved as written. If the command can't start, that's an error; a failing command is an outcome.

**Merge.** The task branch is merged into the workspace branch inside the workspace worktree, with `--no-ff` and the message `Merge task <id>: <title>`. Merges into one workspace must not run at the same time. Git hooks run as usual. A merge is refused if the workspace has uncommitted or untracked files, or isn't on its branch. On a conflict cos runs `git merge --abort`, so the workspace is exactly as before, and returns the conflicting paths so the task can be marked blocked. If the task branch is already in the workspace branch, the result is `AlreadyMerged` and nothing changes.

**Cleanup.** `remove` deletes the task's folder, but refuses if there are uncommitted changes unless forced. It always keeps the branch. `delete_branch` deletes the branch only once the workspace branch contains it, so commits are never lost.

**Limits.** `WorkerLimiter::default()` allows 3 workers per workspace and 6 in total. A worker takes a workspace slot first and then an overall one, so a worker queued behind its own workspace's cap never holds a slot another workspace could use.
## Remote access

Your other devices (the iPhone app, another Mac, `cos --host`) reach the daemon over Tailscale. It is **off by default**: `cos remote on` turns it on, `cos remote off` turns it off. The setting is stored in the database (`settings` table, key `tailnet`), so it survives restarts and needs no launchd change, and the daemon applies it live.

```
cos remote on            # listen on port 7700
cos remote on --port 7701
cos remote               # where it listens, or why it can't
cos devices              # who has tried to connect
cos devices approve 2    # by number, MagicDNS name or Tailscale node ID
```

### How it works

- **Binding.** The daemon asks `tailscale status --json` for this machine's tailnet addresses (100.64.0.0/10 and fd7a:115c:a1e0::/48 only) and binds the port on each of them. It never binds `0.0.0.0` or loopback. Plain HTTP, because WireGuard already encrypts the traffic.
- **Finding `tailscale`.** `$COS_TAILSCALE` if set, else `tailscale` on `PATH`, else `/Applications/Tailscale.app/Contents/MacOS/Tailscale`. It is looked up on every call, so installing Tailscale later needs no restart.
- **When Tailscale isn't usable** (not installed, not running, logged out, switched off, or this machine is a tagged node), the daemon keeps serving the Unix socket, reports why in `tailnet.reason`, and tries again every 30 s. While listening it checks every 30 s that this machine's addresses and owner haven't changed, and rebinds or stops if they have.
- **macOS firewall.** If the application firewall is on and blocks incoming connections to the `cos` binary, the listener still binds but macOS drops every connection before cos sees it. `tailnet.reason` then says so, with the command to allow it (`sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp <path to cos>`), or use System Settings > Network > Firewall > Options. A rebuilt binary at a new path needs allowing again.

### Who gets in

Every request on the tailnet listener goes through two checks, in this order:

1. **Same Tailscale user.** `tailscale whois --json <client ip:port>` must find the peer, the peer must not be a tagged device, and its `UserProfile.ID` must equal this machine's `Self.UserID` from `tailscale status --json`, with matching login names (case-insensitive) when both are known. Anything else gets 403. The answer is cached per client IP for 60 s, so the CLI runs about once a minute per device, not per request.
2. **Approved device.** The first request from a new device (by Tailscale's stable node ID) records it as `pending` and logs `device.requested`. Pending and denied devices get 403 with a message naming the command to run on the host. Approval is read from the database on every request, so approving or denying takes effect at once.

The Unix socket is trusted as before: only your user can open it. Device and listener management (`/v1/devices…`, `/v1/tailnet`) work **only** on the socket; over the tailnet they answer 403 `host_only`, so a device can't approve itself. Every other route is served on both, and the guard runs before routing, so an unapproved client can't even tell which paths exist.

A connection from this Mac to its own tailnet address is a tailnet peer too: whois names this machine, and it needs approving like any other device. It isn't approved automatically, because any local process, including other OS users' processes, can open that address, while the socket is private to you.

### Refusals

Over the tailnet, refusals carry a stable `code` next to the message, so apps can show the right screen:

```json
{"error": "this device (phone) is waiting for approval. On my-mac, run `cos devices approve 2`", "code": "device_pending"}
```

| Status | `code` | When |
|---|---|---|
| 403 | `unknown_peer` | Tailscale doesn't know the client address. |
| 403 | `tagged_device` | The client is a tagged node, which has no user. |
| 403 | `wrong_user` | The client is logged in as a different Tailscale user. |
| 403 | `device_pending` | New device, not yet approved. |
| 403 | `device_denied` | Denied on the host. |
| 403 | `host_only` | Device or listener management over the tailnet. |
| 503 | `tailscale_unavailable` | `tailscale whois` failed; try again. |

### Routes (Unix socket only)

| Route | Body | Returns |
|---|---|---|
| `GET /v1/devices` | | `{devices: [Device]}`, oldest first |
| `POST /v1/devices/{id}/approve` | | `Device` |
| `POST /v1/devices/{id}/deny` | | `Device` |
| `DELETE /v1/devices/{id}` | | 204. If it connects again it starts over as pending. |
| `GET /v1/tailnet` | | `TailnetStatus` |
| `PUT /v1/tailnet` | `{"enabled": true, "port": 7700}` (`port` optional, 1–65535) | `TailnetStatus`, after waiting up to 15 s for the listener to start or stop |

Unknown device ids get 404, a bad port 400.

```jsonc
// Device
{"id": 2, "node_id": "nAbC123CNTRL", "name": "phone", "hostname": "My Phone", "os": "iOS",
 "login_name": "me@example.com", "state": "pending", // or "approved", "denied"
 "last_ip": "100.101.102.103", "first_seen_at": "2026-10-22T09:00:00Z",
 "last_seen_at": "2026-10-22T09:00:00Z", "decided_at": "2026-10-22T09:01:00Z"} // decided_at only once decided
// TailnetStatus
{"enabled": true, "port": 7700,
 "state": "listening", // "off", "starting", "listening" or "unavailable"
 "addresses": ["100.101.102.104:7700", "[fd7a:115c:a1e0::1]:7700"],
 "reason": "…",            // why it isn't listening, or a warning; absent when there's nothing to say
 "host": "my-mac",         // this machine's MagicDNS name, once Tailscale has answered
 "owner": "me@example.com"} // the Tailscale user devices must be logged in as
```

`last_seen_at` and `last_ip` are refreshed when the whois cache is (about once a minute while a device is active), not on every request.

### Events

| Kind | Data |
|---|---|
| `device.requested` | `{device_id, node_id, name, hostname, os}` |
| `device.approved` | `{device_id, node_id, name}` |
| `device.denied` | `{device_id, node_id, name}` |
| `device.removed` | `{device_id, node_id, name}` |
| `tailnet.changed` | the new `TailnetStatus`, logged whenever it changes (on, off, listening, unavailable with a new reason) |

## Runs, tasks and the board

A **run** is one goal in one workspace. The chief splits it into **tasks**, each done by one worker in its own short-lived worktree and merged back into the workspace's branch. The daemon keeps the books: which state everything is in, which moves are legal, and when a task may start. It refuses illegal moves rather than trusting the agents.

A workspace has at most one unfinished run, because every task of a run merges into the workspace's branch.

### Run states

```
planning → awaiting_plan_approval → running → reporting → done
   (any state that isn't finished) → failed | cancelled
```

| State | Meaning | Leaves when |
|---|---|---|
| `planning` | The chief is working out the tasks. | The chief proposes a plan. |
| `awaiting_plan_approval` | A plan is on the board. Proposing again replaces it. | You approve it. |
| `running` | Workers are on the tasks. | Every task is merged, failed or cancelled; the daemon then moves the run on by itself. A `blocked` task keeps the run here until the chief or you decide. |
| `reporting` | The chief is writing the report. | The report is recorded. |
| `done`, `failed`, `cancelled` | Finished. `failed` and `cancelled` keep a `reason`. | Never. |

Cancelling or failing a run cancels every task that hasn't finished.

### Task states

```
proposed → ready → running → checking → review → merged
                     ↑          │          │
                     └──────────┴──────────┘   retry (once per task)
```

| From | May go to |
|---|---|
| `proposed` | `ready`, `blocked` |
| `ready` | `running`, `blocked` |
| `running` | `checking`, `blocked`, `failed`; `ready` as a retry (the worker crashed) |
| `checking` | `review`, `blocked`, `failed`; `running` as a retry (scope or tests failed) |
| `review` | `merged`, `blocked`, `failed`; `running` as a retry (the reviewer wants changes) |
| `blocked` | `failed`; `ready` as a retry |
| any unfinished state | `cancelled` |

- A task becomes `ready` only while its run is `running` and every task it depends on is `merged`. The daemon does this itself: on approval for tasks without dependencies, and whenever a task merges for the tasks waiting on it. Asking for `ready` earlier is a 409.
- Each task gets one retry. A second is refused (409); fail or block the task instead.
- Moving to `blocked` or `failed`, or retrying, needs a `reason` (400 without one). It is kept as the task's `last_failure` so the worker, the chief and you can see why.
- When a task fails or is cancelled, every `proposed` task that depends on it, directly or not, becomes `blocked`, since it can never start.
- Tasks only move while their run is `running`.

### Plans

The chief proposes a plan with `board_propose`. It is checked before it reaches the board, and every problem is listed in one 400 so the chief can fix them all at once:

- 1 to 12 tasks, each with a title (unique, ignoring case), instructions and at least one file.
- Files are paths or globs relative to the repo (`src/limit.rs`, `src/limit/**`), not absolute and without `..`.
- `depends_on` names other tasks in the plan by title. No task may depend on itself, and there may be no cycles.
- Two tasks that can run at the same time (neither depends on the other, even indirectly) may not list the same file. This only catches identical entries; the diff check after each worker is what enforces the scope.

### Routes

| Route | Body | Returns |
|---|---|---|
| `GET /v1/runs` | | `{runs: [RunSummary]}`, newest first |
| `POST /v1/runs` | `{"workspace_id": 1, "goal": "...", "preset": "normal", "approval_cap": 20}`; `preset` and `approval_cap` are optional | 201 `Run`. 404 unknown workspace, 409 if it already has an unfinished run. |
| `GET /v1/runs/{id}` | | `Board` |
| `POST /v1/runs/{id}/plan` | `{"tasks": [ProposedTask]}` | `Board`. 400 invalid plan, 409 once approved. |
| `POST /v1/runs/{id}/approve-plan` | | `Board`. 409 if no plan is waiting. |
| `POST /v1/runs/{id}/cancel` | optional `{"reason": "..."}` | `Board`. 409 if already finished. |
| `PATCH /v1/tasks/{id}` | `{"state": "checking", "reason": "optional"}` | `Board`, since other tasks and the run may have moved too. 400 missing reason, 409 illegal move. |

```jsonc
// Run
{"id": 3, "workspace_id": 1, "goal": "add rate limiting", "state": "running",
 "preset": "normal", "approval_cap": 20,
 "report_path": "reports/…md",            // once written
 "reason": "…",                           // once failed or cancelled
 "created_at": "2026-10-22T09:00:00Z", "updated_at": "2026-10-22T09:05:00Z"}
// RunSummary: a Run with task counts added
{"id": 3, …, "tasks": TaskCounts}
// TaskCounts
{"proposed": 0, "ready": 1, "running": 1, "checking": 0, "review": 0,
 "merged": 2, "blocked": 0, "failed": 0, "cancelled": 0, "total": 4}
// Board
{"run": Run, "workspace": Workspace /* null once removed */, "tasks": [Task], "counts": TaskCounts}
// Task
{"id": 7, "run_id": 3, "position": 0, "title": "Add limiter", "instructions": "…",
 "files": ["src/limit.rs"], "test_command": "cargo test",  // optional
 "depends_on": [6], "state": "running", "retries": 0,
 "branch": "…", "worktree": "/…",           // once the worktree exists
 "last_failure": "…",                       // once blocked, failed or retried
 "created_at": "…", "updated_at": "…"}
// ProposedTask, in a plan
{"title": "Add limiter", "instructions": "…" /* or "description" */, "files": ["src/limit.rs"],
 "test_command": "cargo test", "depends_on": ["Title of another task"]}
```

`preset` is `strict`, `normal` or `trusted` (see "Approving on your behalf" in [proposal.md](proposal.md)).

The board is also rendered as markdown by `cos_core::render_board`. `cos board` prints it, and the daemon will write it to `<repo>/.cos/board.md`.

### CLI

| Command | What it does |
|---|---|
| `cos run "<goal>" [--workspace <id, name or title>] [--preset …] [--approval-cap N]` | Starts a run. Without `--workspace`, it uses the workspace you're in, or the only workspace of the project you're in. |
| `cos runs` | Every run, newest first, with task counts. |
| `cos board [run] [--json]` | The board as markdown. |
| `cos approve-plan [run]` | Approves the waiting plan. |
| `cos cancel [run] [--reason …]` | Cancels the run and its unfinished tasks. |

Without a run id, a command picks the unfinished run in the workspace you're in, or else the only unfinished run (for `approve-plan`, the only run waiting for approval). `cos board` falls back to the newest run.

### Events

All of them carry the run's id in `run_id`.

| Kind | Data |
|---|---|
| `run.created` | `{run_id, workspace_id, goal, preset, approval_cap}` |
| `run.state_changed` | `{run_id, from, to, reason?, report?}` |
| `plan.proposed` | `{run_id, tasks, replaced}`: how many tasks, and how many from an earlier plan were replaced |
| `plan.approved` | `{run_id, tasks}` |
| `task.state_changed` | `{run_id, task_id, title, from, to, retries, reason?}` |
| `task.worktree_assigned` | `{run_id, task_id, branch, worktree}` |

One change can log several events. Approving a plan logs `plan.approved`, then the run's move to `running`, then each task that became `ready`.

## Agents

An **agent** is a Claude Code session the daemon runs in a pseudo-terminal it owns, in a workspace's folder. Apps show its terminal live and type into it; the daemon follows it through hooks, not by reading the screen. The routes, JSON, events and CLI are in [agents-api.md](agents-api.md); this section is how the daemon does it. The code is `crates/cos/src/daemon/agents.rs` (process side) and `cos_core::agents` (the `agents` and `approvals` tables).

- **Starting:** the daemon picks the session id, registers it, then runs `claude --session-id <id> --settings <hooks>` in the workspace folder. The hooks run `cos hook`, which relays each one to `<home>/hooks.sock`. `COS_CLAUDE_BIN` overrides which `claude` runs (tests use `cos-fake-claude`).
- **States** come from hooks: `SessionStart` → `idle`, `UserPromptSubmit` and `PreToolUse` → `working`, `PermissionRequest` → `waiting_approval`, `Stop` → `idle`, process exit → `exited`. The one thing read from the screen is Claude's trust dialog (`needs_trust`).
- **Approvals:** a `PermissionRequest` hook waits on the socket while the approval is `pending`. Answering it through the API answers the hook. If the hook goes away (answered in the terminal) or the agent's next hook arrives first, the approval is `settled` by `terminal`. Approvals of agents runs start are denied after 5 minutes; yours wait.
- **Restarts:** when the daemon stops, it asks each agent to exit (`/exit`, then kill) and marks it `interrupted`. If it crashed instead, agents still marked running become `interrupted` on the next start. `resume` runs `claude --resume <session id>` as the same agent.
- **Other sessions:** every 2 s the daemon reads Claude's `sessions/<pid>.json` files and logs `session.changed` when the list of sessions it didn't start changes (`GET /v1/sessions`).
