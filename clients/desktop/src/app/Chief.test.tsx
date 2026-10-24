import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FakeOptions } from "../bridge/fake";
import { SPLIT_KEY } from "../pages/chief/split";
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

async function renderApp(page: Page = { kind: "waiting" }, options: FakeOptions = {}) {
  const harness = fakeStore(options);
  const ui = createUiStore({ page });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const sidebar = () => within(screen.getByRole("complementary", { name: "Sidebar" }));
const rail = () => within(screen.getByRole("navigation", { name: "Projects" }));
const main = () => within(screen.getByRole("main"));

/** The session rows of the sidebar, in order. */
function sessionNames(): string[] {
  const tree = sidebar().getByRole("tree", { name: "Sessions" });
  return within(tree)
    .getAllByRole("treeitem")
    .map((row) => row.getAttribute("aria-label")!.split(",")[0]!);
}

describe("project rail", () => {
  it("has a tile per project with its shortcut and what waits on you there", async () => {
    await renderApp();
    const cos = rail().getByRole("button", { name: /^Chief of Staff, cos/ });
    expect(cos).toHaveTextContent("⌘1");
    expect(cos.getAttribute("aria-label")).toMatch(/, \d+ waiting$/);
    expect(rail().getByRole("button", { name: /^Chief of Staff, opal-app/ })).toHaveTextContent(
      "⌘2",
    );
  });

  it("opens a project's chief and shows that project in the sidebar", async () => {
    const { user, ui } = await renderApp();
    await user.click(rail().getByRole("button", { name: /^Chief of Staff, opal-app/ }));
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 2 });
    expect(sidebar().getByRole("heading", { name: "opal-app" })).toBeInTheDocument();
    expect(sidebar().getByRole("button", { name: "Chief of Staff, opal-app" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("opens each chief with ⌘1 to ⌘9, even from a terminal", async () => {
    const { user, ui } = await renderApp({ kind: "agent", id: 5 });
    await user.keyboard("{Meta>}2{/Meta}");
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 2 });
    await user.keyboard("{Meta>}1{/Meta}");
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
    // A project that doesn't exist does nothing.
    await user.keyboard("{Meta>}9{/Meta}");
    expect(ui.store.getState().page).toEqual({ kind: "chief", id: 1 });
  });
});

describe("sidebar", () => {
  it("shows the project's sessions, one row each, with no workspace rows", async () => {
    await renderApp({ kind: "chief", id: 1 });
    expect(sidebar().getByRole("heading", { name: "cos" })).toBeInTheDocument();
    expect(sessionNames()).toEqual([
      "Check the docs for broken links",
      "Try the new parser",
      "Write the release notes",
      "Bump dependencies",
      "Add left-pad to the formatter",
      "Fix login bug",
    ]);
    expect(sidebar().queryByRole("button", { name: /^porto/ })).toBeNull();
    expect(sidebar().queryByText("opal-app")).not.toBeInTheDocument();
  });

  it("joins sessions that share a folder, and names it in the tooltip", async () => {
    await renderApp({ kind: "chief", id: 1 });
    const lisbon = sidebar().getByRole("group", { name: "2 sessions in Fix the login flow" });
    expect(within(lisbon).getAllByRole("treeitem")).toHaveLength(2);
    expect(sidebar().getByRole("treeitem", { name: /^Fix login bug/ })).toHaveAttribute(
      "title",
      expect.stringContaining("Shares Fix the login flow with Add left-pad to the formatter"),
    );
  });

  it("shows the project of the page you're on, and keeps it on pages outside one", async () => {
    const { ui } = await renderApp({ kind: "agent", id: 4 });
    expect(sidebar().getByRole("heading", { name: "opal-app" })).toBeInTheDocument();
    act(() => ui.navigate({ kind: "waiting" }));
    expect(sidebar().getByRole("heading", { name: "opal-app" })).toBeInTheDocument();
    act(() => ui.navigate({ kind: "chief", id: 1 }));
    expect(sidebar().getByRole("heading", { name: "cos" })).toBeInTheDocument();
  });

  it("says what a session needs from you", async () => {
    await renderApp({ kind: "chief", id: 1 });
    expect(
      sidebar().getByRole("treeitem", { name: "Add left-pad to the formatter, Needs approval" }),
    ).toHaveTextContent("Approve");
    expect(
      sidebar().getByRole("treeitem", { name: "Try the new parser, Needs trust" }),
    ).toHaveTextContent("Trust");
  });

  it("opens a session from the sidebar", async () => {
    const { user, ui } = await renderApp({ kind: "chief", id: 1 });
    await user.click(sidebar().getByRole("treeitem", { name: /Fix login bug/ }));
    expect(ui.store.getState().page.kind).toBe("agent");
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
    const items = log
      .getAllByRole("listitem")
      .filter((li) => !li.closest('[aria-label="Files sent"]'))
      .map((li) => li.textContent);
    expect(items).toEqual([
      expect.stringMatching(/^Review and merge lisbon/),
      expect.stringMatching(/^Trust the folder in porto/),
      expect.stringMatching(/^Finish the release notes/),
    ]);
    // A wakeup isn't shown as your message.
    expect(log.queryByText(/\[cos ·/)).not.toBeInTheDocument();
  });

  it("shows the files you sent with your ask", async () => {
    await renderApp({ kind: "chief", id: 1 });
    const sent = within(await screen.findByRole("list", { name: "Files sent" }));
    expect(await sent.findByRole("img", { name: "login-expired.svg" })).toHaveAttribute(
      "src",
      expect.stringMatching(/^data:image\/svg\+xml;base64,/),
    );
    expect(sent.getByText("auth.log")).toBeInTheDocument();
  });

  it("attaches a file, uploads it at once, and sends it with the message", async () => {
    const { user, bridge } = await renderApp({ kind: "chief", id: 1 });
    const input = screen.getByTestId("attach-input");
    const file = new File(["fn main() {}"], "main rs notes.txt", { type: "text/plain" });
    await user.upload(input, file);
    const chips = within(screen.getByRole("list", { name: "Attached files" }));
    expect(await chips.findByText("main-rs-notes.txt")).toBeInTheDocument();
    await user.type(
      screen.getByRole("textbox", { name: "Message the Chief of Staff" }),
      "Read this",
    );
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.queryByRole("list", { name: "Attached files" })).not.toBeInTheDocument();
    const posted = bridge.daemon.requests.find((r) => r.path.endsWith("/staff/message"));
    expect(posted?.body).toEqual({ text: "Read this", attachments: ["main-rs-notes.txt"] });
    const log = within(screen.getByRole("log"));
    expect(await log.findByText("main-rs-notes.txt")).toBeInTheDocument();
  });

  it("sends files without text, and can take one back before sending", async () => {
    const { user, bridge } = await renderApp({ kind: "chief", id: 1 });
    const input = screen.getByTestId("attach-input");
    await user.upload(input, [
      new File(["a"], "a.txt", { type: "text/plain" }),
      new File(["b"], "b.txt", { type: "text/plain" }),
    ]);
    const chips = within(screen.getByRole("list", { name: "Attached files" }));
    await chips.findByText("b.txt");
    await user.click(chips.getByRole("button", { name: "Remove a.txt" }));
    expect(chips.queryByText("a.txt")).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Send" }));
    const posted = bridge.daemon.requests.find((r) => r.path.endsWith("/staff/message"));
    expect(posted?.body).toEqual({ text: "", attachments: ["b.txt"] });
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

  it("denies the first request with ⌘⌫, but not while you're typing", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    const strip = within(screen.getByRole("region", { name: "Agents waiting on you" }));
    expect(strip.getByRole("button", { name: "Deny" })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+Backspace",
    );
    const box = screen.getByRole("textbox", { name: "Message the Chief of Staff" });
    await user.type(box, "hold on");
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    await act(() => settle());
    expect(daemon.requests.some((r) => r.path === "/v1/approvals/12")).toBe(false);
    await user.clear(box);
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/approvals/12",
      body: { decision: "deny" },
    });
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

  it("shows how full its context is, with the exact numbers in the tooltip", async () => {
    await renderApp({ kind: "chief", id: 1 });
    const meter = screen.getByRole("meter", { name: "84,123 of 200,000 tokens in context" });
    expect(meter).toHaveTextContent("42% context");
    expect(meter).toHaveAttribute("aria-valuenow", "42");
  });

  it("pauses coordination from the menu, says so above the composer, and resumes", async () => {
    const { user, daemon } = await renderApp({ kind: "chief", id: 1 });
    expect(screen.queryByRole("region", { name: "Coordination paused" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Chief of Staff actions" }));
    await user.click(screen.getByRole("menuitem", { name: /^Pause coordination/ }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/projects/1/staff",
      body: { paused: true },
    });
    const strip = screen.getByRole("region", { name: "Coordination paused" });
    expect(strip).toHaveTextContent("Coordination paused · no updates waiting");

    await user.click(screen.getByRole("button", { name: "Chief of Staff actions" }));
    expect(screen.getByRole("menuitem", { name: /^Resume coordination/ })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(within(strip).getByRole("button", { name: "Resume" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/projects/1/staff",
      body: { paused: false },
    });
    expect(screen.queryByRole("region", { name: "Coordination paused" })).toBeNull();
  });

  it("counts the updates waiting while paused, and they go once it resumes", async () => {
    const { user, cos } = await renderApp({ kind: "chief", id: 1 }, { staffPaused: true });
    const strip = screen.getByRole("region", { name: "Coordination paused" });
    expect(strip).toHaveTextContent("Coordination paused · 3 updates waiting");
    await user.click(within(strip).getByRole("button", { name: "Resume" }));
    await act(() => settle());
    expect(cos.get().staffs[1]).toMatchObject({ paused: false, updatesWaiting: 0 });
  });
});

describe("the terminal split", () => {
  afterEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  const pane = () => screen.getByRole("complementary", { name: "The chief's terminal" });
  const divider = () => screen.getByRole("separator", { name: "Resize the chief's terminal" });

  it("keeps the width you set for next time, and double-click resets it", async () => {
    const { user } = await renderApp({ kind: "chief", id: 1 });
    await user.click(screen.getByRole("button", { name: "Show the chief's terminal" }));
    // jsdom lays nothing out, so the default is the floor.
    expect(pane().style.width).toBe("360px");
    divider().focus();
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(pane().style.width).toBe("408px");
    expect(localStorage.getItem(SPLIT_KEY)).toBe("408");

    await user.click(
      screen.getByRole("button", { name: "Hide the chief's terminal", pressed: true }),
    );
    await user.click(screen.getByRole("button", { name: "Show the chief's terminal" }));
    expect(pane().style.width).toBe("408px");
    expect(divider()).toHaveAttribute("aria-valuenow", "408");

    await user.dblClick(divider());
    expect(pane().style.width).toBe("360px");
    expect(localStorage.getItem(SPLIT_KEY)).toBeNull();
  });

  it("still opens and resizes when storage refuses", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const { user } = await renderApp({ kind: "chief", id: 1 }, {});
    await user.click(screen.getByRole("button", { name: "Show the chief's terminal" }));
    divider().focus();
    await user.keyboard("{ArrowLeft}");
    expect(pane().style.width).toBe("384px");
  });
});

describe("workspace page", () => {
  it("shows its sessions as cards, the one waiting on you first, with the chief's mark", async () => {
    const { user, ui } = await renderApp({ kind: "workspace", id: 3 });
    const cards = within(main().getByRole("list", { name: "Sessions" })).getAllByRole("article");
    const first = cards[0]!;
    expect(first).toHaveAccessibleName("Add left-pad to the formatter");
    expect(within(first).getByRole("img", { name: "Started by the Chief of Staff" })).toBeVisible();
    expect(within(first).getByText("Claude wants to run")).toBeInTheDocument();
    expect(within(first).getByRole("button", { name: /Allow/ })).toBeInTheDocument();
    const login = cards.find((c) => c.getAttribute("aria-label") === "Fix login bug")!;
    await user.click(within(login).getByRole("button", { name: "Fix login bug" }));
    const page = ui.store.getState().page;
    expect(page.kind).toBe("agent");
  });

  it("creates a workspace without a title", async () => {
    const { user, daemon, ui } = await renderApp({ kind: "chief", id: 2 });
    await user.click(sidebar().getByRole("button", { name: "More for opal-app" }));
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
