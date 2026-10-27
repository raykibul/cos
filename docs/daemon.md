# The cos daemon

`cos daemon` is the long-running process that owns your projects, their workspaces, the agents working in them and their permission requests. It keeps the database and the event log, and serves an HTTP API on a Unix socket (and over Tailscale when remote access is on). It runs agents (Claude Code in terminals it owns) and holds their permission requests as approvals for you to answer (see "Agents" below). Each project has a Chief of Staff, a permanent Claude session you talk to, which coordinates the other agents (see "The Chief of Staff" below).

It runs on macOS and Linux. To run it on a Linux server such as EC2 and drive it from the Mac app over Tailscale, see [linux.md](linux.md).

## Commands

| Command | What it does |
|---|---|
| `cos daemon` | Run in the foreground. Ctrl-C or SIGTERM stops it cleanly, asking its agents to exit first. |
| `cos daemon install` | Run it as a service: on macOS a launchd agent (`local.cos.daemon`) that starts now, at every login, and again about 10 s after a crash; on Linux a systemd user unit (`~/.config/systemd/user/cos.service`), enabled and started, restarted 10 s after a crash. See [Running as a service](#running-as-a-service). |
| `cos daemon uninstall` | Stop the service and remove its plist or unit. |
| `cos upgrade` | Upgrade `cos` to the latest release, and the Mac app with it, then restart the service on the new binary (asking first if agents are at work). `--check` only compares versions. See "Upgrading with `cos upgrade`" in [releasing.md](releasing.md#upgrading-with-cos-upgrade). |
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
| `hooks.sock` | Where the agents' hooks (relayed by `cos hook`) reach the daemon (mode 0600). One socket for every agent; hooks are routed by session id, and by the agent id `cos hook --agent` adds when Claude starts a new session in the same terminal (`/clear`). |
| `cos.db` | SQLite in WAL mode. The schema version is in `PRAGMA user_version`; migrations are in `crates/cos-core/src/store.rs`. |
| `daemon.lock` | Holds the running daemon's pid. Its lock means only one daemon runs per home. |
| `logs/daemon.log` | The daemon's stderr when launchd or systemd runs it. |
| `daemon.env` | Optional: `NAME=value` lines for secrets such as `ANTHROPIC_API_KEY` or jev's `TYPESAFE_API_KEY`, which `install` never copies into the plist or the unit. On Linux the systemd unit loads it into the daemon's environment (so agents get it too); on both macOS and Linux the daemon reads it itself at startup for its own lookups (jev's key), and refuses it if other users can read or write it. `cos daemon install` copies `TYPESAFE_API_KEY` from your shell into it. Keep it mode 0600. |

## Running as a service

`cos daemon install` records the binary's path, `COS_HOME` and your `PATH` at install time, and also `CLAUDE_CONFIG_DIR`, `COS_CLAUDE_BIN`, `COS_TAILSCALE` and `COS_STAFF_MODEL` if they're set, so agents use the same Claude config and binary as your shell. Run `cos daemon install` again after moving the binary or changing any of them. If `claude` isn't on your `PATH` but is where its installers put it (`~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin`, nvm's newest Node, `/usr/local/bin`, `/opt/homebrew/bin`), its folder is added to the recorded `PATH`, and the daemon finds it there at start too.

- **macOS:** a launchd agent, `~/Library/LaunchAgents/local.cos.daemon.plist`, in your login session (`gui/<uid>`).
- **Linux:** a systemd user unit, `~/.config/systemd/user/cos.service` (`$XDG_CONFIG_HOME` if set). `install` runs `systemctl --user daemon-reload`, `enable` and `restart`; `uninstall` runs `disable --now`, removes the file and reloads. The unit has `Restart=on-failure`, `KillMode=mixed` (SIGTERM to the daemon alone, which stops its agents itself) and appends output to `logs/daemon.log`. systemd stops a user's services when their last session ends unless lingering is on, so `install` warns when `loginctl` says it's off; on a server, run `sudo loginctl enable-linger $USER`. `install` needs your user's systemd, so run it in an SSH login rather than a `su` shell. See [linux.md](linux.md).
- **Elsewhere,** or without systemd (containers), `install` fails with a message; run `cos daemon` under your own supervisor.
- **Upgrading:** `cos upgrade` replaces the binary, then restarts the service if its plist or unit runs that binary (`launchctl kickstart -k gui/<uid>/local.cos.daemon`, or `systemctl --user restart cos.service`) and waits for `/v1/status` to report the new version. The daemon stops as it does for any SIGTERM: agents are marked `interrupted`, and Chiefs of Staff resume by themselves.

## Event log

Every event has a sequence number (`seq`) starting at 1. Numbers only go up and are never reused, so a client can always resume from the last one it saw.

```json
{"seq":3,"ts":"2026-10-21T18:06:01.843Z","kind":"daemon.started","data":{"pid":58448,"version":"0.1.0"}}
```

Each event also carries `run_id`, which is always `null`: it belonged to runs, which cos no longer has, and is kept so older clients still decode the log. The daemon's own kinds are `daemon.started` and `daemon.stopped` (with the reason); the rest are listed with what they belong to below. A crash leaves no `daemon.stopped`, so a `started` straight after a `started` means the daemon crashed.

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

A **project** is a registered git repo. A **workspace** is a git worktree of a project that cos creates and owns. Agents run inside workspaces: the ones you start and the ones your Chief of Staff starts share the workspace's folder.

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

## Port forwarding

An approved device can reach a port on this machine's loopback, so the Mac app can show a dev server running on an EC2 box as `localhost:5173` on the Mac ([desktop.md](desktop.md#ports)). One route does it, served on the socket and the tailnet listener. Over the tailnet the guard runs first, as for every route, so an unapproved device gets the usual refusals and never learns the route exists. Why it works this way is in [specs/port-forwarding.md](specs/port-forwarding.md).

| Route | Headers | Returns |
|---|---|---|
| `GET /v1/forward/{port}` | none | A probe: 200 `{"port": 3343, "listening": true, "address": "127.0.0.1:3343"}`, or `"listening": false, "address": null`. It opens a connection and closes it without sending anything. |
| `GET /v1/forward/{port}` | `Connection: upgrade`, `Upgrade: cos-forward` | 101 with `Upgrade: cos-forward`, then raw bytes both ways. The daemon connects before answering, so 101 means something accepted. |

- **Where it connects.** `127.0.0.1:{port}`, then `[::1]:{port}` if that's refused (Node and Vite often listen on `::1` only), each with a 3 s timeout. There is no host and no name lookup, so the route can't reach the machine's network or a cloud metadata address. Any port from 1 to 65535: an approved device already has a shell here, so a deny-list would add nothing.
- **Tunnels.** Bytes are copied both ways with half-close passed on, and there is no idle timeout. A tunnel ends when either side closes, when the daemon stops or the tailnet listener closes, and when its device is denied or removed. Each device can have 256 open, and the daemon 1024. Accepted tailnet connections use TCP keepalive (60 s, then every 10 s, 3 probes), so the tunnels of a Mac that went to sleep are reaped within about 90 s.
- **No events.** One page load opens several tunnels, so they aren't logged; approving the device is the logged act.

Errors are `{"error", "code"}` on both listeners:

| Status | `code` | When |
|---|---|---|
| 400 | `bad_port` | `{port}` isn't a number from 1 to 65535. |
| 400 | `bad_upgrade` | `Upgrade` is something other than `cos-forward`. |
| 502 | `nothing_listening` | Both addresses refused: nothing runs on that port. The message names the machine and the port. |
| 504 | `connect_timeout` | Neither answered within 3 s. |
| 503 | `too_many_forwards` | The device or the daemon is at its cap. |
| 403 | the guard's codes | As in "Refusals". |

A daemon older than the route answers 404 with no code; the app says that machine's cos is too old.

## Agents

An **agent** is a Claude Code session the daemon runs in a pseudo-terminal it owns, in a workspace's folder. Apps show its terminal live and type into it; the daemon follows it through hooks, not by reading the screen. The routes, JSON, events and CLI are in [agents-api.md](agents-api.md); this section is how the daemon does it. The code is `crates/cos/src/daemon/agents.rs` (process side) and `cos_core::agents` (the `agents` and `approvals` tables).

- **Starting:** the daemon picks the session id, registers it, then runs `claude --session-id <id> --settings <hooks>` in the workspace folder (the folder, system prompt and MCP config are recorded so `resume` starts it the same way). The hooks run `cos hook --agent <id>`, which relays each one to `<home>/hooks.sock`. `COS_CLAUDE_BIN` overrides which `claude` runs (tests use `cos-fake-claude`).
- **Environment:** each launch and resume gets the global and project variables as they are at that moment, under the ones cos sets itself (see [Environment](#environment)). The agent records their names in `env_names`.
- **States** come from hooks: `SessionStart` → `idle`, `UserPromptSubmit` and `PreToolUse` → `working`, `PermissionRequest` → `waiting_approval`, `Stop` → `idle`, process exit → `exited`. The only things read from the screen, before the first hook, are Claude's trust dialog (`needs_trust`) and its login screen (`needs_login`).
- **Approvals:** a `PermissionRequest` hook waits on the socket while the approval is `pending`. Answering it through the API answers the hook. If the hook goes away (answered in the terminal) or the agent's next hook arrives first, the approval is `settled` by `terminal`. Every approval waits for you, however long it takes; the approvals policy only says why it asks (see "Approvals policy" below).
- **Restarts:** when the daemon stops, it asks each agent to exit (`/exit`, then kill) and marks it `interrupted`. If it crashed instead, agents still marked running become `interrupted` on the next start. `resume` (`cos agent resume <id>`, the app's Resume) runs `claude --resume <session id>` as the same agent. Only Chiefs of Staff come back by themselves (see [The Chief of Staff](#the-chief-of-staff)); your agents wait for you.
- **Trust:** a Chief of Staff and the agents it started answer Claude's trust dialog by themselves, since they work in a worktree cos made for a registered project. Agents you start leave it to you (`needs_trust`).
- **Transcripts:** every hook carries `transcript_path`, which is stored on the agent when it changes (`agents/transcripts.rs`). After `UserPromptSubmit`, `PostToolUse` and `Stop`, the daemon waits 250 ms, and if the file grew since the last time, logs `agent.transcript` with how many messages are final. A hook that comes while a check is waiting moves the check to 250 ms after it, and `Stop` asks for one more check 2 s later, because Claude can write a turn's last reply after its `Stop` hook. It logs at most once a second per agent. `GET /v1/agents/{id}/transcript?after=` reads the file through a cache (`daemon/transcript.rs`): each file's parse is kept, keyed by its length and modification time, and when the file grew only the appended lines are parsed (checking that the last line read still ends where it did); a file that shrank or was rewritten is parsed from the top. At most 64 files are kept. The answer also has `context`, how full the conversation is: see [agents-api.md](agents-api.md).

## Environment

Agents get the environment variables you set in cos: a global list, and one per project. The usual reason is several Claude accounts on one machine. `CLAUDE_CONFIG_DIR` picks the account, so `cos env set CLAUDE_CONFIG_DIR=~/.claude-personal --project topflexi` makes every agent of topflexi use that one while other projects keep `~/.claude`. API keys, `CLAUDE_CODE_OAUTH_TOKEN` and anything else work the same way. The code is `cos_core::env` (the `env` table, migration 9), `daemon/env_api.rs` and `agent_env` in `daemon/agents.rs`.

- **Who gets them:** every agent launched for the project, whatever its role: yours, its Chief of Staff and the agents that one starts.
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
- **Trust:** Claude remembers trusted folders per config dir, so the first agent in a new one may ask again. cos answers it for a Chief of Staff and its agents as usual; yours show `needs_trust`.
- **Login:** an agent whose config dir nobody has logged into shows Claude's first-run or login screen in its terminal, before any hook. The daemon spots it, the only screen read besides the trust dialog, and sets the agent to `needs_login`. Open its terminal (the app, or `cos agent attach <id>`) and log in; Claude keeps the login for that config dir, and the agent carries on (`SessionStart` makes it `idle`). Stopping it there kills it. `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`) logs an agent in with no screen at all. A session that loses its login later shows "Not logged in · Please run /login" in its turn; that isn't detected.
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

The `cos-policy` crate classifies each permission request; the rules are in [approvals.md](approvals.md). The daemon side is `crates/cos/src/daemon/agents/approvals.rs`.

- **Classifying:** each `PermissionRequest` becomes a `cos_policy::ApprovalRequest`: the tool, its input, the hook's `cwd`, `$HOME`, and the agent's workspace as the worktree, with everything in it in scope. `classify_with(…, &Filesystem)` follows symlinks. The tier and its reason are stored on the approval (`tier`, `reason`), so the apps can say why cos asks.
- **Answering:** you answer every request, through the API, an app, `cos approve`/`cos deny` or the terminal. The policy never answers one by itself, and nothing expires: a request waits until you answer it or the agent moves on.

## jev (shadow mode)

jev, the fast judgment layer, judges two decision points in shadow mode: permission requests, and items added to a Chief of Staff's inbox. It stores what it would have done next to what happened and changes nothing. Off per project by default. Everything (the key lookup, the questions and thresholds, storage in migration 11, stats) is in [judge.md](judge.md); the code is `daemon/judge.rs` and `daemon/judge_api.rs`.

| Route | Body | Result |
|---|---|---|
| `GET /v1/judge` | | where the key was found, `active`, `off_reason`, the per-project switches and each decision point's numbers |
| `POST /v1/judge/reload` | | the same, after looking for the key again (login shell included) |
| `PUT /v1/projects/{id}/judge` | `{"enabled": bool}` | `{project_id, name, enabled}` |

| Event | Data |
|---|---|
| `judge.decided` | `{decision_id, point, project_id, subject, would, latency_ms, error}`, never the input |
| `judge.changed` | `{project_id, enabled}`, or `{key_source}` after a key lookup |

CLI: `cos judge status [--reload]`, `cos judge stats`, `cos judge on|off --project P`, `cos judge test`.

## The Chief of Staff

Each project can have a Chief of Staff: one permanent Claude session you talk to, which creates workspaces, starts agents in them, follows them and reports back. The contract is [chief-of-staff.md](chief-of-staff.md); the code is `crates/cos/src/daemon/staff.rs` (with `staff/prompt.rs` and `staff/tools.rs`), `daemon/staff_api.rs`, `crates/cos/src/mcp.rs` and `cos_core::staff` (the `staff`, `staff_notes`, `staff_inbox` and `staff_wakeups` tables).

- **Starting:** lazily, on `…/staff/start`, the first `…/staff/message` or `cos staff say`. The daemon makes the project's hidden `<slug>/chief` worktree if needed and starts an agent with `role: staff` there: your settings (`user_settings: true`), permission mode `default`, the project's model (else `COS_STAFF_MODEL`, else Claude's default), `--mcp-config` with the `cos` server (`cos mcp --agent <id>`) and `--allowedTools mcp__cos__*`. Its standing instructions, its brief and the list of its memory notes (name and description) go in `--append-system-prompt`. Its id is kept in the `staff` table. Its trust dialog is answered for it.
- **Restarting:** starting it again while it runs does nothing. An exited or interrupted chief is relaunched as the same agent with the instructions, brief, notes and model as they are now: `claude --resume` if Claude saved the session, else a fresh session with the same id (Claude saves nothing until the first message). Each start logs `staff.started`.
- **New session:** `…/staff/new-session` (`cos staff new`) stops a running chief, waits for it to exit, gives the same agent a new session id with no transcript path, logs `agent.transcript` `{next: 0}`, and launches it without `--resume`. A chief that never started just starts.
- **Daemon restarts:** once the daemon serves, it resumes every chief left `interrupted` (it was running when the daemon stopped or crashed), one a second, as above. A chief you stopped stays stopped: the stop route sets `staff.stopped` for a chief's agent, and any start or resume of it clears that. Agents the chief started stay `interrupted`, like yours.
- **Tools:** `cos mcp` looks up its agent's role when it starts, and offers a Chief of Staff the staff tools (`workspaces_list`, `workspace_create`, `agent_spawn`, `agent_send`, `agent_read`, `agent_stop`, `brief_update`, `memory_list`, `memory_read`, `memory_write`, `memory_delete`), which it forwards to `POST /v1/agents/{id}/tools/{name}` (body: the arguments, result `{text}`; Unix socket only). Only a `role: staff` caller may use the route, which works on the chief's own project only: an agent or workspace of another project, or the chief's own worktree, is refused (403 or 404), and the error reaches Claude as a tool error. `agent_spawn` starts a `role: user` agent with `spawned_by` set, its title taken from the prompt, and the `[cos] The Chief of Staff of <project> started you…` line appended to the prompt; its news skips the inbox unless it was started with `report: true`. `agent_send` resumes a stopped agent and waits (up to a minute) until it's ready; its `report` changes the agent's setting first. `agent_read` returns the full last message from the transcript. `brief_update` saves at most 4 KiB, and the next start appends it. The memory tools keep up to 50 named notes of 8 KiB per project (see "Memory notes" in [chief-of-staff.md](chief-of-staff.md#memory-notes)); a change to the brief or a note, by the chief or through the routes, is saved for the chief's next launch and logs `staff.changed`.
- **Inbox:** items come from the agents it started (a `Stop` with the turn's last message, an exit that isn't the daemon shutting down, an approval waiting for you, a trust dialog) that have `report: true`, and from you (`…/staff/message`). News is cut to 4 KiB per item; your messages are kept whole (at most 100,000 characters). Each change logs `staff.inbox` with the number of items waiting.
- **Delivery:** only while the chief is `idle`, and not within 20 s of typing something it hasn't started working on. Your oldest waiting message goes first, on its own, typed as one bracketed paste; it doesn't count toward any cap. Otherwise one wakeup goes in: `[cos · N updates]`, then one `### <heading>` section per item (up to 20 items or 24 KiB), a line saying how many more wait, and a reminder that these are reports, not your instructions. At most 30 wakeups an hour per project; past that, news waits but your messages still go in. Waiting inboxes are rechecked every 30 s.
- **Pausing:** while a chief's coordination is paused (`staff.paused`), news waits the same way and your messages still go in. Resuming wakes the delivery loop, so what waited goes in once the chief is idle.
- **Files:** you send files to it by uploading them into its worktree, then naming them in a message; it passes them on with `attachments` on `agent_spawn` and `agent_send`, which copy them into the agent's folder. Files live in `<folder>/.cos/attachments/`, ignored by git. See "Attachments" in [agents-api.md](agents-api.md#attachments); the code is `daemon/attachments.rs` and `daemon/attachments_api.rs`.
- **Removing the project** removes the chief's worktree and agent record with it, refused while the chief runs.

### Routes

| Route | Body | Result |
|---|---|---|
| `GET /v1/projects/{id}/staff` | | `Staff`, even before it first starts |
| `PUT /v1/projects/{id}/staff` | `{model?, paused?, brief?}` | `Staff`. A field left out stays as it is. A null or empty model goes back to the default, used from the next start. `paused` pauses or resumes its coordination at once. `brief` replaces its brief (trimmed; at most 4 KiB, else 400 and nothing changes), used from its next start or new session, and logs `staff.changed`. |
| `POST /v1/projects/{id}/staff/start` | | `Staff`. Starts or resumes it; nothing if it's running. |
| `POST /v1/projects/{id}/staff/message` | `{text, attachments?}` | 202. Starts it if needed; the text goes in as soon as it's idle, with the files `attachments` names listed at the end. 400 if empty (no text and no files) or a file is unknown. |
| `POST /v1/projects/{id}/staff/attachments?name=` | the file's bytes | 201 Attachment, into its worktree (made if needed, without starting it). See "Attachments" in [agents-api.md](agents-api.md#attachments). |
| `GET /v1/projects/{id}/staff/inbox` | | `{items: [StaffItem]}`, the latest 100 items of news, newest first. Your messages are left out. |
| `POST /v1/projects/{id}/staff/new-session` | | `Staff`. A new Claude session for the same chief; 409 if it didn't stop within a minute. |
| `GET /v1/projects/{id}/staff/memory` | | `{notes: [StaffNote]}`, by name. |
| `PUT /v1/projects/{id}/staff/memory/{name}` | `{description, body}` | `StaffNote`. Creates or replaces it; 400 if a cap is broken. |
| `DELETE /v1/projects/{id}/staff/memory/{name}` | | 204; 404 if there is no such note. |

```jsonc
// Staff
{"project_id": 1, "agent_id": 14, "state": "idle", "model": "haiku", "brief": "…",
 "worktree": "/Users/me/.cos/worktrees/topflexi-1/chief", "branch": "topflexi/chief",
 "working_agents": 2, "waiting_on_you": 1, "paused": false, "updates_waiting": 0}
// StaffItem
{"id": 7, "project_id": 1, "kind": "agent" | "message", "agent_id": 15,
 "text": "Agent 15 \"Add a README line\" finished a turn\nDone: …", "created_ms": 0, "delivered_ms": null}
```

`model` is what it runs: the project's model, else `COS_STAFF_MODEL`; null means Claude's default. `working_agents` counts the agents it started that are `starting` or `working`; `waiting_on_you` those that are `waiting_approval` or `needs_trust`. `paused` is whether its coordination is paused, and `updates_waiting` how many items of news (not your messages) wait to be delivered. An item's first line is its heading.

### CLI

| Command | What it does |
|---|---|
| `cos staff [project] [--model M]` | Its state, model, worktree, the agents it started, its brief and the latest news. `--model default` goes back to the default. |
| `cos staff say [project] "text" [--file PATH]…` | A message to it, with files; starts it if needed. Works with `--host`, so files go from any machine. |
| `cos staff attach [project]` | Its terminal, like `cos agent attach`; starts it if needed. |
| `cos staff pause [project]` | Pauses its coordination: news waits, your messages still go in. |
| `cos staff resume [project]` | Resumes it; what waited goes in once it's idle. |
| `cos staff new [project]` | A new Claude session for it, with its brief and notes. |
| `cos staff memory [project]` | Prints its memory notes. |

The project defaults to the one the current folder is in, else the only one.

### Events

| Kind | Data |
|---|---|
| `staff.started` | `{project_id, agent_id}` |
| `staff.inbox` | `{project_id, count}`: items waiting |
| `staff.changed` | `{project_id}`: its model, brief, memory notes or pause changed |
| `staff.stopped` | `{project_id, agent_id}`: its process ended (stopped, exited, or the daemon stopping) |
