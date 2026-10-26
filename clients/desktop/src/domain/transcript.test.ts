import { describe, expect, it } from "vitest";
import {
  type InboxItem,
  SHELL_OUTPUT,
  type TranscriptMessage,
  decodeTranscript,
} from "../api/staff";
import {
  type PendingMessage,
  conversationItems,
  itemsForUpdate,
  mergeMessages,
  reconcilePending,
  toolTitle,
  updateBody,
  updateCount,
} from "./transcript";

const user = (n: number, text: string, tsMs = n * 1000): TranscriptMessage => ({
  n,
  role: "user",
  kind: "text",
  text,
  tool: null,
  tsMs,
  attachments: [],
});
const said = (n: number, text: string): TranscriptMessage => ({
  n,
  role: "assistant",
  kind: "text",
  text,
  tool: null,
  tsMs: n * 1000,
  attachments: [],
});
const tool = (n: number, name: string, summary = ""): TranscriptMessage => ({
  n,
  role: "assistant",
  kind: "tool",
  text: "",
  tool: { name, summary, isError: false, agentId: null, output: null },
  tsMs: n * 1000,
  attachments: [],
});
const update = (n: number, text: string, tsMs = n * 1000): TranscriptMessage => ({
  n,
  role: "user",
  kind: "update",
  text,
  tool: null,
  tsMs,
  attachments: [],
});
const pending = (id: number, text: string, afterN: number): PendingMessage => ({
  id,
  text,
  afterN,
  sentMs: 0,
  state: "sent",
});

describe("merging transcript pages", () => {
  it("appends a page, dropping messages it already has, oldest first", () => {
    const merged = mergeMessages(
      [user(1, "a"), said(2, "b")],
      [said(2, "b"), said(4, "d"), tool(3, "Bash")],
    );
    expect(merged.map((m) => m.n)).toEqual([1, 2, 3, 4]);
  });

  it("keeps the same array for an empty page, so nothing re-renders", () => {
    const current = [user(1, "a")];
    expect(mergeMessages(current, [])).toBe(current);
  });
});

describe("reconciling what you sent", () => {
  it("drops a pending message once a later user message has its text", () => {
    const messages = [user(1, "Fix it"), said(2, "ok"), user(3, "Fix  it\n")];
    expect(reconcilePending([pending(1, "Fix it", 2)], messages)).toEqual([]);
  });

  it("ignores the same text from before it was sent", () => {
    const list = [pending(1, "Fix it", 2)];
    expect(reconcilePending(list, [user(1, "Fix it"), said(2, "ok")])).toBe(list);
  });

  it("matches each transcript message to one pending message", () => {
    const left = reconcilePending(
      [pending(1, "again", 0), pending(2, "again", 0)],
      [user(1, "again")],
    );
    expect(left.map((p) => p.id)).toEqual([2]);
  });

  it("counts a wakeup that carried the text, as when it was typed while the chief worked", () => {
    const messages = [update(5, "[cos · 2 updates]\n- agent 7 finished\n\nAlso check the docs")];
    expect(reconcilePending([pending(1, "Also check the docs", 4)], messages)).toEqual([]);
  });
});

describe("conversation rows", () => {
  it("groups the chief's text and tools into turns between your messages and updates", () => {
    const items = conversationItems(
      [
        user(1, "Plan it"),
        said(2, "Looking"),
        tool(3, "mcp__cos__agent_spawn", "Fix login"),
        tool(4, "mcp__cos__agent_spawn", "Add left-pad"),
        user(5, ""),
        said(6, "Started both"),
        update(7, "[cos · 2 updates]\n- …"),
        said(8, "Here's where things stand"),
      ],
      [pending(9, "Thanks", 8)],
    );
    expect(items.map((i) => i.type)).toEqual(["user", "turn", "update", "turn", "pending"]);
    const turn = items[1] as Extract<(typeof items)[number], { type: "turn" }>;
    expect(turn.messages.map((m) => m.n)).toEqual([2, 3, 4, 6]);
    const row = items[2] as Extract<(typeof items)[number], { type: "update" }>;
    expect(row.count).toBe(2);
  });

  it("reads an update's count and body", () => {
    expect(updateCount("[cos · 3 updates]\n- a")).toBe(3);
    expect(updateCount("[cos · 1 update]")).toBe(1);
    expect(updateCount("[cos] something")).toBe(1);
    expect(updateBody("[cos · 2 updates]\n- a\n- b")).toBe("- a\n- b");
  });

  it("finds the inbox items an update carried by time", () => {
    const item = (key: string, tsMs: number): InboxItem => ({
      key,
      kind: "stop",
      agentId: 7,
      title: null,
      text: key,
      tsMs,
    });
    const inbox = [item("late", 9_000), item("b", 4_500), item("a", 3_000), item("old", 500)];
    const found = itemsForUpdate(inbox, {
      message: update(5, "[cos · 2 updates]", 5_000),
      sinceMs: 1_000,
    });
    expect(found.map((i) => i.key)).toEqual(["a", "b"]);
  });

  it("names cos's tools in words", () => {
    expect(toolTitle("mcp__cos__agent_spawn")).toBe("Started an agent");
    expect(toolTitle("mcp__cos__something_new")).toBe("something new");
    expect(toolTitle("Bash")).toBe("Bash");
  });
});

describe("commands you ran in Claude's shell mode", () => {
  const raw = (n: number, text: string) => ({ n, role: "user", kind: "text", text, ts_ms: n });

  it("reads an older daemon's <bash-input> as you typed it, and its output as one row", () => {
    const { messages } = decodeTranscript({
      messages: [
        raw(1, "<bash-input>git push origin a -> b</bash-input>"),
        raw(
          2,
          "<bash-stdout>To github.com\n + 1...2 a -&gt; b (forced update)</bash-stdout><bash-stderr></bash-stderr>",
        ),
        raw(3, "<bash-stdout></bash-stdout><bash-stderr>fatal: nope</bash-stderr>"),
      ],
      next: 3,
    });
    expect(messages[0]).toMatchObject({ kind: "text", text: "! git push origin a -> b" });
    expect(messages[1]).toMatchObject({
      kind: "tool",
      tool: {
        name: SHELL_OUTPUT,
        summary: "To github.com",
        output: "To github.com\n + 1...2 a -> b (forced update)",
        isError: false,
      },
    });
    expect(messages[2]!.tool).toMatchObject({ output: "fatal: nope", isError: true });
    const items = conversationItems(messages);
    expect(items.map((item) => item.type)).toEqual(["user", "output", "output"]);
  });

  it("settles what you sent with ! once the transcript has it, so it shows once", () => {
    const pending: PendingMessage[] = [
      { id: 1, text: "! git push origin a", sentMs: 0, afterN: 0, state: "sent" },
    ];
    const { messages } = decodeTranscript({
      messages: [raw(1, "<bash-input>git push origin a</bash-input>")],
      next: 1,
    });
    expect(reconcilePending(pending, messages)).toEqual([]);
  });
});
