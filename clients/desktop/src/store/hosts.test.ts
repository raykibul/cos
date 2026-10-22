import { describe, expect, it } from "vitest";
import { CosError } from "../api/errors";
import { parseAddress } from "../bridge/fake";
import { nameFromAddress, quotedCommands, refusalCopy } from "../domain/hosts";
import { fakeStore, rawEvent, requestLines, settle } from "../test/harness";
import { pendingDevices, waitingCount } from "./selectors";

async function started(options: Parameters<typeof fakeStore>[0] = {}) {
  const harness = fakeStore({ retryMs: 10, ...options });
  await harness.cos.start();
  await settle();
  return harness;
}

/** Waits until `check` passes, letting timers run. */
async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 200; i++) {
    if (check()) return;
    await settle(2, 5);
  }
  throw new Error(`timed out: ${what}`);
}

describe("hosts", () => {
  it("starts on This Mac and knows every host", async () => {
    const { cos } = await started();
    const s = cos.get();
    expect(s.host).toEqual({ id: "local", name: "This Mac", address: null });
    expect(s.hosts.map((h) => h.name)).toEqual(["This Mac", "Studio", "Travel laptop"]);
    expect(s.connection).toBe("running");
  });

  it("switching resets the state and follows the new host's events", async () => {
    const { cos, bridge } = await started();
    expect(cos.get().projects.length).toBeGreaterThan(0);
    bridge.approve("h1");
    const studio = bridge.daemons.get("h1")!;

    await cos.switchHost("h1");
    // Nothing from This Mac is kept while Studio loads.
    expect(cos.get().host?.name).toBe("Studio");
    expect(cos.get().devices).toEqual([]);
    await until(() => cos.get().connection === "running", "connected to Studio");
    await settle();
    expect(cos.get().projects.length).toBeGreaterThan(0);
    // One stream, on Studio's daemon; This Mac's was closed.
    expect(bridge.openStreams).toBe(1);
    expect(bridge.daemon.followers.size).toBe(0);
    expect(studio.followers.size).toBe(1);
    expect(requestLines(studio)).toContain("GET /v1/projects");
    // Devices and the listener are managed only on This Mac.
    expect(requestLines(studio)).not.toContain("GET /v1/devices");
    expect(cos.get().tailnet).toBeNull();

    // Studio's events reach the store; This Mac's don't.
    studio.log("studio.only", {});
    bridge.daemon.log("this_mac.only", {});
    await settle();
    const kinds = cos.get().events.map((e) => e.kind);
    expect(kinds.at(-1)).toBe("studio.only");
    expect(kinds).not.toContain("this_mac.only");
  });

  it("drops what the old host's stream sends after a switch", async () => {
    const { cos, bridge } = await started();
    bridge.approve("h1");
    const follower = [...bridge.daemon.followers.values()][0]!;
    await cos.switchHost("h1");
    await until(() => cos.get().connection === "running", "connected to Studio");
    const before = cos.get().events.length;
    // A late message from This Mac's stream, already closed by the switch.
    follower.onMessage({ type: "disconnected", reason: "gone", notRunning: true });
    follower.onMessage({ type: "event", event: rawEvent(9999, "late.event") });
    await settle();
    expect(cos.get().connection).toBe("running");
    expect(cos.get().events).toHaveLength(before);
  });

  it("forgets the old host's chiefs and conversations", async () => {
    const { cos, bridge } = await started();
    bridge.approve("h1");
    const unwatchOld = cos.watchTranscript(40);
    await settle();
    expect(cos.isWatchingTranscript(40)).toBe(true);

    await cos.switchHost("h1");
    expect(cos.get().transcripts).toEqual({});
    expect(cos.isWatchingTranscript(40)).toBe(false);
    // A page on Studio watches the same id; the old page letting go late
    // doesn't end it.
    const unwatchNew = cos.watchTranscript(40);
    unwatchOld();
    expect(cos.isWatchingTranscript(40)).toBe(true);
    unwatchNew();
    expect(cos.isWatchingTranscript(40)).toBe(false);
  });

  it("a refusal is its own connection state, until the host approves", async () => {
    const { cos, bridge } = await started();
    await cos.switchHost("h1");
    await until(() => cos.get().connection === "refused", "refused");
    expect(cos.get().refusal).toEqual({
      code: "device_pending",
      message:
        "this device (my-mac) is waiting for approval. On studio, run `cos devices approve 4`",
    });
    bridge.approve("h1");
    await until(() => cos.get().connection === "running", "approved");
    expect(cos.get().refusal).toBeNull();
  });

  it("an unreachable host is not running, with the address in the reason", async () => {
    const { cos } = await started();
    await cos.switchHost("h2");
    await until(() => cos.get().connection === "not_running", "unreachable");
    expect(cos.get().connectionReason).toContain("could not reach 100.101.102.105:7700");
  });

  it("a request refused by the host carries the code", async () => {
    const { cos } = await started();
    await cos.switchHost("h1");
    const error = await cos.client.projects().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CosError);
    expect((error as CosError).status).toBe(403);
    expect((error as CosError).code).toBe("device_pending");
  });

  it("adds, removes and goes back to This Mac when the host in use goes", async () => {
    const { cos, bridge } = await started();
    const mini = await cos.addHost("Mini", "mini");
    expect(mini).toEqual({ id: "h3", name: "Mini", address: "mini:7700" });
    expect(cos.get().hosts.map((h) => h.id)).toEqual(["local", "h1", "h2", "h3"]);
    await expect(cos.addHost("Again", "mini:7700")).rejects.toMatchObject({
      kind: "bad_request",
    });

    await cos.switchHost("h3");
    expect(cos.get().host?.id).toBe("h3");
    await cos.removeHost("h3");
    expect(cos.get().host?.id).toBe("local");
    expect(bridge.current).toBe("local");
    await until(() => cos.get().connection === "running", "back on This Mac");
    expect(cos.get().hosts.map((h) => h.id)).toEqual(["local", "h1", "h2"]);

    // Removing another host keeps the one in use.
    await cos.removeHost("h2");
    expect(cos.get().host?.id).toBe("local");
    expect(cos.get().hosts.map((h) => h.id)).toEqual(["local", "h1"]);
  });
});

