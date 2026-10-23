// Screenshots of every screen against the fake bridge, light and dark.
// Run through `make ui-shots`, which builds dist/ and runs this in
// Playwright's image. Writes PNGs to $SHOTS_DIR (default ./shots).

import { createReadStream, existsSync, mkdirSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("../dist/", import.meta.url));
const out = process.env.SHOTS_DIR ?? "shots";
const only = process.argv[2] ?? null;

const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
};

const shots = [
  ["chief-conversation", "page=chief:1"],
  [
    "chief-rows-open",
    "page=chief:1",
    async (page) => {
      await page.getByRole("button", { name: "2 updates from agents" }).click();
      await page
        .getByRole("button", { name: /Started an agent/ })
        .first()
        .click();
    },
  ],
  ["chief-empty", "page=chief:2"],
  ["chief-terminal", "page=chief:1&terminal"],
  ["chief-paused", "page=chief:1&paused"],
  ["chief-context-high", "page=chief:1&context=92"],
  [
    "chief-menu-pause",
    "page=chief:1",
    (page) => page.getByRole("button", { name: "Chief of Staff actions" }).click(),
  ],
  ["sidebar-expanded", "page=workspace:3"],
  [
    "tabs",
    "page=chief:1",
    async (page) => {
      await page.getByRole("treeitem", { name: /Fix login bug/ }).click();
      await page.getByRole("button", { name: /^Chief of Staff, opal-app/ }).click();
    },
  ],
  ["switcher", "page=chief:1", (page) => page.keyboard.press("Meta+k")],
  [
    "switcher-message",
    "page=chief:1",
    async (page) => {
      await page.keyboard.press("Meta+k");
      await page.keyboard.type("opal");
      await page.keyboard.press("Tab");
      await page.keyboard.type("Ship the onboarding copy and refresh the screenshots.");
    },
  ],
  ["waiting", "page=waiting"],
  [
    "waiting-login",
    "page=waiting",
    (page) => page.getByRole("heading", { name: "Claude logins" }).scrollIntoViewIfNeeded(),
  ],
  ["events", "page=events"],
  ["project", "page=project:1"],
  ["workspace", "page=workspace:3"],
  ["agent-working", "page=agent:7"],
  ["agent-approval", "page=agent:8"],
  ["agent-trust", "page=agent:9"],
  ["agent-login", "page=agent:11"],
  ["not-running", "daemon=off"],
  ["sheet-new-agent", "page=workspace:3&dialog=newAgent:3"],
  ["sheet-add-project", "page=waiting&dialog=addProject"],
  ["sheet-new-workspace", "page=project:1&dialog=newWorkspace:1"],
  [
    "host-picker",
    "page=waiting",
    (page) => page.getByRole("button", { name: "Machine: This Mac" }).click(),
  ],
  [
    "host-picker-tailnet",
    "page=waiting",
    async (page) => {
      await page.getByRole("button", { name: "Machine: This Mac" }).click();
      await page.getByText("On your tailnet").waitFor();
    },
  ],
  ["sheet-add-host", "page=waiting&dialog=addHost"],
  ["sheet-rename-host", "page=settings&dialog=renameHost:h1"],
  [
    "sheet-rename-host-taken",
    "page=settings&dialog=renameHost:h1",
    async (page) => {
      const name = page.getByRole("dialog").getByLabel("Name", { exact: true });
      await name.fill("Travel laptop");
      await name.press("Enter");
      await page.getByRole("dialog").getByRole("alert").waitFor();
    },
  ],
  [
    "sheet-add-host-picked",
    "page=waiting&dialog=addHost",
    async (page) => {
      const tailnet = page.getByRole("region", { name: "On your tailnet" });
      await tailnet.getByRole("button", { name: /Show 1 other device/ }).click();
      await tailnet.getByRole("button", { name: /build-box/ }).click();
    },
  ],
  ["sheet-add-host-tailscale-off", "page=waiting&dialog=addHost&tailscale=off"],
  ["sheet-add-host-daemon-off", "daemon=off&dialog=addHost"],
  ["host-device-pending", "host=h1"],
  ["host-unreachable", "host=h2"],
  ["env-project", "page=projectEnv:1"],
  [
    "env-project-shown",
    "page=projectEnv:1",
    (page) => page.getByRole("button", { name: "Show CLAUDE_CONFIG_DIR" }).click(),
  ],
  ["env-project-empty", "page=projectEnv:2"],
  [
    "sheet-env-edit",
    "page=projectEnv:1",
    (page) => page.getByRole("button", { name: "Edit CLAUDE_CONFIG_DIR" }).click(),
  ],
  [
    "sheet-env-add",
    "page=projectEnv:1&dialog=envVar:1",
    async (page) => {
      await page.getByRole("dialog").getByLabel("Name", { exact: true }).fill("GITHUB_TOKEN");
      await page.getByRole("dialog").getByLabel("Value", { exact: true }).fill("ghp_example");
    },
  ],
  [
    "sheet-env-refused",
    "page=projectEnv:1&dialog=envVar:1",
    async (page) => {
      await page.getByRole("dialog").getByLabel("Name", { exact: true }).fill("CLAUDECODE");
      await page.getByRole("dialog").getByLabel("Value", { exact: true }).fill("1");
      await page.getByRole("button", { name: "Add variable" }).click();
      await page.getByRole("alert").waitFor();
    },
  ],
  [
    "settings-env",
    "page=settings",
    (page) => page.getByRole("region", { name: "Environment" }).scrollIntoViewIfNeeded(),
  ],
  ["remote-access", "page=settings"],
  [
    "remote-access-firewall",
    "page=settings&firewall",
    (page) => page.getByRole("list", { name: "Devices" }).scrollIntoViewIfNeeded(),
  ],
  // Last, and held mid-drag: a released drag saves the width, which later
  // shots in the same browser context would pick up.
  [
    "chief-terminal-wide",
    "page=chief:1&terminal",
    async (page) => {
      const divider = page.getByRole("separator", { name: "Resize the chief's terminal" });
      const box = await divider.boundingBox();
      const x = box.x + box.width / 2;
      const y = box.y + box.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x - 140, y, { steps: 4 });
      await page.mouse.move(x - 260, y, { steps: 4 });
    },
  ],
];

const server = createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
  let file = join(root, path);
  if (!file.startsWith(root) || !existsSync(file) || statSync(file).isDirectory()) {
    file = join(root, "index.html");
  }
  res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
});
await new Promise((resolve) => server.listen(4180, "127.0.0.1", resolve));

mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
try {
  for (const scheme of ["light", "dark"]) {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 820 },
      deviceScaleFactor: 1,
      colorScheme: scheme,
    });
    for (const [name, query, action] of shots) {
      if (only && !name.startsWith(only)) continue;
      const page = await context.newPage();
      page.on("pageerror", (error) => console.error(`${name}: ${error.message}`));
      await page.goto(`http://127.0.0.1:4180/?static&${query}`);
      await page.waitForLoadState("networkidle");
      await page.evaluate(() => document.fonts.ready);
      if (action) await action(page);
      // Let the store load and terminals draw their snapshot.
      await page.waitForTimeout(1500);
      const file = join(out, `${name}-${scheme}.png`);
      await page.screenshot({ path: file });
      console.log(file);
      await page.close();
    }
    await context.close();
  }
} finally {
  await browser.close();
  server.close();
}
