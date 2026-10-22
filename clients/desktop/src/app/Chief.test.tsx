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

async function renderApp(page: Page = { kind: "waiting" }) {
  const harness = fakeStore();
  const ui = createUiStore({ page });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const sidebar = () => within(screen.getByRole("tree", { name: "Projects" }));
const main = () => within(screen.getByRole("main"));

describe("sidebar", () => {
  it("shows a Chief of Staff row per project, with its workspace count, folded", async () => {
    await renderApp();
    const cos = sidebar().getByRole("treeitem", { name: "Chief of Staff, cos" });
    expect(cos).toHaveAttribute("aria-expanded", "false");
    expect(within(cos).getByRole("button", { name: "Show workspaces in cos" })).toHaveTextContent(
      "4",
    );
    expect(sidebar().getByRole("treeitem", { name: "Chief of Staff, opal-app" })).toBeVisible();
    // Agents and runs are on the workspace page now.
    expect(sidebar().queryByText("Fix login bug")).not.toBeInTheDocument();
    expect(sidebar().queryByText("Fix the login flow")).not.toBeInTheDocument();
  });

  it("opens the chief and unfolds its workspaces, newest first; a second click folds them", async () => {
    const { user, ui } = await renderApp();
    const row = sidebar().getByRole("treeitem", { name: "Chief of Staff, cos" });
    await user.click(within(row).getByTitle(/Chief of Staff for cos/));
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
    expect(row).toHaveAttribute("aria-expanded", "true");
    const group = within(row.parentElement!).getByRole("group");
    const names = within(group)
      .getAllByRole("treeitem")
      .map((item) => item.textContent);
    expect(names).toEqual(["React 19", "Dashboard speed", "porto", "Fix the login flow"]);

    await user.click(within(row).getByTitle(/Chief of Staff for cos/));
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(within(row.parentElement!).queryByRole("group")).not.toBeInTheDocument();
  });

  it("shows a workspace's city without a title, with its branch and path in the tooltip", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    const porto = sidebar().getByRole("button", { name: /^porto/ });
    expect(porto).toHaveAttribute("title", "cos/porto\n/Users/me/.cos/worktrees/cos-1/porto");
    await user.click(porto);
    expect(ui.store.getState().page).toEqual({ kind: "workspace", id: 4 });
    expect(main().getByRole("heading", { name: "porto", level: 1 })).toBeInTheDocument();
  });

  it("unfolds the project of the page you're on", async () => {
    await renderApp({ kind: "agent", id: 4 });
    expect(sidebar().getByRole("treeitem", { name: "Chief of Staff, opal-app" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(sidebar().getByRole("treeitem", { name: "Chief of Staff, cos" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("marks a workspace with an agent waiting on you", async () => {
    await renderApp({ kind: "chief", id: 1 });
    const lisbon = sidebar().getByRole("button", { name: /^Fix the login flow/ });
    expect(within(lisbon).getByRole("img", { name: "Waiting on you" })).toBeInTheDocument();
  });
});

describe("chief page", () => {
  it("shows the conversation: your ask, its tool calls, an update row and its answer", async () => {
    await renderApp({ kind: "chief", id: 1 });
    const log = within(screen.getByRole("log"));
    expect(main().getByRole("heading", { level: 1 })).toHaveTextContent("Chief of Staff · cos");
    expect(main().getByText("Ready for your next message")).toBeInTheDocument();
    expect(log.getByText(/Logins expire after five minutes/)).toBeInTheDocument();
    expect(log.getAllByRole("button", { name: /Started an agent/ })).toHaveLength(2);
    // The answer's numbered list, rendered as markdown.
    const items = log.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toEqual([
      expect.stringMatching(/^Review and merge lisbon/),
      expect.stringMatching(/^Approve the plan in porto/),
      expect.stringMatching(/^Finish the release notes/),
    ]);
    // A wakeup isn't shown as your message.
    expect(log.queryByText(/\[cos ·/)).not.toBeInTheDocument();
  });

  it("expands a tool row and an update row", async () => {
    const { user } = await renderApp({ kind: "chief", id: 1 });
    const log = within(screen.getByRole("log"));
    const [spawn] = log.getAllByRole("button", { name: /Started an agent/ });
    expect(spawn).toHaveAttribute("aria-expanded", "false");
    await user.click(spawn!);
    expect(spawn).toHaveAttribute("aria-expanded", "true");
    expect(log.getByText("mcp__cos__agent_spawn")).toBeInTheDocument();

    const updates = log.getByRole("button", { name: "2 updates from agents" });
    await user.click(updates);
    expect(log.getByText("Fix login bug")).toBeInTheDocument();
    expect(
      log.getByText("Waiting for your approval: npm install left-pad --save"),
    ).toBeInTheDocument();
  });

  it("sends a message: it shows at once and once only after the transcript catches up", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    const box = screen.getByRole("textbox", { name: "Message the Chief of Staff" });
    await user.type(box, "Start on the release notes{Enter}");
    expect(box).toHaveValue("");
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/projects/1/staff/message",
      body: { text: "Start on the release notes" },
    });
    expect(within(screen.getByRole("log")).getAllByText("Start on the release notes")).toHaveLength(
      1,
    );
    expect(screen.queryByText("Sending…")).not.toBeInTheDocument();
  });

  it("shows Sending… while the daemon hasn't answered", async () => {
    const { user, bridge } = await renderApp({ kind: "chief", id: 1 });
    let release = () => undefined as void;
    const original = bridge.daemonRequest.bind(bridge);
    vi.spyOn(bridge, "daemonRequest").mockImplementation(async (method, path, body) => {
      if (path.endsWith("/staff/message")) await new Promise<void>((r) => (release = r));
      return original(method, path, body);
    });
    await user.type(
      screen.getByRole("textbox", { name: "Message the Chief of Staff" }),
      "Hold on{Enter}",
    );
    expect(within(screen.getByRole("log")).getByText("Hold on")).toBeInTheDocument();
    expect(screen.getAllByText("Sending…").length).toBeGreaterThan(0);
    await act(async () => {
      release();
      await settle();
    });
    expect(screen.queryByText("Sending…")).not.toBeInTheDocument();
  });

  it("shows the empty state before it starts, and a suggestion fills the composer", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 2 });
    expect(main().getByText("Not started")).toBeInTheDocument();
    expect(main().getByRole("heading", { name: "Coordinate opal-app" })).toBeInTheDocument();
    const suggestions = [
      "Plan the next feature and split it across agents",
      "Review what every workspace is doing right now",
      "Collect the open questions from running agents",
    ];
    for (const text of suggestions)
      expect(main().getByRole("button", { name: text })).toBeVisible();
    daemon.requests.length = 0;
    await user.click(main().getByRole("button", { name: suggestions[1] }));
    expect(screen.getByRole("textbox", { name: "Message the Chief of Staff" })).toHaveValue(
      suggestions[1],
    );
    // Filling doesn't send.
    expect(daemon.requests.some((r) => r.method === "POST")).toBe(false);
  });

  it("lists the agents it started that wait on you, and answers them there", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    const strip = within(screen.getByRole("region", { name: "Agents waiting on you" }));
    expect(strip.getByText("Add left-pad to the formatter")).toBeInTheDocument();
    expect(strip.getByText("npm install left-pad --save")).toBeInTheDocument();
    await user.click(strip.getByRole("button", { name: "Allow" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/12",
      body: { decision: "allow" },
    });
    expect(screen.queryByRole("region", { name: "Agents waiting on you" })).not.toBeInTheDocument();
  });

  it("opens its terminal in a split and changes its model", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    await user.click(screen.getByRole("button", { name: "Show the chief's terminal" }));
    expect(screen.getByRole("complementary", { name: "The chief's terminal" })).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Hide the chief's terminal", pressed: true }),
    );
    expect(screen.queryByRole("complementary", { name: "The chief's terminal" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Model: Opus" }));
    await user.click(screen.getByRole("menuitem", { name: "Sonnet" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/projects/1/staff",
      body: { model: "sonnet" },
    });
    expect(screen.getByRole("button", { name: "Model: Sonnet" })).toBeInTheDocument();
  });
});

describe("workspace page", () => {
  it("shows its agents as tabs, the one waiting on you first, with the chief's mark", async () => {
    const { user } = await renderApp({ kind: "workspace", id: 3 });
    const tabs = within(screen.getByRole("tablist", { name: "Agents" }));
    const selected = tabs.getByRole("tab", { selected: true });
    expect(selected).toHaveTextContent("Add left-pad to the formatter");
    expect(
      within(selected).getByRole("img", { name: "Started by the Chief of Staff" }),
    ).toBeVisible();
    expect(screen.getByText("Claude wants to run")).toBeInTheDocument();
    await user.click(tabs.getByRole("tab", { name: /Fix login bug/ }));
    expect(tabs.getByRole("tab", { selected: true })).toHaveTextContent("Fix login bug");
    expect(screen.getByRole("tabpanel", { name: "Fix login bug" })).toBeInTheDocument();
  });

  it("creates a workspace without a title", async () => {
    const { user, daemon, ui } = await renderApp({ kind: "chief", id: 2 });
    const row = sidebar().getByRole("treeitem", { name: "Chief of Staff, opal-app" });
    await user.click(within(row).getByRole("button", { name: "More for opal-app" }));
    await user.click(screen.getByRole("menuitem", { name: "New workspace…" }));
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/opal-app\/<city>/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Create workspace" }));
    await act(() => settle());
    const created = daemon.requests.find((r) => r.path === "/v1/projects/2/workspaces");
    expect(created?.body).toEqual({ base: "origin" });
    const page = ui.store.getState().page;
    expect(page.kind).toBe("workspace");
    expect(main().getByRole("heading", { level: 1 })).toHaveTextContent("lima");
    expect(main().getByText("opal-app/lima")).toBeInTheDocument();
  });
});
