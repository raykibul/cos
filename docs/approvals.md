# Approvals policy

How cos decides who may answer an agent's permission request. The code is the `cos-policy` crate. It is pure: no database, no clock, no daemon, so the daemon calls it and stores what it returns. The design is in [proposal.md](proposal.md#approving-on-your-behalf).

## Flow

```
PermissionRequest hook / can_use_tool
  → ApprovalRequest {tool, input, cwd, worktree_root, scope, role, test_command, home}
  → classify()            → Verdict {tier, reason}
  → Limits::route()       → Allow | Chief | User {why}
  → chief calls approval_decide(id, allow|deny, reason)
  → Limits::apply_chief_decision()  → applied, or refused and sent to you
  → AuditRecord           → report section "Approved on your behalf (N)"
```

Unanswered requests are refused after 5 minutes (`timeout::UNANSWERED_AFTER`), and the worker is told to carry on without that step (`timeout::UNANSWERED_MESSAGE`). How the daemon fills in the request is under [Daemon wiring](#daemon-wiring).

## Tiers

| Tier | Who answers | What lands here |
|---|---|---|
| `allowed` | Rust rules, straight away | reads and searches inside the worktree; edits inside the task's scope; the task's test command (also with extra arguments); `git status/diff/log/show/add/commit` and other read-only git; `rm -rf` of generated folders (`target`, `node_modules`, a top-level `dist`/`build`); shell builtins |
| `chief` (worktree) | the chief under Normal or Trusted | running scripts and builds in the worktree (`cargo build`, `npm run x`, `pytest`, `./scripts/x.sh`, inline `python -c`); edits inside the worktree but outside the task's scope; other recursive deletes in the worktree; local git changes (`checkout`, `reset`, `rebase`, `merge`, creating a branch or tag) |
| `chief` (network) | the chief under Trusted only | GET requests (`curl`, `wget`, `WebFetch`, `WebSearch`), `git fetch/pull/clone`, reading from GitHub with `gh` |
| `chief` (install) | the chief under Trusted only | installs into the worktree: `npm/pnpm/yarn/bun install/add`, `npx`, `cargo add/update`, `uv add/sync`, `pip install` from a worktree virtualenv, `go get`, `bundle install`, Gradle and Maven builds |
| `ask_user` | you | anything cos can't classify confidently: unknown programs and git aliases, commands it can't parse, paths only known at run time, `pip install` that might be global, the stash, interactive tools, containers |
| `never_delegated` | you, whatever the preset | see below |

jev, when it arrives, will sit in front of the chief for the worktree-only chief tier. It never sees network, install or never-delegated requests.

### Never delegated

- `git push` in any form, through any wrapper (`bash -c`, `eval`, `env -S`, `find -exec`, `xargs`, a heredoc fed to a shell, a command substitution).
- Any path outside the worktree, for reading or writing, after `~`, `$HOME`, `..`, `cd`, `git -C`, `make -C`, `--manifest-path` and the like are resolved. `/dev/null` and friends are fine.
- Secrets anywhere, even inside the worktree: `.env` and `.env.*` (templates like `.env.example` may be read), `.envrc`, `*.pem`, `*.key`, `*.p12`, `id_rsa*`/`id_ed25519*`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `credentials(.json)`, `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.docker`, `~/.kube`, `~/.config/gh`, keychains. Also expanding a variable whose name looks like a credential (`$GITHUB_TOKEN`, `$API_KEY`), and `env`/`printenv` with no arguments.
- Changes to settings, permissions or agent instructions inside the worktree: `.claude/`, `.git/`, `.codex/`, `.cos/` and any `.env*` name, at any depth and in any letter case (the same list as `cos_core::taskgit::is_protected`, which rejects a task branch that touches them), plus `.mcp.json`, `CLAUDE.md`, `CLAUDE.local.md` and `AGENTS.md`. Reading them is fine.
- Sending data out: `curl -X POST/PUT/DELETE`, `-d`, `--data*`, `-F`, `-T`, `--json`; `wget --post-*`; HTTPie with fields; `gh` commands that create, comment, merge or delete; `gh api` with fields or a non-GET method; `ssh`, `scp`, `rsync` to a host.
- Installing or changing things outside the worktree: `cargo install`, `npm -g`, `yarn global`, `pip --user`, `go install`, `brew`, `apt`, `rustup`/`nvm`/`pyenv` changes, `uv tool install`, `pre-commit install`, `husky`.
- Shared repository state: `git branch -d/-m/-f`, `tag -d`, `stash drop/clear`, `config` writes, `remote add/set-url`, `worktree add/remove`, `gc`, `update-ref`, `filter-branch`.
- `sudo`, `su`, `launchctl`, `crontab`, `systemctl`, `defaults write`, `security`, `pkill`, `killall`, cloud CLIs (`aws`, `gcloud`, `kubectl`, `terraform`, …), symlinks pointing outside the worktree, `rm -rf` of the worktree itself, of `*` at its root, or of a path only known at run time.

### Decisions where the proposal was open

- **`CLAUDE.md` and `AGENTS.md` are never delegated to write.** They don't grant permissions, but every agent in the workspace (the chief and the reviewer included) loads them, so a rewrite is a lasting prompt injection that outlives the worker.
- **Installs and network are Trusted-only.** The proposal lists them as "approved for you" and also says Trusted adds network and installs. Installs run third-party code and GET query strings can carry data out, so Normal only lets the chief approve actions contained in the worktree.
- **Global installs are never delegated**; installs into the worktree are chief (install). `brew` is never delegated: it changes your Mac, not the worktree. Plain `pip install` asks you because cos can't tell whether a virtualenv is active; `.venv/bin/pip` counts as the worktree.
- **Reads outside the worktree are never delegated**, like writes, even `~/.cargo/registry`. Fewer surprises; agents can usually find the same thing inside the worktree.
- **Arbitrary code in the worktree is chief tier.** `python -c`, `npm run x`, `make` and scripts can do anything their code says. The chief sees the command; Rust can't see inside the script. This is the main trust gap, and why the worktree is throwaway and nothing merges before review.
- **The chief can't approve its own requests**, and requests from a session you started go to you.
- **A reviewer's edits ask you**: reviewers have no reason to edit.
- **cos's own MCP tools** (`mcp__cos__*`) are allowed for the chief and ask you for anyone else. Other MCP tools ask you.
- **The task scope** uses the same glob rules as `cos_core::taskgit::Scope` (`*`, `?`, `**`, `[` literal, anchored at the worktree root, a folder pattern covers everything in it).

## Bash

Commands are parsed by a small shell parser (`crates/cos-policy/src/shell.rs`), not split on spaces:

- Lists and pipelines (`&&`, `||`, `;`, `|`, `&`, newlines), subshells (`(…)` keep their `cd` to themselves), `if/while/for` bodies, `[[ … ]]`.
- Quotes and escapes, so `g\it push` and `"git" 'push'` are still `git push`.
- Command substitutions `$(…)`, backticks and process substitutions `<(…)`, classified recursively, as are `bash -c`, `sh -c`, `eval`, `env -S`, a heredoc fed to a shell, `find -exec` and `xargs` commands. Heredoc bodies with an unquoted delimiter are scanned for substitutions.
- Redirections: `>`, `>>`, `&>`, `<>` write their target, `<` reads it, `2>&1` and `>&2` are ignored.
- Variables: `NAME=value` and `for NAME in …` values are tracked; `$HOME` and `$PWD` are known; anything else makes the word unknown. Setting `PATH`, `LD_*`, `GIT_DIR`, `NODE_OPTIONS` and similar asks you.
- `cd`, `pushd`, `popd` move the directory later commands resolve paths against; the starting directory is the request's `cwd` (Claude's shell keeps its directory between commands). `git -C`, `make -C`, `cargo -C`, `npm --prefix` and `env -C` apply to that command only.
- `xargs` arguments are known only when the previous stage listed files under a folder (`find`, `git ls-files`, `grep -l`, `rg -l`, `ls`, `fd`); otherwise they're unknown.

