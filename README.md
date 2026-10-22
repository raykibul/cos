# cos

A chief of staff for coding agents. A Rust daemon runs Claude Code agents in terminals in git worktrees, plans runs with a chief, checks and merges their work, and asks you only what it must. The desktop app drives it, locally or over Tailscale.

## Install

On Linux (x86_64 or arm64) or macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh
```

It picks the build for your OS and CPU from the latest [release](https://github.com/raykibul/cos/releases), checks it against `SHA256SUMS` and installs `cos` in `~/.local/bin`. Run it again to upgrade. Optional settings:

- `COS_VERSION=v0.1.0` installs that release instead of the latest.
- `COS_INSTALL_DIR=…` picks another folder.

With Node 18 or later, you can install through npm instead:

```sh
npx cos-agent@latest           # install or upgrade cos in ~/.local/bin
npx cos-agent status           # run cos, installing it first if needed
```

To build from source, you need Docker for the Linux builds:

```sh
make build    # native release build: target/release/cos
make linux    # static Linux binaries for x86_64 and aarch64: dist/
```

Then start the daemon with `cos daemon`, or run `cos daemon install` to make it a service (launchd on macOS, systemd on Linux).

## Docs

- [docs/daemon.md](docs/daemon.md): the daemon, its API and remote access.
- [docs/linux.md](docs/linux.md): running the daemon on a Linux server such as EC2 and using it from the Mac app over Tailscale.
- [docs/releasing.md](docs/releasing.md): cutting a release.
- [docs/proposal.md](docs/proposal.md): the design and progress.
