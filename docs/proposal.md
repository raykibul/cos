# cos: design

cos is a chief of staff for coding agents. One coordinator plans the work, hands each task to a worker (Claude Code, later Codex) in its own git worktree, checks and reviews what comes back, merges it, and writes you a report.

This file collects the design agreed on 21 Oct 2026, before any code existed. The project was called cos2bot at the time. Progress and anything learned while building go in the sections at the end.

## Proposal

**In one line:** take Selene's Chief of Staff and drop everything around it. One Rust daemon drives Claude Code and Codex through their headless JSON interfaces, so no hooks. A SwiftUI Mac app and iPhone app connect to it over your tailnet.

> **Changed on 23 Oct 2026:** runs are gone. You talk to each project's Chief of Staff, which coordinates agents; there are no goals, plans, boards, questions or reports, and every approval comes to you. See "Runs removed" under Progress. The run entries below are history.

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
- **Task git lifecycle (git side of milestones 4 and 6): done as a library.** `cos_core::taskgit` creates a worktree and branch per task from the workspace branch, caps parallel workers (3 per workspace, 6 in total), and runs the file-scope check. That check covers the allowed globs, protected paths, renames and deletions, and work left uncommitted. It also runs the test command with a timeout that kills the whole process group, and merges into the workspace with `--no-ff`. A conflict aborts the merge and returns the conflicting paths. The orchestrator, which launches workers and stores task state, still has to call it.
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
- **Runs in the Mac app (22 Oct 2026): done on the app side.**
  - **CosKit:** models and calls for runs, the board, tasks (with `agent_id`, `checks`, `review`), questions, messages to the chief, reports and limits, and approvals' `tier`, `route` and `reason`. Unknown states and fields decode. `DaemonMonitor` keeps runs and open questions current, and the board, limits, report and activity of a run a page shows, debounced on `run.*`, `plan.*`, `task.*`, `question.*` and `approval.*` events. A 404 on a newer route means "not supported yet", not an error.
  - **Mac app:** runs under their workspace in the sidebar, a New Run sheet, and a run page with the plan and Approve Plan, the board with retry/fail/cancel and the workers' terminals, the chief's terminal and questions, a message box, limits, an activity list of the run's permission requests, and the report. "Waiting on you" and the menu bar add plans and questions and count only requests routed to you; the menu bar lists active runs with progress.
  - **Tested:** against fake daemons (requests, decoding, monitor refreshes), live against the daemon for the run routes and limits, and offscreen renders of a run in each state.