Each simple command is classified by program, and the whole command gets the strictest tier of its parts. Anything the parser can't follow confidently (`case`, function definitions, arrays, `$'\x..'` escapes, unbalanced quotes, nesting deeper than 6) asks you.

Programs are recognized by name; one run by path from `/bin`, `/usr/bin`, `/usr/local/bin` or `/opt/homebrew/bin` is treated as that name. A program inside the worktree is a script (chief tier), except interpreters and tool shims like `.venv/bin/python` or `node_modules/.bin/jest`, which are treated by name. Any other program asks you. For programs cos doesn't model argument by argument, every argument that looks like a path must stay inside the worktree.

## Symlinks

`classify` checks paths lexically. `classify_with(request, &Filesystem)` also follows symlinks on disk: a path inside the worktree that resolves outside it is outside. The daemon should use `Filesystem`, since worktrees are on disk. Creating a symlink to anything outside the worktree is never delegated either way.

## Limits

| | Strict | Normal (default) | Trusted |
|---|---|---|---|
| chief (worktree) | you | chief | chief |
| chief (network) | you | you | chief |
| chief (install) | you | you | chief |

- **Cap:** `Limits::chief_cap`, 20 by default. Once `RunLedger::chief_approvals` reaches it, everything goes to you.
- **Checking the chief:** `apply_chief_decision` always accepts a deny. It accepts an allow only if the request would route to the chief at that moment (so a preset change or a cap reached in the meantime still holds), and only with a non-empty reason. An accepted allow counts against the cap.

