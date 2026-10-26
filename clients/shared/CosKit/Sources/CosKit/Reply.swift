import Foundation

/// Replying to one message, as in Messenger (the desktop's
/// `domain/reply.ts`). The quote travels in the message's text, so the chief
/// (or a session's Claude) reads what you're answering, and an app finds it
/// again in the transcript to show it above your bubble. The daemon sees
/// plain text and needs no change.
public struct ChatReply: Equatable, Hashable, Sendable {
    public enum From: Equatable, Hashable, Sendable {
        /// The one you're talking to: the chief, or a session's Claude.
        case them
        /// You.
        case me
    }

    public var from: From
    /// The part you quote.
    public var text: String

    public init(from: From, text: String) {
        self.from = from
        self.text = text
    }

    /// The longest quote that goes along; past it, the quote ends in "…".
    public static let quoteLimit = 400

    private static func header(_ from: From) -> String {
        switch from {
        case .them: "Replying to your message:"
        case .me: "Replying to my earlier message:"
        }
    }

    /// The quote as it goes along: lines trimmed at the end, runs of blank
    /// lines folded, cut at a word.
    public static func quoteText(_ text: String, limit: Int = quoteLimit) -> String {
        let lines = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
            .components(separatedBy: "\n")
            .map { line in String(line.reversed().drop(while: \.isWhitespace).reversed()) }
        let clean = lines.joined(separator: "\n")
            .replacing(/\n{3,}/, with: "\n\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.count > limit else { return clean }
        let cut = String(clean.prefix(limit))
        if let space = cut.lastIndex(where: \.isWhitespace), cut.distance(from: cut.startIndex, to: space) > limit * 6 / 10 {
            return String(cut[..<space]).trimmingCharacters(in: .whitespaces) + "…"
        }
        return cut.trimmingCharacters(in: .whitespaces) + "…"
    }

    /// The text that is sent: a line naming whose message it answers, the
    /// quote as a Markdown blockquote, a blank line, then what you wrote.
    public static func compose(_ text: String, replyingTo reply: ChatReply?) -> String {
        guard let reply else { return text }
        let quote = quoteText(reply.text)
            .components(separatedBy: "\n")
            .map { $0.isEmpty ? ">" : "> \($0)" }
            .joined(separator: "\n")
        let head = "\(header(reply.from))\n\(quote)"
        return text.isEmpty ? head : "\(head)\n\n\(text)"
    }

    /// A sent message split back into its quote and what you wrote, or
    /// `nil` when it quotes nothing.
    public static func parse(_ text: String) -> (reply: ChatReply, body: String)? {
        guard let match = text.prefixMatch(of: /Replying to (your message|my earlier message):\n((?:>[^\n]*(?:\n|$))+)\n*/) else {
            return nil
        }
        let quote = String(match.2)
            .components(separatedBy: "\n")
            .filter { !$0.isEmpty }
            .map { line in line.hasPrefix("> ") ? String(line.dropFirst(2)) : String(line.dropFirst()) }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let from: From = match.1 == "your message" ? .them : .me
        return (ChatReply(from: from, text: quote), String(text[match.range.upperBound...]))
    }

    /// A quote as a page shows it: one run of plain text, without Markdown's marks.
    public static func preview(_ text: String) -> String {
        text.replacing(/```[^\n]*\n?/, with: "")
            .replacing(/(?m)^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+)/, with: "")
            .replacing(/\*\*|__|~~|`/, with: "")
            .replacing(/\[([^\]]+)\]\([^)]+\)/) { String($0.1) }
            .replacing(/\s+/, with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
