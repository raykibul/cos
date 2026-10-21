# cos

A chief of staff for coding agents. You give it a goal; it plans the work, gives each task to a Claude Code worker in its own git worktree, checks and reviews the results, merges them and writes you a report. A Rust daemon does the bookkeeping, and models only make the judgment calls.

The design is in [docs/proposal.md](docs/proposal.md). Status: phase 1, milestones 1 and 2 of 7 are done.

## Try it

`cos try` runs one prompt through Claude Code and asks you in the terminal before Claude runs anything that needs approval.

```sh
make build
mkdir -p .sandbox
./target/release/cos try --cwd .sandbox --model haiku "create hello.txt containing hi"
```

```
● claude-haiku-5-5 · default mode · …/cos/.sandbox
→ Bash echo hi > hello.txt
? permission: Bash echo hi > hello.txt
  allow? [y/N] y
  ✓ (Bash completed with no output)
■ success · $0.0020 · 5.1s
```

Options: `--approve allow|deny` answers without asking, `--mode acceptEdits|plan|dontAsk` changes Claude's permission mode, and `--raw` prints every event. Ctrl-C interrupts the turn, and a second Ctrl-C kills Claude.

## Develop

| Command | Runs | What |
|---|---|---|
| `make test` | Docker | All tests. They use `cos-fake-claude`, so they don't need a Claude login. |
| `make lint` | Docker | rustfmt check and clippy with warnings as errors |
| `make fmt` | Docker | Format the code |
| `make build` | Mac | Release build of `cos` |

`cos` itself has to run on the Mac, because it drives your local `claude`.

## Layout

```
crates/cos/          the binary: CLI now, daemon next
crates/cos-harness/  runs Claude Code, turns its output into events
crates/cos-core/     board, task states, storage, git (empty until milestone 3)
docs/                design and protocol notes
```
