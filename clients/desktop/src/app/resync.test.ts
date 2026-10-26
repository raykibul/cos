import { afterEach, describe, expect, it, vi } from "vitest";
import { followWindow } from "./resync";

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("catching up with the daemon", () => {
  afterEach(() => {
    vi.useRealTimers();
    setVisibility("visible");
  });

  it("resyncs on focus, on becoming visible, and every so often while shown", () => {
    vi.useFakeTimers();
    const cos = { resync: vi.fn() };
    const stop = followWindow(cos, window, 1000);

    window.dispatchEvent(new Event("focus"));
    expect(cos.resync).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(1000);
    expect(cos.resync).toHaveBeenCalledTimes(2);

    // Hidden: no polling, and hiding isn't a reason to catch up.
    setVisibility("hidden");
    vi.advanceTimersByTime(3000);
    expect(cos.resync).toHaveBeenCalledTimes(2);

    setVisibility("visible");
    expect(cos.resync).toHaveBeenCalledTimes(3);

    stop();
    window.dispatchEvent(new Event("focus"));
    vi.advanceTimersByTime(3000);
    expect(cos.resync).toHaveBeenCalledTimes(3);
  });
});
