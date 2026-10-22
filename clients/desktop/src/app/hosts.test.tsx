import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { FakeOptions } from "../bridge/fake";
import { waitingCount } from "../store/selectors";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { type Page, createUiStore } from "./ui";

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
  await user.click(sidebar().getByRole("button", { name: /^Mac:/ }));
  await user.click(screen.getByRole("menuitem", { name }));
  await act(() => settle());
}

describe("host picker", () => {
  it("shows This Mac, the saved Macs and Add a Mac…", async () => {
    const { user } = await renderApp();
    const picker = sidebar().getByRole("button", { name: "Mac: This Mac" });
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
      "Add a Mac…",
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
    expect(sidebar().getByRole("button", { name: "Mac: Studio" })).toBeInTheDocument();
    // The footer names the host; no device count from This Mac.
    expect(sidebar().getByText(/^cos v0\.1\.0/).parentElement).toHaveTextContent("Studio");
    expect(screen.queryByText("Devices asking to reach this Mac")).not.toBeInTheDocument();
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
    await user.click(sidebar().getByRole("button", { name: /^Mac:/ }));
    await user.click(screen.getByRole("menuitem", { name: "Add a Mac…" }));
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
    await user.click(sidebar().getByRole("button", { name: "Settings" }));
    expect(main().getByText("Managed on Studio")).toBeInTheDocument();
    expect(main().queryByRole("switch")).not.toBeInTheDocument();
    expect(bridge.daemons.get("h1")!.requests.some((r) => r.path === "/v1/tailnet")).toBe(false);
  });

  it("removes a saved Mac", async () => {
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