## Audit

Every answer becomes an `AuditRecord` {request id, time, role, tool, summary, tier, rule, decided by, outcome, reason}. `render_report_section` lists the requests the chief (or later jev) allowed, with the chief's reason, and a line counting what the rules allowed, what the chief refused, what you answered, and what went unanswered:

```markdown
## Approved on your behalf (2)

1. `npm install zod` — Bash, approved by the chief (install) at 14:03. Reason: the task adds zod for validation
2. `cargo build --release` — Bash, approved by the chief (worktree) at 14:10. Reason: checking the release build

Rust rules allowed 41 routine requests, the chief refused 1.
```

## Daemon wiring

The daemon calls the policy for every permission request of an agent it runs; how it does that (routes, events, storage) is in "Approvals policy" in [daemon.md](daemon.md#approvals-policy). What it fills in:

| `ApprovalRequest` field | Worker / reviewer | Chief | Your own agent |
|---|---|---|---|
| `worktree_root` | the task's worktree (the workspace if the task isn't known) | the workspace | the workspace |
| `scope` | the task's `files` (nothing if the task isn't known) | nothing | everything |
| `test_command` | the task's | none | none |
| `role` | `worker` / `reviewer` | `chief` | `user-session` |
| `cwd`, `home` | the hook's `cwd`, `$HOME` | same | same |

- **Which task:** the daemon asks a `TaskLookup` (`daemon::agents::approvals`) which task a worker or reviewer works on, and only trusts a task of the agent's own run. Once agents record their task (`agents.task_id`), the lookup reads that.
- **Nothing in scope for the chief:** it explores read-only, so its edits are chief tier, and the chief can't approve its own requests, so they come to you.
- **Your own agents** are classified only so the apps can show the tier and reason; they always ask you.
- **Who answered** is stored on the approval (`decided_by`: `rule`, `chief`, `user`, `terminal`, or none for `expired`), and in the run's audit trail for the report.

## Known gaps

These go to you, or rely on the chief and review rather than on rules:

- Scripts, package scripts, `make` targets and inline interpreter code can do anything; they're chief tier because the worktree is throwaway, not because cos checked them.
- A recursive search of the worktree (`grep -r`, `Grep`) can read an untracked `.env` inside it. cos worktrees are fresh checkouts, so untracked secrets aren't normally there.
- Loop variables over command output (`for f in $(git ls-files)`) and `cd "$(…)"` make paths unknown, so those commands ask you.
- Without the `Filesystem` probe, symlinks already inside the worktree aren't followed.
- Git aliases, cargo plugins and unknown programs ask you.
- `POST /v1/approvals/{id}/chief` doesn't check that the caller is the run's chief: anyone who can reach the API can answer as the chief, within the limits. That's no more than they could do with `POST /v1/approvals/{id}`.
