# cos: design

cos is a chief of staff for coding agents. One coordinator plans the work, hands each task to a worker (Claude Code, later Codex) in its own git worktree, checks and reviews what comes back, merges it, and writes you a report.

This file collects the design agreed on 21 Oct 2026, before any code existed. The project was called cos2bot at the time. Progress and anything learned while building go in the sections at the end.

## Proposal

**In one line:** take Selene's Chief of Staff and drop everything around it. One Rust daemon drives Claude Code and Codex through their headless JSON interfaces, so no hooks. A SwiftUI Mac app and iPhone app connect to it over your tailnet.

> **Changed on 22 Oct 2026:** agents run in terminals after all. See "Terminals and hooks" under Progress. Every agent cos launches (chief, workers, reviewer) runs Claude's own UI in a PTY that the daemon owns, so you can watch or type into any of them from the apps. cos follows each session through hooks it passes for that session only, and never through your settings or a shell wrapper. The headless harness stays for `cos try`.

### What Selene teaches

Selene is about 100K lines: 47K Go, 25K Electron, 26K Flutter and 4.5K Swift. Most of that weight comes from things a chief of staff doesn't need.

| Keep (the ideas worked) | Drop |
|---|---|
| The daemon is the product; the apps just display its state | Claude and Codex hooks |
| Tailscale, plain HTTP inside WireGuard | The terminal host, terminal emulation and Ghostty |
| A durable inbox where the chief is woken with batched events (20 events or 24 KiB per wakeup) | 8 of the 9 tunnel providers |
| Chief memory and brief stored outside the model's history | QR pairing and JWT (Tailscale identity replaces them) |
| Caps on context size and wakeups per hour | Electron, Flutter, Telegram, voice, file browser, scheduler |
| "Agents waiting on you" approvals with a 5-minute timeout | |

The target is roughly 6–8K lines of Rust and 5K lines of Swift.

### Main design choice: the LLM makes judgment calls, Rust does the bookkeeping

In the original Chief of Staff pattern, the chief model does everything, including steps that need no thinking. I'd move those steps into the daemon, where they always behave the same way and cost no tokens:

| Chief (LLM) | Daemon (Rust) |
|---|---|
| Split the goal into tasks with file lists | Reject a plan where two tasks share a file |
| Re-plan when a task is blocked | Create a worktree per task (from HEAD), spawn the worker, cap concurrency |
| Answer worker questions | After a worker finishes: check `git diff --name-only` against its allowed files, then run the tests |
| Write the final report | Spawn the reviewer, allow one retry on "changes", merge with `--no-ff`, mark BLOCKED on a conflict |

This also fixes a weak spot the pasted doc admits: worktrees aren't a sandbox, and the file list only works because the model follows it. Here Rust checks the diff, so a worker that edits files outside its list is rejected automatically.

Each task moves through: `planned → (you approve on phone) → running → checking → reviewing → merged | changes (1 retry) | blocked`.

### Running Claude and Codex without hooks

I checked your installed versions: Claude Code 2.1.306 and Codex 0.160.1.

- **Claude:** run it as a long-lived child process with JSON going in and out:
  `claude -p --input-format stream-json --output-format stream-json --permission-prompt-tool stdio --session-id <uuid> --mcp-config <cosd>` (the spike found that `--permission-prompt-tool stdio` is the flag that matters; see [claude-protocol.md](claude-protocol.md))
  The daemon sends messages, reads events and answers permission requests itself, the same way the Agent SDK does. `--resume` brings a session back after a restart.
- **Codex:** `codex app-server` speaks JSON-RPC over stdio, and approvals arrive as requests from Codex to the daemon. `codex app-server generate-json-schema` produces the message types. It's marked experimental.
- Both sit behind one Rust `Harness` trait: start, send, interrupt, a stream of events, and approval requests.
- The chief's tools come from an MCP server inside `cosd`, using the official `rmcp` crate: `board_propose`, `task_send`, `task_answer`, `ask_user`, `memory_update`, `report_write`.

### Network: Tailscale only

```
iPhone ──tailnet──▶ cosd @ 100.x:7700        (bound to the tailnet IP only)
Mac app ──unix socket──▶ cosd
                         ├─ chief   (claude)
                         ├─ worker × N  (claude/codex, one worktree each)
                         └─ reviewer
cosd ──▶ APNs (push "3 merged, 1 blocked")
```

