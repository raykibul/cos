# jev in cos

jev is TypeSafe's "System One" model: structured state in, typed probabilistic answers out, never text. cos uses it as a fast judgment layer between the daemon's Rust rules and the chief model (the design is "jev: a fast judgment layer" in [proposal.md](proposal.md#jev-a-fast-judgment-layer)). **Today it runs in shadow mode only:** at each decision point jev is asked, cos stores what it would have done next to what actually happened, and nothing it says changes anything. Each decision point goes live separately, once its agreement with you looks good.

The code: the `cos-judge` crate (client, questions, would-haves, key lookup), `cos_core::judge` (storage), `crates/cos/src/daemon/judge.rs` (the decision points), `daemon/judge_api.rs` (routes) and `judge_cmd.rs` (`cos judge`).

## The client (`cos-judge`)

- **Endpoint:** `POST https://api.typesafe.ai/v1/systemone`, model `jev-latest`, `Authorization: Bearer $TYPESAFE_API_KEY`. The body is `{model, state, questions}`; answers come back under the same keys.
- **One call per decision point**, with all of that point's questions in it: they're evaluated in parallel and in isolation, so batching is about 12x cheaper than one call per question, with the same answers.
- **Typed answers:** a yes/no probability (`noul`, no confidence field), a choice (the picked option, every option's probability and a confidence) and a score (value, probabilities, confidence). Choices decode straight into Rust enums (`ActionKind`, `Wake`, declared with `choice_options!`); an option jev returns that the enum doesn't have is an error, never a fallback.
- **Failing safe:** a 1 s timeout. On an error, a timeout or a choice confidence below 0.5, jev has **no opinion**, and things go on exactly as without it.
- **Trimmed inputs:** structured fields only, never whole files. A Bash command is cut to 1,000 characters, other fields to 300, a task's instructions to 600; file contents (`content`, `old_string`, `new_string`, `edits`) are replaced by their size; nested values by their length. The worktree's path is written as `.` and your home as `~`, so jev never sees where things are on your Mac.
- **Tests** run against a fake HTTP server on 127.0.0.1; nothing calls the real API.

## `judge/questions.json`

`crates/cos-judge/judge/questions.json` holds every question and every threshold, and is embedded in the binary. It's the file to review and tune; a unit test checks it the way the jev skill's `check` does (types, instructions, 2–255 choice options, 2–10 described score levels) and that each choice's options match its Rust enum.

| Point | Question | Type | Used for |
|---|---|---|---|
| `permission` | `action_kind` | choice: `read_only`, `build_or_test`, `edit_in_worktree`, `install_dep`, `external`, `destructive` | what the request does |
| | `serves_task` | yes/no | is it plausibly needed for the task? |
| | `leaves_worktree` | yes/no | a cross-check: does it read, write or send anything outside the worktree? (a near-free extra question, as the skill suggests) |
| `wake` | `wake` | choice: `now`, `batch`, `none` | how soon the chief should read an inbox item |
| | `is_question_for_chief` | yes/no | does the item ask the chief something? |

Each option has a `what`, a `not_for` and examples, written as situations rather than degrees, per the skill. The thresholds: `min_confidence` 0.5; for permission requests `action_kind_confidence` 0.9, `serves_task` 0.8, `leaves_worktree` 0.5, and `auto_approve` = `read_only`, `build_or_test`, `edit_in_worktree`.

## Decision point 1: permission requests

For every permission request the approvals policy sends **to you** (`route: user`: everything from your own agents and Chiefs of Staff, and run agents' requests the chief may not take), the daemon sends jev the trimmed request (tool, input, `cwd`, the asking agent's role), the task's title and instructions (or the agent's title when it has no task), and cos-policy's tier and reason. Then code works out what jev **would** do:

- **approve** only if Rust says the action stays in the worktree (cos-policy's tier is `allowed` or `chief (worktree)`: every path checked, no network, no install), `action_kind` is one of `auto_approve` with confidence **above 0.9**, `serves_task` is **above 0.8** and `leaves_worktree` is below 0.5;
- **ask you** otherwise, with the reason (`not a kind jev may approve: install_dep (0.97), …`), or as **no opinion** on an error, a timeout or low confidence.

jev can't approve anything that leaves the worktree, whatever it answers: that check is code, so instructions injected into a request can't talk their way past it. The request still goes to you exactly as before.

When you answer, the daemon records what you did next to jev's would-have (from `approval.decided`, or straight away if you answered before jev did):

| `actual` | When |
|---|---|
| `allowed`, `denied` | you answered through cos (app, phone, `cos allow`/`deny`) |
| `terminal` | answered in the agent's terminal; allow or deny isn't known |
| `expired` | nobody answered in 5 minutes |
| `chief_denied` | the chief denied it (it may deny any run agent's request) |

Stats count only `allowed` and `denied`:

- **would approve:** of those, how many jev would have auto-approved ("would have auto-approved 41 of 45");
- **agreed:** jev would approve and you allowed, or jev (with an opinion) would ask and you denied;
- **disagreed:** jev would approve and you **denied**: the mistakes that matter, listed as examples;
- **missed:** jev (with an opinion) would ask and you allowed: interruptions jev wouldn't have saved.

Every decision row (input, answers, your answer) is a labelled example for tuning `questions.json`.

## Decision point 2: waking a chief

Every item added to a run chief's inbox (`inbox`) or a Chief of Staff's (`staff_inbox`) is sent to jev with which chief it's for, its kind, its text (cut to 2,000 characters) and, for a run, the run's goal. Your own messages and answers are skipped: they always wake the chief. The would-have is `now`, `batch`, `none`, or `no_opinion`. Wakeups are unchanged; this only collects what jev would have done, to replace the flat cap of 30 wakeups an hour with something that reads the item.

The daemon follows both inbox tables by id (every 2 s, from the newest item at startup), so the chiefs' code doesn't know jev exists. Items added while a project is switched off, or while there's no key, are passed by.

## The per-project switch

jev is **off by default** for every project, because what it judges (commands, paths, task titles, inbox text) goes to TypeSafe. `cos judge on --project P` (or the app's Settings, or `PUT /v1/projects/{id}/judge`) switches a project on. Check work repos before switching them on. Removing a project removes its switch; its decisions stay as examples.

## The API key

`TYPESAFE_API_KEY`. You keep it in `~/.zshrc` or `~/.bashrc`, but the daemon usually runs under launchd or systemd, which never read those, so it looks in order and logs which source won, never the value:

1. **the daemon's environment**;
2. **`daemon.env` in the cos home** (`~/.cos/daemon.env`): `NAME=value` lines, read by the daemon itself at startup on macOS and Linux (systemd also loads it into the environment, so a value found in both counts as `daemon_env`). A file other users can read or write is **refused**, with a warning saying `chmod 600`;
3. **your login shell, once:** `$SHELL -lic` printing the variable between two markers, with no stdin, stderr discarded and a 3 s timeout (the markers let it ignore whatever the rc files print). It runs in the background at startup; the result is kept for the daemon's lifetime. `cos judge status --reload` (`POST /v1/judge/reload`) looks again, `daemon.env` included.

`cos daemon install` copies `TYPESAFE_API_KEY` from the shell you run it in into `daemon.env` (mode 0600, other lines kept), never into the plist or the unit. Without a key jev is off, and `cos judge status` says why. Every lookup and switch change logs `judge.changed`.

## Storage

Migration 11 (`MIGRATIONS[11]`): `judge_projects (project_id, enabled, updated_ms)` and `judge_decisions`:

| Column | |
|---|---|
| `point` | `permission` or `wake` |
| `project_id`, `subject`, `approval_id` | what was judged: `approval:12`, `inbox:5`, `staff_inbox:9` |
| `summary` | one line for people |
| `input_hash` | SHA-256 of the trimmed input as JSON |
| `input` | the trimmed input sent (your own database; never in an event) |
| `answers` | the typed answers, null on error |
| `would`, `why`, `no_opinion` | the would-have and its reason |
| `actual`, `actual_ms` | what you did (permission requests only) |
| `latency_ms`, `error`, `model` | the call |

No foreign keys: decisions outlive their project, approval or inbox item.

## API, events and CLI

| Route | Body | Result |
|---|---|---|
| `GET /v1/judge` | | `{key: {source, detail, probing}, active, off_reason, endpoint, model, projects: [{project_id, name, enabled}], points: [{point, decisions, errors, no_opinion, would, answered, would_approve, agreed, disagreed, missed, disagreements}]}` |
| `POST /v1/judge/reload` | | the same, after looking for the key again |
| `PUT /v1/projects/{id}/judge` | `{"enabled": bool}` | `{project_id, name, enabled}`; 404 for an unknown project |

`key.source` is `environment`, `daemon_env`, `shell` or `missing`. Served on the socket and the tailnet listener.

| Event | Data |
|---|---|
| `judge.decided` | `{decision_id, point, project_id, subject, would, latency_ms, error}`: never the input |
| `judge.changed` | `{project_id, enabled}` for a switch, `{key_source}` after a key lookup |

| Command | |
|---|---|
| `cos judge status [--reload]` | where the key was found (or why jev is off), the switches, counts |
| `cos judge stats` | per decision point: decisions, would-have auto-approvals, agreement, disagreements with examples, wake counts |
| `cos judge on\|off --project P` | the switch |
| `cos judge test` | one real call from the CLI (not the daemon) with a synthetic `ls` request, printing the typed answers and the would-have. It finds the key as the daemon would. Nothing from your repos is sent. |

The desktop app shows it in Settings → "Judgment (jev)" (see [desktop.md](desktop.md)).

## Cost and limits

About $0.042 per million input tokens; a permission check is around 1.5K tokens. At most four calls run at once. jev reads text literally and has no built-in defence against instructions in the state, which is why it may only ever approve what Rust already knows stays in a throwaway worktree.