describe("remote access", () => {
  it("loads the listener and devices on This Mac", async () => {
    const { cos } = await started();
    const s = cos.get();
    expect(s.tailnet?.state).toBe("listening");
    expect(s.tailnet?.addresses).toContain("100.101.102.104:7700");
    expect(s.devices.map((d) => d.state)).toEqual(["approved", "pending", "denied"]);
  });

  it("counts pending devices as waiting on you", async () => {
    const { cos } = await started();
    const without = { ...cos.get(), devices: [] };
    expect(waitingCount(cos.get())).toBe(waitingCount(without) + 1);
    expect(pendingDevices(cos.get()).map((d) => d.name)).toEqual(["studio"]);

    await cos.approveDevice(2);
    expect(waitingCount(cos.get())).toBe(waitingCount(without));
  });

  it("refreshes devices when one asks in", async () => {
    const { cos, daemon } = await started();
    const before = waitingCount(cos.get());
    daemon.requests.length = 0;
    daemon.requestDevice("ipad", "My iPad", "iPadOS");
    await settle();
    expect(requestLines(daemon)).toEqual(["GET /v1/devices"]);
    expect(pendingDevices(cos.get()).map((d) => d.name)).toEqual(["studio", "ipad"]);
    expect(waitingCount(cos.get())).toBe(before + 1);
  });

  it("turns the listener off and on, and keeps the status from the event", async () => {
    const { cos, daemon } = await started();
    await cos.setTailnet(false);
    expect(daemon.requests.at(-1)).toEqual({
      method: "PUT",
      path: "/v1/tailnet",
      body: { enabled: false },
    });
    expect(cos.get().tailnet?.state).toBe("off");
    await cos.setTailnet(true, 7701);
    await settle();
    expect(cos.get().tailnet).toMatchObject({ enabled: true, port: 7701, state: "listening" });
  });

  it("denies and removes devices", async () => {
    const { cos } = await started();
    await cos.denyDevice(1);
    expect(cos.get().devices.find((d) => d.id === 1)?.state).toBe("denied");
    await cos.removeDevice(3);
    await settle();
    expect(cos.get().devices.map((d) => d.id)).toEqual([1, 2]);
  });
});

describe("host words", () => {
  it("reads addresses as the shell does", () => {
    expect(parseAddress("my-mac")).toBe("my-mac:7700");
    expect(parseAddress("100.1.2.3:7701")).toBe("100.1.2.3:7701");
    expect(parseAddress("fd7a:115c:a1e0::1")).toBe("[fd7a:115c:a1e0::1]:7700");
    for (const bad of ["", "http://x", "my mac", "x:0", "x:70000"]) {
      expect(() => parseAddress(bad)).toThrow();
    }
  });

  it("names a host from its address", () => {
    expect(nameFromAddress("my-mac.tail1234.ts.net:7700")).toBe("my-mac");
    expect(nameFromAddress("100.101.102.103")).toBe("100.101.102.103");
    expect(nameFromAddress("[fd7a::1]:7700")).toBe("fd7a::1");
    expect(nameFromAddress("fd7a::1")).toBe("fd7a::1");
    expect(nameFromAddress("studio:7701")).toBe("studio");
  });

  it("finds the commands a refusal names", () => {
    expect(
      quotedCommands("On studio, run `cos devices approve 2` or `cos devices`, then retry"),
    ).toEqual(["cos devices approve 2", "cos devices"]);
    expect(refusalCopy("device_pending", "Studio").title).toBe("Waiting for approval on Studio");
    expect(refusalCopy("something_new", "Studio").title).toBe("Studio refused this Mac");
  });
});
