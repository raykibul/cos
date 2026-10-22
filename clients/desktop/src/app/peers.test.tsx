import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { decodeTailnetPeers } from "../api/models";
import type { FakeOptions } from "../bridge/fake";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { type DialogState, type Page, createUiStore } from "./ui";

async function renderApp(
  options: FakeOptions = {},
  page: Page = { kind: "waiting" },
  dialog: DialogState | null = null,
) {
  const harness = fakeStore({ retryMs: 10, ...options });
  const ui = createUiStore({ page, dialog });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const openSheet = (options: FakeOptions = {}) =>
  renderApp(options, { kind: "waiting" }, { kind: "addHost" });

const dialog = () => within(screen.getByRole("dialog"));
const tailnet = () => within(dialog().getByRole("region", { name: "On your tailnet" }));
const row = (name: string) => within(tailnet().getByRole("listitem", { name }));
const sidebar = () => within(screen.getByRole("complementary", { name: "Sidebar" }));

describe("the tailnet in the Add a machine sheet", () => {
  it("lists your devices with a chip each, above the manual fields", async () => {
    await openSheet();
    await waitFor(() =>
      expect(tailnet().getByRole("list", { name: "Your devices" })).toBeVisible(),
    );
    const names = within(tailnet().getByRole("list", { name: "Your devices" }))
      .getAllByRole("listitem")
      .map((item) => item.getAttribute("aria-label"));
    expect(names).toEqual(["mac-mini", "studio", "build-box", "phone", "travel-laptop"]);

    expect(row("mac-mini").getByText("cos running")).toBeInTheDocument();
    expect(row("mac-mini").getByText("macOS · 100.101.102.112")).toBeInTheDocument();
    expect(row("build-box").getByText("Online")).toBeInTheDocument();
    expect(row("phone").getByText("Offline · seen 2h ago")).toBeInTheDocument();
    // Saved already: Studio by name, the travel laptop by address.
    expect(row("studio").getByText("Added")).toBeInTheDocument();
    expect(row("travel-laptop").getByText("Added")).toBeInTheDocument();

    // Only Macs running cos get Connect; added ones offer Switch instead.
    expect(row("mac-mini").getByRole("button", { name: "Connect to mac-mini" })).toBeVisible();
    expect(row("studio").queryByRole("button", { name: /Connect/ })).not.toBeInTheDocument();
    expect(row("studio").getByRole("button", { name: "Switch to Studio" })).toBeVisible();
    expect(row("build-box").queryByRole("button", { name: /Connect/ })).not.toBeInTheDocument();

    // The manual fields stay as the fallback.
    expect(dialog().getByRole("textbox", { name: "Address" })).toBeInTheDocument();
  });

  it("Connect adds the Mac and switches to it", async () => {
    const { user, bridge, cos } = await openSheet();
    await waitFor(() => expect(tailnet().getByText("mac-mini")).toBeVisible());
    await user.click(row("mac-mini").getByRole("button", { name: "Connect to mac-mini" }));
    await act(() => settle());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const added = bridge.hosts.at(-1)!;
    expect(added).toMatchObject({ name: "mac-mini", address: "100.101.102.112:7700" });
    expect(bridge.current).toBe(added.id);
    expect(cos.get().host?.id).toBe(added.id);
  });

  it("clicking a device without cos fills the form and says what to run there", async () => {
    const { user } = await openSheet();
    await waitFor(() => expect(tailnet().getByText("build-box")).toBeVisible());
    await user.click(row("build-box").getByRole("button", { name: /build-box/ }));
    expect(dialog().getByRole("textbox", { name: "Address" })).toHaveValue("100.101.102.115:7700");
    expect(dialog().getByRole("textbox", { name: "Name" })).toHaveValue("build-box");
    expect(tailnet().getByText(/build-box isn't running cos/)).toHaveTextContent(
      "Install cos there and run cos daemon install first.",
    );
    expect(tailnet().getByRole("listitem", { name: "build-box" })).toHaveAttribute(
      "aria-current",
      "true",
    );
  });

  it("hides devices that aren't yours behind a toggle, without actions", async () => {
    const { user } = await openSheet();
    await waitFor(() => expect(tailnet().getByText("mac-mini")).toBeVisible());
    expect(tailnet().queryByText("ci-runner")).not.toBeInTheDocument();
    await user.click(tailnet().getByRole("button", { name: /Show 1 other device/ }));
    const others = within(tailnet().getByRole("list", { name: "Other devices" }));
    const ci = within(others.getByRole("listitem", { name: "ci-runner" }));
    expect(ci.getByText("linux · 100.101.102.130 · tagged")).toBeInTheDocument();
    expect(ci.queryByRole("button")).not.toBeInTheDocument();
  });

  it("treats a device with no address as a plain row", async () => {
    const harness = fakeStore();
    harness.daemon.peers.push({
      id: "nNOIP",
      name: "no-ip",
      hostname: "no-ip",
      dns_name: "",
      os: "macOS",
      ips: [],
      online: true,
      last_seen_ms: null,
      mine: true,
      tagged: false,
      cos: "unknown",
      address: "",
    });
    const ui = createUiStore({ dialog: { kind: "addHost" } });
    render(<App cos={harness.cos} ui={ui} />);
    await act(async () => {
      await harness.cos.start();
      await settle();
    });
    await waitFor(() => expect(tailnet().getByText("no-ip")).toBeVisible());
    expect(row("no-ip").getByText("Offline")).toBeInTheDocument();
    expect(row("no-ip").queryByRole("button")).not.toBeInTheDocument();
  });

  it("says when there are no other devices", async () => {
    const harness = fakeStore();
    harness.daemon.peers = [];
    const ui = createUiStore({ dialog: { kind: "addHost" } });
    render(<App cos={harness.cos} ui={ui} />);
    await act(async () => {
      await harness.cos.start();
      await settle();
    });
    await waitFor(() =>
      expect(tailnet().getByText("No other devices on your tailnet.")).toBeVisible(),
    );
  });

  it("shows Tailscale's reason when it can't list the tailnet", async () => {
    await openSheet({ tailscaleOff: true });
    await waitFor(() =>
      expect(tailnet().getByText(/Tailscale is not running/)).toBeInTheDocument(),
    );
    expect(tailnet().queryByRole("list")).not.toBeInTheDocument();
  });

  it("notes a stopped daemon and still adds a Mac by hand", async () => {
    const { user, bridge } = await openSheet({ running: false });
    await waitFor(() =>
      expect(
        tailnet().getByText(
          "This Mac's cos daemon isn't running, so your tailnet can't be listed.",
        ),
      ).toBeInTheDocument(),
    );
    await user.type(dialog().getByRole("textbox", { name: "Address" }), "mini");
    await user.click(dialog().getByRole("button", { name: "Add and connect" }));
    await act(() => settle());
    expect(bridge.hosts.at(-1)).toMatchObject({ name: "mini", address: "mini:7700" });
  });

  it("asks This Mac's daemon even while another Mac is in use", async () => {
    const { user, bridge, ui } = await renderApp({ host: "h1" });
    bridge.approve("h1");
    act(() => ui.open({ kind: "addHost" }));
    await waitFor(() => expect(tailnet().getByText("mac-mini")).toBeVisible());
    expect(bridge.daemon.requests.some((r) => r.path === "/v1/tailnet/peers")).toBe(true);
    expect(bridge.daemons.get("h1")!.requests.some((r) => r.path.startsWith("/v1/tailnet"))).toBe(
      false,
    );
    // Studio is in use, so its row offers nothing to switch to.
    expect(row("studio").queryByRole("button")).not.toBeInTheDocument();
    await user.click(row("build-box").getByRole("button", { name: /build-box/ }));
    expect(dialog().getByRole("textbox", { name: "Address" })).toHaveValue("100.101.102.115:7700");
  });
});

describe("the tailnet in the host menu", () => {
  it("lists only Macs running cos that aren't saved, and adds one in a click", async () => {
    const { user, bridge } = await renderApp();
    await user.click(sidebar().getByRole("button", { name: /^Machine:/ }));
    await waitFor(() => expect(screen.getByText("On your tailnet")).toBeInTheDocument());
    const items = screen.getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toEqual([
      expect.stringMatching(/^This Mac/),
      expect.stringMatching(/^Studio/),
      expect.stringMatching(/^Travel laptop/),
      "mac-minicos running · 100.101.102.112",
      "Rename This Mac…",
      "Add a machine…",
      expect.stringMatching(/^Settings/),
    ]);
    await user.click(screen.getByRole("menuitem", { name: /^mac-mini/ }));
    await act(() => settle());
    expect(bridge.hosts.at(-1)).toMatchObject({
      name: "mac-mini",
      address: "100.101.102.112:7700",
    });
    expect(bridge.current).toBe(bridge.hosts.at(-1)!.id);
  });

  it("hides the section when nothing new runs cos", async () => {
    const harness = fakeStore();
    harness.daemon.peers = harness.daemon.peers.filter((p) => p.name !== "mac-mini");
    const ui = createUiStore();
    render(<App cos={harness.cos} ui={ui} />);
    await act(async () => {
      await harness.cos.start();
      await settle();
    });
    const user = userEvent.setup();
    await user.click(sidebar().getByRole("button", { name: /^Machine:/ }));
    await act(() => settle());
    expect(screen.queryByText("On your tailnet")).not.toBeInTheDocument();
    expect(screen.getAllByRole("menuitem")).toHaveLength(6);
  });
});

describe("the tailnet in Settings", () => {
  it("lists your devices read-only", async () => {
    await renderApp({}, { kind: "settings" });
    const section = within(await screen.findByRole("region", { name: "Devices on your tailnet" }));
    await waitFor(() => expect(section.getByText("mac-mini")).toBeVisible());
    expect(section.getByText("Offline · seen 2h ago")).toBeInTheDocument();
    expect(section.queryByText("ci-runner")).not.toBeInTheDocument();
    expect(section.queryByRole("button", { name: /Connect/ })).not.toBeInTheDocument();
  });
});

describe("decoding the peers", () => {
  it("is lenient", () => {
    const decoded = decodeTailnetPeers({
      peers: [
        { id: "n1", name: "box", ips: ["fd7a::1", "100.1.2.3"], online: true, cos: "running" },
        { hostname: "only-hostname", address: "" },
        { nothing: true },
        "junk",
      ],
      self_name: null,
      reason: null,
    });
    expect(decoded.selfName).toBeNull();
    expect(decoded.peers).toHaveLength(2);
    expect(decoded.peers[0]).toMatchObject({
      name: "box",
      address: "100.1.2.3:7700",
      mine: false,
      lastSeenMs: null,
    });
    expect(decoded.peers[1]).toMatchObject({ name: "only-hostname", address: "", cos: "unknown" });
  });
});
