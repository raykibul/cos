import { describe, expect, it } from "vitest";
import { CosError } from "../api/errors";
import { fakeStore, settle } from "../test/harness";

async function onStudio(options: Parameters<typeof fakeStore>[0] = {}) {
  const harness = fakeStore({ retryMs: 10, host: "h1", ...options });
  harness.bridge.approve("h1");
  await harness.cos.start();
  await settle();
  return harness;
}

async function failure(promise: Promise<unknown>): Promise<CosError> {
  try {
    await promise;
  } catch (error) {
    return error as CosError;
  }
  throw new Error("expected a failure");
}

describe("forwarded ports", () => {
  it("This Mac has none, follows none and refuses one", async () => {
    const { cos, bridge } = fakeStore({ retryMs: 10 });
    await cos.start();
    await settle();
    expect(cos.get().forwards).toEqual([]);
    expect(bridge.openStreams).toBe(1);
    const error = await failure(cos.addForward(8080));
    expect(error.kind).toBe("bad_request");
    expect(error.message).toBe("This Mac's ports are already here");
  });

  it("Studio starts with its canned forwards, followed live", async () => {
    const { cos, bridge } = await onStudio();
    expect(cos.get().forwards).toEqual([
      expect.objectContaining({
        remotePort: 3343,
        localPort: 3344,
        remote: "nothing_listening",
        connections: 0,
      }),
      expect.objectContaining({
        remotePort: 5173,
        localPort: 5173,
        remote: "listening",
        connections: 2,
      }),
    ]);
    // A change in the shell reaches the store without asking.
    bridge.forwards.set("h1", 5173, { connections: 5 });
    expect(cos.get().forwards[1]!.connections).toBe(5);
  });

  it("adds, moves up from a busy local port, refuses duplicates and removes", async () => {
    const { cos } = await onStudio();
    const added = await cos.addForward(8080);
    expect(added).toMatchObject({
      remotePort: 8080,
      localPort: 8080,
      via: null,
      remote: "listening",
    });
    // Below 1024 it keeps its port, redirected to one the OS picked.
    expect(await cos.addForward(443)).toMatchObject({ localPort: 443, via: 52443 });
    await cos.removeForward(443);
    expect(cos.get().forwards.map((f) => f.remotePort)).toEqual([3343, 5173, 8080]);

    const moved = await cos.addForward(3000, 3343);
    expect(moved.localPort).toBe(3345);
    expect(moved.message).toMatch(/^localhost:3343 was taken on this Mac, so this is on 3345\./);

    const duplicate = await failure(cos.addForward(5173));
    expect(duplicate.kind).toBe("bad_request");
    expect(duplicate.message).toBe(":5173 is already forwarded");

    await cos.removeForward(8080);
    expect(cos.get().forwards.map((f) => f.remotePort)).toEqual([3000, 3343, 5173]);
  });

  it("opens a forward at its local port", async () => {
    const { cos, bridge } = await onStudio();
    await cos.openForward(3343);
    expect(bridge.forwards.opened).toEqual(["http://localhost:3344/"]);
    expect((await failure(cos.openForward(9999))).kind).toBe("bad_request");
  });

  it("an older cos answers too_old, and nothing is kept", async () => {
    const { cos } = await onStudio({ portsTooOld: true });
    expect(cos.get().forwards).toEqual([]);
    const error = await failure(cos.addForward(5173));
    expect(error.code).toBe("too_old");
    expect(error.message).toBe(
      "Studio's cos is too old to forward ports. Run `cos upgrade` there.",
    );
    expect(cos.get().forwards).toEqual([]);
  });

  it("a machine waiting for approval refuses with its code", async () => {
    const { cos, bridge } = fakeStore({ retryMs: 10, host: "h1" });
    await cos.start();
    await settle();
    const error = await failure(cos.addForward(5173));
    expect(error.code).toBe("device_pending");
    expect(cos.get().forwards).toEqual([]);
    bridge.approve("h1");
    await settle(4, 15);
    expect(cos.get().forwards.map((f) => f.remotePort)).toEqual([3343, 5173]);
  });

  it("an unreachable machine keeps the port and says why", async () => {
    const { cos } = fakeStore({ retryMs: 10, host: "h2" });
    await cos.start();
    await settle();
    const forward = await cos.addForward(5173);
    expect(forward.remote).toBe("unreachable");
    expect(forward.message).toMatch(/^Could not reach /);
  });

  it("switching away closes the stream and empties the list; switching back restores it", async () => {
    const { cos, bridge } = await onStudio();
    await cos.addForward(8080);
    await cos.switchHost("local");
    await settle();
    expect(cos.get().forwards).toEqual([]);
    expect(bridge.openStreams).toBe(1);
    await cos.switchHost("h1");
    await settle();
    expect(cos.get().forwards.map((f) => f.remotePort)).toEqual([3343, 5173, 8080]);
  });
});
