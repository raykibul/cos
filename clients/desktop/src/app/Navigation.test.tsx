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
    await user.click(sidebar().getByRole("treeitem", { name: /Fix login bug/ }));
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
      ui.navigate({ kind: "chief", id: 2 });
      ui.navigate({ kind: "settings" });
      ui.navigate({ kind: "events" });
    });
    await user.click(tabs().getByRole("button", { name: "Close opal-app" }));
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
    act(() => ui.navigate({ kind: "chief", id: 2 }));
    await user.keyboard("{Control>}{Tab}{/Control}");
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
  });

  it("drop the tabs of a removed agent", async () => {
    const { user, ui } = await renderApp({ kind: "agent", id: 5 });
    act(() => ui.open({ kind: "removeAgent", agentId: 5 }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(ui.store.getState().tabs.some((t) => t.kind === "agent" && t.id === 5)).toBe(false);
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
