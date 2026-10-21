# The cos daemon

`cos daemon` is the long-running process that will own runs, workers and approvals. As of milestone 3 it owns the database and the event log, and serves an HTTP API on a Unix socket. Runs and workers arrive in milestones 4 and 5.

## Commands

| Command | What it does |
|---|---|
| `cos daemon` | Run in the foreground. Ctrl-C or SIGTERM stops it cleanly. |
| `cos daemon install` | Install a launchd agent (`local.cos.daemon`): starts now, at every login, and again about 10 s after a crash. |
| `cos daemon uninstall` | Stop the agent and remove its plist. |
| `cos status` | Whether the daemon is running, and its pid, uptime, event count and launchd state. Exits 1 when it isn't running. |
| `cos events` | The last 20 events. `-n 50` for more, `--after N` to start after event N, `--json` for one JSON object per line. |
| `cos events -f` | Keep printing new events. If the daemon restarts, it waits and carries on from the last event it printed. |

## Files

Everything lives in `~/.cos`, or in `$COS_HOME` if that's set. The directory is readable by you only.

| Path | |
|---|---|
| `cos.sock` | The API socket (mode 0600). Removed on a clean stop, and replaced on start if a crash left it behind. |
| `cos.db` | SQLite in WAL mode. The schema version is in `PRAGMA user_version`; migrations are in `crates/cos-core/src/store.rs`. |
| `daemon.lock` | Holds the running daemon's pid. Its lock means only one daemon runs per home. |
| `logs/daemon.log` | The daemon's stderr when launchd runs it. |

The launchd plist records the binary's path, `COS_HOME` and your `PATH` at install time. Run `cos daemon install` again after moving the binary or changing `PATH`.

## Event log

Every event has a sequence number (`seq`) starting at 1. Numbers only go up and are never reused, so a client can always resume from the last one it saw.

```json
{"seq":3,"ts":"2026-10-21T18:06:01.843Z","kind":"daemon.started","data":{"pid":58448,"version":"0.1.0"}}
```

`run_id` is added once events belong to a run. The only kinds so far are `daemon.started` and `daemon.stopped` (with the reason). A crash leaves no `daemon.stopped`, so a `started` straight after a `started` means the daemon crashed.

A live subscriber first reads stored events from the database, then switches to an in-memory channel. If it falls more than 1024 events behind, it catches up from the database again, so a slow client never misses events and never sees one twice.

## API

HTTP/1.1 over `~/.cos/cos.sock`. The paths are versioned because the iPhone will use the same routes over the tailnet later.

| Route | Returns |
|---|---|
| `GET /v1/status` | `{version, pid, started_at, home, events, last_seq}` |
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
| `DELETE /v1/workspaces/{id}` | | 204. 409 if the worktree has uncommitted changes. |

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
