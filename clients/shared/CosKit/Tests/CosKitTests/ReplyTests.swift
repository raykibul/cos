import Testing
@testable import CosKit

/// The same cases as the desktop's `domain/reply.test.ts`, so a reply
/// written on one app reads the same on the other.
@Suite struct ReplyTests {
    @Test func sendsTheQuoteAsABlockquoteUnderWhoseMessageItAnswers() {
        let text = ChatReply.compose("Do the second one first", replyingTo: ChatReply(from: .them, text: "1. Merge lisbon\n\n\n\n2. Trust porto"))
        #expect(text == "Replying to your message:\n> 1. Merge lisbon\n>\n> 2. Trust porto\n\nDo the second one first")
        #expect(ChatReply.compose("Plain", replyingTo: nil) == "Plain")
    }

    @Test func readsASentReplyBackIntoItsQuoteAndWhatYouWrote() throws {
        let sent = ChatReply.compose("And add a test\nfor it", replyingTo: ChatReply(from: .me, text: "Fix the login bug"))
        let parsed = try #require(ChatReply.parse(sent))
        #expect(parsed.reply == ChatReply(from: .me, text: "Fix the login bug"))
        #expect(parsed.body == "And add a test\nfor it")

        // Files only: the quote and nothing else.
        let bare = try #require(ChatReply.parse(ChatReply.compose("", replyingTo: ChatReply(from: .them, text: "Send me the log"))))
        #expect(bare.reply == ChatReply(from: .them, text: "Send me the log"))
        #expect(bare.body == "")

        // A blank line inside the quote stays.
        let lines = try #require(ChatReply.parse("Replying to your message:\n> a\n>\n> b\n\nok"))
        #expect(lines.reply.text == "a\n\nb")

        #expect(ChatReply.parse("> just a quote I typed\n\nhi") == nil)
        #expect(ChatReply.parse("Replying to your message: sure") == nil)
    }

    @Test func keepsTheQuoteShortCutAtAWord() {
        let quote = ChatReply.quoteText(String(repeating: "word ", count: 200))
        #expect(quote.count <= ChatReply.quoteLimit + 1)
        #expect(quote.hasSuffix("word…"))
        #expect(ChatReply.quoteText("  short  ") == "short")
    }

    @Test func showsAQuoteAsPlainTextOnOneRun() {
        let text = "Here's where things stand.\n\n**The login bug is fixed.** `TTL_MS` was\n- off, see [the PR](https://x)"
        #expect(ChatReply.preview(text) == "Here's where things stand. The login bug is fixed. TTL_MS was off, see the PR")
    }
}
