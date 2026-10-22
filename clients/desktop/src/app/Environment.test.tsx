import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { type Page, createUiStore } from "./ui";

vi.mock("../terminal/engine", () => ({
  loadEngine: () => Promise.resolve(null),
  createEngine: () =>
    Promise.resolve({
      open: () => undefined,
      write: () => undefined,
      reset: () => undefined,
      resize: () => undefined,
      fitSize: () => ({ cols: 100, rows: 30 }),
      focus: () => undefined,
      onData: () => ({ dispose: () => undefined }),
      dispose: () => undefined,
    }),
}));

const PERSONAL = "~/.claude-personal";
const GATEWAY = "https://llm-gateway.example.dev/anthropic";

async function renderApp(page: Page) {
  const harness = fakeStore();
  const ui = createUiStore({ page });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const main = () => within(screen.getByRole("main"));
const variables = () => within(main().getByRole("list", { name: "Variables" }));
const row = (name: string) => variables().getByRole("listitem", { name });
const sheet = () => within(screen.getByRole("dialog"));

/** Every text node in the page and every input's value that isn't a password field's. */
function plainTexts(): string[] {
  const texts: string[] = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    texts.push(node.textContent ?? "");
  }
  for (const input of document.querySelectorAll("input, textarea")) {
    const field = input as HTMLInputElement;
    if (field.type !== "password") texts.push(field.value);
  }
  return texts;
}

function inPage(value: string): boolean {
  return (
    plainTexts().some((t) => t.includes(value)) ||
    // Attributes too: a title or label must not carry it either.
    document.body.innerHTML.includes(value)
  );
}