- **Pipeline (orchestration of milestones 4–6, daemon side): done.** Once a task is `ready`, the daemon takes a worker slot, creates the task worktree, starts a worker there, reads its `DONE`/`STUCK` line, checks scope, tests and commits, starts a plan-mode reviewer, reads its `VERDICT`, merges (one at a time per workspace) and cleans up, with one retry per task typed into the same worker. Cancelling a run stops its agents and removes task worktrees with no commits. A restart picks up every task where it was. Task JSON has `agent_id`, `checks` and `review`; new events `task.checked`, `task.reviewed`, `task.merged`. Tested against `cos-fake-claude`, which now plays scripted workers and reviewers; verified live with haiku (two dependent tasks merged in order).
- **The chief (part of milestones 4–5): done.** Every run starts a chief agent in its workspace (`runs.chief_agent_id`, model from `COS_CHIEF_MODEL`, else `COS_RUN_MODEL`) with its role in the system prompt and one MCP server, `cos mcp --agent <id>` (the `rmcp` crate), whose tools `board_get`, `board_propose`, `task_update`, `task_send`, `ask_user`, `approval_decide` and `report_write` go through the daemon, which checks the caller and the run's state. A durable inbox turns task results, approvals routed to the chief, your answers and messages, and the run reaching `reporting` into wakeups typed into the chief only while it's idle: 20 items or 24 KiB each, at most 30 an hour, with a brief on top. Questions (`cos questions`, `cos answer`), messages (`cos say`), the board file and the report (`cos report`, with "Approved on your behalf") are in. Tested against `cos-fake-claude`, which can now call an MCP server (`mcp <tool> <json>`); live with Claude 2.1.307 and haiku, the chief explored read-only, proposed a one-task plan through `board_propose`, and on wakeups called `board_get` and `ask_user`, with no permission or MCP prompts; after rebasing onto the pipeline, a pasted wakeup arrived as one message and cancelling the run stopped the chief.
- **Desktop app UI (Tauri, 22 Oct 2026): done on the web side, against [desktop.md](desktop.md).** React 19, TypeScript and Tailwind in `clients/desktop/src`, built and tested in Docker (`make ui-*`). A typed client for every route over the bridge, a store kept current from the event stream with coalesced refetches per event kind, and the sidebar, Waiting on you, event log, project, workspace, agent and run pages with their sheets. New agents start from a composer like Claude's chat input. Agents' terminals use ghostty-web, its WASM shipped as an asset. A fake bridge with a daemon in memory runs the UI in a browser; tests and `make ui-shots` use it. The contract gained `PATCH` for `daemon_request`. The shell is built in parallel.
- **End to end, live (22 Oct 2026): done.** With haiku on a sandbox daemon and a throwaway repo, `cos run "Add subtract and multiply to calc.py, with tests"` went all the way: the chief explored the repo and proposed two dependent tasks, the plan was approved, each worker passed the scope and test checks and a reviewer's approval, both branches merged in order, and the chief wrote the report. 4 routine requests were allowed by the rules; nothing needed you. The first try found a bug: the daemon answered Claude's trust dialog before Claude read keys, so it picked "No, exit". The answer now waits until "Yes" is selected.
- **Desktop app shell (Tauri, 22 Oct 2026): done, against [desktop.md](desktop.md).** It replaces the SwiftUI Mac app, which is removed; CosKit stays for the iPhone. `crates/cos-bridge` talks to the daemon without Tauri: requests limited to `/v1/`, the event stream with reconnects (0.5 s doubling to 5 s), resume after the last `seq` and `log_reset`, and terminal streams that reconnect to a fresh snapshot. It is tested in Docker against a fake daemon. `clients/desktop/src-tauri` exposes it as commands and channels, and adds the overlay title bar, hide on close, the Dock icon reopening the window, a menu bar icon with the waiting count, notifications, Reveal in Finder and Open in Terminal. `make desktop` builds `cos.app`, ad-hoc signed. Checked live against a sandbox daemon with `cos-fake-claude`: the release build with the real UI listed the project, opened an agent's terminal and resized it.
- **Chief of Staff, daemon side (22 Oct 2026): done, against [chief-of-staff.md](chief-of-staff.md).**
  - **Workspace names:** branches are `<project slug>/<city>`, with `-vN` when the branch or the folder is taken. Titles are optional (`null`); Workspace JSON has `city`. Migration 8 rebuilds `workspaces` (migrations now run with foreign keys off and check them before committing).
  - **The chief:** one `role: staff` agent per project, started lazily in a hidden `<slug>/chief` worktree from `HEAD`, with your settings, its instructions and brief in `--append-system-prompt`, and the `cos` MCP server with the staff tools, scoped to its project. Restarts resume the same agent and session.
  - **Inbox:** the agents and runs it starts report there (turn ends with the last message, exits, approvals waiting, run state changes). Your messages go in alone and first; news goes in `[cos · N updates]` wakeups under the run chief's caps.
  - **Transcripts:** every agent's `transcript_path` comes from its hooks; `GET /v1/agents/{id}/transcript` reads Claude's JSONL as chat messages with tool results folded in, and `agent.transcript` says when it grew (at most once a second).
  - **Routes and CLI:** `/v1/projects/{id}/staff` (GET, PUT, `start`, `message`, `inbox`), `staff.started`, `staff.inbox`, `staff.changed` and `staff.stopped` events, and `cos staff`, `cos staff say`, `cos staff attach`.
  - **Tested** against `cos-fake-claude`, which now writes a transcript. Transcript fixtures come from real Claude 2.1.306 transcripts, anonymised. What differs from the contract is under "Deviations" at its end.
  - **Live** with haiku on a sandbox daemon and a throwaway repo: asked to "create a workspace and start an agent that adds a README line", the chief called `workspace_create` (its first try, `from: origin`, failed as the repo has no remote, so it retried with `head`), `agent_spawn` and `brief_update`, and ended its turn. The agent committed the line on `demo/durban`; its Stop came back as a `[cos · 1 updates]` wakeup, the chief checked the commit and reported it. The transcript route returned the chat with the tool calls folded in and the wakeup as an `update`. No permission prompts.
