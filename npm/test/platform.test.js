import assert from "node:assert/strict";
import { test } from "node:test";

import { assetName, installDir, releaseUrl, targetFor } from "../lib/platform.js";

test("maps each supported OS and CPU to its release target", () => {
  assert.equal(targetFor("linux", "x64"), "x86_64-unknown-linux-musl");
  assert.equal(targetFor("linux", "arm64"), "aarch64-unknown-linux-musl");
  assert.equal(targetFor("darwin", "x64"), "x86_64-apple-darwin");
  assert.equal(targetFor("darwin", "arm64"), "aarch64-apple-darwin");
});

test("refuses other OSes and CPUs clearly", () => {
  assert.throws(() => targetFor("win32", "x64"), /no cos build for win32/);
  assert.throws(() => targetFor("linux", "ia32"), /no cos build for the ia32 CPU/);
  assert.throws(() => targetFor("freebsd", "arm64"), /cos runs on Linux and macOS/);
});

test("asset names carry no version", () => {
  assert.equal(assetName("aarch64-apple-darwin"), "cos-aarch64-apple-darwin.tar.gz");
});

test("release URLs: latest, or a tag with or without its v", () => {
  const base = "https://github.com/raykibul/cos/releases/";
  assert.equal(releaseUrl(base, undefined), "https://github.com/raykibul/cos/releases/latest/download");
  assert.equal(releaseUrl(base, "latest"), "https://github.com/raykibul/cos/releases/latest/download");
  assert.equal(releaseUrl(base, "v0.2.0"), "https://github.com/raykibul/cos/releases/download/v0.2.0");
  assert.equal(releaseUrl(base, "0.2.0"), "https://github.com/raykibul/cos/releases/download/v0.2.0");
});

test("installs to ~/.local/bin unless COS_INSTALL_DIR says otherwise", () => {
  assert.equal(installDir({ HOME: "/home/me" }), "/home/me/.local/bin");
  assert.equal(installDir({ HOME: "/home/me", COS_INSTALL_DIR: "/opt/bin" }), "/opt/bin");
});
