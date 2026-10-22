import { describe, expect, it } from "vitest";
import { contextMeter, formatTokens } from "./contextMeter";

describe("the context meter", () => {
  it("shows a percentage of the window, with the exact numbers in the title", () => {
    expect(contextMeter({ usedTokens: 84_123, windowTokens: 200_000 })).toEqual({
      label: "42% context",
      title: "84,123 of 200,000 tokens in context",
      percent: 42,
      level: "ok",
    });
  });

  it("turns high at 70% and full at 90%, and never passes 100", () => {
    const level = (used: number) => contextMeter({ usedTokens: used, windowTokens: 100 })?.level;
    expect(level(69)).toBe("ok");
    expect(level(70)).toBe("high");
    expect(level(90)).toBe("full");
    expect(contextMeter({ usedTokens: 250, windowTokens: 100 })?.percent).toBe(100);
  });

  it("shows tokens alone when the window isn't known, and nothing before any usage", () => {
    expect(contextMeter({ usedTokens: 84_123, windowTokens: null })).toMatchObject({
      label: "84K context",
      percent: null,
      level: "ok",
    });
    expect(contextMeter(null)).toBeNull();
    expect(contextMeter({ usedTokens: 0, windowTokens: 200_000 })).toBeNull();
  });

  it("formats token counts", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(84_123)).toBe("84K");
    expect(formatTokens(1_000_000)).toBe("1M");
    expect(formatTokens(1_240_000)).toBe("1.2M");
  });
});
