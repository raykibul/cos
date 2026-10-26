import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { FakeOptions } from "../bridge/fake";
import { waitingCount } from "../store/selectors";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { type Page, createUiStore } from "./ui";

// The emulator needs WebAssembly and a canvas: terminal tabs get a stand-in.
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
  const harness = fakeStore({ retryMs: 10, ...options });
  const ui = createUiStore({ page });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const main = () => within(screen.getByRole("main"));
const sidebar = () => within(screen.getByRole("complementary", { name: "Sidebar" }));

async function pickHost(user: ReturnType<typeof userEvent.setup>, name: string | RegExp) {
  await user.click(sidebar().getByRole("button", { name: /^Machine:/ }));
  await user.click(screen.getByRole("menuitem", { name }));
  await act(() => settle());
}

describe("host picker", () => {
  it("shows This Mac, the saved machines, Rename and Add a machine…", async () => {
    const { user } = await renderApp();
    const picker = sidebar().getByRole("button", { name: "Machine: This Mac" });
    expect(picker).toHaveTextContent("T");
    await user.click(picker);
    // A Mac on the tailnet running cos joins once This Mac's daemon answers.
    await waitFor(() => expect(screen.getByText("On your tailnet")).toBeInTheDocument());
    const items = screen.getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toEqual([
      expect.stringMatching(/^This Mac/),
      expect.stringMatching(/^Studio.*studio:7700/),
      expect.stringMatching(/^Travel laptop.*100\.101\.102\.105:7700/),
      expect.stringMatching(/^mac-mini/),
      "Rename This Mac…",
      "Add a machine…",
      expect.stringMatching(/^Settings/),
    ]);
  });

  it("switching host resets the store, follows the new host and says so in the footer", async () => {
    const { user, bridge, cos, ui } = await renderApp({ kind: "project", id: 1 });
    bridge.approve("h1");
    await pickHost(user, /^Studio/);
    expect(ui.store.getState().page).toEqual({ kind: "waiting" });
    await waitFor(() => expect(cos.get().connection).toBe("running"));
    expect(bridge.current).toBe("h1");
    expect(bridge.daemons.get("h1")!.followers.size).toBe(1);
    expect(bridge.daemon.followers.size).toBe(0);
    expect(sidebar().getByRole("button", { name: "Machine: Studio" })).toBeInTheDocument();
    // The footer names the host; no device count from This Mac.
    expect(sidebar().getByText(/^cos v0\.1\.0/).parentElement).toHaveTextContent("Studio");
    expect(screen.queryByText("Devices asking to reach this Mac")).not.toBeInTheDocument();
  });
});

