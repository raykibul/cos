import type { RunState, RunTask, TaskCounts, TaskState, TestCheck } from "../api/models";

export function isRunFinished(state: RunState): boolean {
  return state === "done" || state === "failed" || state === "cancelled";
}

export function isTaskFinished(state: TaskState): boolean {
  return state === "merged" || state === "failed" || state === "cancelled";
}

// Task actions, as the Mac app had them. The daemon is the judge; these only
// decide which buttons are offered.

/** Each task gets one retry, and only a blocked task is retried by hand. */
export function canRetry(task: RunTask): boolean {
  return task.state === "blocked" && task.retries < 1;
}

/** Fail applies to a stuck task; the pipeline fails the others itself. */
export function canFail(task: RunTask): boolean {
  return task.state === "blocked";
}

export function canCancel(task: RunTask): boolean {
  return !isTaskFinished(task.state);
}

export type TaskActionKind = "retry" | "fail" | "cancel";

/** The state each action moves the task to (`PATCH /v1/tasks/{id}`). */
export function targetState(kind: TaskActionKind): TaskState {
  return kind === "retry" ? "ready" : kind === "fail" ? "failed" : "cancelled";
}

/** Retry and fail need a reason (400 without one); cancel doesn't. */
export function needsReason(kind: TaskActionKind): boolean {
  return kind !== "cancel";
}

/** Whether an action is offered for a task in a run in `runState`. */
export function taskActionAllowed(
  kind: TaskActionKind,
  task: RunTask,
  runState: RunState,
): boolean {
  if (isRunFinished(runState)) return false;
  switch (kind) {
    case "retry":
      return canRetry(task);
    case "fail":
      return canFail(task);
    case "cancel":
      return canCancel(task);
  }
}

export function finishedCount(counts: TaskCounts): number {
  return counts.merged + counts.failed + counts.cancelled;
}

export function activeCount(counts: TaskCounts): number {
  return counts.running + counts.checking + counts.review;
}

/** 0…1, finished tasks over all of them. */
export function progress(counts: TaskCounts): number {
  return counts.total === 0 ? 0 : finishedCount(counts) / counts.total;
}

/** `2 of 4 merged · 1 working · 1 blocked`. */
export function countsSummary(counts: TaskCounts): string {
  const parts = [`${counts.merged} of ${counts.total} merged`];
  const active = activeCount(counts);
  if (active > 0) parts.push(`${active} working`);
  if (counts.ready > 0) parts.push(`${counts.ready} ready`);
  if (counts.blocked > 0) parts.push(`${counts.blocked} blocked`);
  if (counts.failed > 0) parts.push(`${counts.failed} failed`);
  if (counts.cancelled > 0) parts.push(`${counts.cancelled} cancelled`);
  return parts.join(" · ");
}

export function testPassed(tests: TestCheck): boolean {
  return tests.status === "passed" || tests.status === "ok";
}

export function testTitle(tests: TestCheck): string {
  switch (tests.status) {
    case "passed":
      return "Tests passed";
    case "failed":
      return "Tests failed" + (tests.exitCode !== null ? ` (exit ${tests.exitCode})` : "");
    case "timed_out":
      return "Tests timed out";
    case "skipped":
      return "No test command";
    default:
      return `Tests: ${tests.status.replaceAll("_", " ")}`;
  }
}

export function scopeProblems(scope: {
  outOfScope: string[];
  protected: string[];
  uncommitted: string[];
  wrongBranch: boolean;
}): string[] {
  const out: string[] = [];
  if (scope.outOfScope.length) out.push(`outside its files: ${scope.outOfScope.join(", ")}`);
  if (scope.protected.length) out.push(`protected paths: ${scope.protected.join(", ")}`);
  if (scope.uncommitted.length) out.push(`uncommitted: ${scope.uncommitted.join(", ")}`);
  if (scope.wrongBranch) out.push("the worktree left its branch");
  return out;
}
