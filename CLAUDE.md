# cos

Rust chief of staff for coding agents. Read `docs/proposal.md` for the design and milestone list. Its Progress section says what's done. `docs/daemon.md` covers the daemon, the event log and the API. `docs/claude-protocol.md` covers how cos drives Claude Code. `docs/approvals.md` covers who may answer a permission request (the `cos-policy` crate). `docs/judge.md` covers jev, the fast judgment layer (`cos-judge`), in shadow mode.

## Commands

- Tests and lints run in Docker: `make test`, `make lint`, `make fmt`. They use the `cos-dev:1.99` image built from `docker/dev.Dockerfile`, with Linux build output in `target/docker`.
- The `cos` binary runs natively, because it drives the local `claude`: `make build`, then `./target/release/cos try --cwd .sandbox --model haiku "..."`. A live run costs real tokens, so use haiku.
- Static Linux binaries (x86_64 and aarch64 musl) build in Docker with `make linux`, into `dist/`. `make linux-smoke` runs them on stock distro images. `make install-test` tests `install.sh` against a fake release made from `dist/`. `make npm-test`, `make shellcheck` and `make actionlint` cover the npm installer (`npm/`), the shell scripts and the workflows. Releases come from `v*` tags: see `docs/releasing.md`. For a Linux server (EC2), see `docs/linux.md`.
- `cos try --pty --trust --model haiku "..."` runs Claude's interactive UI in a PTY and prints the hooks it fires (add `--screen` to see the terminal). Run it from `.sandbox/` or a worktree of a trusted repo. `cos sessions` lists the Claude sessions running on the Mac (read-only).
- To run the daemon live without touching `~/.cos`, set `COS_HOME=$PWD/.sandbox/home`. `cos daemon install` writes `~/Library/LaunchAgents/local.cos.daemon.plist`, so run `cos daemon uninstall` after testing it.
- The Mac app is a Tauri 2 app in `clients/desktop` (contract: `docs/desktop.md`). Its web UI builds and tests in Docker (`make ui-*`). Its daemon bridge, `crates/cos-bridge`, is in the repo workspace and tested by `make test`. The Tauri shell, `clients/desktop/src-tauri`, is its own Cargo workspace and builds natively: `make desktop` bundles `cos.app`; see `clients/desktop/README.md`.
- `clients/shared/CosKit` (Swift, for the iPhone app) builds natively, not in Docker: `swift test` there.
- The iPhone app, `clients/iphone` (contract: `docs/iphone.md`), builds natively too: `xcodegen generate` there, then `xcodebuild -project Cos.xcodeproj -scheme Cos -destination 'platform=iOS Simulator,name=iPhone 17' test`. Launch it with `-fakeData` (and `-screen <name>`) for invented data; see `clients/iphone/README.md`.
- Use `.sandbox/` (gitignored) as the working directory for live runs. Claude refuses writes under `~/.claude/`, so a scratch folder there won't work.

## Conventions

- `cos-harness` parses Claude's output leniently: unknown messages become `Event::Other`, never errors. The protocol is undocumented.
- Agents run in a PTY (`ClaudeTerminal`) and cos follows them through per-session hooks (`claude/hooks.rs`), never by reading the screen. The exceptions are detecting the trust dialog and the login screen (`needs_login`, when a `CLAUDE_CONFIG_DIR` in a project's environment isn't logged in).
- Integration tests run against `cos-fake-claude` (`crates/cos-harness/src/bin/cos-fake-claude/`). Without `-p` it plays the interactive UI and runs hooks. When a test needs new Claude behaviour, extend the fake instead of calling the real CLI.
- Update the Progress section of `docs/proposal.md` when a milestone lands.