describe("a project's environment", () => {
  it("lists its variables masked, CLAUDE_CONFIG_DIR first, with the note", async () => {
    await renderApp({ kind: "projectEnv", id: 1 });
    expect(main().getByRole("heading", { name: "Environment", level: 1 })).toBeInTheDocument();
    expect(
      main().getByText(
        "Used by every agent in cos: its Chief of Staff, workspace agents and runs. Running agents keep their current values until they restart.",
      ),
    ).toBeInTheDocument();
    const names = variables()
      .getAllByRole("listitem")
      .map((item) => item.getAttribute("aria-label"));
    expect(names).toEqual(["CLAUDE_CONFIG_DIR", "ANTHROPIC_BASE_URL"]);
    expect(within(row("CLAUDE_CONFIG_DIR")).getByLabelText("Hidden value")).toBeInTheDocument();
  });

  it("keeps values out of the page until Show, and hides them again", async () => {
    const { user } = await renderApp({ kind: "projectEnv", id: 1 });
    expect(inPage(PERSONAL)).toBe(false);
    expect(inPage(GATEWAY)).toBe(false);
    expect(inPage("DISABLE_TELEMETRY")).toBe(true);

    await user.click(variables().getByRole("button", { name: "Show CLAUDE_CONFIG_DIR" }));
    expect(within(row("CLAUDE_CONFIG_DIR")).getByText(PERSONAL)).toBeVisible();
    // Only the one shown.
    expect(inPage(GATEWAY)).toBe(false);

    await user.click(variables().getByRole("button", { name: "Hide CLAUDE_CONFIG_DIR" }));
    expect(inPage(PERSONAL)).toBe(false);
  });

  it("adds a variable through the editor, its value a password field until shown", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(main().getByRole("button", { name: "Add environment variable" }));
    expect(sheet().getByRole("heading", { name: "Add environment variable" })).toBeInTheDocument();
    await user.type(sheet().getByLabelText("Name"), "GITHUB_TOKEN");
    const value = sheet().getByLabelText("Value");
    expect(value).toHaveAttribute("type", "password");
    await user.type(value, "ghp_secret");
    expect(value).toHaveAttribute("type", "password");
    expect(inPage("ghp_secret")).toBe(false);
    await user.click(sheet().getByRole("button", { name: "Show value" }));
    expect(value).toHaveAttribute("type", "text");
    await user.click(sheet().getByRole("button", { name: "Hide value" }));
    expect(value).toHaveAttribute("type", "password");

    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(daemon.env.projects[1]?.GITHUB_TOKEN).toBe("ghp_secret");
    expect(row("GITHUB_TOKEN")).toBeInTheDocument();
    expect(inPage("ghp_secret")).toBe(false);
  });

  it("checks the name before sending it", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(main().getByRole("button", { name: "Add environment variable" }));
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    expect(sheet().getByRole("alert")).toHaveTextContent("Give it a name.");

    const name = sheet().getByLabelText("Name");
    await user.type(name, "9LIVES");
    expect(sheet().getByRole("alert")).toHaveTextContent("A name can't start with a digit.");
    expect(name).toHaveAttribute("aria-invalid", "true");
    await user.clear(name);
    await user.type(name, "MY-VAR");
    expect(sheet().getByRole("alert")).toHaveTextContent(/Letters, digits and _ only/);
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    expect(daemon.requests.some((r) => r.method === "PUT")).toBe(false);

    await user.clear(name);
    await user.type(name, "ANTHROPIC_BASE_URL");
    expect(sheet().queryByRole("alert")).not.toBeInTheDocument();
    expect(sheet().getByText(/already set here; this replaces its value/)).toBeInTheDocument();
    expect(sheet().getByRole("button", { name: "Replace" })).toBeEnabled();
  });

  it("shows the daemon's reason inline when it refuses a name", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(main().getByRole("button", { name: "Add environment variable" }));
    // A name cos keeps that only the daemon knows: Claude's session marker.
    await user.type(sheet().getByLabelText("Name"), "CLAUDECODE");
    await user.type(sheet().getByLabelText("Value"), "1");
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    await act(() => settle());
    expect(sheet().getByRole("alert")).toHaveTextContent(
      "cos sets CLAUDECODE itself for every agent, so it can't be set here",
    );
    expect(daemon.env.projects[1]?.CLAUDECODE).toBeUndefined();
    // Editing clears it.
    await user.type(sheet().getByLabelText("Name"), "X");
    expect(sheet().queryByRole("alert")).not.toBeInTheDocument();
  });

  it("refuses COS_ names before asking the daemon", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(main().getByRole("button", { name: "Add environment variable" }));
    await user.type(sheet().getByLabelText("Name"), "COS_HOOK_SOCKET");
    expect(sheet().getByRole("alert")).toHaveTextContent(
      "Names starting with COS_ are kept for cos itself.",
    );
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    expect(daemon.requests.some((r) => r.method === "PUT")).toBe(false);
  });

  it("edits a value with the name fixed", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(variables().getByRole("button", { name: "Edit CLAUDE_CONFIG_DIR" }));
    const name = sheet().getByLabelText("Name");
    expect(name).toHaveValue("CLAUDE_CONFIG_DIR");
    expect(name).toHaveAttribute("readonly");
    const value = sheet().getByLabelText("Value");
    expect(value).toHaveAttribute("type", "password");
    expect(value).toHaveValue(PERSONAL);
    // In the field's property only, not its markup (the placeholder happens to be the same path).
    expect(value).not.toHaveAttribute("value");
    expect(plainTexts().some((t) => t.includes(PERSONAL))).toBe(false);
    expect(sheet().getByText(/Which Claude account this project's agents use/)).toBeInTheDocument();
    await user.clear(value);
    await user.type(value, "~/.claude-work");
    await user.click(sheet().getByRole("button", { name: "Save" }));
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(daemon.env.projects[1]?.CLAUDE_CONFIG_DIR).toBe("~/.claude-work");
  });

  it("removes a variable, with Undo", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    await user.click(variables().getByRole("button", { name: "Remove ANTHROPIC_BASE_URL" }));
    await act(() => settle());
    expect(variables().queryByRole("listitem", { name: "ANTHROPIC_BASE_URL" })).toBeNull();
    expect(daemon.env.projects[1]?.ANTHROPIC_BASE_URL).toBeUndefined();
    const undo = main().getByRole("status");
    expect(undo).toHaveTextContent("Removed ANTHROPIC_BASE_URL.");
    // Undo puts back the value without showing it.
    expect(inPage(GATEWAY)).toBe(false);

    await user.click(within(undo).getByRole("button", { name: "Undo" }));
    await act(() => settle());
    expect(daemon.env.projects[1]?.ANTHROPIC_BASE_URL).toBe(GATEWAY);
    expect(row("ANTHROPIC_BASE_URL")).toBeInTheDocument();
    expect(main().queryByText(/Removed/)).not.toBeInTheDocument();
  });

  it("lists inherited global variables, and overrides one", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 1 });
    const inherited = within(main().getByRole("region", { name: "Inherited from all projects" }));
    const telemetry = inherited.getByRole("listitem", { name: "DISABLE_TELEMETRY" });
    expect(within(telemetry).getByLabelText("Hidden value")).toBeInTheDocument();
    await user.click(within(telemetry).getByRole("button", { name: "Show DISABLE_TELEMETRY" }));
    expect(within(telemetry).getByText("1")).toBeVisible();

    await user.click(within(telemetry).getByRole("button", { name: "Override DISABLE_TELEMETRY" }));
    expect(
      sheet().getByRole("heading", { name: "Override DISABLE_TELEMETRY" }),
    ).toBeInTheDocument();
    expect(sheet().getByLabelText("Name")).toHaveAttribute("readonly");
    expect(sheet().getByLabelText("Value")).toHaveValue("");
    await user.type(sheet().getByLabelText("Value"), "0");
    await user.click(sheet().getByRole("button", { name: "Override" }));
    await act(() => settle());

    expect(daemon.env.projects[1]?.DISABLE_TELEMETRY).toBe("0");
    expect(daemon.env.global.DISABLE_TELEMETRY).toBe("1");
    expect(within(row("DISABLE_TELEMETRY")).getByText("overrides all projects")).toBeVisible();
    expect(
      main().queryByRole("region", { name: "Inherited from all projects" }),
    ).not.toBeInTheDocument();
  });

  it("offers CLAUDE_CONFIG_DIR when the project sets nothing", async () => {
    const { user, daemon } = await renderApp({ kind: "projectEnv", id: 2 });
    expect(main().getByText("This project sets no variables of its own yet.")).toBeInTheDocument();
    const chip = main().getByRole("button", { name: /CLAUDE_CONFIG_DIR/ });
    expect(chip).toHaveTextContent("Which Claude account this project's agents use");
    await user.click(chip);
    expect(sheet().getByLabelText("Name")).toHaveValue("CLAUDE_CONFIG_DIR");
    expect(sheet().getByLabelText("Value")).toHaveAttribute("placeholder", PERSONAL);
    await user.type(sheet().getByLabelText("Value"), "~/.claude-work");
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    await act(() => settle());
    expect(daemon.env.projects[2]).toEqual({ CLAUDE_CONFIG_DIR: "~/.claude-work" });
    expect(row("CLAUDE_CONFIG_DIR")).toBeInTheDocument();
  });

  it("opens from the project's menu in the sidebar and from the chief's menu", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 2 });
    const sidebar = within(screen.getByRole("tree", { name: "Projects" }));
    await user.click(sidebar.getByRole("button", { name: "More for cos" }));
    await user.click(screen.getByRole("menuitem", { name: /Environment…/ }));
    expect(ui.store.getState().page).toEqual({ kind: "projectEnv", id: 1 });

    ui.navigate({ kind: "chief", id: 2 });
    await act(() => settle());
    await user.click(main().getByRole("button", { name: "Chief of Staff actions" }));
    await user.click(screen.getByRole("menuitem", { name: /Environment…/ }));
    expect(ui.store.getState().page).toEqual({ kind: "projectEnv", id: 2 });
  });
});