- **Hosts in the desktop app (22 Oct 2026): done, against "App: hosts and remote access" in [chief-of-staff.md](chief-of-staff.md).**
  - **Bridge:** `cos-bridge` reaches a daemon through the socket or over TCP (`name[:port]`, default 7700, bracketed IPv6, read as the CLI reads it). Requests, events and terminals work over both. A 403 with a `code` is a normal response; streams report it as `disconnected` with the `code` and ask again every 5 s. A Mac that can't be reached is `not_running`, naming its address. Tested in Docker against a fake daemon on 127.0.0.1, including a `device_pending` refusal lifted mid-stream.
  - **Shell:** `hosts_list`, `host_add`, `host_remove` and `host_select`. Hosts and the one in use are saved in `hosts.json` in the app's config folder; This Mac is fixed. A switch closes every stream, and a stream started before it is dropped. `daemon_info` adds the host in use.
  - **UI:** a host picker at the top of the sidebar, an Add a Mac sheet, and full pages for each refusal (the approve command for `device_pending`, retried by itself) and for an unreachable Mac. Settings lists the Macs and, on This Mac, turns remote access on or off, sets its port, shows the addresses, the reason it can't listen and the firewall command, and approves, denies or removes devices. Pending devices count in "Waiting on you" and refresh on `device.requested`. The footer names the host. A switch resets the store, chiefs and conversations included.
  - **Not done:** no live check against a second Mac.
