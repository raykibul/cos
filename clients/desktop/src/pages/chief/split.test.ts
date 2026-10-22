import { describe, expect, it } from "vitest";
import { MIN_SIDE, clampWidth, defaultWidth } from "./split";

describe("the chief's split", () => {
  it("keeps both sides at least MIN_SIDE wide", () => {
    expect(clampWidth(100, 1200)).toBe(MIN_SIDE);
    expect(clampWidth(1100, 1200)).toBe(1200 - MIN_SIDE);
    expect(clampWidth(500.4, 1200)).toBe(500);
    // Too narrow for both: the terminal keeps its floor.
    expect(clampWidth(500, 600)).toBe(MIN_SIDE);
    // Not laid out yet: only the floor applies.
    expect(clampWidth(5000, 0)).toBe(5000);
  });

  it("defaults to 46% of the page, at most 760 px", () => {
    expect(defaultWidth(1200)).toBe(552);
    expect(defaultWidth(2400)).toBe(760);
    expect(defaultWidth(700)).toBe(MIN_SIDE);
  });
});
