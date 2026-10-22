#!/usr/bin/env node
// `npx cos-agent` (or `npx cos-agent install`): installs or upgrades cos in
// ~/.local/bin (or $COS_INSTALL_DIR) from its GitHub releases.
// `npx cos-agent <args>`: runs the installed `cos <args>`, installing it
// first if it's missing. cos always runs from the install folder, never from
// npx's cache, so `cos daemon install` points the service at a stable path.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

import { install } from "../lib/install.js";
import { installDir } from "../lib/platform.js";

async function main(args) {
  if (args.length === 0 || (args.length === 1 && args[0] === "install")) {
    await install();
    return 0;
  }
  let cos = join(installDir(), "cos");
  if (!existsSync(cos)) {
    cos = await install();
  }
  const result = spawnSync(cos, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.signal) {
    process.kill(process.pid, result.signal);
  }
  return result.status ?? 1;
}

main(process.argv.slice(2)).then(
  (code) => process.exit(code),
  (error) => {
    console.error(`cos-agent: error: ${error.message}`);
    process.exit(1);
  },
);
