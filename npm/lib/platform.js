// Which release asset fits this machine, and where things go.

import { homedir } from "node:os";
import { join } from "node:path";

export const DEFAULT_BASE = "https://github.com/raykibul/cos/releases";

/** The Rust target cos is built for on `platform` (process.platform) and `arch` (process.arch). */
export function targetFor(platform, arch) {
  const cpu = { x64: "x86_64", arm64: "aarch64" }[arch];
  const os = { linux: "unknown-linux-musl", darwin: "apple-darwin" }[platform];
  if (!os) {
    throw new Error(`no cos build for ${platform}; cos runs on Linux and macOS`);
  }
  if (!cpu) {
    throw new Error(`no cos build for the ${arch} CPU; build it from source (see the README)`);
  }
  return `${cpu}-${os}`;
}

/** The release asset for `target`. Names carry no version. */
export function assetName(target) {
  return `cos-${target}.tar.gz`;
}

/** The folder release files are fetched from: the latest release, or tag `version`. */
export function releaseUrl(base, version) {
  const root = base.replace(/\/+$/, "");
  if (!version || version === "latest") {
    return `${root}/latest/download`;
  }
  return `${root}/download/${version.startsWith("v") ? version : `v${version}`}`;
}

/**
 * Where `cos` is installed: `$COS_INSTALL_DIR`, else `~/.local/bin`. Always a
 * stable folder, never npx's cache, because `cos daemon install` points
 * launchd or systemd at the binary's path.
 */
export function installDir(env = process.env) {
  return env.COS_INSTALL_DIR || join(env.HOME || homedir(), ".local", "bin");
}
