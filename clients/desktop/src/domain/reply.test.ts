import { describe, expect, it } from "vitest";
import { QUOTE_LIMIT, parseReply, quotePreview, quoteText, replyLabel, withReply } from "./reply";

describe("replies", () => {
  it("sends the quote as a blockquote under whose message it answers", () => {
    const text = withReply("Do the second one first", {
      from: "them",
      text: "1. Merge lisbon\n\n\n\n2. Trust porto",
    });
    expect(text).toBe(
      "Replying to your message:\n> 1. Merge lisbon\n>\n> 2. Trust porto\n\nDo the second one first",
    );
    expect(withReply("Plain", null)).toBe("Plain");
  });

  it("reads a sent reply back into its quote and what you wrote", () => {
    const sent = withReply("And add a test\nfor it", { from: "me", text: "Fix the login bug" });
    expect(parseReply(sent)).toEqual({
      reply: { from: "me", text: "Fix the login bug" },
      body: "And add a test\nfor it",
    });
    // Files only: the quote and nothing else.
    expect(parseReply(withReply("", { from: "them", text: "Send me the log" }))).toEqual({
      reply: { from: "them", text: "Send me the log" },
      body: "",
    });
    expect(parseReply("> just a quote I typed\n\nhi")).toBeNull();
    expect(parseReply("Replying to your message: sure")).toBeNull();
  });

  it("keeps the quote short, cut at a word", () => {
    const long = "word ".repeat(200);
    const quote = quoteText(long);
    expect(quote.length).toBeLessThanOrEqual(QUOTE_LIMIT + 1);
    expect(quote).toMatch(/word…$/);
    expect(quoteText("  short  ")).toBe("short");
  });

  it("shows a quote as plain text on one run", () => {
    expect(
      quotePreview(
        "Here's where things stand.\n\n**The login bug is fixed.** `TTL_MS` was\n- off, see [the PR](https://x)",
      ),
    ).toBe("Here's where things stand. The login bug is fixed. TTL_MS was off, see the PR");
  });

  it("names whose message the composer answers", () => {
    expect(replyLabel({ from: "them", text: "x" }, "Claude")).toBe("Replying to Claude");
    expect(replyLabel({ from: "me", text: "x" }, "Claude")).toBe("Replying to yourself");
  });
});
