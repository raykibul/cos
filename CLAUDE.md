# cos

Rust chief of staff for coding agents. Read `docs/proposal.md` for the design and milestone list. Its Progress section says what's done. `docs/daemon.md` covers the daemon, the event log and the API. `docs/claude-protocol.md` covers how cos drives Claude Code.

## Commands

- Tests and lints run in Docker: `make test`, `make lint`, `make fmt`. They use the `cos-dev:1.99` image built from `docker/dev.Dockerfile`, with Linux build output in `target/docker`.
- The `cos` binary runs natively, because it drives the local `claude`: `make build`, then `./target/release/cos try --cwd .sandbox --model haiku "..."`. A live run costs real tokens, so use haiku.
- To run the daemon live without touching `~/.cos`, set `COS_HOME=$PWD/.sandbox/home`. `cos daemon install` writes `~/Library/LaunchAgents/local.cos.daemon.plist`, so run `cos daemon uninstall` after testing it.
- Swift clients build natively, not in Docker: `swift test` in `clients/shared/CosKit`. For the Mac app, run `xcodegen generate` then `xcodebuild ... build|test` in `clients/desktop`; see its README. `Cos.xcodeproj` is generated, so edit `project.yml`.
- Use `.sandbox/` (gitignored) as the working directory for live runs. Claude refuses writes under `~/.claude/`, so a scratch folder there won't work.

## Conventions

- `cos-harness` parses Claude's output leniently: unknown messages become `Event::Other`, never errors. The protocol is undocumented.
- Integration tests run against `cos-fake-claude` (`crates/cos-harness/src/bin/`). When a test needs new Claude behaviour, extend the fake instead of calling the real CLI.
- Update the Progress section of `docs/proposal.md` when a milestone lands.
