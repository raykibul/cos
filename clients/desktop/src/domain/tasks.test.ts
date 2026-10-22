import { describe, expect, it } from "vitest";
import { decodeTask } from "../api/models";
import {
  canCancel,
  canFail,
  canRetry,
  countsSummary,
  needsReason,
  progress,
  targetState,
  taskActionAllowed,
} from "./tasks";
import { countTasks } from "../api/models";

const task = (state: string, retries = 0) =>
  decodeTask({ id: 1, run_id: 1, title: "T", state, retries });

describe("task actions", () => {
  it("retries a blocked task once", () => {
    expect(canRetry(task("blocked"))).toBe(true);
    expect(canRetry(task("blocked", 1))).toBe(false);
    expect(canRetry(task("running"))).toBe(false);
    expect(canRetry(task("failed"))).toBe(false);
  });

  it("fails only a blocked task", () => {
    expect(canFail(task("blocked"))).toBe(true);
    for (const state of ["proposed", "ready", "running", "checking", "review", "merged"]) {
      expect(canFail(task(state))).toBe(false);
    }
  });

  it("cancels any unfinished task", () => {
    for (const state of ["proposed", "ready", "running", "checking", "review", "blocked"]) {
      expect(canCancel(task(state))).toBe(true);
    }
    for (const state of ["merged", "failed", "cancelled"]) {
      expect(canCancel(task(state))).toBe(false);
    }
  });

  it("offers nothing once the run is finished", () => {
    for (const run of ["done", "failed", "cancelled"]) {
      expect(taskActionAllowed("cancel", task("blocked"), run)).toBe(false);
      expect(taskActionAllowed("retry", task("blocked"), run)).toBe(false);
    }
    expect(taskActionAllowed("retry", task("blocked"), "running")).toBe(true);
  });

  it("maps actions to states, and asks a reason for retry and fail", () => {
    expect(targetState("retry")).toBe("ready");
    expect(targetState("fail")).toBe("failed");
    expect(targetState("cancel")).toBe("cancelled");
    expect(needsReason("retry")).toBe(true);
    expect(needsReason("fail")).toBe(true);
    expect(needsReason("cancel")).toBe(false);
  });
});

describe("counts", () => {
  it("summarizes and measures progress", () => {
    const counts = countTasks([
      task("merged"),
      task("merged"),
      task("running"),
      task("blocked"),
      task("ready"),
    ]);
    expect(counts.total).toBe(5);
    expect(countsSummary(counts)).toBe("2 of 5 merged · 1 working · 1 ready · 1 blocked");
    expect(progress(counts)).toBeCloseTo(0.4);
    expect(progress(countTasks([]))).toBe(0);
  });
});
