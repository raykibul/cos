import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { type Page, createUiStore } from "./ui";

// The emulator needs WebAssembly and a canvas; components get a stand-in
// that records what the session writes.
const written: string[] = [];
vi.mock("../terminal/engine", () => ({
  loadEngine: () => Promise.resolve(null),
  createEngine: () =>
    Promise.resolve({
      open: () => undefined,
      write: (bytes: Uint8Array) => written.push(new TextDecoder().decode(bytes)),
      reset: () => undefined,
      resize: () => undefined,
      fitSize: () => ({ cols: 100, rows: 30 }),
      focus: () => undefined,
      onData: () => ({ dispose: () => undefined }),
      dispose: () => undefined,
    }),
}));

beforeEach(() => {
  written.length = 0;
});

/** The page, without the sidebar (which repeats titles). */
const page = () => within(screen.getByRole("main"));

async function renderApp(page: Page = { kind: "waiting" }, running = true) {
  const harness = fakeStore({ running });
  const ui = createUiStore({ page });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

describe("Waiting on you", () => {
  it("lists your approvals, trust dialogs and Claude logins", async () => {
    await renderApp();
    expect(screen.getByRole("heading", { name: "Waiting on you", level: 1 })).toBeInTheDocument();
    expect(screen.getByText("npm install left-pad --save")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Trust folder" })).toBeInTheDocument();
    expect(screen.getByText("Claude needs you to log in")).toBeInTheDocument();
  });

  it("allows a request, which then leaves the list", async () => {
    const { user, daemon } = await renderApp();
    const card = screen.getByText("npm install left-pad --save").closest("article")!;
    await user.click(within(card).getByRole("button", { name: "Allow" }));
    await act(() => settle());
    expect(screen.queryByText("npm install left-pad --save")).not.toBeInTheDocument();
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/12",
      body: { decision: "allow" },
    });
  });

  it("answers the chosen request from the keyboard", async () => {
    const harness = fakeStore();
    const first = harness.daemon.approvals[0]!;
    harness.daemon.approvals.push({
      ...first,
      id: 13,
      agent_id: 7,
      tool_input: { command: "cargo test", description: "Run the tests" },
      summary: "Bash cargo test",
      created_ms: Number(first.created_ms) + 1000,
    });
    const ui = createUiStore({ page: { kind: "waiting" } });
    render(<App cos={harness.cos} ui={ui} />);
    await act(async () => {
      await harness.cos.start();
      await settle();
    });
    const user = userEvent.setup();
    const card = (text: string) => screen.getByText(text).closest("article")!;
    expect(card("npm install left-pad --save")).toHaveAttribute("aria-current", "true");
    expect(
      within(card("npm install left-pad --save")).getByRole("button", { name: "Allow" }),
    ).toHaveAttribute("aria-keyshortcuts", "Meta+Enter");
    await user.keyboard("{ArrowDown}");
    expect(card("cargo test")).toHaveAttribute("aria-current", "true");
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    await act(() => settle());
    expect(harness.daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/13",
      body: { decision: "deny" },
    });
    // The one left takes its place; alone, it needs no ring.
    expect(card("npm install left-pad --save")).not.toHaveAttribute("aria-current");
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    await act(() => settle());
    expect(harness.daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/12",
      body: { decision: "allow" },
    });
  });

  it("shows the waiting count in the sidebar", async () => {
    const { cos } = await renderApp();
    const { waitingCount } = await import("../store/selectors");
    const nav = within(screen.getByRole("navigation")).getByRole("button", {
      name: /Waiting on you/,
    });
    expect(nav).toHaveTextContent(String(waitingCount(cos.get())));
  });
});

describe("daemon not running", () => {
  it("shows how to start it", async () => {
    await renderApp({ kind: "waiting" }, false);
    expect(screen.getByRole("heading", { name: "cos isn't running" })).toBeInTheDocument();
    expect(screen.getByText("cos daemon")).toBeInTheDocument();
    expect(screen.getByText("cos daemon install")).toBeInTheDocument();
  });
});

describe("event log", () => {
  it("lists events newest first and filters them", async () => {
    const { user } = await renderApp({ kind: "events" });
    const rows = screen.getAllByRole("listitem");
    expect(rows.length).toBeGreaterThan(10);
    await user.click(screen.getByRole("tab", { name: "Approvals" }));
    for (const row of screen.getAllByRole("listitem")) {
      expect(row).toHaveTextContent(/approval\./);
    }
  });
});

