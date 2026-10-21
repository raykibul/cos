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
  it("lists plans, questions, your approvals and trust dialogs, not the chief's", async () => {
    await renderApp();
    expect(screen.getByRole("heading", { name: "Waiting on you", level: 1 })).toBeInTheDocument();
    expect(page().getByText("Move the parser to a streaming tokenizer")).toBeInTheDocument();
    expect(screen.getByText(/30-day revenue rollup/)).toBeInTheDocument();
    expect(screen.getByText("npm install left-pad --save")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Trust folder" })).toBeInTheDocument();
    // Routed to the chief of run 10: not yours.
    expect(screen.queryByText("cargo build --release")).not.toBeInTheDocument();
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

  it("answers a question with a choice", async () => {
    const { user, daemon } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Precompute nightly" }));
    await act(() => settle());
    expect(daemon.requests.some((r) => r.path.endsWith("/answer"))).toBe(true);
    expect(screen.queryByText(/30-day revenue rollup/)).not.toBeInTheDocument();
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
  it("shows the approval banner and streams the terminal", async () => {
    await renderApp({ kind: "agent", id: 8 });
    expect(screen.getByText("Claude wants to run")).toBeInTheDocument();
    await waitFor(() => expect(written.join("")).toContain("Do you want to proceed?"));
  });

  it("says the chief is deciding for a chief-routed request", async () => {
    await renderApp({ kind: "agent", id: 25 });
    expect(screen.getByText("The chief is deciding")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Allow" })).not.toBeInTheDocument();
  });

  it("offers Resume for an exited agent", async () => {
    await renderApp({ kind: "agent", id: 5 });
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});

describe("run page", () => {
  it("approves a proposed plan", async () => {
    const { user, daemon } = await renderApp({ kind: "run", id: 11 });
    await act(() => settle());
    expect(screen.getByText("The chief's plan is ready")).toBeInTheDocument();
    expect(screen.getByText("Write a streaming tokenizer")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Approve plan" }));
    await act(() => settle());
    expect(daemon.requests.some((r) => r.path === "/v1/runs/11/approve-plan")).toBe(true);
    expect(screen.queryByText("The chief's plan is ready")).not.toBeInTheDocument();
  });

  it("shows the board with task actions by the rules", async () => {
    const { user } = await renderApp({ kind: "run", id: 10 });
    await act(() => settle());
    // Its worker carries the same title.
    expect(page().getAllByText("Rate-limit the API routes").length).toBeGreaterThan(0);
    // Merged tasks have no actions; blocked ones can be retried.
    expect(screen.queryByRole("button", { name: "Actions for Add a token bucket" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Actions for Rate-limit the API routes" }));
    const items = screen.getAllByRole("menuitem").map((i) => i.textContent);
    expect(items).toEqual(
      expect.arrayContaining([
        expect.stringMatching(/Mark failed/),
        expect.stringMatching(/Cancel task/),
      ]),
    );
  });

  it("asks a reason before failing a task", async () => {
    const { user, daemon } = await renderApp({ kind: "run", id: 10 });
    await act(() => settle());
    await user.click(screen.getByRole("button", { name: "Actions for Rate-limit the API routes" }));
    await user.click(screen.getByRole("menuitem", { name: /Mark failed/ }));
    const dialog = screen.getByRole("dialog");
    const submit = within(dialog).getByRole("button", { name: "Mark failed" });
    expect(submit).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox"), "Out of scope");
    await user.click(submit);
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PATCH",
      path: "/v1/tasks/103",
      body: { state: "failed", reason: "Out of scope" },
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("renders the report of a finished run", async () => {
    await renderApp({ kind: "run", id: 7 });
    await act(() => settle());
    expect(screen.getByRole("heading", { name: "What changed" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel run" })).not.toBeInTheDocument();
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
    expect(ui.store.getState().page.kind).toBe("project");
  });

  it("starts a run with a preset and cap", async () => {
    const { user, daemon } = await renderApp({ kind: "workspace", id: 8 });
    await user.click(screen.getByRole("button", { name: "New run" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "Goal" }), "Speed up parsing");
    await user.click(within(dialog).getByRole("radio", { name: /Strict/ }));
    const cap = within(dialog).getByRole("spinbutton");
    await user.clear(cap);
    await user.type(cap, "5");
    await user.click(within(dialog).getByRole("button", { name: "Start run" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/runs",
      body: { workspace_id: 8, goal: "Speed up parsing", preset: "strict", approval_cap: 5 },
    });
  });
});
