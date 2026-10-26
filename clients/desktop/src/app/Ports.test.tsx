import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { FakeOptions } from "../bridge/fake";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { createUiStore } from "./ui";

// The emulator needs WebAssembly and a canvas, which jsdom lacks.
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

async function renderOn(host: string, options: FakeOptions = {}) {
  const harness = fakeStore({ retryMs: 10, host, ...options });
  harness.bridge.approve("h1");
  const ui = createUiStore({ page: { kind: "waiting" } });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const ports = () => within(screen.getByRole("region", { name: "Ports" }));

describe("Ports section", () => {
  it("is hidden on This Mac", async () => {
    await renderOn("local");
    expect(screen.queryByRole("region", { name: "Ports" })).not.toBeInTheDocument();
  });

  it("lists the machine's forwards: a moved port, its state and open connections", async () => {
    await renderOn("h1");
    const rows = ports().getAllByRole("listitem");
    expect(rows.map((row) => row.textContent)).toEqual([":3343→ 3344not running", ":51732"]);
    expect(rows[0]).toHaveAttribute(
      "title",
      expect.stringContaining("localhost:3344 on this Mac → localhost:3343 on Studio"),
    );
    expect(within(rows[1]!).getByLabelText("2 open")).toBeInTheDocument();
  });

  it("adds a port typed in the field, checked as you type", async () => {
    const { user, cos } = await renderOn("h1");
    await user.click(ports().getByRole("button", { name: "Forward a port from Studio" }));
    const field = ports().getByRole("textbox", { name: "Port on Studio" });
    await user.type(field, "5173");
    expect(ports().getByRole("alert")).toHaveTextContent(":5173 is already forwarded.");
    await user.clear(field);
    await user.type(field, "99999");
    expect(ports().getByRole("alert")).toHaveTextContent("A port is 1–65535.");
    await user.clear(field);
    await user.type(field, "8080{Enter}");
    await waitFor(() =>
      expect(ports().queryByRole("textbox", { name: "Port on Studio" })).not.toBeInTheDocument(),
    );
    expect(cos.get().forwards.map((f) => f.remotePort)).toEqual([3343, 5173, 8080]);
    expect(ports().getAllByRole("listitem")[2]).toHaveTextContent(":8080");
  });

  it("takes another local port behind More", async () => {
    const { user, cos } = await renderOn("h1");
    await user.click(ports().getByRole("button", { name: "Forward a port from Studio" }));
    await user.type(ports().getByRole("textbox", { name: "Port on Studio" }), "3000");
    await user.click(ports().getByRole("button", { name: "More" }));
    await user.type(ports().getByRole("textbox", { name: "Local port" }), "4000{Enter}");
    await waitFor(() => expect(cos.get().forwards).toHaveLength(3));
    expect(cos.get().forwards[0]).toMatchObject({ remotePort: 3000, localPort: 4000 });
    expect(ports().getAllByRole("listitem")[0]).toHaveTextContent(":3000→ 4000");
  });

  it("says when the machine's cos is too old", async () => {
    const { user } = await renderOn("h1", { portsTooOld: true });
    await user.click(ports().getByRole("button", { name: "Forward a port from Studio" }));
    await user.type(ports().getByRole("textbox", { name: "Port on Studio" }), "5173{Enter}");
    await waitFor(() =>
      expect(ports().getByRole("alert")).toHaveTextContent(
        "Studio's cos is too old to forward ports. Run `cos upgrade` there.",
      ),
    );
    expect(ports().queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("opens a port in the browser and stops forwarding one", async () => {
    const { user, bridge } = await renderOn("h1");
    await user.click(ports().getByRole("button", { name: "Open localhost:3344" }));
    await waitFor(() => expect(bridge.forwards.opened).toEqual(["http://localhost:3344/"]));
    await user.click(ports().getByRole("button", { name: "Stop forwarding :3343" }));
    await waitFor(() => expect(ports().getAllByRole("listitem")).toHaveLength(1));
  });

  it("⌘K lists each forward and opens it", async () => {
    const { user, bridge, ui } = await renderOn("h1");
    act(() => ui.openSwitcher(true));
    const dialog = within(screen.getByRole("dialog", { name: "Jump to" }));
    await user.type(dialog.getByRole("combobox"), "localhost");
    const options = dialog.getAllByRole("option").map((o) => o.textContent);
    expect(options).toEqual([
      expect.stringMatching(/^Open localhost:3344:3343 on Studio · not running/),
      expect.stringMatching(/^Open localhost:5173:5173 on Studio/),
    ]);
    await user.keyboard("{ArrowDown}{Enter}");
    await waitFor(() => expect(bridge.forwards.opened).toEqual(["http://localhost:5173/"]));
  });
});
