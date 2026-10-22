# Releasing cos

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed. Asset names carry no version, so `https://github.com/raykibul/cos/releases/latest/download/cos-<target>.tar.gz` always points at the newest build. `install.sh` and the `cos-agent` npm package both depend on that.

## Steps

1. **Bump the version.** Set `version` under `[workspace.package]` in `Cargo.toml`, run `make test` so `Cargo.lock` picks it up, and commit. You don't need to change `npm/package.json`. The workflow sets the package's version from the tag.
2. **Tag and push.**

   ```sh
   git tag -a v0.2.0 -m "cos v0.2.0"
   git push origin main v0.2.0
   ```

3. **Watch the run** with `gh run watch`, or in the repo's Actions tab.

To rebuild a release for an existing tag, after fixing the workflow for example, run it by hand: `gh workflow run release.yml -f tag=v0.2.0`. The assets are replaced (`--clobber`).

## What the workflow does

1. **version:** fails unless the tag is `v` plus the workspace version in `Cargo.toml`, so `cos --version` always matches the tag.
2. **linux:** `make linux`, the same Docker and cargo-zigbuild build as on a dev machine, builds static musl binaries for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, and checks that each is statically linked. The job then runs the x86_64 build's `--version` and packs both with `scripts/package-release.sh`.
3. **macos:** on `macos-latest` (Apple Silicon), `cargo build --release` for `aarch64-apple-darwin` and, by cross-compiling, `x86_64-apple-darwin`. The minimum macOS is 11. The binaries aren't signed or notarized, except for the ad-hoc signature the linker adds on arm64. Downloads with `curl` or Node get no quarantine flag, so they run without a Gatekeeper prompt.
4. **release:** writes `SHA256SUMS` and creates the GitHub Release with generated notes, or uploads over an existing one. It needs only `GITHUB_TOKEN` with `contents: write`.
5. **npm:** publishes `npm/` as `cos-agent@<version>`, but only if the repo has an `NPM_TOKEN` secret, and skips a version that's already on npm.

Each release has these assets:

| Asset | |
|---|---|
| `cos-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64, static |
| `cos-aarch64-unknown-linux-musl.tar.gz` | Linux arm64 (Graviton), static |
| `cos-aarch64-apple-darwin.tar.gz` | macOS, Apple Silicon |
| `cos-x86_64-apple-darwin.tar.gz` | macOS, Intel |
| `SHA256SUMS` | `sha256sum` output for the four tarballs |

Each tarball holds `cos`, plus `README.md` and `LICENSE` when the repo has them.

## npm

The `cos-agent` package (`npm/`) is a small installer with no dependencies. `npx cos-agent@latest` installs or upgrades `cos` in `~/.local/bin`, and `npx cos-agent <args>` runs `cos <args>`, installing it first if needed. It never runs cos from npx's cache, because `cos daemon install` records the binary's path.

To publish it, set it up once:

1. Create an npm account, and an automation access token that can publish.
2. Add the token as the repo secret `NPM_TOKEN`: `gh secret set NPM_TOKEN`.

The next tagged release then publishes it. The package's tests run in CI and with `make npm-test`.

## Checking before a release

```sh
make lint test          # what CI runs
make linux linux-smoke  # static binaries, run on Amazon Linux 2023 and Debian, both arches
make install-test       # install.sh against a fake release made from dist/
make npm-test shellcheck actionlint
```
