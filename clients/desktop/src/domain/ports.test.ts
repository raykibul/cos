import { describe, expect, it } from "vitest";
import type { Forward } from "../bridge/types";
import { checkLocalPort, checkRemotePort, portLabel, portTitle, portTone } from "./ports";

function forward(change: Partial<Forward> = {}): Forward {
  return {
    remotePort: 5173,
    localPort: 5173,
    via: null,
    state: "listening",
    remote: "listening",
    code: null,
    message: null,
    connections: 0,
    ...change,
  };
}

describe("ports", () => {
  it("checks a typed port as you type", () => {
    const forwards = [forward(), forward({ remotePort: 3343, localPort: 3344 })];
    expect(checkRemotePort("", forwards)).toEqual({ port: null, problem: null });
    expect(checkRemotePort(" 8080 ", forwards)).toEqual({ port: 8080, problem: null });
    expect(checkRemotePort("80a", forwards).problem).toBe("Type a port number.");
    expect(checkRemotePort("0", forwards).problem).toBe("A port is 1–65535.");
    expect(checkRemotePort("65536", forwards).problem).toBe("A port is 1–65535.");
    expect(checkRemotePort("5173", forwards).problem).toBe(":5173 is already forwarded.");
    // The local port can be the one another forward's remote port names,
    // but not one already in use here.
    expect(checkLocalPort("3343", forwards)).toEqual({ port: 3343, problem: null });
    expect(checkLocalPort("3344", forwards).problem).toBe("localhost:3344 is :3343.");
  });

  it("reads a forward's state as a tone and a word", () => {
    expect([portTone(forward()), portLabel(forward())]).toEqual(["green", null]);
    const nothing = forward({ remote: "nothing_listening" });
    expect([portTone(nothing), portLabel(nothing)]).toEqual(["amber", "not running"]);
    expect(portTone(forward({ remote: "unknown" }))).toBe("amber");
    const refused = forward({ remote: "refused", code: "device_pending" });
    expect([portTone(refused), portLabel(refused)]).toEqual(["red", "refused"]);
    expect(portLabel(forward({ remote: "refused", code: "too_old" }))).toBe("too old");
    expect(portLabel(forward({ remote: "unreachable" }))).toBe("can't reach");
    expect(portTone(forward({ state: "error" }))).toBe("red");
  });

  it("says where a forward goes, with its latest message", () => {
    expect(portTitle(forward({ localPort: 5174, message: "Moved." }), "Studio")).toBe(
      "localhost:5174 on this Mac → localhost:5173 on Studio\nMoved.",
    );
    expect(portTitle(forward({ remotePort: 443, localPort: 443, via: 8443 }), "Studio")).toBe(
      "localhost:443 on this Mac (redirected to 8443) → localhost:443 on Studio",
    );
  });
});
