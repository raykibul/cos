// Replying to one message, as in Messenger. The quote travels in the
// message's text, so the chief (or an agent's Claude) reads what you're
// answering, and the app finds it again in the transcript to show it above
// your bubble. The daemon sees plain text and needs no change.

/** A message you picked to answer. */
export interface Reply {
  /** Who wrote it: the one you're talking to, or you. */
  from: "them" | "me";
  /** The part you quote: the message, or the text you selected in it. */
  text: string;
}

/** The longest quote that goes along; past it, the quote ends in "…". */
export const QUOTE_LIMIT = 400;

const HEADERS: Record<Reply["from"], string> = {
  them: "Replying to your message:",
  me: "Replying to my earlier message:",
};

/** The quote as it goes along: trimmed, blank-line runs folded, cut at a word. */
export function quoteText(text: string, limit = QUOTE_LIMIT): string {
  const clean = text
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  if (clean.length <= limit) return clean;
  const cut = clean.slice(0, limit);
  const space = cut.search(/\s\S*$/);
  return `${(space > limit * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/**
 * The text that is sent: a line naming whose message it answers, the quote
 * as a Markdown blockquote, a blank line, then what you wrote.
 */
export function withReply(text: string, reply: Reply | null): string {
  if (!reply) return text;
  const quote = quoteText(reply.text)
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
  const head = `${HEADERS[reply.from]}\n${quote}`;
  return text ? `${head}\n\n${text}` : head;
}

const REPLY = /^Replying to (your message|my earlier message):\n((?:>[^\n]*(?:\n|$))+)\n*/;

/** A sent message split back into its quote and what you wrote, or null when it quotes nothing. */
export function parseReply(text: string): { reply: Reply; body: string } | null {
  const match = REPLY.exec(text);
  if (!match) return null;
  const quote = match[2]!
    .replace(/\n$/, "")
    .split("\n")
    .map((line) => line.replace(/^> ?/, ""))
    .join("\n")
    .trim();
  return {
    reply: { from: match[1] === "your message" ? "them" : "me", text: quote },
    body: text.slice(match[0].length),
  };
}

/**
 * What a Reply button quotes: the text selected inside `element`, when
 * there is some, else the whole message.
 */
export function replyText(element: HTMLElement | null, whole: string): string {
  const selection = typeof window !== "undefined" ? window.getSelection() : null;
  if (element && selection && !selection.isCollapsed && selection.rangeCount > 0) {
    const range = selection.getRangeAt(0);
    if (element.contains(range.startContainer) && element.contains(range.endContainer)) {
      const picked = selection.toString().trim();
      if (picked) return picked;
    }
  }
  return whole;
}

/** The composer's line over the quote: "Replying to Claude", "Replying to yourself". */
export function replyLabel(reply: Reply, name: string): string {
  return `Replying to ${reply.from === "me" ? "yourself" : name}`;
}

/** A quote as the page shows it: one run of plain text, without Markdown's marks. */
export function quotePreview(text: string): string {
  return text
    .replace(/```[^\n]*\n?/g, "")
    .replace(/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+)/gm, "")
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}
