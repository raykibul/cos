# Orchestration (contract)

How a run goes from a goal to merged branches and a report. Four pieces are built in parallel against this document:

1. **Pipeline:** workers, checks, the reviewer and merging.
2. **Chief:** its MCP tools, its inbox and wakeups, questions, and the report.
3. **Approvals wiring:** `cos-policy` in the daemon, chief decisions and the audit trail.
4. **Mac app:** runs, the board, questions and the report.

Change this document only together with all sides.

Building blocks already on `cos/build`, all described in [daemon.md](daemon.md) and [agents-api.md](agents-api.md):
- `Runs` (runs, tasks, plan validation, the board),
- `cos_core::taskgit` (task worktrees, the scope check, test runs, merges),
- the agent runtime (agents in PTYs with hooks, `role` = `user` | `chief` | `worker` | `reviewer`),
- `cos-policy` (the approval tiers),
- the event log.

## The run, end to end

1. **Start.** `cos run "goal"` or `POST /v1/runs` creates the run (`planning`). The chief component starts a **chief agent** (`role: chief`) in the workspace folder and stores its id as `runs.chief_agent_id`.
2. **Plan.** The chief explores the repo read-only and calls `board_propose`. The run moves to `awaiting_plan_approval`, and the board is written to `<cos home>/runs/<run id>/board.md`. Nothing is written inside the repo.
3. **Approve.** You approve (CLI, app or `POST /v1/runs/{id}/approve-plan`). Tasks with no dependencies become `ready`.
4. **Work.** For each task that becomes `ready` (event `task.state_changed` with `to: "ready"`), the pipeline:
   1. Waits for a `WorkerLimiter` slot.
   2. Creates the task worktree (`TaskWorktrees` under `<cos home>/tasks`) and calls `Runs::assign_worktree`.
   3. Starts a **worker agent** (`role: worker`, `run_id`, `task_id`, cwd = the task worktree).
   4. Moves the task to `running`.
5. **Done.** The worker is done when its `Stop` hook fires and it says it's done. The pipeline moves the task to `checking`, then runs:
   - the scope check: fail → one retry with the violations as the reason, else `blocked`,
   - the task's test command: fail → one retry with the test output tail, else `blocked`,
   - a commit check: no commits ahead of base → `blocked` with "the worker made no changes".
6. **Review.** The task moves to `review`, and the pipeline starts a **reviewer agent** (`role: reviewer`, cwd = the task worktree, permission mode `plan`, read-only) with the task, the diff summary and the test result. Its verdict is approve, or changes with notes (see Verdicts). Changes → one retry: the notes are sent to the worker, and the task goes back to `running`.
7. **Merge.** On approve, `TaskWorktree::merge`. `Merged` → task `merged`, the worktree removed and the branch deleted. `Conflict` → task `blocked` with the conflicting paths. Merges into one workspace run one at a time.
8. **Report.** When every task is merged, failed or cancelled, the run moves to `reporting` by itself. The chief is woken and calls `report_write(markdown)`. The daemon:
   - appends the "Approved on your behalf" section (from the approvals component),
   - saves the report to `<cos home>/runs/<run id>/report.md`,
   - calls `Runs::finish`.

   The run is `done`.
9. **Blocked tasks.** These wake the chief, which retries (once), fails or cancels the task with `task_update`, or asks you with `ask_user`.

**Cancelling a run** stops all of its agents (`stop`), removes task worktrees that have no commits, and keeps branches that have commits.

## Ownership

| Piece | Owns | Store migration |
|---|---|---|
| Pipeline | `daemon/pipeline.rs` (or `daemon/pipeline/`); the `agents.task_id` column; worker and reviewer prompts; `Verdict` parsing | adds `agents.task_id`, `tasks.checks_json`, `tasks.review_json` |
| Chief | `cos mcp` (stdio MCP bridge, `rmcp`); MCP tools; chief prompt; the inbox and wakeups; the questions table and routes; `runs.chief_agent_id`; the report route | adds `runs.chief_agent_id`, `inbox`, `questions` |
| Approvals | the `decide` seam in `daemon/agents/hooks.rs`; per-run `Limits` and `RunLedger`; chief decisions; the audit table | adds `approval_audit`, `runs.chief_approvals`, and on approvals `tier`, `route`, `reason` |
| Mac app | `clients/` only | none |

Each piece appends **one** entry to `MIGRATIONS` and names its index in its report; the coordinator renumbers when merging. Keep edits to shared files (`api.rs` router, `AppState`, `main.rs`, `events.rs`, `lib.rs`) to a few additive lines.

## Agents for a run

Every run agent is a normal runtime agent (see agents-api.md), so you can watch it, type into it, stop it and resume it. Run agents differ from your own:

