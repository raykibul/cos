// Installs the Mac app from a fake release served on localhost. The macOS
// tools are faked: the "zip" is a version string, `ditto -x -k` turns it
// into a cos.app whose Info.plist holds it, and `ditto` copies a folder.
// So this runs anywhere, Docker included; `make app-install-test` runs the
// same cases through install.sh with the real tools on a Mac.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";

import { APP_ASSET, installApp, parsePids, pickAppDir } from "../lib/app.js";

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const files = new Map();
let server;
let base;

function publish(tag, version, { corrupt = false } = {}) {
  const zip = Buffer.from(`app ${version}`);
  files.set(`/${tag}/${APP_ASSET}`, zip);
  files.set(`/${tag}/SHA256SUMS`, Buffer.from(`${corrupt ? "0".repeat(64) : sha256(zip)}  ${APP_ASSET}\n`));
}

before(async () => {
  publish("latest/download", "0.2.0");
  publish("download/v0.1.0", "0.1.0");
  publish("download/v0.0.9", "0.0.9", { corrupt: true });
  server = createServer((request, response) => {
    const body = files.get(request.url);
    response.writeHead(body ? 200 : 404);
    response.end(body);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

/** Fake macOS tools; `running` is the pids pgrep finds. */
function fakeTools({ running = [], macos = "15.1" } = {}) {
  const calls = [];
  const alive = new Set(running);
  return {
    calls,
    run(command, args) {
      calls.push([command, ...args]);
      if (command === "sw_vers") return `${macos}\n`;
      if (command === "ditto" && args[0] === "-x") {
        const [, , zip, dest] = args;
        const contents = join(dest, "cos.app", "Contents");
        mkdirSync(contents, { recursive: true });
        writeFileSync(join(contents, "Info.plist"), readFileSync(zip, "utf8").replace(/^app /, ""));
        writeFileSync(join(contents, `from-${readFileSync(zip, "utf8").replace(/^app /, "")}`), "");
        return "";
      }
      if (command === "ditto") {
        cpSync(args[0], args[1], { recursive: true });
        return "";
      }
      if (command === "xattr") return "";
      if (command === "plutil") return `${readFileSync(args.at(-1), "utf8")}\n`;
      throw new Error(`unexpected ${command}`);
    },
    pids: (name) => {
      calls.push(["pgrep", name]);
      return [...running];
    },
    kill: (pid) => {
      calls.push(["kill", pid]);
      alive.delete(pid);
    },
    alive: (pid) => alive.has(pid),
  };
}

function setup(extra = {}) {
  const root = mkdtempSync(join(tmpdir(), "cos-app-"));
  const env = {
    HOME: join(root, "home"),
    COS_APP_DIR: join(root, "Applications"),
    COS_DOWNLOAD_BASE: base,
    ...extra,
  };
  mkdirSync(env.HOME);
  mkdirSync(env.COS_APP_DIR);
  const logs = [];
  const run = (tools = fakeTools(), overrides = {}) =>
    installApp({ env: { ...env, ...overrides }, platform: "darwin", tools, uid: 501, log: (m) => logs.push(m) });
  return { root, env, logs, run, systemApp: join(env.COS_APP_DIR, "cos.app"), userApp: join(env.HOME, "Applications", "cos.app") };
}

const versionOf = (app) => readFileSync(join(app, "Contents", "Info.plist"), "utf8");

test("installs into the app folder, then upgrades the same folder in place", async () => {
  const { run, systemApp } = setup();

  const tools = fakeTools();
  assert.equal(await run(tools, { COS_VERSION: "v0.1.0" }), systemApp);
  assert.equal(versionOf(systemApp), "0.1.0");
  const inode = statSync(systemApp).ino;

  await run();

  assert.equal(versionOf(systemApp), "0.2.0");
  assert.equal(existsSync(join(systemApp, "Contents", "from-0.1.0")), false, "old contents removed");
  assert.equal(statSync(systemApp).ino, inode, "the app folder is kept");
  assert.ok(tools.calls.some((call) => call.join(" ") === `xattr -dr com.apple.quarantine ${systemApp}`));
});

test("a user-owned cos.app in a read-only folder upgrades in place", async () => {
  const { env, logs, run, systemApp, userApp } = setup();
  await run();
  chmodSync(env.COS_APP_DIR, 0o555);
  try {
    await run(fakeTools(), { COS_VERSION: "0.1.0" });
  } finally {
    chmodSync(env.COS_APP_DIR, 0o755);
  }
  assert.equal(versionOf(systemApp), "0.1.0");
  assert.equal(existsSync(userApp), false);
  assert.equal(logs.some((line) => line.includes("sudo")), false);
});

test("falls back to ~/Applications when the app folder isn't writable, with the sudo command", async () => {
  const { env, logs, run, systemApp, userApp } = setup();
  chmodSync(env.COS_APP_DIR, 0o555);
  try {
    assert.equal(await run(), userApp);
  } finally {
    chmodSync(env.COS_APP_DIR, 0o755);
  }
  assert.equal(versionOf(userApp), "0.2.0");
  assert.equal(existsSync(systemApp), false);
  assert.ok(logs.some((line) => line.includes("isn't writable by you, so installing in")));
  assert.ok(logs.some((line) => line.includes("| sudo sh -s -- --app")));

  // Next time the copy in ~/Applications is upgraded, not a second one made.
  await run(fakeTools(), { COS_VERSION: "v0.1.0" });
  assert.equal(versionOf(userApp), "0.1.0");
  assert.equal(existsSync(systemApp), false);
});

test("a read-only cos.app stays; the new one goes in ~/Applications with a warning", () => {
  const { env } = setup();
  const system = env.COS_APP_DIR;
  const user = join(env.HOME, "Applications");
  mkdirSync(join(system, "cos.app", "Contents"), { recursive: true });
  chmodSync(join(system, "cos.app"), 0o555);
  try {
    const { dir, notes } = pickAppDir({ system, user });
    assert.equal(dir, user);
    assert.match(notes.join("\n"), /the old copy in .* stays/);
  } finally {
    chmodSync(join(system, "cos.app"), 0o755);
  }
});

test("refuses a download whose checksum doesn't match and leaves the app alone", async () => {
  const { run, systemApp } = setup();
  await run();

  await assert.rejects(run(fakeTools(), { COS_VERSION: "v0.0.9" }), /checksum mismatch/);
  assert.equal(versionOf(systemApp), "0.2.0");
});

test("quits a running app by its pid first, unless COS_APP_NO_QUIT=1", async () => {
  const { logs, run } = setup();

  const tools = fakeTools({ running: [4242] });
  await run(tools);
  assert.deepEqual(
    tools.calls.filter(([command]) => command === "pgrep" || command === "kill"),
    [["pgrep", "cos-desktop"], ["kill", 4242]],
  );
  assert.ok(logs.includes("quitting the running cos app (pid 4242)"));
  assert.ok(logs.some((line) => line.startsWith("it was running; open it again")));

  const skipped = fakeTools({ running: [4242] });
  await run(skipped, { COS_APP_NO_QUIT: "1" });
  assert.equal(skipped.calls.some(([command]) => command === "pgrep" || command === "kill"), false);
});

test("is for macOS 13 or later only", async () => {
  const { env } = setup();
  await assert.rejects(installApp({ env, platform: "linux", tools: fakeTools() }), /the cos app is for macOS/);
  await assert.rejects(
    installApp({ env, platform: "darwin", tools: fakeTools({ macos: "12.7" }), log: () => {} }),
    /macOS 13 or later/,
  );
});

test("reads pgrep's output", () => {
  assert.deepEqual(parsePids("123\n456\n"), [123, 456]);
  assert.deepEqual(parsePids(""), []);
});