describe("the environment for all projects", () => {
  it("is a section of Settings, using the same list", async () => {
    const { user, daemon, cos } = await renderApp({ kind: "settings" });
    const section = within(main().getByRole("region", { name: "Environment" }));
    expect(section.getByText("All projects")).toBeInTheDocument();
    const list = within(section.getByRole("list", { name: "Variables" }));
    expect(list.getByRole("listitem", { name: "DISABLE_TELEMETRY" })).toBeInTheDocument();
    expect(section.queryByText("Inherited from all projects")).not.toBeInTheDocument();
    expect(inPage(PERSONAL)).toBe(false);

    await user.click(section.getByRole("button", { name: "Add environment variable" }));
    expect(sheet().getByText(/For every project's agents/)).toBeInTheDocument();
    await user.type(sheet().getByLabelText("Name"), "ANTHROPIC_API_KEY");
    await user.type(sheet().getByLabelText("Value"), "sk-ant-x");
    await user.click(sheet().getByRole("button", { name: "Add variable" }));
    await act(() => settle());
    expect(daemon.env.global.ANTHROPIC_API_KEY).toBe("sk-ant-x");
    expect(list.getByRole("listitem", { name: "ANTHROPIC_API_KEY" })).toBeInTheDocument();
    // Every project's merge has it now.
    expect(cos.get().projectEnvs[2]?.effective.ANTHROPIC_API_KEY).toBe("sk-ant-x");
    expect(inPage("sk-ant-x")).toBe(false);
  });

  it("follows env.changed from elsewhere (the CLI, say)", async () => {
    const { daemon } = await renderApp({ kind: "settings" });
    await act(async () => {
      daemon.env.global.HTTPS_PROXY = "http://proxy:3128";
      daemon.log("env.changed", { project_id: null, name: "HTTPS_PROXY", action: "set" });
      await settle();
    });
    const section = within(main().getByRole("region", { name: "Environment" }));
    expect(section.getByRole("listitem", { name: "HTTPS_PROXY" })).toBeInTheDocument();
  });
});

describe("environment hints", () => {
  it("shows the chief's env names and Claude account in its header", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    expect(main().getByLabelText("Claude account: .claude-personal")).toHaveTextContent(
      ".claude-personal",
    );
    const hint = main().getByRole("button", { name: "· env: CLAUDE_CONFIG_DIR, +1" });
    expect(hint).toHaveAttribute(
      "title",
      "Environment for cos's agents:\nCLAUDE_CONFIG_DIR\nANTHROPIC_BASE_URL",
    );
    // The account, not the full value.
    expect(plainTexts().some((t) => t.includes(PERSONAL))).toBe(false);
    await user.click(hint);
    expect(ui.store.getState().page).toEqual({ kind: "projectEnv", id: 1 });
  });

  it("shows neither for a project without variables", async () => {
    await renderApp({ kind: "chief", id: 2 });
    expect(main().queryByLabelText(/Claude account/)).not.toBeInTheDocument();
    expect(main().queryByText(/env:/)).not.toBeInTheDocument();
  });

  it("puts the hint in the project row's tooltip", async () => {
    await renderApp({ kind: "waiting" });
    const sidebar = within(screen.getByRole("tree", { name: "Projects" }));
    expect(sidebar.getByTitle(/Chief of Staff for cos/)).toHaveAttribute(
      "title",
      "Chief of Staff for cos\n/Users/me/personal/apps/cos\nenv: CLAUDE_CONFIG_DIR, +1",
    );
    expect(sidebar.getByTitle(/Chief of Staff for opal-app/).getAttribute("title")).not.toMatch(
      /env:/,
    );
  });

  it("shows the names an agent launched with on its tab", async () => {
    await renderApp({ kind: "workspace", id: 3 });
    const panel = main().getByRole("tabpanel");
    const hint = within(panel).getByText("· env: CLAUDE_CONFIG_DIR, +2");
    expect(hint).toHaveAttribute(
      "title",
      "Environment set at launch:\nCLAUDE_CONFIG_DIR\nANTHROPIC_BASE_URL\nDISABLE_TELEMETRY",
    );
    expect(main().getAllByRole("tab")[0]?.getAttribute("title")).toMatch(
      /env: CLAUDE_CONFIG_DIR, ANTHROPIC_BASE_URL, DISABLE_TELEMETRY/,
    );
  });
});

describe("an agent waiting for a Claude login", () => {
  const LOGIN = "CLAUDE_CONFIG_DIR=~/.claude-personal claude";

  it("waits on you, with how to log in and Open terminal", async () => {
    const { user, ui, cos } = await renderApp({ kind: "waiting" });
    expect(cos.get().agents.find((a) => a.id === 11)?.state).toBe("needs_login");
    const section = main().getByRole("heading", { name: "Claude logins" }).closest("section")!;
    const card = within(section).getByRole("article");
    expect(card).toHaveTextContent("Check the docs for broken links");
    expect(card).toHaveTextContent("Claude needs you to log in");
    expect(card).toHaveTextContent(`Open its terminal and log in, or run ${LOGIN} once.`);
    await user.click(within(card).getByRole("button", { name: "Open terminal" }));
    expect(ui.store.getState().page).toEqual({ kind: "agent", id: 11 });
  });

  it("shows its state and a banner over its terminal", async () => {
    await renderApp({ kind: "agent", id: 11 });
    expect(main().getByText("Needs Claude login")).toBeInTheDocument();
    const banner = main().getByRole("region", { name: "Waiting" });
    expect(banner).toHaveTextContent("Claude needs you to log in");
    expect(banner).toHaveTextContent(`Log in in the terminal below, or run ${LOGIN} once.`);
  });

  it("shows in the chief's waiting strip when the chief started it", async () => {
    const harness = fakeStore();
    harness.daemon.agents.find((a) => a.id === 11)!.spawned_by = 40;
    const ui = createUiStore({ page: { kind: "chief", id: 1 } });
    render(<App cos={harness.cos} ui={ui} />);
    await act(async () => {
      await harness.cos.start();
      await settle();
    });
    const strip = within(main().getByRole("region", { name: "Agents waiting on you" }));
    expect(strip.getByText("2 agents waiting on you")).toBeInTheDocument();
    expect(strip.getByText(/needs a Claude login/)).toBeInTheDocument();
    await userEvent.setup().click(strip.getByRole("button", { name: "Open terminal" }));
    expect(ui.store.getState().page).toEqual({ kind: "agent", id: 11 });
  });

  it("carries on once logged in in its terminal", async () => {
    const { cos } = await renderApp({ kind: "agent", id: 11 });
    await act(async () => {
      await cos.client.sendKeys(11, new TextEncoder().encode("\r"));
      await settle();
    });
    expect(cos.get().agents.find((a) => a.id === 11)?.state).toBe("idle");
    expect(main().queryByRole("region", { name: "Waiting" })).not.toBeInTheDocument();
  });
});