- **Settings:** `user_settings: false`. Only the repo's settings apply, so your personal allow rules don't approve things before cos sees them.
- **Permission mode:** `acceptEdits` for workers, `plan` for the reviewer, `default` for the chief.
- **Instructions:** `--append-system-prompt` carries each role's instructions. The first prompt is typed in as usual.
- **Chief tools:** the chief gets `--mcp-config` with one server, named **`cos`** (tools show up as `mcp__cos__…`, which `cos-policy` recognizes): `{"mcpServers":{"cos":{"command":"<cos exe>","args":["mcp","--agent","<agent id>"]}}}`. The worker and reviewer get no MCP server.
- **Trust:** run agents are started in workspaces and task worktrees of registered projects. If one still shows the trust dialog, the daemon answers it, because registering a project counts as trusting it. This is the one exception to "cos never answers the trust dialog by itself".
- **When a run agent counts as done:** each `Stop` hook. The worker and reviewer are told to finish in one turn. The pipeline reads the `Stop` hook's `last_assistant_message`.

### Verdicts

The reviewer's last message ends with exactly one line:

```
VERDICT: APPROVE
VERDICT: CHANGES: <one paragraph of what to fix>
```

The worker's last message ends with `DONE` or `STUCK: <why>`. `STUCK` moves the task to `blocked` with that reason. A missing or unparseable line counts as `STUCK: no verdict line` (or `CHANGES: no verdict line` for the reviewer), and the agent is asked once to finish with the line.

## Chief tools (MCP server `cos`)

| Tool | Arguments | Effect |
|---|---|---|
| `board_get` | – | The board as JSON and markdown. |
| `board_propose` | `{tasks: [{title, instructions, files, depends_on?, test_command?}]}` | `Runs::propose_plan`. Validation errors come back as a tool error listing every problem. |
| `task_update` | `{task_id, action: "retry" \| "fail" \| "cancel", reason}` | A retry of a blocked task goes to `ready`. |
| `task_send` | `{task_id, message}` | Types a message into the task's worker, if it's running. |
| `ask_user` | `{question, choices?}` | Creates a question for you. The answer reaches the chief through its inbox. Returns at once. |
| `approval_decide` | `{approval_id, decision: "allow" \| "deny", reason}` | Only for approvals routed to the chief. The approvals component checks it against the limits. |
| `report_write` | `{markdown}` | Only while the run is `reporting`. |

## Inbox and wakeups (chief)

These events go into the chief's inbox:
- `task.state_changed` for its run (`merged`, `blocked`, `failed`),
- approvals routed to it (`approval.routed` with `route: "chief"`),
- your answers to its questions,
- your messages to it (`POST /v1/runs/{id}/message` / `cos say`),
- the run entering `reporting`.

When the chief is `idle` and its inbox isn't empty, the daemon types one wakeup message holding up to 20 inbox items or 24 KiB, oldest first, and marks them delivered. There are at most 30 wakeups an hour; past the cap, items wait.

## New routes

| Owner | Route | Body | Result |
|---|---|---|---|
| Chief | `GET /v1/questions?state=open` | | `{questions: [Question]}` |
| Chief | `POST /v1/questions/{id}/answer` | `{answer}` | Question |
| Chief | `POST /v1/runs/{id}/message` | `{text}` | 202. Goes to the chief's inbox. |
| Chief | `GET /v1/runs/{id}/report` | | `{markdown}`, or 404 before it exists |
| Approvals | `POST /v1/approvals/{id}/chief` | `{decision, reason}` | Approval; 403 if the limits refuse it |
| Approvals | `GET /v1/runs/{id}/limits`, `PUT /v1/runs/{id}/limits` | `{preset, approval_cap}` | `{preset, approval_cap, chief_approvals}` |

```jsonc
// Question
{"id": 4, "run_id": 2, "agent_id": 9, "question": "Use Redis or in-memory?", "choices": ["redis", "memory"],
 "state": "open", "answer": null, "created_ms": 0, "answered_ms": null}   // state: "open" | "answered"
```

**Approval JSON gains:**
- `tier` (`allowed` | `chief` | `ask_user` | `never_delegated`),
- `route` (`auto` | `chief` | `user`),
- `reason`: the policy's one-line reason.

`decided_by` may now also be `rule` or `chief`.

- **Auto:** auto-allowed requests are still recorded, with `state: "allowed", decided_by: "rule"`, but they don't notify anyone.
- **Your own agents:** a `role: user` agent always routes to `user`, the same as today.

**Task JSON gains:**
- `agent_id` (the current worker),
- `checks`: `{scope: {ok, out_of_scope, protected, uncommitted}, tests: {status, exit_code, output_tail}}`,
- `review`: `{verdict: "approve" | "changes", notes}`.

**Run JSON gains:**
- `chief_agent_id`,
- `report_path` (it exists already).

## New events

| Kind | Data | Emitted by |
|---|---|---|
| `task.checked` | `{task_id, scope_ok, tests}` | pipeline |
| `task.reviewed` | `{task_id, verdict}` | pipeline |
| `task.merged` | `{task_id, commit}` | pipeline |
| `approval.routed` | `{approval_id, route, tier}` | approvals |
| `question.asked` / `question.answered` | `{question_id, run_id}` | chief |
| `run.report_written` | `{run_id, path}` | chief |

## CLI additions

| Command | Owner |
|---|---|
| `cos questions`, `cos answer <id> "text"` | chief |
| `cos say [run] "text"` (a message to the chief) | chief |
| `cos report [run]` | chief |
| `cos limits [run] [--preset …] [--cap N]` | approvals |