- **Tailnet peers (22 Oct 2026): done on the daemon side.** `GET /v1/tailnet/peers` (Unix socket only) lists the other devices on this Mac's tailnet from `tailscale status --json`, marks which are the same Tailscale user's, and probes cos on the online ones with a bare TCP connect (700 ms, cached 10 s), so the app can offer them when you add a Mac. `cos remote peers` prints the list. See "Remote access" in [daemon.md](daemon.md#remote-access).
- **Linux servers and releases (22 Oct 2026): done.** The daemon runs on Linux, for an EC2 box driven from the Mac app over Tailscale ([linux.md](linux.md)).
  - **Builds:** `make linux` cross-compiles static musl binaries for x86_64 and aarch64 in Docker (cargo-zigbuild). `make linux-smoke` runs the daemon, a project and a workspace on stock Amazon Linux 2023 and Debian 12, for both arches.
  - **Service:** `cos daemon install` writes a systemd user unit on Linux and warns when lingering is off. launchd stays on macOS.
  - **Lookups and fixes:** the daemon finds `claude` where its installers put it when it isn't on `PATH`, and `tailscale` in `/usr/bin` on Linux. `cos sessions` works without `ps`. A tagged Tailscale host is told how to fix it.
  - **Releases:** a `v*` tag builds Linux and macOS binaries into a GitHub Release with version-free asset names. `install.sh` (`curl … | sh`) and the `cos-agent` npm package install the right one, checked against `SHA256SUMS`. CI runs `make lint`, `make test`, shellcheck and the npm tests. See [releasing.md](releasing.md).
  - **Verified in Docker:** `cos daemon install` and `uninstall`, restart after a crash, and lingering, under a real systemd. `install.sh` against a fake release, on both distros and both arches.
  - **Not done:** a live EC2 run.
- **Tailnet peers in the desktop app (22 Oct 2026): done.** A `tailnet_peers` command always asks This Mac's socket, whichever host is in use, since the route is host-only. The Add a Mac sheet lists the tailnet above the manual fields, with a chip each (cos running, Online, Offline · seen 3h ago, Added). A Mac running cos connects in one click (`host_add`, then `host_select`), and any other device of yours fills the form with a hint to run `cos remote on` there. Devices that aren't yours sit behind a toggle. The host menu offers running Macs not saved yet, and Settings lists the tailnet read-only. Tested with Vitest against the fake bridge, and in Rust against a one-shot daemon on a Unix socket while another host is selected; not yet tried live against a second Mac.
- **Environment per project (22 Oct 2026): done on the daemon side.** For several Claude accounts on one machine: a global list of environment variables and one per project (the `env` table, migration 9), merged as daemon < global < project < cos's own, and given to every agent launched for the project, of every role, at every launch, resume and restart. A leading `~` is expanded. Names cos sets itself (`TERM`, `COLORTERM`, the hook socket, Claude's session markers, `COS_*`) are refused, listed once in `cos_harness::claude::reserved_env_names`. Values never go in events or logs: `env.changed` and the agent's new `env_names` carry names. Routes on the socket and the tailnet (`/v1/env`, `/v1/projects/{id}/env`), and `cos env [set|unset] [--project P] [--show]`. A config dir that isn't logged in shows Claude's login screen, which the daemon spots and reports as the new state `needs_login`. `cos sessions` also scans the config dirs set in cos. Tested against `cos-fake-claude`, which can now dump its environment and play a login screen. See "Environment" in [daemon.md](daemon.md#environment).
- **Environment variables in the desktop app (22 Oct 2026): done on the app side, against the env routes in [daemon.md](daemon.md).** For several Claude accounts on one machine: a project's `CLAUDE_CONFIG_DIR` picks the account its Chief of Staff, the chief's agents, workspace agents and runs use. Each project has an Environment page (from its ⋯ menu and the chief's), Settings has the list for all projects, and the project page shows the global variables it inherits with Override. Values stay masked until Show, and a name the daemon refuses shows its reason. The chief's header shows `env: …` and the account (`.claude-personal`); agent tabs show `env_names`. The store follows `env.changed`. An agent on Claude's login screen (`needs_login`) waits on you, with the command that logs its config dir in and Open terminal. Tested with Vitest against the fake bridge; the daemon side is built in parallel.
- **Chief of Staff and hosts, the gaps (22 Oct 2026): done.** Pause coordination per project (`staff.paused`, migration 11: news waits, your messages still go in; `PUT …/staff {paused}`, `cos staff pause|resume`, a ⋯ item and a "Coordination paused · N updates waiting" strip). Chiefs left `interrupted` by a daemon restart or crash come back with `--resume`, one a second, unless you stopped them (`staff.stopped`); your agents stay `interrupted` with Resume and `cos agent resume`. Transcripts are cached per file and parse only appended lines, and carry `context {used_tokens, window_tokens}`, shown as a context meter in the chief's header. The chief's terminal split is resizable and remembered. Hosts can be renamed (`host_rename`, This Mac too), with unique names of at most 60 characters, and the app says "machine" for hosts. Tested against `cos-fake-claude` (restart and pause), with transcript fixtures, in Rust in `src-tauri`, and with Vitest and screenshots against the fake bridge.
- **Mac app releases (22 Oct 2026): done, not yet run on a tag.** The release workflow builds one universal `cos.app` (Tauri, `--target universal-apple-darwin`, ad-hoc signed, version from the tag) and uploads `cos-desktop-universal-apple-darwin.zip`, with `SHA256SUMS` written once over every asset. `install.sh --app` (and `npx cos-agent app`) checks it, quits a running copy by pid, replaces `cos.app`'s contents in `/Applications` or `~/Applications`, and clears the quarantine flag, since the app isn't notarized; `--with-cli` adds the CLI. `make release-local` builds every asset into `dist/release`. Tested natively on a Mac against fake releases (`make app-install-test`, also in CI on macOS), in Docker for the npm side, and with a real local universal build through both installers into scratch folders. See [releasing.md](releasing.md#the-mac-app).
- **iPhone app, without push (phase 3, 22 Oct 2026): done, against [iphone.md](iphone.md).**
  - **CosKit:** reaches a daemon over TCP (`name[:port]`, default 7700, bracketed IPv6, parsed as the CLI parses `--host`) as well as the Unix socket, with HTTP and the event stream over both. A refusal is `CosError.refused` carrying its `code` (`device_pending`, …); a host that doesn't answer is `.unreachable` with why. New: the Chief of Staff routes, transcripts, the environment (read-only), `needs_login`, `spawned_by`, workspace cities and nullable titles, and a `CosService` protocol. Decoding is lenient: a bad row is dropped, not the list. Tested with `swift test` against fake daemons on a Unix socket and on 127.0.0.1.
  - **App:** SwiftUI for iOS 26 in `clients/iphone`, generated by XcodeGen. Hosts by Tailscale name or `100.x` address, with full-screen states per refusal (the approve command for `device_pending`, retried by itself) and for an unreachable host. Tabs: Waiting on you (approvals, chief questions, plans, trust dialogs, Claude logins, with a badge), Projects (each Chief of Staff as a chat from its transcript, workspaces), Runs (board, plan approval, report, cancel, new run) and Hosts; agent pages with Send message and a screen snapshot. Live from `/v1/events` in the foreground only. Notifications go through a no-op seam, ready for APNs.
  - **Tested:** the models against an in-app fake daemon (`FakeCos`, also behind `-fakeData` for screenshots), with `xcodebuild test` on the iOS 26.3 simulator; screenshots of each screen in light and dark were reviewed.
  - **Not done:** push (what the daemon needs is in [iphone.md](iphone.md#push-later)), and a live check from a real iPhone.
- **jev in shadow mode (22 Oct 2026): done.** See [judge.md](judge.md).
  - **`cos-judge`:** an async client for TypeSafe's System One endpoint: one call per decision point with all its questions, a 1 s timeout, typed answers (yes/no, choice decoded into Rust enums with unknown options as errors, score). `judge/questions.json`, embedded, holds every question and threshold. Errors, timeouts and confidence below 0.5 are no opinion. Inputs are trimmed to structured fields, with paths relative to the worktree or home.
  - **Permission sorting:** every request cos-policy sends to you is judged (`action_kind`, `serves_task`, plus `leaves_worktree` as a cross-check); jev would approve only worktree-local `read_only`, `build_or_test` or `edit_in_worktree` above 0.9 confidence with `serves_task` above 0.8. Your answer is stored next to it, for agreement stats and labelled examples.
  - **Waking a chief:** every run chief and Chief of Staff inbox item except your own words gets `wake` (now/batch/none) and `is_question_for_chief`. Wakeups are unchanged.
  - **Storage, switch, API:** migration 11 (`judge_projects`, `judge_decisions`); off per project by default; `GET /v1/judge`, `POST /v1/judge/reload`, `PUT /v1/projects/{id}/judge`, `judge.decided` (no inputs), `judge.changed`; `cos judge status|stats|on|off|test`.
  - **Key:** `TYPESAFE_API_KEY` from the daemon's environment, then `daemon.env` (now read by the daemon itself on macOS too, refused if others can read it), then one login-shell probe; `cos daemon install` copies it into `daemon.env` (0600).
  - **Desktop:** a "Judgment (jev)" section in Settings with the key's source, per-project switches and "would have auto-approved 41 of 45, disagreed on 4" with the disagreements.
  - **Tested** against a fake jev server and `cos-fake-claude`, and against a sandbox daemon. No live call yet: the key isn't in this Mac's login shell under `TYPESAFE_API_KEY`.
- **Tailnet discovery on the iPhone (22 Oct 2026): done.** `GET /v1/tailnet/peers` is served to approved devices over the tailnet too (still not `/v1/devices` or `GET`/`PUT /v1/tailnet`), marks the asking device `is_you`, and names the answering host in `self`. CosKit decodes it leniently. The iPhone's add screen lists "On your tailnet" from every saved host that answers, merged by node ID, with Connect for machines running cos, and the hosts tab shows unsaved ones running cos; an older cos's `host_only` is a quiet note. See [iphone.md](iphone.md#your-tailnet). Tested with daemon tests (an approved client gets the list, `is_you`), CosKit tests and view-model tests on `FakeCos`; not yet from a real iPhone.
- **`cos upgrade` (23 Oct 2026): done.** Finds the latest release from GitHub's `releases/latest` redirect and runs that tag's `install.sh` on the folder `cos` is in, so the installer keeps the checksum check and cos stays without TLS. Source builds are refused (`--force` overrides), and so are folders you can't write. On macOS it also upgrades an installed `cos.app` and reopens it if it was running. If the daemon's service runs this binary, it restarts it (`launchctl kickstart -k` or `systemctl --user restart`), asking first when agents are at work, and waits for the new version. Options: `--check`, `--version`, `--yes`, `--no-restart`, `--no-app`. `--host` is refused. Tested in Docker against a fake release folder, and through a fake of the OS for the app and restart paths. Not yet run against a real GitHub release. See [releasing.md](releasing.md#upgrading-with-cos-upgrade).
- **iPhone chats layout (23 Oct 2026): done, against [iphone.md](iphone.md).** The home tab lists each project's Chief of Staff as a chat (latest first, what waits on you as an orange count, "2 sessions working…" as a chat app shows "typing…", a dot for news since you last opened it), with Waiting on you as a pinned row. A chief's chat uses bubbles on a wallpaper, shows what its agents need from you as messages with Allow and Deny in them, and its ⋯ menu opens the project's workspaces (with their sessions), sessions and runs. Tabs are now Chiefs, Runs and Hosts. Picked from the variations at https://claude.ai/artifact/V8xtL99Mnz8Ge2n1i6Jo9B (variation B).

- **iPhone terminal and chat cards (23 Oct 2026): done, against [iphone.md](iphone.md).** Every session and each chief has a full-screen live terminal on the phone (SwiftTerm 1.10.1, typing, a key bar, resize to fit, landscape, chips to switch between the chief and its live sessions, approvals in a bar). In a chief's chat, a session it started is a card that opens its terminal, and Claude's `<task-notification>` is an "Agent finished" card with the report in a sheet instead of a raw bubble. The daemon's transcript adds `agent_id` to cos's agent tool rows (from `agent_spawn`'s result, or the `agent_id` argument). Mockups: row C at https://claude.ai/artifact/V8xtL99Mnz8Ge2n1i6Jo9B. Tested with daemon, CosKit and view-model tests on `FakeCos`; not yet built in Xcode or tried on a real iPhone.
- **iPhone terminal on Ghostty (24 Oct 2026): done.** The phone's terminal is Ghostty (libghostty-spm's `GhosttyTerminal`, fed through an in-memory session) instead of SwiftTerm, so it scrolls through its scrollback with a drag. Built and installed on an iPhone 17; the iPhone tests pass.
- **Runs removed (23 Oct 2026): done.** The goal machinery is gone from every layer: the run pipeline, the run chief and its MCP tools, the board, tasks, task worktrees (`cos_core::taskgit`), questions, messages to a run's chief, reports, limits and presets, the `cos run`/`runs`/`questions`/`answer`/`say`/`report` commands, the `/v1/runs` and `/v1/questions` routes, and the run screens in the Mac and iPhone apps. The Chief of Staff's `run_start` tool is gone too. Migration 12 drops the run tables; the event log keeps its `run_id` column, always null now. Agents have the roles `user` and `staff` only. cos-policy still classifies every request (tier and reason, shown with it and given to jev), but nothing answers on your behalf and nothing expires: every approval waits for you. Agents a Chief of Staff starts still answer their own trust dialog.
- **File attachments (24 Oct 2026): done.** Send files to a Chief of Staff or an agent from the Mac app, the iPhone, or any machine with `cos --host … --file`. A file is uploaded first (`POST …/attachments?name=`, 32 MB at most) into the agent's own folder, `.cos/attachments/`, which a self-ignoring `.gitignore` keeps out of git; a message then names it in `attachments` (20 at most), and cos types the paths at the end as an "Attached files" block, which the transcript takes back out into `attachments`. The chief passes files on with `attachments` on `agent_spawn` and `agent_send`, which copy them into the agent's folder. `GET /v1/agents/{id}/attachments/{name}` serves them back for the apps to show. `attachment.added` is logged. The Mac app takes files from a paperclip, drag and drop or paste on the chief's and New agent composers, with chips while they upload and pictures in the chat; the iPhone takes them from Photos and Files in the chief's chat. Tested against `cos-fake-claude` (upload before the chief starts, the chief's transcript, download, `agent_spawn` and `agent_send` copies, uploads to an agent and a workspace), in cos-bridge, with Vitest and screenshots against the fake bridge, and with CosKit and view-model tests written for `FakeCos`. Not yet built: the Tauri shell and the Swift code (no Xcode where this was written), and not tried against the real Claude CLI, which might turn a pasted image path into an image of its own. See "Attachments" in [agents-api.md](agents-api.md#attachments).
- **Desktop UI/UX pass (24 Oct 2026): done, against [desktop.md](desktop.md).** Every session gets the chief's chat: an agent's page is its conversation with a one-line composer, and Ctrl-Shift-J opens Claude's terminal beside it (Ctrl-J until 25 Oct). The sidebar lists sessions flat, with no workspace rows, and joins sessions that share a folder with a coloured bracket; state icons differ by shape and say what a session needs. It steps aside below 1040 px, and ⌘B toggles it. A New session sheet picks a folder. The workspace page shows session cards that answer requests in place, instead of tabs of terminals. ⌘↩ and ⌘⌫ allow and deny, and ↑↓ choose on Waiting on you. Every composer is one line, like Claude's desktop app. Tested with Vitest against the fake bridge and checked in screenshots; not yet tried in the Tauri shell.
- **Chief memory notes and new sessions (24 Oct 2026): done.** A chief keeps up to 50 named notes of 8 KiB per project (`staff_notes`), with `memory_list`, `memory_read`, `memory_write` and `memory_delete`; only their names and descriptions go in its system prompt, after the brief. You can read and edit them through `…/staff/memory` and `cos staff memory`. `…/staff/new-session` (`cos staff new`, the Mac app's ⋯ menu) drops the chief's Claude session and starts a fresh one as the same agent, so its agents stay its own. Not yet: a memory page in the iPhone app, and the iPhone's New session (the Mac app's page came on 26 Oct).
- **Terminal tabs (25 Oct 2026): done, against [desktop.md](desktop.md) and "Shells" in [agents-api.md](agents-api.md#shells).** Ctrl-J in the Mac app opens a terminal tab: a login shell the daemon runs (`/v1/shells`, in memory; over the tailnet too since 26 Oct) in the project's root from a chief, or in the session's or workspace's folder, with the project's environment. Closing the tab ends the shell. Tested in Rust against a real `/bin/sh` and in cos-bridge, and with Vitest against the fake bridge; the Tauri command isn't built yet.
- **Session menu (25 Oct 2026): done, against [desktop.md](desktop.md).** Right-click a session in the sidebar to start another one in its folder or remove it. Removing stops a running session first and deletes its worktree too when no other session uses it (unless it has uncommitted changes, or you untick it). Tested with Vitest against the fake bridge.
- **Chat sync fixes (25 Oct 2026): done.** The Mac app's streams to another host now give up on a connection that brings nothing for 45 s (the daemon sends a keep-alive every 15 s) and reconnect, as the iPhone already did. Before, a connection to an EC2 host that died silently (the Mac slept, the network changed) stayed open for hours, so a session's chat stopped updating and stayed on "Working…" while its terminal, opened later, was live. The transcript also shows a message you sent while Claude was busy, which Claude records as a `queued_command` attachment when it reads it mid-turn; before, it stayed "Sent · it reads this when it's free" for good. Tested in cos-bridge against a fake host that goes silent, and with a transcript test whose `queued_command` entry has the shape of a live Claude Code transcript (only the attachment is written, no user entry). Mid-turn `<task-notification>`s arrive the same way, so they now show as cards too.
- **Brief & memory in the Mac app (26 Oct 2026): done.** You can set the chief's brief yourself (`PUT …/staff {brief}`), and the Mac app has a Brief & memory page for each chief: the brief in an editor, and its memory notes to read, add, edit and delete. It opens from the chief's header, its ⋯ menu, the project's menu, and a "Give it a brief first" card before the chief starts. Changes reach the chief on its next start, so the page offers New session. Tested against `cos-fake-claude` (the brief before and after it starts, too long, cleared) and with Vitest against the fake bridge. Not yet: the iPhone.
- **Chief identity, sessions beside the chief, numbered terminals (26 Oct 2026): done.** You can write the chief an identity (`PUT …/staff {identity}`, about 500 words, at most 8 KiB): who it is and how it works. It goes near the top of its system prompt, and only you can change it, since the chief has no tool for it. The Mac app's page is now "Identity & memory", with the identity first. Clicking a session in the sidebar opens it beside the project's chief instead of in a tab (⌘-click still opens a tab), and that pane switches between the session's transcript and its terminal (Ctrl-Shift-J). Ctrl-J terminal tabs are numbered per project ("Terminal 2 · cos") and listed under Terminals in the sidebar. Tested against `cos-fake-claude` (set before and while it runs, the chief can't set it, too long, cleared) and with Vitest against the fake bridge. Not yet: the identity on the iPhone, and a CLI command for it.
- **Terminals on other machines (26 Oct 2026): done.** Ctrl-J terminals now open on a machine the Mac app reaches over the tailnet, such as an EC2 host: `/v1/shells` is no longer host-only, so approved devices of your Tailscale user can start, type into and close shells, as they already type into agents' terminals. The daemon logs each shell a device starts, with the device. The sidebar's Terminals list shows on every machine, and an older cos's `host_only` becomes a toast that says to update it. Tested in Rust over a TCP listener with a fake Tailscale and with Vitest against the fake bridge; not yet tried live against an EC2 host.
- **Next:** daily use on real repos.
