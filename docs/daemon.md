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
