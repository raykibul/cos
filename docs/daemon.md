# The cos daemon

`cos daemon` is the long-running process that will own runs, workers and approvals. As of milestone 3 it owns the database and the event log, and serves an HTTP API on a Unix socket. It also keeps the books for runs and their tasks (see "Runs, tasks and the board" below); the pipeline drives each task through a worker and a reviewer (see "Pipeline"), and the chief that plans runs arrives in milestone 5. It runs agents (Claude Code in terminals it owns) and holds their permission requests as approvals (see "Agents" below).

It runs on macOS and Linux. To run it on a Linux server such as EC2 and drive it from the Mac app over Tailscale, see [linux.md](linux.md).

## Commands

| Command | What it does |
|---|---|
| `cos daemon` | Run in the foreground. Ctrl-C or SIGTERM stops it cleanly, asking its agents to exit first. |
| `cos daemon install` | Run it as a service: on macOS a launchd agent (`local.cos.daemon`) that starts now, at every login, and again about 10 s after a crash; on Linux a systemd user unit (`~/.config/systemd/user/cos.service`), enabled and started, restarted 10 s after a crash. See [Running as a service](#running-as-a-service). |
| `cos daemon uninstall` | Stop the service and remove its plist or unit. |
| `cos status` | Whether the daemon is running, and its pid, uptime, event count and service state. Exits 1 when it isn't running. |
| `cos events` | The last 20 events. `-n 50` for more, `--after N` to start after event N, `--json` for one JSON object per line. |
| `cos events -f` | Keep printing new events. If the daemon restarts, it waits and carries on from the last event it printed. |
| `cos remote [on [--port N] \| off]` | Turn remote access over Tailscale on or off, or show where it listens. See [Remote access](#remote-access). |
| `cos devices [list \| approve \| deny \| remove <device>]` | The tailnet devices that may use this daemon. |
| `cos env [set NAME=value \| unset NAME] [--project P]` | The environment variables agents get, globally or per project. See [Environment](#environment). |
| `cos --host <ip or name>[:port] …` | Talk to another machine's daemon over its tailnet listener instead of the local socket (also `COS_HOST`). The port defaults to 7700. |

## Files

Everything lives in `~/.cos`, or in `$COS_HOME` if that's set. The directory is readable by you only.

| Path | |
|---|---|
| `cos.sock` | The API socket (mode 0600). Removed on a clean stop, and replaced on start if a crash left it behind. |
| `hooks.sock` | Where the agents' hooks (relayed by `cos hook`) reach the daemon (mode 0600). One socket for every agent; hooks are routed by session id. |
| `cos.db` | SQLite in WAL mode. The schema version is in `PRAGMA user_version`; migrations are in `crates/cos-core/src/store.rs`. |
| `daemon.lock` | Holds the running daemon's pid. Its lock means only one daemon runs per home. |
| `logs/daemon.log` | The daemon's stderr when launchd or systemd runs it. |
| `daemon.env` | Optional, Linux only: `NAME=value` lines the systemd unit loads, for secrets such as `ANTHROPIC_API_KEY` that `install` never copies into the unit. Keep it mode 0600. |
| `tasks/` | Task worktrees (`tasks/<workspace>-<id>/t<task id>-<slug>`), one per task a run is working on. Removed once the task merges, or when it fails or is cancelled with no commits. |
| `runs/<run id>/board.md` | The run's board as markdown, rewritten whenever it changes. Nothing is written inside the repo. |
| `runs/<run id>/report.md` | The chief's report, with the "Approved on your behalf" section appended. |

## Running as a service

`cos daemon install` records the binary's path, `COS_HOME` and your `PATH` at install time, and also `CLAUDE_CONFIG_DIR`, `COS_CLAUDE_BIN`, `COS_TAILSCALE`, `COS_CHIEF_MODEL`, `COS_STAFF_MODEL` and `COS_RUN_MODEL` if they're set, so agents use the same Claude config and binary as your shell. Run `cos daemon install` again after moving the binary or changing any of them. If `claude` isn't on your `PATH` but is where its installers put it (`~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin`, nvm's newest Node, `/usr/local/bin`, `/opt/homebrew/bin`), its folder is added to the recorded `PATH`, and the daemon finds it there at start too.

- **macOS:** a launchd agent, `~/Library/LaunchAgents/local.cos.daemon.plist`, in your login session (`gui/<uid>`).
- **Linux:** a systemd user unit, `~/.config/systemd/user/cos.service` (`$XDG_CONFIG_HOME` if set). `install` runs `systemctl --user daemon-reload`, `enable` and `restart`; `uninstall` runs `disable --now`, removes the file and reloads. The unit has `Restart=on-failure`, `KillMode=mixed` (SIGTERM to the daemon alone, which stops its agents itself) and appends output to `logs/daemon.log`. systemd stops a user's services when their last session ends unless lingering is on, so `install` warns when `loginctl` says it's off; on a server, run `sudo loginctl enable-linger $USER`. `install` needs your user's systemd, so run it in an SSH login rather than a `su` shell. See [linux.md](linux.md).
- **Elsewhere,** or without systemd (containers), `install` fails with a message; run `cos daemon` under your own supervisor.

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
- A workspace's folder is `<cos home>/worktrees/<project name>-<project id>/<name>`. Its branch is `<project slug>/<name>`, where `<name>` is a city (`lisbon`, then `lisbon-v1`, `lisbon-v2`, … if the branch or the folder is taken). The slug is the project's name in lowercase, with runs of anything other than `[a-z0-9]` turned into `-` and the ends trimmed (`My App_2` → `my-app-2`). Workspaces made before this keep their `cos/<name>` branches. If the repo already has a branch named exactly like the slug, git can't hold branches under it, so creating a workspace is refused with that reason.
- A workspace's title is optional. Without one it is `null`, and clients show the city (`city`, the last segment of the branch).
- Each project also has a hidden worktree for its Chief of Staff (`<project slug>/chief`, folder `chief`), made from the main checkout's `HEAD` the first time the chief starts. It is stored as a workspace row of kind `staff`, so its agent has a workspace, but it isn't listed and the workspace routes don't touch it. See "The Chief of Staff" below.
- A workspace starts from either:
  - `origin` (the default): cos fetches `origin/<base branch>` and starts from exactly that commit. If the fetch fails, nothing is created; cos never falls back to a stale branch.
  - `head`: the main checkout's current `HEAD`, so agents see commits you haven't pushed. Uncommitted changes are not copied.
- Removing a workspace deletes its folder only if it has no uncommitted changes, and always keeps its branch, so commits are never lost. Removing a project only unregisters it; your repo is never touched. A project with workspaces can't be removed. Its Chief of Staff goes with it (its worktree, agent record and inbox), unless the chief is running.

### Routes

| Route | Body | Returns |
|---|---|---|
| `GET /v1/projects` | | `{projects: [Project]}`, in the order they were added, each with its workspaces |
| `POST /v1/projects` | `{"path": "/abs/path"}` | `Project`: 201 if new, 200 if already registered |
| `DELETE /v1/projects/{id}` | | 204. 409 if it still has workspaces, or its Chief of Staff is running. |
| `POST /v1/projects/{id}/workspaces` | `{"base": "origin" or "head", "title": "optional"}` | 201 `Workspace` |
| `PATCH /v1/workspaces/{id}` | `{"title": "..."}` | `Workspace`. A null or empty title clears it. |
| `DELETE /v1/workspaces/{id}` | | 204. 409 if the worktree has uncommitted changes or an agent is running in it. Its exited agents and their approvals are forgotten with it. |

Errors are `{"error": "message"}`, and the message is written to be shown to the user as-is. Codes: 400 bad input, or git refused (not a repo, no `origin` remote, fetch failed); 404 unknown id; 409 conflict.

```jsonc
// Project
{"id": 1, "name": "cos", "root": "/Users/me/personal/apps/cos", "base_branch": "main",
 "has_origin": true, "created_at": "2026-10-22T09:00:00Z", "workspaces": [/* Workspace */]}
// Workspace
{"id": 3, "project_id": 1, "name": "lisbon", "city": "lisbon", "title": null, "branch": "cos/lisbon",
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
| `workspace.created` | `{workspace_id, project_id, name, city, title, branch, path, base}` |
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

**Naming.** A task worktree starts at the tip of the workspace branch. Its branch is `<workspace branch>-t<task id>-<slug of title>`, for example `cos/lisbon-t7-add-login-page`, and its folder is `<dir>/<workspace>-<workspace id>/t7-add-login-page`. If the branch or folder exists, `-2`, `-3`, … is added. It can't be `cos/lisbon/t7-…`, because git can't hold a branch `cos/lisbon` and branches under `cos/lisbon/` at once. Uncommitted changes in the workspace aren't copied into the task worktree.

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

Your other devices (the iPhone app, another Mac, `cos --host`) reach the daemon over Tailscale. It is **on by default**, so a new daemon is reachable as soon as Tailscale is up: `cos remote off` turns it off, `cos remote on` turns it back on. Only devices logged in as your Tailscale user, and approved on this machine, get in. The setting is stored in the database (`settings` table, key `tailnet`), so it survives restarts and needs no launchd change, and the daemon applies it live.

```
cos remote on            # listen on port 7700
cos remote on --port 7701
cos remote               # where it listens, or why it can't
cos devices              # who has tried to connect
cos devices approve 2    # by number, MagicDNS name or Tailscale node ID
cos remote peers         # the other devices on your tailnet; * marks those running cos
```

### How it works

- **Binding.** The daemon asks `tailscale status --json` for this machine's tailnet addresses (100.64.0.0/10 and fd7a:115c:a1e0::/48 only) and binds the port on each of them. It never binds `0.0.0.0` or loopback. Plain HTTP, because WireGuard already encrypts the traffic.
- **Finding `tailscale`.** `$COS_TAILSCALE` if set, else `tailscale` on `PATH`, else `/Applications/Tailscale.app/Contents/MacOS/Tailscale` on macOS, or `/usr/bin/tailscale` and `/usr/local/bin/tailscale` on Linux. It is looked up on every call, so installing Tailscale later needs no restart.
- **When Tailscale isn't usable** (not installed, not running, logged out, switched off, or this machine is a tagged node, which has no owner to compare devices with: log it in as yourself instead), the daemon keeps serving the Unix socket, reports why in `tailnet.reason`, and tries again every 30 s. While listening it checks every 30 s that this machine's addresses and owner haven't changed, and rebinds or stops if they have.
- **macOS firewall.** If the application firewall is on and blocks incoming connections to the `cos` binary, the listener still binds but macOS drops every connection before cos sees it. `tailnet.reason` then says so, with the command to allow it (`sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp <path to cos>`), or use System Settings > Network > Firewall > Options. A rebuilt binary at a new path needs allowing again. The check only runs on macOS; on Linux, Tailscale accepts traffic on `tailscale0` itself, and only a host firewall you added (ufw, firewalld) would need the port opened there.

### Who gets in

Every request on the tailnet listener goes through two checks, in this order:

1. **Same Tailscale user.** `tailscale whois --json <client ip:port>` must find the peer, the peer must not be a tagged device, and its `UserProfile.ID` must equal this machine's `Self.UserID` from `tailscale status --json`, with matching login names (case-insensitive) when both are known. Anything else gets 403. The answer is cached per client IP for 60 s, so the CLI runs about once a minute per device, not per request.
2. **Approved device.** The first request from a new device (by Tailscale's stable node ID) records it as `pending` and logs `device.requested`. Pending and denied devices get 403 with a message naming the command to run on the host. Approval is read from the database on every request, so approving or denying takes effect at once.

The Unix socket is trusted as before: only your user can open it. Device and listener management (`/v1/devices…`, `GET`/`PUT /v1/tailnet`) work **only** on the socket; over the tailnet they answer 403 `host_only`, so a device can't approve itself. Every other route is served on both, `GET /v1/tailnet/peers` included, and the guard runs before routing, so an unapproved client can't even tell which paths exist. The peer list is served to approved devices because an iPhone can't ask Tailscale itself: it asks a host it already reaches, and offers the other machines running cos. Older daemons answer it with `host_only` over the tailnet; apps treat that as "no list from this host".

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
| 403 | `host_only` | Device or listener management over the tailnet (and, on older daemons, the peer list). |
| 503 | `tailscale_unavailable` | `tailscale whois` failed; try again. |

### Routes

All on the Unix socket; only `GET /v1/tailnet/peers` is served over the tailnet too.

| Route | Body | Returns |
|---|---|---|
| `GET /v1/devices` | | `{devices: [Device]}`, oldest first |
| `POST /v1/devices/{id}/approve` | | `Device` |
| `POST /v1/devices/{id}/deny` | | `Device` |
| `DELETE /v1/devices/{id}` | | 204. If it connects again it starts over as pending. |
| `GET /v1/tailnet` | | `TailnetStatus` |
| `PUT /v1/tailnet` | `{"enabled": true, "port": 7700}` (`port` optional, 1–65535) | `TailnetStatus`, after waiting up to 15 s for the listener to start or stop |
| `GET /v1/tailnet/peers?port=7700` | | `PeersPage`: the other devices on the tailnet. `port` (optional, 1–65535) is where to look for cos on them, default the listener's port. Also over the tailnet, for approved devices, with the asking device marked `is_you`. |

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
// PeersPage
{"peers": [{
   "id": "nPEER1stableIDCNTRL",            // Tailscale node ID
   "name": "build-box",                     // first label of DNSName, else HostName
   "hostname": "build-box",
   "dns_name": "build-box.tail0000.ts.net", // trailing dot trimmed; "" if none
   "os": "linux",                           // as Tailscale reports it: macOS, linux, iOS, windows, android…
   "ips": ["100.100.2.2", "fd7a:115c:a1e0::2:2"],
   "online": true,
   "last_seen_ms": null,                    // ms since the epoch when offline and known, else null
   "mine": true,                            // same Tailscale user as this machine and not tagged
   "tagged": false,
   "cos": "running",                        // "running", "not_running" or "unknown"
   "address": "100.100.2.2:7700",           // first IPv4 (else first IP, bracketed for v6) and the port; "" if no IP
   "is_you": false}],                       // the device asking over the tailnet; always false on the socket
 "self_name": "my-mac",                     // null when Tailscale is unavailable
 "self": {"name": "my-mac",                 // the answering host, so apps can say "via my-mac"; null when
          "dns_name": "my-mac.tail0000.ts.net", // Tailscale is unavailable
          "os": "macOS",
          "address": "100.100.1.1:7700"},   // its first tailnet IPv4 (else first IP) and the same port
 "reason": null}                            // why there are no peers: the Tailscale error, still HTTP 200
```

**Peers.** The list comes from the `Peer` map of the same `tailscale status --json` call, parsed leniently: missing fields default, an odd entry is skipped, Tailscale's zero `LastSeen` (`0001-01-01`) is null, and this machine is never listed. Only peers that are `mine` can be approved as devices. For each online peer that is `mine`, the daemon opens a plain TCP connection to `address` (all at once, 700 ms timeout) and closes it without sending anything: a connect is `running`, a refusal or timeout `not_running`. It never sends an HTTP request, which would register this Mac as a pending device over there. Offline and other peers are `unknown`. Answers are reused for 10 s per address and port, so a sheet that refetches doesn't probe again. Order: `running` first, then online, then by name. Over the tailnet the guard's whois answer says who is asking: the peer with that Tailscale node ID, else the one with the client's address, gets `is_you: true`, so a phone can leave itself out. The host itself is never a peer; it is `self`.

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
 "agent_id": 12,                            // the newest worker, once started
 "checks": {"scope": {"ok": true, "out_of_scope": [], "protected": [], "uncommitted": []},
            "tests": {"status": "passed", "exit_code": 0, "output_tail": "…"},  // null if not run
            "commits_ahead": 1},            // once checked
 "review": {"verdict": "approve", "notes": "…", "agent_id": 13},  // once reviewed
 "created_at": "…", "updated_at": "…"}
// ProposedTask, in a plan
{"title": "Add limiter", "instructions": "…" /* or "description" */, "files": ["src/limit.rs"],
 "test_command": "cargo test", "depends_on": ["Title of another task"]}
```

`preset` is `strict`, `normal` or `trusted` (see "Approving on your behalf" in [proposal.md](proposal.md)).

The board is also rendered as markdown by `cos_core::render_board`. `cos board` prints it, and the chief component writes it to `<cos home>/runs/<run id>/board.md` whenever the board changes.

### CLI

| Command | What it does |
|---|---|
| `cos run "<goal>" [--workspace <id, name or title>] [--preset …] [--approval-cap N]` | Starts a run. Without `--workspace`, it uses the workspace you're in, or the only workspace of the project you're in. |
| `cos runs` | Every run, newest first, with task counts. |
| `cos board [run] [--json]` | The board as markdown. |
| `cos approve-plan [run]` | Approves the waiting plan. |
| `cos cancel [run] [--reason …]` | Cancels the run and its unfinished tasks. |
| `cos limits [run] [--preset strict\|normal\|trusted] [--cap N]` | Shows or changes how much the chief may approve for you in the run (see "Approvals policy"). |

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
| `task.checked` | `{run_id, task_id, scope_ok, tests}`: `tests` is `passed`, `failed`, `timed_out`, `skipped` or null |
| `task.reviewed` | `{run_id, task_id, verdict}` |
| `task.merged` | `{run_id, task_id, commit}`: the merge commit, null if the branch was already merged |
| `run.limits_changed` | `{run_id, preset, approval_cap, chief_approvals}` |

One change can log several events. Approving a plan logs `plan.approved`, then the run's move to `running`, then each task that became `ready`.

### Pipeline

The pipeline drives each task from `ready` to `merged` (`crates/cos/src/daemon/pipeline.rs`, one driver per task in `pipeline/driver.rs`). It follows the event log: a task moving to `ready` gets a driver; a run moving to `cancelled` or `failed` has its agents stopped. The steps are those of [orchestration.md](orchestration.md); what differs from that contract is under "Pipeline" at its end.

1. **Start:** wait for a worker slot (3 per workspace, 6 in total), create the task worktree under `<home>/tasks` and record it, then start a worker there (role `worker`, `acceptEdits`, your settings off, `--append-system-prompt` with the rules) with the task as its first prompt. `COS_RUN_MODEL` picks the model for workers and reviewers.
2. **Running:** each `Stop` hook ends a turn. `DONE` → `checking`; `STUCK: why` → `blocked`; no verdict line → the worker is asked once, then `blocked`.
3. **Checking:** the scope check, the test command, and at least one commit. A failure is retried once with the reason typed into the same worker; the second failure blocks the task.
4. **Review:** a new reviewer in the task worktree (role `reviewer`, `plan` mode) gets the diff summary, the diff (cut to 24 KiB), and the test result, and answers `VERDICT: APPROVE` or `VERDICT: CHANGES: …`. Changes are a retry like a failed check.
5. **Merge:** one merge at a time per workspace. A conflict blocks the task with the conflicting paths.
6. **Cleanup:** the worker and reviewer are stopped (their agent rows stay). A merged task's worktree and branch are removed. A failed or cancelled task's are removed only if the branch has no commits.

A blocked task the chief or you move back to `ready` starts again in the same worktree, with the same worker session.

**Restarts.** When the daemon starts, it picks up every running run's `ready`, `running`, `checking` and `review` tasks:

- `ready`: started as usual, reusing a worktree that's already there and resuming the task's worker with the retry reason.
- `running`: the worker is resumed and told cos restarted, to finish and end with its verdict. If it had already ended its turn, its last message is read for the verdict; with no worker, a new one starts.
- `checking`: the checks run again.
- `review`: merged at once if the branch is already in the workspace (the merge happened before the stop); otherwise a new reviewer.
- Cancelled and failed runs: leftover task worktrees are cleaned up by the rule above.

Run agents never show `needs_trust`: the daemon answers their trust dialog itself.

## Agents

An **agent** is a Claude Code session the daemon runs in a pseudo-terminal it owns, in a workspace's folder. Apps show its terminal live and type into it; the daemon follows it through hooks, not by reading the screen. The routes, JSON, events and CLI are in [agents-api.md](agents-api.md); this section is how the daemon does it. The code is `crates/cos/src/daemon/agents.rs` (process side) and `cos_core::agents` (the `agents` and `approvals` tables).

- **Starting:** the daemon picks the session id, registers it, then runs `claude --session-id <id> --settings <hooks>` in the workspace folder (for a worker or reviewer, its task's worktree; the folder, system prompt and MCP config are recorded so `resume` starts it the same way). The hooks run `cos hook`, which relays each one to `<home>/hooks.sock`. `COS_CLAUDE_BIN` overrides which `claude` runs (tests use `cos-fake-claude`).
- **Environment:** each launch and resume gets the global and project variables as they are at that moment, under the ones cos sets itself (see [Environment](#environment)). The agent records their names in `env_names`.
- **States** come from hooks: `SessionStart` → `idle`, `UserPromptSubmit` and `PreToolUse` → `working`, `PermissionRequest` → `waiting_approval`, `Stop` → `idle`, process exit → `exited`. The only things read from the screen, before the first hook, are Claude's trust dialog (`needs_trust`) and its login screen (`needs_login`).
- **Approvals:** a `PermissionRequest` hook waits on the socket while the approval is `pending`. Answering it through the API answers the hook. If the hook goes away (answered in the terminal) or the agent's next hook arrives first, the approval is `settled` by `terminal`. Approvals of agents runs start are denied after 5 minutes; yours wait. Before any of that, the approvals policy decides who answers (see "Approvals policy" below).
- **Restarts:** when the daemon stops, it asks each agent to exit (`/exit`, then kill) and marks it `interrupted`. If it crashed instead, agents still marked running become `interrupted` on the next start. `resume` (`cos agent resume <id>`, the app's Resume) runs `claude --resume <session id>` as the same agent. Only Chiefs of Staff come back by themselves (see [The Chief of Staff](#the-chief-of-staff)); your agents wait for you.
- **Trust:** agents a run or a Chief of Staff started answer Claude's trust dialog by themselves, since they work in a worktree cos made for a registered project. Agents you start leave it to you (`needs_trust`).
- **Transcripts:** every hook carries `transcript_path`, which is stored on the agent when it changes (`agents/transcripts.rs`). After `UserPromptSubmit`, `PostToolUse` and `Stop`, the daemon waits 250 ms, and if the file grew since the last time, logs `agent.transcript` with how many messages are final. It does this at most once a second per agent. `GET /v1/agents/{id}/transcript?after=` reads the file through a cache (`daemon/transcript.rs`): each file's parse is kept, keyed by its length and modification time, and when the file grew only the appended lines are parsed (checking that the last line read still ends where it did); a file that shrank or was rewritten is parsed from the top. At most 64 files are kept. The answer also has `context`, how full the conversation is: see [agents-api.md](agents-api.md).

## Environment

Agents get the environment variables you set in cos: a global list, and one per project. The usual reason is several Claude accounts on one machine. `CLAUDE_CONFIG_DIR` picks the account, so `cos env set CLAUDE_CONFIG_DIR=~/.claude-personal --project topflexi` makes every agent of topflexi use that one while other projects keep `~/.claude`. API keys, `CLAUDE_CODE_OAUTH_TOKEN` and anything else work the same way. The code is `cos_core::env` (the `env` table, migration 9), `daemon/env_api.rs` and `agent_env` in `daemon/agents.rs`.

- **Who gets them:** every agent launched for the project, whatever its role: yours, its Chief of Staff and the agents that one starts, a run's chief, workers and reviewers.
- **Order:** the daemon's own environment, then the global list, then the project's, then what cos sets itself. A project's value wins over a global one with the same name, and both win over the daemon's environment (what `cos daemon install` recorded, see [Running as a service](#running-as-a-service)).
- **When it applies:** the variables are read at every launch, resume and restart. A change reaches agents started or resumed after it. **A running agent keeps the environment it started with**: stop and resume it to pick up a change.
- **`~`:** a value that is exactly `~` or starts with `~/` gets the daemon user's home (`$HOME`) at launch. Nothing else is expanded: no `$VAR`, no `~user`, no `~` after a `:`. The API returns values as stored, unexpanded.
- **`PATH`:** allowed. It replaces the daemon's `PATH` for the agent, so Claude and every tool it runs see only yours: include `/usr/bin:/bin` and the rest. cos still finds `claude` itself on the daemon's `PATH`.
- **Names cos keeps:** refused with 400 and a reason, because cos sets or removes them itself for every agent: `TERM`, `COLORTERM`, `COS_HOOK_SOCKET`, the Claude session markers it removes (`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SESSION_ID`, … see "Inherited environment" in [claude-protocol.md](claude-protocol.md)), and every name starting with `COS_`. The list is `cos_harness::claude::reserved_env_names`. If the database holds one anyway, launches skip it.
- **Validation:** a name matches `^[A-Za-z_][A-Za-z0-9_]*$` and has at most 128 characters. A value has at most 32 KiB and no NUL byte; empty is fine.
- **Secrecy:** values are stored as plain text in `cos.db` (the cos home is readable by you only). The API returns them as they are, on the socket and to approved tailnet devices, which are you; the apps mask them. They never go in events, the daemon's log or error messages: `env.changed` and the agent's `env_names` carry names only.
- **Removing a project** removes its variables. The global list stays.

### `CLAUDE_CONFIG_DIR`

- **Transcripts** come from each hook's `transcript_path`, so they're found in whichever config dir the agent uses.
- **Trust:** Claude remembers trusted folders per config dir, so the first agent in a new one may ask again. cos answers it for run and Chief of Staff agents as usual; yours show `needs_trust`.
- **Login:** an agent whose config dir nobody has logged into shows Claude's first-run or login screen in its terminal, before any hook. The daemon spots it, the only screen read besides the trust dialog, and sets the agent to `needs_login`. Open its terminal (the app, or `cos agent attach <id>`) and log in; Claude keeps the login for that config dir, and the agent carries on (`SessionStart` makes it `idle`). Stopping it there kills it. A run's worker on that screen times out like one that never started. `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`) logs an agent in with no screen at all. A session that loses its login later shows "Not logged in · Please run /login" in its turn; that isn't detected.
- **`cos sessions`** also looks in the `CLAUDE_CONFIG_DIR` of the global list and of each project when the daemon is running.

### Routes

| Route | Body | Result |
|---|---|---|
| `GET /v1/env` | | `{"variables": {"NAME": "value", …}}`: the global list |
| `PUT /v1/env/{name}` | `{"value": "…"}` | 200 with the global list |
| `DELETE /v1/env/{name}` | | 200 with the global list, whether or not it was set |
| `GET /v1/projects/{id}/env` | | `{"variables", "global", "effective"}`: the project's own list, the global list, and their merge (the project's win) that its agents get |
| `PUT /v1/projects/{id}/env/{name}` | `{"value": "…"}` | 200 with the project's lists |
| `DELETE /v1/projects/{id}/env/{name}` | | 200 with the project's lists, whether or not it was set |

Served on the socket and the tailnet listener. Errors: 400 for a bad or reserved name, a bad value or a body that isn't `{"value": "<string>"}`; 404 for an unknown project.

### Events

| Kind | Data |
|---|---|
| `env.changed` | `{project_id, name, action}`: `project_id` is null for the global list, `action` is `set` or `removed`. Never the value. |

### CLI

| Command | What it does |
|---|---|
| `cos env [--project P] [--show]` | Lists the global variables, or a project's above the global ones, marking those it overrides. Values are masked as their first three characters and their length (`sk-…  (108 chars)`; short values show none of themselves) unless `--show`. |
| `cos env set NAME=value [--project P]` | Sets a variable, globally or for a project (id or name). `cos env set NAME` reads the value from stdin, which keeps it out of your shell history. |
| `cos env unset NAME [--project P]` | Removes it. |

## Approvals policy

The `cos-policy` crate decides who answers each permission request; the rules are in [approvals.md](approvals.md). The daemon side is `crates/cos/src/daemon/agents/approvals.rs`, the routes are in `daemon/approvals_api.rs`, and the books (limits, the chief's count, the audit) are in `cos_core::runs` (`runs/limits.rs`).

- **Classifying:** each `PermissionRequest` becomes a `cos_policy::ApprovalRequest`: the tool, its input, the hook's `cwd`, the agent's role, `$HOME`, and a worktree. A worker or reviewer with a known task is judged against the task's worktree, its `files` as the scope and its test command. The chief, and a worker without a known task, are judged against the workspace with nothing in scope. Your own agents are judged against the workspace with everything in scope, only to show the tier. `classify_with(…, &Filesystem)` follows symlinks.
- **Routing:** `Limits::route` with the run's preset, cap and `chief_approvals`:
  - `allow` → the hook is answered at once and the approval recorded as `allowed` by `rule`, `route: auto`, with no `approval.requested` (nobody is notified);
  - `chief` → pending, `route: chief`, and `approval.routed`, which the chief's inbox picks up;
  - `user` → pending, `route: user`, as before the policy.

  `role: user` agents always route to you. A request from an agent outside any run that the chief could take goes to you, since there's no chief.
- **Chief answers:** `POST /v1/approvals/{id}/chief` `{decision, reason}`. A deny is accepted for any pending approval of a run's agent (not yours). An allow needs a reason and `route: chief`, and passes `Limits::apply_chief_decision` against the run's limits as they are at that moment, inside one transaction that also counts it in `runs.chief_approvals`. Refused → 403 with why, and the approval moves to `route: user` (`approval.routed`, its reason saying the chief's approval was refused). Claude sees a chief denial as "The chief denied this: <reason>".
- **Limits:** `GET /v1/runs/{id}/limits` → `{preset, approval_cap, chief_approvals}`; `PUT` with `{preset?, approval_cap?}` changes them while the run isn't finished (400 over 1000, 404 unknown run, 409 finished). Nothing is re-checked retroactively: pending chief-routed approvals keep their route, and the new limits apply when the chief answers them.
- **Expiry:** approvals of non-`user` agents are denied after `cos_policy::timeout::UNANSWERED_AFTER` (5 minutes) with `UNANSWERED_MESSAGE`, state `expired`, whoever they were routed to.
- **Audit:** the `approval_audit` table holds a `cos_policy::AuditRecord` (as JSON) for every answer to a run agent's request given through cos: by rule, by the chief (allowed or denied), by timeout, and by you through the API or CLI. Answers typed in the terminal aren't recorded, since their outcome isn't known. `Runs::approvals_report_section(run_id)` renders "Approved on your behalf (N)" from it for the report.

## The chief

Every run gets a chief: a runtime agent (`role: chief`) that plans the run, reacts to what happens and writes the report. The contract is [orchestration.md](orchestration.md); the code is `crates/cos/src/daemon/chief.rs` (with `chief/prompt.rs` and `chief/tools.rs`), `daemon/chief_api.rs`, `crates/cos/src/mcp.rs` (`cos mcp`) and `cos_core::chief` (the `inbox`, `wakeups` and `questions` tables).

- **Starting:** `POST /v1/runs` starts the chief in the workspace folder right after creating the run and stores its id in `runs.chief_agent_id`. It runs with `user_settings: false` (`--setting-sources project --strict-mcp-config`), permission mode `default`, the model in `COS_CHIEF_MODEL` (else `COS_RUN_MODEL`, else Claude's default), its role in `--append-system-prompt` (stored with the agent), `--allowedTools mcp__cos`, and `--mcp-config` with one server, `cos`: `<cos exe> mcp --agent <id>` with `COS_HOME` in its env. The MCP server and the allow rule are added on every launch, so a resumed chief keeps them. If the chief can't start, the run fails with "the chief could not start: …" and the request returns that error. Run agents answer Claude's trust dialog by themselves (the project is registered, so it counts as trusted).
- **Tools:** `cos mcp --agent <id>` is a stdio MCP server (the `rmcp` crate) with `board_get`, `board_propose`, `task_update`, `task_send`, `ask_user`, `approval_decide` and `report_write`. It forwards each call to `POST /v1/agents/{id}/tools/{name}` (body: the arguments, result `{text}`). The daemon checks that the agent is the current chief of a run and acts on that run only (403 otherwise, 404 for an unknown tool, 400 for bad arguments, 409 when the run's state doesn't allow it). A failed call comes back to Claude as a tool error with the daemon's message, which for `board_propose` lists every problem with the plan. `approval_decide` goes from the bridge straight to `POST /v1/approvals/{id}/chief`, after checking the approval comes from an agent of the chief's run.
- **Inbox:** the chief follows the event log. `task.state_changed` to `merged`, `blocked` or `failed`, `approval.routed` with `route: chief`, and the run reaching `reporting` become inbox items; so do your answers and messages. When the chief is `idle`, a wakeup is typed into it: a brief (run state, goal, board counts, open questions) and up to 20 items or 24 KiB, oldest first (an item is cut to 4 KiB). At most 30 wakeups go out per run per hour; past that, items wait. After a wakeup, no other goes out until the chief has started working on it (or 20 s have passed). Waiting inboxes are rechecked every 30 s.
- **Stopping:** when the run is done, the chief is stopped once it ends its turn (at most 2 minutes later). When the run is cancelled or fails, the pipeline stops it with the run's other agents.
- **Report:** `report_write` works only while the run is `reporting`. It saves the markdown plus `Runs::approvals_report_section` to `runs/<id>/report.md`, calls `Runs::finish` and logs `run.report_written`.

### Routes

| Route | Body | Result |
|---|---|---|
| `POST /v1/agents/{id}/tools/{name}` | the tool's arguments | `{text}`. Unix socket only. |
| `GET /v1/questions?state=open\|answered&run_id=N` | | `{questions: [Question]}`, oldest first. Open questions of finished runs are left out of `state=open`. |
| `GET /v1/questions/{id}` | | Question |
| `POST /v1/questions/{id}/answer` | `{answer}` | Question. 409 if already answered or the run is finished. |
| `POST /v1/runs/{id}/message` | `{text}` | 202. Goes to the chief's inbox. 400 if empty or over 10,000 characters, 409 if the run is finished. |
| `GET /v1/runs/{id}/report` | | `{markdown, path}`, or 404 before it exists. |

```jsonc
// Question
{"id": 4, "run_id": 2, "agent_id": 9, "question": "Use Redis or in-memory?", "choices": ["redis", "memory"],
 "state": "open", "answer": null, "created_ms": 0, "answered_ms": null}
```

Run JSON gains `chief_agent_id`. The `agent.started` event gains `run_id`.

### CLI

| Command | What it does |
|---|---|
| `cos questions [--all]` | The questions chiefs are waiting for you to answer (`--all`: answered ones too). |
| `cos answer <id> "text"` | Answers one; the chief gets it at its next wakeup. |
| `cos say [run] "text"` | A message to the run's chief. |
| `cos report [run]` | Prints the run's report. |

### Events

| Kind | Data |
|---|---|
| `question.asked` | `{question_id, run_id}` |
| `question.answered` | `{question_id, run_id}` |
| `run.report_written` | `{run_id, path}` |

## The Chief of Staff

Each project can have a Chief of Staff: one permanent Claude session you talk to, which creates workspaces, starts agents in them, follows them and reports back. The contract is [chief-of-staff.md](chief-of-staff.md); the code is `crates/cos/src/daemon/staff.rs` (with `staff/prompt.rs` and `staff/tools.rs`), `daemon/staff_api.rs`, `crates/cos/src/mcp.rs` and `cos_core::staff` (the `staff`, `staff_inbox` and `staff_wakeups` tables).

- **Starting:** lazily, on `…/staff/start`, the first `…/staff/message` or `cos staff say`. The daemon makes the project's hidden `<slug>/chief` worktree if needed and starts an agent with `role: staff` there: your settings (`user_settings: true`), permission mode `default`, the project's model (else `COS_STAFF_MODEL`, else Claude's default), `--mcp-config` with the `cos` server (`cos mcp --agent <id>`) and `--allowedTools mcp__cos__*`. Its standing instructions and its brief go in `--append-system-prompt`. Its id is kept in the `staff` table. Its trust dialog is answered for it.
- **Restarting:** starting it again while it runs does nothing. An exited or interrupted chief is relaunched as the same agent with the instructions, brief and model as they are now: `claude --resume` if Claude saved the session, else a fresh session with the same id (Claude saves nothing until the first message). Each start logs `staff.started`.
- **Daemon restarts:** once the daemon serves, it resumes every chief left `interrupted` (it was running when the daemon stopped or crashed), one a second, as above. A chief you stopped stays stopped: the stop route sets `staff.stopped` for a chief's agent, and any start or resume of it clears that. Agents the chief started stay `interrupted`, like yours.
- **Tools:** `cos mcp` looks up its agent's role when it starts, and offers a Chief of Staff the staff tools (`workspaces_list`, `workspace_create`, `agent_spawn`, `agent_send`, `agent_read`, `agent_stop`, `run_start`, `brief_update`), which it forwards to `POST /v1/agents/{id}/tools/{name}` like the run chief's. The daemon sends a `role: staff` caller's calls to the staff side, which works on the chief's own project only: an agent or workspace of another project, or the chief's own worktree, is refused (403 or 404), and the error reaches Claude as a tool error. `agent_spawn` starts a `role: user` agent with `spawned_by` set, its title taken from the prompt, and the `[cos] The Chief of Staff of <project> started you…` line appended to the prompt. `agent_send` resumes a stopped agent and waits (up to a minute) until it's ready. `agent_read` returns the full last message from the transcript. `run_start` creates a run through the same path as `POST /v1/runs` and records `runs.spawned_by`. `brief_update` saves at most 4 KiB, and the next start appends it.
- **Inbox:** items come from the agents it started (a `Stop` with the turn's last message, an exit that isn't the daemon shutting down, an approval waiting for you, a trust dialog), from the runs it started (every `run.state_changed`), and from you (`…/staff/message`). News is cut to 4 KiB per item; your messages are kept whole (at most 100,000 characters). Each change logs `staff.inbox` with the number of items waiting.
- **Delivery:** only while the chief is `idle`, and not within 20 s of typing something it hasn't started working on. Your oldest waiting message goes first, on its own, typed as one bracketed paste; it doesn't count toward any cap. Otherwise one wakeup goes in: `[cos · N updates]`, then one `### <heading>` section per item (up to 20 items or 24 KiB), a line saying how many more wait, and a reminder that these are reports, not your instructions. At most 30 wakeups an hour per project; past that, news waits but your messages still go in. Waiting inboxes are rechecked every 30 s.
- **Pausing:** while a chief's coordination is paused (`staff.paused`), news waits the same way and your messages still go in. Resuming wakes the delivery loop, so what waited goes in once the chief is idle.
- **Removing the project** removes the chief's worktree and agent record with it, refused while the chief runs.

### Routes

| Route | Body | Result |
|---|---|---|
| `GET /v1/projects/{id}/staff` | | `Staff`, even before it first starts |
| `PUT /v1/projects/{id}/staff` | `{model?, paused?}` | `Staff`. A field left out stays as it is. A null or empty model goes back to the default, used from the next start. `paused` pauses or resumes its coordination at once. |
| `POST /v1/projects/{id}/staff/start` | | `Staff`. Starts or resumes it; nothing if it's running. |
| `POST /v1/projects/{id}/staff/message` | `{text}` | 202. Starts it if needed; the text goes in as soon as it's idle. 400 if empty. |
| `GET /v1/projects/{id}/staff/inbox` | | `{items: [StaffItem]}`, the latest 100 items of news, newest first. Your messages are left out. |

```jsonc
// Staff
{"project_id": 1, "agent_id": 14, "state": "idle", "model": "haiku", "brief": "…",
 "worktree": "/Users/me/.cos/worktrees/topflexi-1/chief", "branch": "topflexi/chief",
 "working_agents": 2, "waiting_on_you": 1, "paused": false, "updates_waiting": 0}
// StaffItem
{"id": 7, "project_id": 1, "kind": "agent" | "run" | "message", "agent_id": 15, "run_id": null,
 "text": "Agent 15 \"Add a README line\" finished a turn\nDone: …", "created_ms": 0, "delivered_ms": null}
```

`model` is what it runs: the project's model, else `COS_STAFF_MODEL`; null means Claude's default. `working_agents` counts the agents it started that are `starting` or `working`; `waiting_on_you` those that are `waiting_approval` or `needs_trust`. `paused` is whether its coordination is paused, and `updates_waiting` how many items of news (not your messages) wait to be delivered. An item's first line is its heading.

### CLI

| Command | What it does |
|---|---|
| `cos staff [project] [--model M]` | Its state, model, worktree, the agents it started, its brief and the latest news. `--model default` goes back to the default. |
| `cos staff say [project] "text"` | A message to it; starts it if needed. |
| `cos staff attach [project]` | Its terminal, like `cos agent attach`; starts it if needed. |
| `cos staff pause [project]` | Pauses its coordination: news waits, your messages still go in. |
| `cos staff resume [project]` | Resumes it; what waited goes in once it's idle. |

The project defaults to the one the current folder is in, else the only one.

### Events

| Kind | Data |
|---|---|
| `staff.started` | `{project_id, agent_id}` |
| `staff.inbox` | `{project_id, count}`: items waiting |
| `staff.changed` | `{project_id}`: its model, brief or pause changed |
| `staff.stopped` | `{project_id, agent_id}`: its process ended (stopped, exited, or the daemon stopping) |
