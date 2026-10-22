// Installs from a fake release served on localhost: a tarball whose `cos`
// is a shell script printing its version.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { gzipSync } from "node:zlib";

import { expectedSum, extractFile, install } from "../lib/install.js";
import { assetName, targetFor } from "../lib/platform.js";

/** A ustar archive of `files` ({ name: contents }). */
function tar(files) {
  const blocks = [];
  for (const [name, contents] of Object.entries(files)) {
    const body = Buffer.from(contents);
    const header = Buffer.alloc(512);
    header.write(name, 0);
    header.write("0000755\0", 100);
    header.write("0000000\0", 108);
    header.write("0000000\0", 116);
    header.write(`${body.length.toString(8).padStart(11, "0")}\0`, 124);
    header.write("00000000000\0", 136);
    header.write("0", 156);
    header.write("ustar\0", 257);
    header.write("00", 263);
    header.fill(" ", 148, 156);
    const sum = header.reduce((total, byte) => total + byte, 0);
    header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148);
    blocks.push(header, body, Buffer.alloc((512 - (body.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return Buffer.concat(blocks);
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const platform = "linux";
const arch = process.arch === "arm64" ? "arm64" : "x64";
const asset = assetName(targetFor(platform, arch));
const files = new Map();
let server;
let base;

function publish(tag, version, { corrupt = false } = {}) {
  const archive = gzipSync(
    tar({ "README.md": "hi\n", "./cos": `#!/bin/sh\necho "cos ${version}"\n` }),
  );
  const sum = corrupt ? "0".repeat(64) : sha256(archive);
  files.set(`/${tag}/${asset}`, archive);
  files.set(`/${tag}/SHA256SUMS`, Buffer.from(`${sum}  ${asset}\n`));
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

function env(extra = {}) {
  const home = mkdtempSync(join(tmpdir(), "cos-agent-"));
  return { HOME: home, PATH: "/usr/bin:/bin", COS_DOWNLOAD_BASE: base, ...extra };
}

test("installs the latest release in ~/.local/bin, and again over it", async () => {
  const environment = env();

  const first = await install({ env: environment, platform, arch });
  const second = await install({ env: environment, platform, arch });

  assert.equal(first, join(environment.HOME, ".local", "bin", "cos"));
  assert.equal(second, first);
  assert.equal(statSync(first).mode & 0o777, 0o755);
  assert.equal(execFileSync(first, ["--version"], { encoding: "utf8" }), "cos 0.2.0\n");
});

test("installs a given version into a given folder", async () => {
  const environment = env();
  environment.COS_VERSION = "0.1.0";
  environment.COS_INSTALL_DIR = join(environment.HOME, "bin");

  const cos = await install({ env: environment, platform, arch });

  assert.equal(cos, join(environment.HOME, "bin", "cos"));
  assert.equal(execFileSync(cos, ["--version"], { encoding: "utf8" }), "cos 0.1.0\n");
});

test("refuses a download whose checksum doesn't match", async () => {
  const environment = env({ COS_VERSION: "v0.0.9" });

  await assert.rejects(install({ env: environment, platform, arch }), /checksum mismatch/);
});

test("says when a release has no build for this machine", async () => {
  await assert.rejects(install({ env: env({ COS_VERSION: "v9.9.9" }), platform, arch }), /HTTP 404/);
});

test("reads both SHA256SUMS styles", () => {
  const sums = `${"a".repeat(64)}  cos-x.tar.gz\n${"b".repeat(64)} *cos-y.tar.gz\n`;
  assert.equal(expectedSum(sums, "cos-x.tar.gz"), "a".repeat(64));
  assert.equal(expectedSum(sums, "cos-y.tar.gz"), "b".repeat(64));
  assert.equal(expectedSum(sums, "cos-z.tar.gz"), undefined);
});

test("extracts cos from a tarball made by tar(1)", () => {
  const dir = mkdtempSync(join(tmpdir(), "cos-tar-"));
  writeFileSync(join(dir, "cos"), "binary");
  writeFileSync(join(dir, "README.md"), "readme");
  execFileSync("tar", ["-cf", join(dir, "a.tar"), "-C", dir, "README.md", "cos"]);

  const found = extractFile(readFileSync(join(dir, "a.tar")), "cos");

  assert.equal(found.toString(), "binary");
});
