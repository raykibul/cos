import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { fakeStore, requestLines, settle } from "../test/harness";
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

const tabs = () => within(screen.getByRole("tablist", { name: "Open pages" }));
/** The tabs' titles, without the chief's ★. */
const tabTitles = () =>
  tabs()
    .getAllByRole("tab")
    .map((tab) => tab.textContent?.replace("★", ""));
const sidebar = () => within(screen.getByRole("complementary", { name: "Sidebar" }));

describe("tabs", () => {
  it("open a tab for each page, named for what it shows", async () => {
    const { user } = await renderApp();
    await user.click(sidebar().getByRole("button", { name: "Chief of Staff, cos" }));
    await user.keyboard("{Meta>}");
    await user.click(sidebar().getByRole("treeitem", { name: /Fix login bug/ }));
    await user.keyboard("{/Meta}");
    expect(tabTitles()).toEqual(["Waiting on you", "cos", "Fix login bug"]);
    expect(tabs().getByRole("tab", { selected: true })).toHaveTextContent("Fix login bug");
    expect(tabs().getByRole("tab", { name: "cos" })).toHaveAttribute(
      "title",
      "Chief of Staff · cos",
    );
  });

  it("select a page's tab instead of opening another", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    act(() => ui.navigate({ kind: "waiting" }));
    await user.click(tabs().getByRole("tab", { name: "cos" }));
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
    await user.click(sidebar().getByRole("button", { name: "Chief of Staff, cos" }));
    expect(tabTitles()).toEqual(["cos", "Waiting on you"]);
  });

  it("close with ×, a middle click or ⌘W", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    act(() => {
      ui.navigate({ kind: "project", id: 1 });
      ui.navigate({ kind: "settings" });
      ui.navigate({ kind: "events" });
    });
    await user.click(tabs().getByRole("button", { name: "Close cos workspaces" }));
    expect(tabTitles()).toEqual(["cos", "Settings", "Event log"]);

    await user.pointer({
      keys: "[MouseMiddle]",
      target: tabs().getByRole("tab", { name: "Settings" }),
    });
    expect(tabTitles()).toEqual(["cos", "Event log"]);

    await user.keyboard("{Meta>}w{/Meta}");
    expect(tabTitles()).toEqual(["cos"]);
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
  });

  it("move with ⌃Tab", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    act(() => ui.navigate({ kind: "project", id: 1 }));
    await user.keyboard("{Control>}{Tab}{/Control}");
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
  });

  it("show only the project you're in, and come back as you left them", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    act(() => {
      ui.navigate({ kind: "agent", id: 5 });
      ui.navigate({ kind: "events" });
    });
    expect(tabTitles()).toEqual(["cos", "Bump dependencies", "Event log"]);

    // Another project's page: its own tabs.
    const rail = within(screen.getByRole("navigation", { name: "Projects" }));
    await user.click(rail.getByRole("button", { name: /^Chief of Staff, opal-app/ }));
    expect(tabTitles()).toEqual(["opal-app"]);
    expect(sidebar().getByRole("heading", { name: "opal-app" })).toBeInTheDocument();
    act(() => ui.navigate({ kind: "settings" }));
    expect(tabTitles()).toEqual(["opal-app", "Settings"]);

    // Back to cos, on the tab it was on.
    await user.click(rail.getByRole("button", { name: /^Chief of Staff, cos/ }));
    expect(tabTitles()).toEqual(["cos", "Bump dependencies", "Event log"]);
    expect(tabs().getByRole("tab", { selected: true })).toHaveTextContent("Event log");
    // Once there, the tile opens the chief.
    await user.click(rail.getByRole("button", { name: /^Chief of Staff, cos/ }));
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });

    // A session of another project takes you to its tabs too.
    act(() => ui.navigate({ kind: "agent", id: 4 }));
    expect(tabTitles()).toEqual(["opal-app", "Settings", "Export invoices as CSV"]);
  });

  it("drop the tabs of a removed agent", async () => {
    const { user, ui } = await renderApp({ kind: "agent", id: 5 });
    act(() => ui.open({ kind: "removeAgent", agentId: 5 }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(ui.store.getState().tabs.some((t) => t.kind === "agent" && t.id === 5)).toBe(false);
  });
});

