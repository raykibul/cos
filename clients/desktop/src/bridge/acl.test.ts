// Tauri refuses a command its ACL doesn't name ("not allowed by ACL"), so
// every command the UI invokes must be listed in src-tauri/build.rs, which
// makes its `allow-…` permission, and granted in the main window's
// capability. This keeps the three in step.

import { describe, expect, it } from "vitest";
import buildRs from "../../src-tauri/build.rs?raw";
import capability from "../../src-tauri/capabilities/default.json?raw";
import mainRs from "../../src-tauri/src/main.rs?raw";
import bridge from "./tauri.ts?raw";

const invoked = [...bridge.matchAll(/invoke(?:<[^>]*>)?\("([a-z_]+)"/g)].map((m) => m[1]!);
const listed = [...buildRs.matchAll(/^\s*"([a-z_]+)",$/gm)].map((m) => m[1]!);
const handled = [...mainRs.matchAll(/commands::([a-z_]+),/g)].map((m) => m[1]!);
const granted = (JSON.parse(capability) as { permissions: string[] }).permissions;

describe("the Tauri ACL", () => {
  it("finds the commands", () => {
    expect(invoked.length).toBeGreaterThan(10);
    expect(new Set(invoked)).toEqual(new Set(handled));
  });

  it.each(invoked)("lets the UI call %s", (command) => {
    expect(listed).toContain(command);
    expect(granted).toContain(`allow-${command.replaceAll("_", "-")}`);
  });
});
