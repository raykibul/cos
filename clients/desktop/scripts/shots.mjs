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
  ["waiting", "page=waiting"],
  ["events", "page=events"],
  ["project", "page=project:1"],
  ["workspace", "page=workspace:3"],
  ["agent-working", "page=agent:7"],
  ["agent-approval", "page=agent:8"],
  ["agent-trust", "page=agent:9"],
  ["run-running", "page=run:10"],
  ["run-plan", "page=run:11"],
  ["run-done", "page=run:7"],
  ["not-running", "daemon=off"],
  ["sheet-new-agent", "page=workspace:3&dialog=newAgent:3"],
  ["sheet-new-run", "page=workspace:4&dialog=newRun:4"],
  ["sheet-add-project", "page=waiting&dialog=addProject"],
  ["sheet-new-workspace", "page=project:1&dialog=newWorkspace:1"],
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
    for (const [name, query] of shots) {
      if (only && !name.startsWith(only)) continue;
      const page = await context.newPage();
      page.on("pageerror", (error) => console.error(`${name}: ${error.message}`));
      await page.goto(`http://127.0.0.1:4180/?static&${query}`);
      await page.waitForLoadState("networkidle");
      await page.evaluate(() => document.fonts.ready);
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