describe("a session's right-click menu", () => {
  const menu = () => within(screen.getByRole("menu"));
  const dialog = () => within(screen.getByRole("dialog"));
  const rightClick = async (user: ReturnType<typeof userEvent.setup>, name: RegExp) =>
    user.pointer({ keys: "[MouseRight]", target: sidebar().getByRole("treeitem", { name }) });

  it("starts another session in the same folder", async () => {
    const { user } = await renderApp({ kind: "chief", id: 1 });
    await rightClick(user, /^Write the release notes/);
    await user.click(menu().getByRole("menuitem", { name: "New session in porto" }));
    expect(screen.getByRole("dialog", { name: "New session in porto" })).toBeInTheDocument();
  });

  it("removes only the session when others share its folder", async () => {
    const { user, ui, daemon } = await renderApp({ kind: "agent", id: 5 });
    await rightClick(user, /^Bump dependencies/);
    await user.click(menu().getByRole("menuitem", { name: /Remove session/ }));
    expect(dialog().getByText(/porto stays/)).toBeInTheDocument();
    expect(dialog().queryByRole("checkbox")).not.toBeInTheDocument();
    await user.click(dialog().getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(requestLines(daemon)).toContain("DELETE /v1/agents/5");
    expect(requestLines(daemon).some((line) => line.startsWith("DELETE /v1/workspaces"))).toBe(
      false,
    );
    expect(sidebar().queryByRole("treeitem", { name: /^Bump dependencies/ })).toBeNull();
    expect(ui.store.getState().tabs.some((t) => t.kind === "agent" && t.id === 5)).toBe(false);
  });

  it("removes a shared folder with all its sessions, stopping the running ones", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    await rightClick(user, /^Write the release notes/);
    await user.click(menu().getByRole("menuitem", { name: /^Remove porto…/ }));
    expect(
      dialog().getByText(/sessions go with it: .*Write the release notes/),
    ).toBeInTheDocument();
    await user.click(dialog().getByRole("button", { name: /remove/i }));
    await act(() => settle());
    expect(requestLines(daemon)).toContain("DELETE /v1/workspaces/4");
    expect(sidebar().queryByRole("treeitem", { name: /^Write the release notes/ })).toBeNull();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("offers no folder removal for a session alone in it: Remove session does that", async () => {
    const { user } = await renderApp({ kind: "chief", id: 2 });
    await rightClick(user, /^Export invoices as CSV/);
    expect(menu().queryByRole("menuitem", { name: /^Remove Billing export…/ })).toBeNull();
  });

  it("deletes the folder with a session alone in it", async () => {
    const { user, ui, daemon } = await renderApp({ kind: "workspace", id: 8 });
    act(() => ui.navigate({ kind: "chief", id: 2 }));
    await rightClick(user, /^Export invoices as CSV/);
    await user.click(menu().getByRole("menuitem", { name: /Remove session/ }));
    expect(
      dialog().getByRole("checkbox", { name: /Delete its folder, Billing export/ }),
    ).toBeChecked();
    await user.click(dialog().getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(requestLines(daemon)).toContain("DELETE /v1/workspaces/8");
    expect(requestLines(daemon)).not.toContain("DELETE /v1/agents/4");
    expect(sidebar().queryByRole("treeitem", { name: /^Export invoices/ })).toBeNull();
    expect(ui.store.getState().tabs.some((t) => t.kind === "workspace" && t.id === 8)).toBe(false);
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 2 });
  });

  it("stops a running session before removing it", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 2 });
    await rightClick(user, /^Explore the billing service/);
    await user.click(menu().getByRole("menuitem", { name: /Remove session/ }));
    await user.click(dialog().getByRole("button", { name: "Stop and remove" }));
    await act(() => settle());
    const lines = requestLines(daemon);
    expect(lines).toContain("POST /v1/agents/3/stop");
    expect(lines.indexOf("DELETE /v1/workspaces/9")).toBeGreaterThan(
      lines.indexOf("POST /v1/agents/3/stop"),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("keeps a folder with uncommitted changes, and can remove just the session", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    // Fix login bug and Add left-pad share lisbon; remove one, then the other.
    await rightClick(user, /^Fix login bug/);
    await user.click(menu().getByRole("menuitem", { name: /Remove session/ }));
    await user.click(dialog().getByRole("button", { name: "Stop and remove" }));
    await act(() => settle());
    expect(requestLines(daemon)).toContain("DELETE /v1/agents/7");

    await rightClick(user, /^Add left-pad/);
    await user.click(menu().getByRole("menuitem", { name: /Remove session/ }));
    await user.click(dialog().getByRole("button", { name: "Stop and remove" }));
    await act(() => settle());
    expect(dialog().getByText(/uncommitted changes.*The folder stays/)).toBeInTheDocument();
    await user.click(dialog().getByRole("checkbox", { name: /Delete its folder/ }));
    await user.click(dialog().getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(requestLines(daemon)).toContain("DELETE /v1/agents/8");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("⌘K switcher", () => {
  const dialog = () => within(screen.getByRole("dialog", { name: "Jump to" }));

  it("opens with ⌘K, filters as you type and opens with Return", async () => {
    const { user, ui } = await renderApp();
    await user.keyboard("{Meta>}k{/Meta}");
    expect(dialog().getByRole("group", { name: "Chiefs" })).toBeInTheDocument();
    await user.keyboard("opal");
    expect(dialog().getByRole("option", { selected: true })).toHaveTextContent(
      "Chief of Staff · opal-app",
    );
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog", { name: "Jump to" })).not.toBeInTheDocument();
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 2 });
  });

  it("finds the event log", async () => {
    const { user, ui } = await renderApp();
    await user.keyboard("{Meta>}k{/Meta}event{Enter}");
    expect(ui.store.getState().page).toEqual({ kind: "events" });
  });

  it("messages a chief with Tab without leaving the page", async () => {
    const { user, ui, daemon } = await renderApp({ kind: "settings" });
    await user.keyboard("{Meta>}k{/Meta}cos{Tab}");
    const box = dialog().getByRole("textbox", { name: "Message the chief of cos" });
    expect(box).toHaveFocus();
    await user.keyboard("Ship the tray menu{Enter}");
    await act(() => settle());
    expect(screen.queryByRole("dialog", { name: "Jump to" })).not.toBeInTheDocument();
    expect(ui.store.getState().page).toEqual({ kind: "settings" });
    expect(daemon.requests).toContainEqual(
      expect.objectContaining({
        method: "POST",
        path: "/v1/projects/1/staff/message",
        body: { text: "Ship the tray menu" },
      }),
    );
  });

  it("goes back from a message with Escape, and closes with another", async () => {
    const { user } = await renderApp();
    await user.keyboard("{Meta>}k{/Meta}{Tab}");
    expect(dialog().getByRole("textbox", { name: /Message the chief/ })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(dialog().getByRole("combobox", { name: "Jump to" })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Jump to" })).not.toBeInTheDocument();
  });
});

describe("focus on open", () => {
  const chiefInput = () => screen.getByRole("textbox", { name: "Message the Chief of Staff" });

  it("puts the cursor in the chief's box from the rail, a tab or ⌘K", async () => {
    const { user } = await renderApp();
    const rail = within(screen.getByRole("navigation", { name: "Projects" }));
    await user.click(rail.getByRole("button", { name: /^Chief of Staff, cos/ }));
    expect(chiefInput()).toHaveFocus();

    await user.click(tabs().getByRole("tab", { name: "Waiting on you" }));
    await user.click(tabs().getByRole("tab", { name: "cos" }));
    expect(chiefInput()).toHaveFocus();

    await user.keyboard("{Meta>}k{/Meta}opal{Enter}");
    await act(() => settle());
    expect(chiefInput()).toHaveFocus();
  });

  it("does it again when you open the chief that's showing", async () => {
    const { user } = await renderApp({ kind: "chief", id: 1 });
    act(() => chiefInput().blur());
    expect(chiefInput()).not.toHaveFocus();
    await user.keyboard("{Meta>}k{/Meta}cos{Enter}");
    await act(() => settle());
    expect(chiefInput()).toHaveFocus();
  });

  it("puts the cursor in a workspace's new agent box", async () => {
    const { ui } = await renderApp();
    act(() => ui.navigate({ kind: "workspace", id: 4 }));
    await act(() => settle());
    expect(screen.getByRole("textbox", { name: "Prompt" })).toHaveFocus();
  });

  it("puts the cursor in a session's message box, beside the chief or in its tab", async () => {
    const { user } = await renderApp();
    await user.click(sidebar().getByRole("treeitem", { name: /^Write the release notes/ }));
    await act(() => settle());
    await waitFor(() =>
      expect(
        screen.getByRole("textbox", { name: "Message Write the release notes" }),
      ).toHaveFocus(),
    );

    await user.keyboard("{Meta>}");
    await user.click(sidebar().getByRole("treeitem", { name: /^Write the release notes/ }));
    await user.keyboard("{/Meta}");
    await act(() => settle());
    expect(screen.getByRole("textbox", { name: "Message Write the release notes" })).toHaveFocus();
  });

  it("hides and shows the sidebar with ⌘B and its button", async () => {
    const { user } = await renderApp({ kind: "chief", id: 1 });
    await user.keyboard("{Meta>}b{/Meta}");
    expect(screen.queryByRole("complementary", { name: "Sidebar" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show the sidebar" }));
    expect(screen.getByRole("complementary", { name: "Sidebar" })).toBeInTheDocument();
  });
});

describe("status bar and settings", () => {
  it("says what's working and opens Waiting on you", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    const status = within(screen.getByRole("contentinfo", { name: "Status" }));
    expect(status.getByText(/agents? working/)).toBeInTheDocument();
    await user.click(status.getByRole("button", { name: /waiting on you/ }));
    expect(ui.store.getState().page).toEqual({ kind: "waiting" });
  });

  it("keeps the event log in Settings", async () => {
    const { user, ui } = await renderApp({ kind: "settings" });
    expect(screen.queryByRole("button", { name: "Event log" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open the event log" }));
    expect(ui.store.getState().page).toEqual({ kind: "events" });
  });
});