describe("terminals on another machine", () => {
  async function onStudio() {
    const app = await renderApp();
    app.bridge.approve("h1");
    await pickHost(app.user, /^Studio/);
    await waitFor(() => expect(app.cos.get().connection).toBe("running"));
    act(() => app.ui.navigate({ kind: "chief", id: 1 }));
    await act(() => settle());
    return { ...app, studio: app.bridge.daemons.get("h1")! };
  }

  it("opens one on that machine with Ctrl-J and lists it in the sidebar", async () => {
    const { user, cos, studio } = await onStudio();
    const root = cos.get().projects.find((p) => p.id === 1)!.root;
    // + is offered there too.
    expect(sidebar().getByRole("button", { name: "New terminal in cos" })).toBeInTheDocument();
    await user.keyboard("{Control>}j{/Control}");
    await act(() => settle());
    expect(studio.requests).toContainEqual({
      method: "POST",
      path: "/v1/shells",
      body: { cwd: root, project_id: 1 },
    });
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Terminal 1 · cos");
    const listed = within(sidebar().getByRole("list", { name: "Terminals in cos" }));
    expect(listed.getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Terminal 1"]);
  });

  it("says to update cos there when that machine's cos is too old for it", async () => {
    const { user, studio } = await onStudio();
    studio.shells.hostOnly = true;
    await user.keyboard("{Control>}j{/Control}");
    await act(() => settle());
    const toast = screen.getByText(/^The cos on Studio is older/).closest("[role=status]");
    expect(toast).toHaveTextContent("Couldn't open a terminal");
    expect(toast).toHaveTextContent("Update cos there to open them from this Mac.");
    expect(screen.queryByRole("list", { name: "Terminals in cos" })).toBeNull();
  });
});

describe("renaming a machine", () => {
  it("renames the host in use from the picker; Return saves", async () => {
    const { user, bridge } = await renderApp();
    await user.click(sidebar().getByRole("button", { name: "Machine: This Mac" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename This Mac…" }));
    const dialog = within(screen.getByRole("dialog"));
    const input = dialog.getByLabelText("Name");
    expect(input).toHaveFocus();
    await user.clear(input);
    await user.type(input, "  MacBook Pro {Enter}");
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(sidebar().getByRole("button", { name: "Machine: MacBook Pro" })).toBeInTheDocument();
    expect(bridge.hosts[0]).toMatchObject({ id: "local", name: "MacBook Pro" });
  });

  it("shows why a name is refused and keeps the sheet open", async () => {
    const { user, cos } = await renderApp({ kind: "settings" });
    await user.click(main().getByRole("button", { name: "Rename Travel laptop" }));
    const dialog = within(screen.getByRole("dialog"));
    await user.clear(dialog.getByLabelText("Name"));
    await user.type(dialog.getByLabelText("Name"), "studio{Enter}");
    await act(() => settle());
    expect(dialog.getByText('"studio" is already the name of another machine')).toBeInTheDocument();
    expect(cos.get().hosts.map((h) => h.name)).toEqual(["This Mac", "Studio", "Travel laptop"]);
    // Too long is caught before asking.
    await user.clear(dialog.getByLabelText("Name"));
    await user.type(dialog.getByLabelText("Name"), "x".repeat(61));
    expect(dialog.getByText("At most 60 characters.")).toBeInTheDocument();
    expect(dialog.getByRole("button", { name: "Rename" })).toBeDisabled();
  });

  it("Escape cancels without renaming", async () => {
    const { user, cos } = await renderApp({ kind: "settings" });
    await user.click(main().getByRole("button", { name: "Rename Studio" }));
    const input = within(screen.getByRole("dialog")).getByLabelText("Name");
    await user.type(input, " 2");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(cos.get().hosts.map((h) => h.name)).toContain("Studio");
  });
});

describe("refusal screens", () => {
  it("waits for approval with the host's command, then lets you in", async () => {
    const { user, bridge } = await renderApp();
    await pickHost(user, /^Studio/);
    await waitFor(() =>
      expect(
        main().getByRole("heading", { name: "Waiting for approval on Studio" }),
      ).toBeInTheDocument(),
    );
    expect(main().getByText("cos devices approve 4")).toBeInTheDocument();
    expect(main().getByText(/Checking again every few seconds/)).toBeInTheDocument();

    // Approved on Studio: the next retry gets in, with no click.
    bridge.approve("h1");
    await waitFor(() =>
      expect(main().getByRole("heading", { name: "Waiting on you", level: 1 })).toBeInTheDocument(),
    );
  });

  it.each([
    ["device_denied", "Studio turned this Mac away"],
    ["wrong_user", "Signed in to Tailscale as someone else"],
    ["tagged_device", "This Mac is a tagged Tailscale device"],
    ["unknown_peer", "Studio doesn't know this Mac"],
    ["tailscale_unavailable", "Tailscale on Studio didn't answer"],
  ])("explains %s", async (code, title) => {
    const { user, bridge } = await renderApp();
    const message =
      code === "device_denied"
        ? "this device (my-mac) was denied. On studio, run `cos devices approve 4` to let it in"
        : `refused: ${code}`;
    bridge.refuse("h1", code, message, code === "tailscale_unavailable" ? 503 : 403);
    await pickHost(user, /^Studio/);
    await waitFor(() => expect(main().getByRole("heading", { name: title })).toBeInTheDocument());
    expect(main().getByText(message)).toBeInTheDocument();
    if (code === "device_denied") {
      expect(main().getByText("cos devices approve 4")).toBeInTheDocument();
    }
  });

  it("says when a host can't be reached, with its address, and switches back", async () => {
    const { user, cos } = await renderApp();
    await pickHost(user, /^Travel laptop/);
    await waitFor(() =>
      expect(
        main().getByRole("heading", { name: "Can't reach Travel laptop" }),
      ).toBeInTheDocument(),
    );
    expect(main().getByText("100.101.102.105:7700")).toBeInTheDocument();
    expect(sidebar().getByText("Can't reach it")).toBeInTheDocument();

    await user.click(main().getByRole("button", { name: "Switch to This Mac" }));
    await waitFor(() => expect(cos.get().connection).toBe("running"));
    expect(cos.get().host?.id).toBe("local");
  });
});

describe("add a Mac", () => {
  it("saves the host, connects to it and shows its approval page", async () => {
    const { user, bridge } = await renderApp();
    await user.click(sidebar().getByRole("button", { name: /^Machine:/ }));
    await user.click(screen.getByRole("menuitem", { name: "Add a machine…" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("cos daemon install")).toBeInTheDocument();

    await user.type(within(dialog).getByRole("textbox", { name: "Address" }), "http://mini");
    await user.click(within(dialog).getByRole("button", { name: "Add and connect" }));
    await act(() => settle());
    expect(within(dialog).getByRole("alert")).toHaveTextContent("leave out http://");

    const address = within(dialog).getByRole("textbox", { name: "Address" });
    await user.clear(address);
    await user.type(address, "mini.tail1234.ts.net");
    // The name defaults to the first part of the address.
    expect(within(dialog).getByRole("textbox", { name: "Name" })).toHaveAttribute(
      "placeholder",
      "mini",
    );
    await user.click(within(dialog).getByRole("button", { name: "Add and connect" }));
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(bridge.hosts.at(-1)).toMatchObject({
      name: "mini",
      address: "mini.tail1234.ts.net:7700",
    });
    expect(bridge.current).toBe(bridge.hosts.at(-1)!.id);
    await waitFor(() =>
      expect(
        main().getByRole("heading", { name: "Waiting for approval on mini" }),
      ).toBeInTheDocument(),
    );
  });
});

describe("remote access page", () => {
  it("shows the listener and turns it off", async () => {
    const { user, daemon } = await renderApp({ kind: "settings" });
    const toggle = main().getByRole("switch", { name: "Remote access" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(main().getByText("100.101.102.104:7700")).toBeInTheDocument();
    await user.click(toggle);
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/tailnet",
      body: { enabled: false },
    });
    expect(main().getByRole("switch", { name: "Remote access" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("changes the port", async () => {
    const { user, daemon } = await renderApp({ kind: "settings" });
    const port = main().getByRole("textbox", { name: "Port" });
    await user.clear(port);
    await user.type(port, "7701");
    await user.click(main().getByRole("button", { name: "Listen on it" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/tailnet",
      body: { enabled: true, port: 7701 },
    });
    expect(main().getByText("100.101.102.104:7701")).toBeInTheDocument();
  });

  it("shows the firewall fix when the firewall blocks cos", async () => {
    await renderApp({ kind: "settings" }, { firewall: true });
    expect(main().getByText(/The macOS firewall blocks incoming connections/)).toBeInTheDocument();
    expect(
      main().getByText(
        "sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp /Users/me/.cargo/bin/cos",
      ),
    ).toBeInTheDocument();
  });

  it("approves, denies and removes devices", async () => {
    const { user, daemon } = await renderApp({ kind: "settings" });
    const devices = within(main().getByRole("list", { name: "Devices" }));
    await user.click(devices.getByRole("button", { name: "Approve Studio" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/devices/2/approve",
      body: null,
    });
    await user.click(devices.getByRole("button", { name: "Deny Me's iPhone" }));
    await act(() => settle());
    expect(daemon.requests.some((r) => r.path === "/v1/devices/1/deny")).toBe(true);
    await user.click(devices.getByRole("button", { name: "Remove Old MacBook" }));
    await act(() => settle());
    expect(daemon.requests).toContainEqual({
      method: "DELETE",
      path: "/v1/devices/3",
      body: null,
    });
    expect(devices.queryByText("Old MacBook")).not.toBeInTheDocument();
  });

  it("is managed on the Mac itself when another host is in use", async () => {
    const { user, bridge } = await renderApp();
    bridge.approve("h1");
    await pickHost(user, /^Studio/);
    const rail = within(screen.getByRole("navigation", { name: "Projects" }));
    await user.click(rail.getByRole("button", { name: "Settings" }));
    expect(main().getByText("Managed on Studio")).toBeInTheDocument();
    expect(main().queryByRole("switch", { name: "Remote access" })).not.toBeInTheDocument();
    expect(bridge.daemons.get("h1")!.requests.some((r) => r.path === "/v1/tailnet")).toBe(false);
  });

  it("removes a saved machine", async () => {
    const { user, cos } = await renderApp({ kind: "settings" });
    await user.click(main().getByRole("button", { name: "Remove Travel laptop" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await act(() => settle());
    expect(cos.get().hosts.map((h) => h.name)).toEqual(["This Mac", "Studio"]);
    expect(main().queryByText("Travel laptop")).not.toBeInTheDocument();
  });
});

describe("waiting on you", () => {
  it("lists a pending device in the count and approves it there", async () => {
    const { user, cos, daemon } = await renderApp();
    const nav = within(screen.getByRole("navigation")).getByRole("button", {
      name: /Waiting on you/,
    });
    const before = waitingCount(cos.get());
    expect(nav).toHaveTextContent(String(before));
    expect(main().getByText("Devices asking to reach this Mac")).toBeInTheDocument();

    // A new device asking in shows up at once.
    act(() => void daemon.requestDevice("ipad", "My iPad", "iPadOS"));
    await act(() => settle());
    expect(main().getByText("My iPad")).toBeInTheDocument();
    expect(nav).toHaveTextContent(String(before + 1));

    await user.click(main().getByRole("button", { name: "Approve My iPad" }));
    await act(() => settle());
    expect(main().queryByText("My iPad")).not.toBeInTheDocument();
    expect(nav).toHaveTextContent(String(before));
  });
});
