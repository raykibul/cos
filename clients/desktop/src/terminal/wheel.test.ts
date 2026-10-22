import { describe, expect, it } from "vitest";
import { type WheelInput, wheelData } from "./wheel";

// A 100x40 terminal, 800x640 px: cells are 8x16.
const base: WheelInput = {
  deltaY: 0,
  deltaMode: 0,
  x: 84,
  y: 40,
  width: 800,
  height: 640,
  cols: 100,
  rows: 40,
  mouseTracking: true,
  sgr: true,
};

describe("wheelData", () => {
  it("sends SGR wheel reports at the pointer's cell when the app tracks the mouse", () => {
    const state = { carry: 0 };
    // Two lines down at column 11, row 3.
    expect(wheelData({ ...base, deltaY: 32 }, state)).toBe("\x1b[<65;11;3M\x1b[<65;11;3M");
    expect(wheelData({ ...base, deltaY: -16 }, state)).toBe("\x1b[<64;11;3M");
    expect(state.carry).toBe(0);
  });

  it("never sends arrow keys", () => {
    const data = wheelData({ ...base, deltaY: 200 }, { carry: 0 });
    expect(data).not.toContain("\x1b[A");
    expect(data).not.toContain("\x1b[B");
  });

  it("adds up small trackpad movements until they make a line", () => {
    const state = { carry: 0 };
    expect(wheelData({ ...base, deltaY: 6 }, state)).toBe("");
    expect(wheelData({ ...base, deltaY: 6 }, state)).toBe("");
    expect(wheelData({ ...base, deltaY: 6 }, state)).toBe("\x1b[<65;11;3M");
    expect(state.carry).toBeCloseTo(2 / 16);
  });

  it("caps one wheel event at ten reports", () => {
    const data = wheelData({ ...base, deltaY: 10_000 }, { carry: 0 });
    expect(data.split("\x1b[<65").length - 1).toBe(10);
  });

  it("reads line and page delta modes", () => {
    expect(wheelData({ ...base, deltaMode: 1, deltaY: 3 }, { carry: 0 })).toBe(
      "\x1b[<65;11;3M".repeat(3),
    );
    expect(wheelData({ ...base, deltaMode: 2, deltaY: -1 }, { carry: 0 })).toBe(
      "\x1b[<64;11;3M".repeat(10),
    );
  });

  it("uses the X10 encoding without SGR", () => {
    expect(wheelData({ ...base, sgr: false, deltaY: 16 }, { carry: 0 })).toBe(
      `\x1b[M${String.fromCharCode(65 + 32)}${String.fromCharCode(11 + 32)}${String.fromCharCode(3 + 32)}`,
    );
  });

  it("keeps the pointer's cell inside the terminal", () => {
    expect(wheelData({ ...base, x: -5, y: 9_999, deltaY: 16 }, { carry: 0 })).toBe(
      "\x1b[<65;1;40M",
    );
  });

  it("pages without mouse tracking, one key per third of the screen", () => {
    const state = { carry: 0 };
    const off = { ...base, mouseTracking: false };
    // 13 lines make a page on 40 rows.
    expect(wheelData({ ...off, deltaMode: 1, deltaY: 12 }, state)).toBe("");
    expect(wheelData({ ...off, deltaMode: 1, deltaY: 1 }, state)).toBe("\x1b[6~");
    expect(wheelData({ ...off, deltaMode: 1, deltaY: -26 }, state)).toBe("\x1b[5~\x1b[5~");
  });
});