- **Binding:** `cosd` listens only on the tailnet IP, never `0.0.0.0` (Selene's spec found that its server always bound to 0.0.0.0). Binding directly also skips `tailscale serve`.
- **Auth:** for each new client, `cosd` runs `tailscale whois` on its address (cached). It must be your login and a device you've allowed, and a new device asks for approval on the Mac. No pairing step and no tokens.
- **Push:** the daemon calls APNs directly. That needs your Apple Developer `.p8` key. While the app is open, live updates come over a stream that can resume where it left off.
- **API:** about 8 endpoints: list and create runs, read a run's board, approve a plan, answer an approval, send a message, the event stream, and the report.

### Apple apps

- One Xcode project and a shared `CosKit` Swift package (models, API client, event stream).
- **Mac:** a menu bar item plus a window. The app bundles `cosd` and installs it as a background service through Apple's `SMAppService`.
- **iPhone:** list of runs, the board, approvals, the report, and a box to start a new goal. A Live Activity for a running job is optional. Neither app has a terminal view.

### Layout

```
cos/
  crates/cosd/         daemon + `cos` CLI (tokio, axum, rusqlite, rmcp)
  crates/cos-harness/  claude.rs, codex.rs
  crates/cos-core/     board, task states, storage, git operations
  apple/               Xcode project + CosKit
```

### Phases

0. **Spike (1–2 days):** Rust drives Claude through stream-json, including answering a permission request, and does the same with Codex app-server. Also confirm the tailnet-IP binding and `whois` check. Claude's host protocol is what the SDKs speak, but it isn't documented as a public contract. If it turns out unstable, fall back to `--permission-prompt-tool` over MCP.
1. **Daemon and CLI:** one complete run end to end (plan → workers → checks → merge → report), driven only from `cos`.
2. **Mac app.**
3. **iPhone app and APNs.**
4. **Later:** Codex workers, multiple Macs, a Live Activity.

### Trade-offs of staying light

- **No terminal.** If a harness hangs on something interactive, such as a login, you only see an error, not the screen.
- **Restarts interrupt work.** Selene's terminal host keeps sessions alive when the daemon restarts; this design doesn't. Commits in worktrees survive, and sessions come back with `--resume`.
- **Codex's app-server is experimental**, so expect its protocol to change.

## Open decisions from the proposal

1. **Chief and reviewer models:** Claude for both is the recommendation, with workers able to use either Claude or Codex.
2. **Apple Developer account:** phone push needs one. Without it, updates only arrive while the app is open.
3. **One Mac for v1:** recommended. The iPhone app still stores a list of hosts so more Macs can be added later.

## jev: a fast judgment layer

jev fits cos as a fast, cheap layer between the daemon's Rust rules and the chief model. It handles the frequent small decisions that would otherwise interrupt you or wake the chief: approve this command? wake the chief? push to the phone? Your OpenRouter key is already set, so a test can start right away.

### The four layers

```
Rust rules  →  jev         →  chief (Claude)  →  you
exact          one quick      plans, re-plans,   decides what
checks         judgment,      writes reports     the others can't
               typed answer
free, ~0 ms    ~$0.00006,     cents, seconds     minutes
               70–500 ms
```

Each layer handles what it can and passes the rest up. jev never writes text, so it never plans or reports. Rust never guesses, so file-scope checks, tests, merges and conflicts stay in code.

### Where jev goes, in priority order

**1. Sorting permission requests.** This is the biggest win: workers ask permission often, and today every request would reach your phone.
- Rust rules run first. The task's test command is always allowed. `git push`, writes outside the worktree and network calls always go to you.
- jev handles the cases in between. In one call it answers:
  - `action_kind` (choice): `read_only | build_or_test | edit_in_worktree | install_dep | external | destructive`
  - `serves_task` (yes/no probability): is this action plausibly needed for this task?
- Code turns the answers into an action. Read-only, build or edit-in-worktree, with confidence above 0.9 and `serves_task` above 0.8, is approved automatically. Everything else goes to the phone.
- jev can only auto-approve actions that stay inside a throwaway worktree. That's important because jev has no built-in defence against injected instructions. Anything that leaves the worktree is never jev's call.

**2. Deciding when to wake the chief.** Every chief wakeup costs Opus tokens and context. Worker reports already have a `STATUS` field, which code reads directly; jev reads only the free-text `NOTES` and `ASK`.
- `wake` (choice): `now | batch | none`
- `is_question_for_chief` (yes/no)

This could replace Selene's flat cap of 20 wakeups per hour with a filter based on what the report says.

**3. Routing messages you type on the phone.** For "also make the button blue", the choice options are built from the live board:
- `stop`
- `status_question`, which code answers from the board without waking anyone
- `new_goal`
- `task:w1 … task:wN`, each described by its board row, so the instruction goes straight to that worker

**4. Checking a diff before review.** Questions on each worker's diff, asked alongside the task:
- `does_only_the_task` (yes/no)
- `bundles_unrelated_changes` (score)
- `adds_tests` (yes/no)

The results go to the reviewer as hints and choose the reviewer model: Sonnet for small, clean diffs and Opus for flagged ones. Review stays mandatory.

**5. Later:**
- Checking the plan: is each task a single change that can merge on its own?
- Choosing a model per task based on how hard it looks.
- Spotting stuck workers by scoring their last N events as `progressing | looping | waiting`, instead of relying only on a 10-minute inactivity timer.

### How it fits in Rust

- **New crate `cos-judge`:** a small client with typed answers:
  - a yes/no probability (no confidence field)
  - a choice: the picked option, its probabilities and a confidence
  - a score: the value, its probabilities and a confidence

  Choice answers decode straight into Rust enums such as `enum Wake { Now, Batch, None }`. An option jev returns that isn't in the enum is an error, not a silent fallback.
- **One questions file** (`judge/questions.json`, embedded in the binary) holds every question and threshold. That's the file you review and tune, as the jev skill recommends.
- **One call per decision point**, with all of that point's questions in it. The skill says batching is about 12x cheaper than one call per question.
- **Every decision is logged in SQLite:** a hash of the input, the answers, what the daemon did, and what you did. When you override jev, that override becomes a labelled example.
- **Shadow mode first:** at each decision point jev runs and logs what it would have done, but you still decide. The Mac app shows something like "would have auto-approved 41 of 45, disagreed on 4". Each decision point is switched on separately once its agreement looks good. The skill warns that jev's answers look confident even when they're wrong.
- **When jev fails, you decide:** on a timeout (more than 1 s), an error or confidence below 0.5, things go to the phone or the chief exactly as they would without jev. It only reduces noise and never decides whether something is correct.

### Cost and privacy

- **Cost:** about $0.042 per million input tokens. A permission check is around 1.5K tokens, so 1,000 checks a day costs about 6 cents.
- **Privacy:** commands, file paths and diffs would go to OpenRouter and TypeSafe. That's your call for each repo, so I'd add a per-repo `jev: on/off` setting, off by default. For work repos (opal-app, Optimizely), check whether that's allowed first. Inputs are trimmed to structured fields; full files are never sent.

### Changes to the plan

- **Phase 0:** add one more test, the permission-sorting questions on about 30 real commands from your Claude history, compared with what you would have approved.
- **Phase 1:** `cos-judge` ships in shadow mode only.
- **Phase 3 (iPhone):** permission sorting goes live, because that's when cutting phone interruptions matters.

## Approving on your behalf

The chief can approve requests for you, but only inside limits you set, and the Rust daemon should enforce those limits rather than the chief. This is possible because the daemon receives every approval request: Claude sends them over stdout (see [claude-protocol.md](claude-protocol.md)) and Codex through app-server approval requests. Whoever the daemon asks can answer, whether that's a rule, jev, the chief or you.

### Why you need limits

- **The chief is checking its own workers.** It wants the task finished, so it leans toward approving. That isn't an independent check.
- **The chief can be manipulated.** A worker reads a file or webpage containing "run this command", asks to run it, and the chief reads that request. Text in the request can talk the chief into approving.

So the chief gets an `approval_decide(id, allow|deny, reason)` tool, but a call to it is only a request. Rust checks it against your limits before applying it, so even a manipulated chief can't go beyond them.

### Who approves what

| Tier | Who decides | Examples |
|---|---|---|
| Always allowed | Rust rules | the task's test command, reads and edits inside its worktree |
| Quick check | jev | build commands, safe reads with high confidence (see last message) |
| **Approved for you** | chief, within your limits | installing a dependency, network GET requests, running scripts in the worktree |
| **Never delegated** | you | `git push`, anything outside the worktree, `sudo`, `rm -rf`, secrets (`.env`, `~/.ssh`), external POSTs, and **any change to settings or permissions** |

The last row is the important one. If a worker or the chief could edit `.claude/settings.json` or the cos config, it could give itself more permissions, so that is never delegated.

### How it works for you

- **Choose a preset when you start a run** (on the phone): **Strict** (only you approve), **Normal** (the chief handles the "approved for you" tier) or **Trusted** (the chief also handles network and installs). You can change it while the run is going.
- **Cap per run:** for example, at most 20 chief approvals. After that, requests come to you.
- **Everything goes in the report:** a section called "Approved on your behalf (12)", each entry with the chief's reason. One tap marks one as wrong. That tightens the limits and gives jev a labelled example.
- **Mistakes stay contained:** delegated approvals only cover actions inside a throwaway worktree, and nothing is merged until the reviewer approves.
- **Unanswered requests are refused after 5 minutes:** the worker is told nobody answered and carries on without that step. The chief doesn't step in for you on requests outside its limits.

### Fewer requests to begin with

Most approvals can be avoided. Start each worker with `--permission-mode acceptEdits` plus an allowlist for its task, such as `Bash(npm test:*)` and `Bash(git commit:*)`. Then only unusual commands produce a request, and the chief and jev have little left to judge.

**Suggested default:** Normal preset, a cap of 20, and the "never delegated" list built into the code.

## Phase 1 plan

Phase 1 should be one Rust binary, `cos`, that runs as both the daemon and the CLI. It serves its local API only on a Unix socket, with no TCP port, and finishes one complete run driven from the terminal. The network listener, Mac app and iPhone come later.

### Is it a local server?

Yes, but in phase 1 it listens only on `~/.cos/cos.sock`:

- **No auth needed yet.** The socket file is only readable by your user, which is enough.
- **No open port and no Tailscale yet.** The tailnet listener is added in phase 3, when the iPhone needs it.
- **The Mac app (phase 2) uses the same socket.**
- **The chief's tools also go through the socket.** Claude starts them via `--mcp-config` running `cos mcp --session <id>`, a small stdio bridge that forwards to the socket. That keeps phase 1 at zero TCP ports.

### What phase 1 delivers

```
cos daemon                     # run by launchd
cos run "add rate limiting"    # in a repo → chief plans the work
cos board                      # tasks, status, branches
cos approve-plan               # workers start
cos approvals / cos allow <id> # answer requests in the terminal
cos report                     # reports/2026-10-21-rate-limiting.md
```

Phase 1 is done when a goal that splits into two tasks ends in two merged branches and a report. Separately, a worker that touches a file outside its list must be rejected automatically.

### Milestones

1. **Setup:** install Rust, create the cargo workspace, `git init`.
2. **Harness spike:** Rust runs a single Claude process. It sends a prompt, streams the events back and answers a permission request. Test it with `cos try "prompt"`.
3. **Daemon skeleton:** launchd setup, SQLite, the API on the socket, a numbered event log with a live stream, `cos status`.
4. **Workers:** a `git worktree` per task created from HEAD, a cap on parallel workers, and after each worker the file-scope check (`git diff --name-only`) plus a test run.
5. **Chief:** the stdio bridge and its tools (`board_propose`, `task_send`, `ask_user`, `report_write`), an inbox, and batched wakeups.
6. **Review and merge:** a reviewer agent, one retry, merge with `--no-ff`, a merge conflict marks the task BLOCKED, then the report is written.
7. **Approvals:** fixed rules, the never-delegated list, your approvals through the CLI, then the chief's limits. jev in shadow mode comes last.

**Not in phase 1:** the TCP or Tailscale listener, the apps, phone push, Codex, multiple Macs.

### Layout

```
cos/
  Cargo.toml              workspace
  crates/cos/             the single binary: daemon + CLI + mcp bridge
  crates/cos-core/        board, task states, SQLite, git operations
  crates/cos-harness/     claude.rs (codex.rs later)
~/.cos/                   cos.db, cos.sock, logs/
<repo>/.cos/board.md      written by the daemon
<repo>/reports/           reports
```

## Progress

- **Milestone 1 (setup): done.** Rust 1.99.1, cargo workspace with `cos`, `cos-core` and `cos-harness`, git repo. Tests and lints run in Docker (`make test`, `make lint`); `make build` builds the macOS binary.
- **Milestone 2 (harness spike): done.** `cos try "prompt"` runs Claude Code as a child process, streams its events and answers permission requests (ask, allow or deny). Verified against Claude Code 2.1.306. Protocol notes are in [claude-protocol.md](claude-protocol.md).
- **Milestone 3 (daemon skeleton): done.** `cos daemon` serves an HTTP API on `~/.cos/cos.sock`. It keeps a numbered event log in SQLite that clients can page through or follow live and resume by sequence number. `cos status`, `cos events [-f]`, and `cos daemon install|uninstall` (launchd agent) are in. See [daemon.md](daemon.md).
- **Projects and workspaces (added after milestone 3, modelled on Selene): done on the daemon side.** A project is a registered git repo; a workspace is a cos-owned worktree on a `cos/<city>` branch where agents run. Several agents can share a workspace; parallel task workers get their own short-lived worktrees branched from it. `cos project add|list|remove` and `cos workspace new|rename|remove` are in, and the API is in [daemon.md](daemon.md). The Mac app part is in progress.
- **Terminals and hooks (22 Oct 2026): spike done.** You asked for a terminal view of agents, so agents run in a PTY instead of headless.
  - **`cos try --pty`:** runs Claude's interactive UI in a pseudo-terminal (`ClaudeTerminal`: `portable-pty`, with a `vt100` screen) and follows it through per-session hooks passed with `--settings`. A hidden `cos hook` relays each hook to a socket (`HookServer`).
  - **Verified with Claude 2.1.306:** the hooks fire, and the permission hook can allow or deny. Answering in the terminal also works and withdraws the hook.
  - **Folder trust:** a worktree of a trusted repo is trusted. Otherwise `--trust` answers the trust dialog.
  - **`cos sessions`:** lists every Claude session running on the Mac (busy or idle, folder, session id, matching cos workspace) from Claude's own `sessions/<pid>.json` files. It's read-only and needs no wrapper.
  - **Details:** see [claude-protocol.md](claude-protocol.md).
- **Task git lifecycle (git side of milestones 4 and 6): done as a library.** `cos_core::taskgit` creates a worktree and branch per task from the workspace branch, caps parallel workers (3 per workspace, 6 in total), and runs the file-scope check. That check covers the allowed globs, protected paths, renames and deletions, and work left uncommitted. It also runs the test command with a timeout that kills the whole process group, and merges into the workspace with `--no-ff`. A conflict aborts the merge and returns the conflicting paths. The rules are in [daemon.md](daemon.md#task-git-lifecycle). The orchestrator, which launches workers and stores task state, still has to call it.
- **Remote access, step 1 (daemon side): done.** Pulled forward from phase 3 so the Mac app can reach a daemon on another machine. `cos remote on` makes the daemon also listen on TCP 7700 on this machine's tailnet addresses only (off by default, stored in the database, applied live, retried every 30 s while Tailscale is unusable). Each request is checked with `tailscale whois` (cached 60 s per address): only devices logged in as the same Tailscale user get past it, and each new device must be approved on the host with `cos devices approve`. Device and listener management stay on the Unix socket. `cos --host` talks to a remote daemon. Verified against Tailscale 1.102.3 on macOS; the macOS application firewall blocks incoming connections to an unsigned `cos` by default, which `cos status` now reports. Next steps: CosKit/Mac app hosts list, then the iPhone. See "Remote access" in [daemon.md](daemon.md).
- **Runs, tasks and the board (the data model for milestones 4–6): done.** Runs, tasks and plan validation: cycles, missing dependencies, duplicate titles, parallel tasks sharing a file, and at most 12 tasks. Explicit run and task state machines with one retry per task and dependency gating. The board as JSON and markdown, the API, and `cos run|runs|board|approve-plan|cancel`. Nothing launches agents yet. See "Runs, tasks and the board" in [daemon.md](daemon.md).
- **Approvals policy (Rust side of milestone 7): done as a library.** The `cos-policy` crate sorts a permission request (tool, input, folder, worktree, task scope, role, test command) into a tier with a reason: always allowed, chief (worktree, network or install), ask you, or never delegated. Bash commands are parsed, not split on spaces, so lists, pipes, `cd`, substitutions, `bash -c`, heredocs, `find -exec` and `xargs` are each checked and the strictest part wins. Paths are resolved lexically, and through symlinks with the `Filesystem` probe. It also has the Strict/Normal/Trusted routing, the per-run cap, the check on the chief's `approval_decide`, the 5-minute refusal message and the "Approved on your behalf (N)" report section. See [approvals.md](approvals.md).
- **Approvals wiring (daemon side of milestone 7): done.** Every permission request of an agent the daemon runs goes through `cos-policy` with the agent's role, its task's worktree, scope and test command, and the run's limits. Rust rules answer the always-allowed tier at once (recorded as `allowed` by `rule`, nobody notified); chief-tier requests within the preset and cap wait for the chief (`approval.routed`), whose `POST /v1/approvals/{id}/chief` answer is checked against the limits at that moment, and refused answers go to you; everything else, and every request from your own agents, comes to you. Approvals carry `tier`, `route` and `reason`. Per-run limits are `GET/PUT /v1/runs/{id}/limits` and `cos limits`. Unanswered requests expire after 5 minutes with cos-policy's message. Every answer to a run agent goes in an audit table, which renders the report's "Approved on your behalf (N)" section (`Runs::approvals_report_section`). Tested against `cos-fake-claude`, which can now ask for any tool (`ask:<Tool> <arg>`). See "Approvals policy" in [daemon.md](daemon.md#approvals-policy).
- **Agents in the Mac app (22 Oct 2026): done on the app side, against the contract in [agents-api.md](agents-api.md).** Tested against a fake daemon; the daemon side is being built in parallel.
  - **CosKit:** has every agents and approvals route. `TerminalStream` follows an agent's terminal and reconnects with a fresh snapshot. `DaemonMonitor` keeps agents and pending approvals current from the event stream.
  - **Mac app:** the workspace page lists agents and starts new ones from a prompt. Each agent has a live SwiftTerm terminal page with Stop, Resume and Remove, a Trust banner and Allow/Deny banners. "Waiting on you" shows in the sidebar and the menu bar panel.
  - **Socket fix:** the client's socket moved from Network.framework to plain POSIX sockets. Network sometimes dropped a response when the daemon wrote it and closed at once, which also made the existing tests flaky.
- **Agent runtime (daemon side of the terminals work): done.**
  - **Agents:** the daemon runs Claude Code in PTYs it owns, any number per workspace. It follows them through hooks on one socket (`hooks.sock`, routed by session id) as `starting`, `needs_trust`, `working`, `waiting_approval`, `idle`, `exited` or `interrupted`.
  - **Terminal:** streamed over SSE (a vt100 snapshot, then raw output, resizes and exit), with keys, text and resize routes.
  - **Approvals:** permission requests become approvals you answer with `cos allow|deny` or the API. An answer typed in the terminal settles them. Approvals of run-started agents expire after 5 minutes. The decision seam where `cos-policy` plugs in answers nothing yet.
  - **Lifecycle:** stop, resume (`claude --resume`) and forget. Agents survive a daemon restart as `interrupted`. A workspace with a running agent can't be removed.
  - **CLI:** `cos agent new|stop|resume|rm|say|screen|trust|attach`, `cos agents`, `cos approvals`.
  - **Verified:** against `cos-fake-claude` in tests, and live with Claude 2.1.306 (haiku). Live runs covered start, idle/working, stop, resume into the same session, shutdown to `interrupted`, an approval answered through cos, one answered in the terminal, and the SSE snapshot.
  - **Docs:** [agents-api.md](agents-api.md).
- **Runs in the Mac app (22 Oct 2026): done on the app side, against [orchestration.md](orchestration.md).**
  - **CosKit:** models and calls for runs, the board, tasks (with `agent_id`, `checks`, `review`), questions, messages to the chief, reports and limits, and approvals' `tier`, `route` and `reason`. Unknown states and fields decode. `DaemonMonitor` keeps runs and open questions current, and the board, limits, report and activity of a run a page shows, debounced on `run.*`, `plan.*`, `task.*`, `question.*` and `approval.*` events. A 404 on a newer route means "not supported yet", not an error.
  - **Mac app:** runs under their workspace in the sidebar, a New Run sheet, and a run page with the plan and Approve Plan, the board with retry/fail/cancel and the workers' terminals, the chief's terminal and questions, a message box, limits, an activity list of the run's permission requests, and the report. "Waiting on you" and the menu bar add plans and questions and count only requests routed to you; the menu bar lists active runs with progress.
  - **Tested:** against fake daemons (requests, decoding, monitor refreshes), live against the daemon for the run routes and limits, and offscreen renders of a run in each state.
- **Next:** milestone 4 wiring: the chief and workers launched through the agent runtime, `cos_core::taskgit` called from runs, `cos-policy` plugged into the daemon's approval decision seam, and one run end to end.