describe("workspace page", () => {
  it("starts an agent from the composer and opens it", async () => {
    const { user, daemon, ui } = await renderApp({ kind: "workspace", id: 3 });
    expect(screen.getByRole("heading", { name: "Fix the login flow" })).toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Prompt" }), "Add a changelog");
    await user.click(screen.getByRole("button", { name: /Model:/ }));
    await user.click(screen.getByRole("menuitem", { name: /Haiku/ }));
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("textbox", { name: "Prompt" }));
    await user.keyboard("{Enter}");
    await act(() => settle());
    const created = daemon.requests.find((r) => r.path === "/v1/workspaces/3/agents");
    expect(created?.body).toEqual({
      prompt: "Add a changelog",
      model: "haiku",
      permission_mode: "default",
    });
    expect(ui.store.getState().page.kind).toBe("agent");
  });

  it("lists your agents but not a run's", async () => {
    await renderApp({ kind: "workspace", id: 3 });
    expect(page().getByText("Fix login bug")).toBeInTheDocument();
    expect(screen.queryByText(/^Worker:/)).not.toBeInTheDocument();
  });
});

describe("agent page", () => {
  it("shows the conversation and the approval, and the terminal on Ctrl-J", async () => {
    const { user } = await renderApp({ kind: "agent", id: 8 });
    expect(screen.getByText("Claude wants to run")).toBeInTheDocument();
    const log = page().getByRole("log", { name: "Conversation with Claude" });
    expect(within(log).getByText(/The table columns pad by hand/)).toBeInTheDocument();
    // The chief's line is cut from the first prompt and shown as where it came from.
    expect(
      within(log).getByText("Add left-pad to the formatter and use it for the table columns."),
    ).toBeInTheDocument();
    expect(within(log).getByText(/From the Chief of Staff/)).toBeInTheDocument();
    expect(page().queryByRole("complementary", { name: "Claude's terminal" })).toBeNull();
    // Typing would land on the permission prompt, so the box waits.
    expect(
      page().getByRole("textbox", { name: "Message Add left-pad to the formatter" }),
    ).toBeDisabled();
    await user.keyboard("{Control>}j{/Control}");
    expect(page().getByRole("complementary", { name: "Claude's terminal" })).toBeInTheDocument();
    await waitFor(() => expect(written.join("")).toContain("Do you want to proceed?"));
  });

  it("sends a message to the agent from its composer", async () => {
    const { user, daemon } = await renderApp({ kind: "agent", id: 7 });
    const box = page().getByRole("textbox", { name: "Message Fix login bug" });
    await user.type(box, "Also cover the refresh token{Enter}");
    await act(() => settle());
    const sent = daemon.requests.find((r) => r.path === "/v1/agents/7/input");
    expect(sent?.body).toEqual({ text: "Also cover the refresh token" });
    expect(box).toHaveValue("");
    expect(page().getByText("Also cover the refresh token")).toBeInTheDocument();
  });

  it("keeps the chief's own agent page as its terminal", async () => {
    await renderApp({ kind: "agent", id: 40 });
    expect(page().queryByRole("log")).toBeNull();
    expect(page().queryByRole("button", { name: /the terminal/ })).toBeNull();
    await waitFor(() => expect(written.join("")).not.toBe(""));
  });

  it("shows a pending request with why cos asks you", async () => {
    await renderApp({ kind: "agent", id: 8 });
    expect(page().getByText("Claude wants to run")).toBeInTheDocument();
    expect(
      page().getByText(/cos asks you because it changes or downloads dependencies/),
    ).toBeInTheDocument();
    expect(page().getByRole("button", { name: "Allow" })).toBeInTheDocument();
  });

  it("allows its request with ⌘↩", async () => {
    const { user, daemon } = await renderApp({ kind: "agent", id: 8 });
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/12",
      body: { decision: "allow" },
    });
  });

  it("offers Resume for an exited agent", async () => {
    await renderApp({ kind: "agent", id: 5 });
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});

describe("sheets", () => {
  it("adds a project by path and shows the daemon's error as-is", async () => {
    const { user, ui } = await renderApp();
    await user.click(screen.getAllByRole("button", { name: /Add project/ })[0]!);
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "relative/path");
    await user.click(within(dialog).getByRole("button", { name: "Add project" }));
    await act(() => settle());
    expect(within(dialog).getByRole("alert")).toBeInTheDocument();

    const input = within(dialog).getByRole("textbox");
    await user.clear(input);
    await user.type(input, "/Users/me/code/new-repo");
    await user.click(within(dialog).getByRole("button", { name: "Add project" }));
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(ui.store.getState().page.kind).toBe("chief");
  });
});
