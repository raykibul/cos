// Downloads the cos release for this machine, checks it against the
// release's SHA256SUMS and installs `cos` in a stable folder. The same steps
// as install.sh, in Node with no dependencies.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { chmod, mkdir, rename, rm, writeFile } from "node:fs/promises";
import { delimiter, join } from "node:path";
import { gunzipSync } from "node:zlib";

import { DEFAULT_BASE, assetName, installDir, releaseUrl, targetFor } from "./platform.js";

const log = (message) => console.error(`cos-agent: ${message}`);

/** process.arch, except arm64 for an x64 Node running under Rosetta on an Apple Silicon Mac. */
export function nativeArch(platform = process.platform, arch = process.arch) {
  if (platform === "darwin" && arch === "x64") {
    try {
      const translated = execFileSync("sysctl", ["-n", "sysctl.proc_translated"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      });
      if (translated.trim() === "1") return "arm64";
    } catch {
      // Not a Mac that knows about Rosetta.
    }
  }
  return arch;
}

/** The SHA-256 SHA256SUMS lists for `name`, or undefined. */
export function expectedSum(sums, name) {
  for (const line of sums.split("\n")) {
    const [hash, file] = line.trim().split(/\s+/);
    if (file === name || file === `*${name}`) return hash;
  }
  return undefined;
}

/** The contents of the regular file `name` in an uncompressed tar archive, or undefined. */
export function extractFile(tar, name) {
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const field = (start, length) =>
      header.subarray(start, start + length).toString("utf8").replace(/\0.*$/s, "");
    const prefix = field(345, 155);
    const path = (prefix ? `${prefix}/` : "") + field(0, 100);
    const size = Number.parseInt(field(124, 12).trim() || "0", 8);
    const type = field(156, 1);
    const body = offset + 512;
    if ((type === "0" || type === "") && path.replace(/^\.\//, "") === name) {
      return tar.subarray(body, body + size);
    }
    offset = body + Math.ceil(size / 512) * 512;
  }
  return undefined;
}

async function fetchBytes(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`could not download ${url}: HTTP ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

/**
 * Installs cos and returns the binary's path. `env` takes COS_VERSION,
 * COS_INSTALL_DIR and COS_DOWNLOAD_BASE, as install.sh does.
 */
export async function install({ env = process.env, platform = process.platform, arch = nativeArch() } = {}) {
  const target = targetFor(platform, arch);
  const asset = assetName(target);
  const version = env.COS_VERSION || "latest";
  const url = releaseUrl(env.COS_DOWNLOAD_BASE || DEFAULT_BASE, version);

  log(`downloading ${asset} (${version})`);
  const [archive, sums] = await Promise.all([
    fetchBytes(`${url}/${asset}`),
    fetchBytes(`${url}/SHA256SUMS`),
  ]);
  const expected = expectedSum(sums.toString("utf8"), asset);
  if (!expected) throw new Error(`SHA256SUMS has no entry for ${asset}`);
  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== expected) {
    throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
  }

  const binary = extractFile(gunzipSync(archive), "cos");
  if (!binary) throw new Error(`${asset} has no cos binary`);

  const dir = installDir(env);
  const dest = join(dir, "cos");
  // Written next to the target and renamed over it, so a running cos keeps
  // its old file and nobody sees a half-written one.
  const staged = join(dir, `.cos.new.${process.pid}`);
  await mkdir(dir, { recursive: true });
  try {
    await writeFile(staged, binary);
    await chmod(staged, 0o755);
    await rename(staged, dest);
  } finally {
    await rm(staged, { force: true });
  }

  const installed = execFileSync(dest, ["--version"], { encoding: "utf8" }).trim();
  log(`installed ${installed} at ${dest}`);
  if (!(env.PATH || "").split(delimiter).includes(dir)) {
    const shell = env.SHELL || "";
    const rc = shell.endsWith("/zsh") ? "$HOME/.zshrc" : shell.endsWith("/bash") ? "$HOME/.bashrc" : "$HOME/.profile";
    log(`${dir} is not on your PATH. Add it, for example:`);
    log(`  echo 'export PATH="${dir}:$PATH"' >> ${rc} && . ${rc}`);
  }
  serviceHint(dest, env, platform);
  return dest;
}

/** Says how to restart the daemon's service on the new binary, if `cos daemon install` set one up. */
function serviceHint(dest, env, platform) {
  const home = env.HOME || "";
  let unit;
  let restart;
  if (platform === "linux") {
    unit = join(env.XDG_CONFIG_HOME || join(home, ".config"), "systemd", "user", "cos.service");
    restart = "systemctl --user restart cos.service";
  } else if (platform === "darwin") {
    unit = join(home, "Library", "LaunchAgents", "local.cos.daemon.plist");
    restart = `launchctl kickstart -k gui/${process.getuid?.() ?? "$(id -u)"}/local.cos.daemon`;
  } else {
    return;
  }
  if (!existsSync(unit)) return;
  if (readFileSync(unit, "utf8").includes(dest)) {
    log("the cos daemon service still runs the old version until it restarts. Restart it (this interrupts running agents; they can be resumed):");
    log(`  ${restart}`);
  } else {
    log(`a cos daemon service is installed (${unit}) but runs another cos binary.`);
    log(`  to run this one instead: ${dest} daemon install`);
  }
}
