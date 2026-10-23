# Approvals policy

How cos sorts an agent's permission request into a tier, with a reason. The code is the `cos-policy` crate. It is pure: no database, no clock, no daemon, so the daemon calls it and stores what it returns.

**You answer every request.** The tier and reason are shown with the request in the apps, so you can see at a glance whether it's routine. jev ([judge.md](judge.md)) also gets them, in shadow mode. Nothing is answered on your behalf, and nothing expires: a request waits until you answer it, in an app or in Claude's terminal.

## Flow

```
PermissionRequest hook
  → ApprovalRequest {tool, input, cwd, worktree_root, scope, role, test_command, home}
  → classify_with(&Filesystem)  → Verdict {tier, reason}
  → stored on the approval     → shown to you, who allow or deny
```

How the daemon fills in the request is under [Daemon wiring](#daemon-wiring).

## Tiers

| Tier | What it means | What lands here |
|---|---|---|
| `allowed` | routine, inside the workspace | reads and searches inside the worktree; edits inside the scope; the test command (also with extra arguments); `git status/diff/log/show/add/commit` and other read-only git; `rm -rf` of generated folders (`target`, `node_modules`, a top-level `dist`/`build`); shell builtins |
| `chief` (worktree) | contained in the workspace, but runs code or changes local git | running scripts and builds in the worktree (`cargo build`, `npm run x`, `pytest`, `./scripts/x.sh`, inline `python -c`); edits inside the worktree but outside the scope; other recursive deletes in the worktree; local git changes (`checkout`, `reset`, `rebase`, `merge`, creating a branch or tag) |
| `chief` (network) | reads from the network | GET requests (`curl`, `wget`, `WebFetch`, `WebSearch`), `git fetch/pull/clone`, reading from GitHub with `gh` |
| `chief` (install) | installs dependencies into the workspace | installs into the worktree: `npm/pnpm/yarn/bun install/add`, `npx`, `cargo add/update`, `uv add/sync`, `pip install` from a worktree virtualenv, `go get`, `bundle install`, Gradle and Maven builds |
| `ask_user` | cos can't tell | anything cos can't classify confidently: unknown programs and git aliases, commands it can't parse, paths only known at run time, `pip install` that might be global, the stash, interactive tools, containers |
| `never_delegated` | risky whoever asks | see below |

The `chief` tiers keep their name from when a planning agent could approve them. Nothing approves them now; they mark how far a request reaches.

jev ([judge.md](judge.md)) runs in shadow mode: it judges every request, in projects where it's switched on, and logs what it would have done without answering. It may only ever approve what this crate already puts in `allowed` or `chief (worktree)` (every path inside the worktree, no network, no install); that check is Rust, whatever jev answers.

### Never delegated

- `git push` in any form, through any wrapper (`bash -c`, `eval`, `env -S`, `find -exec`, `xargs`, a heredoc fed to a shell, a command substitution).
- Any path outside the worktree, for reading or writing, after `~`, `$HOME`, `..`, `cd`, `git -C`, `make -C`, `--manifest-path` and the like are resolved. `/dev/null` and friends are fine.
- Secrets anywhere, even inside the worktree: `.env` and `.env.*` (templates like `.env.example` may be read), `.envrc`, `*.pem`, `*.key`, `*.p12`, `id_rsa*`/`id_ed25519*`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `credentials(.json)`, `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.docker`, `~/.kube`, `~/.config/gh`, keychains. Also expanding a variable whose name looks like a credential (`$GITHUB_TOKEN`, `$API_KEY`), and `env`/`printenv` with no arguments.
- Changes to settings, permissions or agent instructions inside the worktree: `.claude/`, `.git/`, `.codex/`, `.cos/` and any `.env*` name, at any depth and in any letter case, plus `.mcp.json`, `CLAUDE.md`, `CLAUDE.local.md` and `AGENTS.md`. Reading them is fine.
- Sending data out: `curl -X POST/PUT/DELETE`, `-d`, `--data*`, `-F`, `-T`, `--json`; `wget --post-*`; HTTPie with fields; `gh` commands that create, comment, merge or delete; `gh api` with fields or a non-GET method; `ssh`, `scp`, `rsync` to a host.
- Installing or changing things outside the worktree: `cargo install`, `npm -g`, `yarn global`, `pip --user`, `go install`, `brew`, `apt`, `rustup`/`nvm`/`pyenv` changes, `uv tool install`, `pre-commit install`, `husky`.
- Shared repository state: `git branch -d/-m/-f`, `tag -d`, `stash drop/clear`, `config` writes, `remote add/set-url`, `worktree add/remove`, `gc`, `update-ref`, `filter-branch`.
- `sudo`, `su`, `launchctl`, `crontab`, `systemctl`, `defaults write`, `security`, `pkill`, `killall`, cloud CLIs (`aws`, `gcloud`, `kubectl`, `terraform`, …), symlinks pointing outside the worktree, `rm -rf` of the worktree itself, of `*` at its root, or of a path only known at run time.

### Decisions where the proposal was open

- **`CLAUDE.md` and `AGENTS.md` are never delegated to write.** They don't grant permissions, but every agent in the workspace loads them, so a rewrite is a lasting prompt injection that outlives the agent.
- **Installs and network are their own tiers.** Installs run third-party code and GET query strings can carry data out, so they are kept apart from actions contained in the worktree.
- **Global installs are never delegated**; installs into the worktree are chief (install). `brew` is never delegated: it changes your Mac, not the worktree. Plain `pip install` asks you because cos can't tell whether a virtualenv is active; `.venv/bin/pip` counts as the worktree.
- **Reads outside the worktree are never delegated**, like writes, even `~/.cargo/registry`. Fewer surprises; agents can usually find the same thing inside the worktree.
- **Arbitrary code in the worktree is chief tier.** `python -c`, `npm run x`, `make` and scripts can do anything their code says. Rust can't see inside the script. This is the main trust gap.
- **cos's own MCP tools** (`mcp__cos__*`) are pre-approved for the Chief of Staff with `--allowedTools`, so they never reach the policy. For anyone else they ask you, as do other MCP tools.
- **The scope** globs are `*`, `?`, `**`, `[` literal, anchored at the worktree root, and a folder pattern covers everything in it. The daemon passes no scope, so the whole worktree is in it.

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

## Daemon wiring

The daemon classifies every permission request of an agent it runs; the routes, events and storage are in "Approvals policy" in [daemon.md](daemon.md#approvals-policy). What it fills in, for every agent (yours, the Chief of Staff and the agents it starts):

| `ApprovalRequest` field | Value |
|---|---|
| `worktree_root` | the agent's workspace |
| `scope` | nothing, so the whole workspace |
| `test_command` | none |
| `role` | `user-session` |
| `cwd`, `home` | the hook's `cwd`, `$HOME` |

**Who answered** is stored on the approval (`decided_by`: `user` or `terminal`). Old databases may hold `expired` approvals from before nothing expired.

## Known gaps

What the tiers can't see:

- Scripts, package scripts, `make` targets and inline interpreter code can do anything; they're chief tier because the worktree is throwaway, not because cos checked them.
- A recursive search of the worktree (`grep -r`, `Grep`) can read an untracked `.env` inside it. cos worktrees are fresh checkouts, so untracked secrets aren't normally there.
- Loop variables over command output (`for f in $(git ls-files)`) and `cd "$(…)"` make paths unknown, so those commands ask you.
- Without the `Filesystem` probe, symlinks already inside the worktree aren't followed.
- Git aliases, cargo plugins and unknown programs ask you.
