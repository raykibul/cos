# cos-agent

Installs and runs [cos](https://github.com/raykibul/cos), a chief of staff for coding agents, from its GitHub releases. It has no dependencies.

```sh
npx cos-agent@latest           # install or upgrade cos in ~/.local/bin
npx cos-agent daemon install   # run `cos daemon install`, installing cos first if needed
npx cos-agent@latest app       # install or upgrade the Mac app (add --with-cli for both)
```

`app` installs `cos.app` in `/Applications`, or `~/Applications` when `/Applications` isn't writable, after quitting a running copy. The app is ad-hoc signed, not notarized, so it clears macOS's quarantine flag; see [releasing.md](https://github.com/raykibul/cos/blob/main/docs/releasing.md#the-mac-app).

- **Platforms:** Linux (x86_64 or arm64; static builds) and macOS (Apple Silicon or Intel).
- **Checksums:** every download is checked against the release's `SHA256SUMS`.
- **Install path:** cos always runs from the install folder, never from npx's cache, because `cos daemon install` records the binary's path.

Settings:

- `COS_VERSION=v0.1.0` installs that release instead of the latest.
- `COS_INSTALL_DIR` picks another folder.
- `COS_DOWNLOAD_BASE` fetches releases from somewhere other than GitHub.
- `COS_APP_DIR` puts the app somewhere other than `/Applications`, and `COS_APP_NO_QUIT=1` leaves a running app alone.
