// `npx cos-agent app`: installs or upgrades the cos Mac app, the same steps
// as `install.sh --app`. It downloads the release's universal cos.app zip,
// checks it against SHA256SUMS, quits a running copy by its pid, replaces
// the app's Contents in /Applications (or ~/Applications) and clears the
// quarantine flag, since the app is ad-hoc signed and not notarized.
//
// The macOS tools it uses (ditto, xattr, plutil, sw_vers, pgrep) and
// signals go through `tools`, so the tests can run anywhere.

import { execFileSync } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

import { fetchVerified } from "./install.js";
import { DEFAULT_BASE, releaseUrl } from "./platform.js";

/** The app's release asset: one universal build, so no CPU in the name. */
export const APP_ASSET = "cos-desktop-universal-apple-darwin.zip";
/** The app's process name, as pgrep sees it. */
export const APP_PROCESS = "cos-desktop";

const SUDO_HINT = "curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sudo sh -s -- --app";

/** The real tools, on macOS. */
export const macTools = {
  run: (command, args) => execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }),
  pids(name, uid) {
    try {
      return parsePids(execFileSync("pgrep", ["-x", "-U", String(uid), name], { encoding: "utf8" }));
    } catch (error) {
      if (error.status === 1) return []; // none running
      throw error;
    }
  },
  kill: (pid) => process.kill(pid, "SIGTERM"),
  alive(pid) {
    try {
      process.kill(pid, 0);
      return true;
    } catch (error) {
      return error.code === "EPERM";
    }
  },
};

/** pgrep's output as pids. */
export function parsePids(output) {
  return output
    .split(/\s+/)
    .filter((word) => /^\d+$/.test(word))
    .map(Number);
}

/** Where the app may go: `$COS_APP_DIR` or /Applications first, then ~/Applications. */
export function appDirs(env = process.env) {
  return {
    system: env.COS_APP_DIR || "/Applications",
    user: join(env.HOME || homedir(), "Applications"),
  };
}

function writable(path) {
  try {
    accessSync(path, constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether this user can put cos.app in `dir`. An existing cos.app is
 * replaced from the inside (its Contents), so only the app folder needs to
 * be writable: a user-owned /Applications/cos.app upgrades without sudo.
 */
export function appWritable(dir) {
  const app = join(dir, "cos.app");
  if (existsSync(app)) {
    const contents = join(app, "Contents");
    return writable(app) && (!existsSync(contents) || writable(contents));
  }
  return existsSync(dir) && writable(dir);
}

/**
 * Picks the folder for cos.app, as install.sh does: a copy in
 * ~/Applications (with none in the system folder) is upgraded where it is;
 * else the system folder if writable; else ~/Applications, with notes.
 */
export function pickAppDir({ system, user }) {
  if (!existsSync(join(system, "cos.app")) && existsSync(join(user, "cos.app"))) {
    return { dir: user, notes: [] };
  }
  if (appWritable(system)) return { dir: system, notes: [] };
  const notes = existsSync(join(system, "cos.app"))
    ? [
        `${system}/cos.app isn't writable by you, so installing in ${user} instead.`,
        `the old copy in ${system} stays, and Spotlight may still open it. To replace that one instead, run:`,
      ]
    : [`${system} isn't writable by you, so installing in ${user} instead. To install in ${system}, run:`];
  notes.push(`  ${SUDO_HINT}`);
  return { dir: user, notes };
}

/** Quits running copies of the app by pid and waits for them; returns whether one ran. */
export async function quitApp({ env, tools, uid, log, timeoutMs = 10_000 }) {
  if (env.COS_APP_NO_QUIT === "1") return false;
  const pids = tools.pids(env.COS_APP_PROCESS || APP_PROCESS, uid);
  for (const pid of pids) {
    log(`quitting the running cos app (pid ${pid})`);
    try {
      tools.kill(pid);
    } catch {
      // Gone already.
    }
  }
  const deadline = Date.now() + timeoutMs;
  for (const pid of pids) {
    while (tools.alive(pid)) {
      if (Date.now() > deadline) {
        throw new Error(`the cos app (pid ${pid}) didn't quit; quit it from its menu bar icon and run this again`);
      }
      await sleep(200);
    }
  }
  return pids.length > 0;
}

/**
 * Installs the app and returns its path. `env` takes COS_VERSION,
 * COS_APP_DIR, COS_APP_NO_QUIT and COS_DOWNLOAD_BASE, as install.sh does.
 */
export async function installApp({
  env = process.env,
  platform = process.platform,
  tools = macTools,
  uid = process.getuid?.(),
  log = (message) => console.error(`cos-agent: ${message}`),
} = {}) {
  if (platform !== "darwin") {
    throw new Error(
      `the cos app is for macOS. On ${platform}, install the CLI (npx cos-agent) and use the app on a Mac, over Tailscale`,
    );
  }
  const major = Number.parseInt(tools.run("sw_vers", ["-productVersion"]).trim(), 10);
  if (!(major >= 13)) throw new Error("the cos app needs macOS 13 or later");

  const version = env.COS_VERSION || "latest";
  const url = releaseUrl(env.COS_DOWNLOAD_BASE || DEFAULT_BASE, version);
  log(`downloading ${APP_ASSET} (${version})`);
  const zip = await fetchVerified(url, APP_ASSET);

  const tmp = await mkdtemp(join(tmpdir(), "cos-app-"));
  try {
    const zipPath = join(tmp, APP_ASSET);
    await writeFile(zipPath, zip);
    tools.run("ditto", ["-x", "-k", zipPath, join(tmp, "unpacked")]);
    const source = join(tmp, "unpacked", "cos.app");
    if (!existsSync(join(source, "Contents"))) throw new Error(`${APP_ASSET} has no cos.app`);

    const { dir, notes } = pickAppDir(appDirs(env));
    for (const note of notes) log(note);
    await mkdir(dir, { recursive: true });
    const app = join(dir, "cos.app");
    const wasRunning = await quitApp({ env, tools, uid, log });

    // Replace what's inside the app folder, not the folder: it may be ours
    // inside an /Applications we can't write to.
    try {
      await rm(join(app, "Contents"), { recursive: true, force: true });
    } catch (error) {
      throw new Error(`could not remove the old ${app}/Contents (${error.code}); run the installer with sudo`);
    }
    tools.run("ditto", [source, app]);
    // Unsigned (ad hoc) and not notarized: with the quarantine flag that
    // browsers set, Gatekeeper would refuse to open it.
    try {
      tools.run("xattr", ["-dr", "com.apple.quarantine", app]);
    } catch {
      // No flag to clear.
    }

    let installed = "?";
    try {
      installed = tools
        .run("plutil", ["-extract", "CFBundleShortVersionString", "raw", "-o", "-", join(app, "Contents", "Info.plist")])
        .trim();
    } catch {
      // An app without a version still installed.
    }
    log(`installed the cos app ${installed} at ${app}`);
    log(wasRunning ? `it was running; open it again with: open '${app}'` : `open it with: open '${app}'`);
    return app;
  } finally {
    await rm(tmp, { recursive: true, force: true });
  }
}
