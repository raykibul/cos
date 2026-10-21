import Testing
@testable import CosKit

@Suite struct SSEParserTests {
    /// The body of one chunk from the real daemon, then axum's keep-alive.
    let daemonBody = "id: 1\ndata: {\"seq\":1,\"ts\":\"2026-10-21T18:36:19.126Z\",\"kind\":\"daemon.started\",\"data\":{\"pid\":76227,\"version\":\"0.1.0\"}}\n\n:\n\n"

    private func parse(_ pieces: [String]) -> [SSEMessage] {
        var parser = SSEParser()
        return pieces.flatMap { parser.feed(Array($0.utf8)) }
    }

    @Test func parsesDaemonMessageAndSkipsKeepAlive() {
        let messages = parse([daemonBody])
        #expect(messages.count == 1)
        #expect(messages[0].id == "1")
        #expect(messages[0].data.hasPrefix("{\"seq\":1,"))
    }

    @Test func sameResultWhenSplitAtEveryByte() {
        let whole = parse([daemonBody + daemonBody])
        let bytes = parse((daemonBody + daemonBody).map { String($0) })
        #expect(whole.count == 2)
        #expect(bytes == whole)
    }

    @Test func joinsDataLinesAndReadsFields() {
        let messages = parse(["event: note\nid: 7\ndata: one\ndata:two\ndata\n\n"])
        #expect(messages == [SSEMessage(id: "7", event: "note", data: "one\ntwo\n")])
    }

    @Test func handlesCRLFAndCREndings() {
        #expect(parse(["data: a\r\n\r\n"]) == [SSEMessage(id: nil, event: nil, data: "a")])
        #expect(parse(["data: a\r\r"]) == [SSEMessage(id: nil, event: nil, data: "a")])
        // A CRLF split across two reads is still one line end.
        #expect(parse(["data: a\r", "\n\r", "\n"]) == [SSEMessage(id: nil, event: nil, data: "a")])
    }

    @Test func ignoresCommentsRetryAndEmptyMessages() {
        #expect(parse([": keep-alive\n\n", "retry: 100\n\n", "\n\n", "id: 3\n\n"]).isEmpty)
    }

    @Test func waitsForTheBlankLine() {
        var parser = SSEParser()
        #expect(parser.feed(Array("id: 1\ndata: x\n".utf8)).isEmpty)
        #expect(parser.feed(Array("\n".utf8)) == [SSEMessage(id: "1", event: nil, data: "x")])
    }

    @Test func keepsUnicodeSplitInsideACharacter() {
        let bytes = Array("data: héllo ✓\n\n".utf8)
        var parser = SSEParser()
        var messages: [SSEMessage] = []
        for byte in bytes { messages += parser.feed([byte]) }
        #expect(messages.map(\.data) == ["héllo ✓"])
    }
}
