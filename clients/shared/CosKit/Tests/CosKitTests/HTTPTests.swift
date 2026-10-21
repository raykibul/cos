import Testing
@testable import CosKit

/// Hands out pre-recorded pieces, then reports the connection closed.
actor ScriptedReader: ByteReader {
    private var pieces: [[UInt8]]

    init(_ pieces: [String]) {
        self.pieces = pieces.map { Array($0.utf8) }
    }

    init(bytes: [UInt8], pieceSize: Int) {
        pieces = stride(from: 0, to: bytes.count, by: pieceSize).map {
            Array(bytes[$0..<min($0 + pieceSize, bytes.count)])
        }
    }

    func read() async throws -> [UInt8]? {
        pieces.isEmpty ? nil : pieces.removeFirst()
    }
}

@Suite struct HTTPTests {
    /// The start of a real `/v1/events/stream` response, chunk sizes included.
    static let streamResponse =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n"
        + "transfer-encoding: chunked\r\ndate: Wed, 21 Oct 2026 18:36:25 GMT\r\n\r\n"
        + "76\r\nid: 1\ndata: {\"seq\":1,\"ts\":\"2026-10-21T18:36:19.126Z\",\"kind\":\"daemon.started\",\"data\":{\"pid\":76227,\"version\":\"0.1.0\"}}\n\n\r\n"
        + "3\r\n:\n\n\r\n"
        + "0\r\n\r\n"

    static let statusResponse =
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 164\r\ndate: Wed, 21 Oct 2026 18:36:25 GMT\r\n\r\n"
        + Samples.status

    @Test func requestIsMinimalHTTP11() {
        let request = String(decoding: httpRequest(method: "GET", path: "/v1/events/stream?after=4", headers: [("Last-Event-ID", "4")]), as: UTF8.self)
        #expect(request == "GET /v1/events/stream?after=4 HTTP/1.1\r\nHost: cos\r\nLast-Event-ID: 4\r\nConnection: close\r\n\r\n")
    }

    @Test func parsesHeadAndLeavesBody() throws {
        var buffer = Array(Self.statusResponse.utf8)
        let head = try #require(try HTTPResponseHead.parse(&buffer))
        #expect(head.status == 200)
        #expect(head.headers["content-length"] == "164")
        #expect(head.headers["content-type"] == "application/json")
        #expect(String(decoding: buffer, as: UTF8.self) == Samples.status)
    }

    @Test func incompleteHeadWaits() throws {
        var buffer = Array("HTTP/1.1 200 OK\r\ncontent-le".utf8)
        #expect(try HTTPResponseHead.parse(&buffer) == nil)
        #expect(buffer.count == 27)
    }

    @Test func badStatusLineThrows() {
        var buffer = Array("SSH-2.0-OpenSSH\r\n\r\n".utf8)
        #expect(throws: CosError.self) { try HTTPResponseHead.parse(&buffer) }
    }

    @Test(arguments: [1, 2, 3, 7, 64, 10_000])
    func readsContentLengthBody(pieceSize: Int) async throws {
        let reader = ScriptedReader(bytes: Array(Self.statusResponse.utf8), pieceSize: pieceSize)
        let response = try await HTTPResponse(reader: reader)
        let status = try CosKit.decode(Status.self, from: try await response.readBody())
        #expect(status.pid == 76227)
    }

    @Test(arguments: [1, 2, 5, 13, 10_000])
    func readsChunkedSSEBody(pieceSize: Int) async throws {
        let reader = ScriptedReader(bytes: Array(Self.streamResponse.utf8), pieceSize: pieceSize)
        let response = try await HTTPResponse(reader: reader)
        var parser = SSEParser()
        var messages: [SSEMessage] = []
        while let chunk = try await response.nextBodyChunk() {
            messages += parser.feed(chunk)
        }
        #expect(messages.map(\.id) == ["1"])
        let event = try CosKit.decode(LoggedEvent.self, from: Array(messages[0].data.utf8))
        #expect(event.kind == "daemon.started")
    }

    @Test func chunkedDecoderHandlesExtensionsAndTrailers() throws {
        var decoder = ChunkedDecoder()
        let body = try decoder.feed(Array("4;name=x\r\nWiki\r\n5\r\npedia\r\n0\r\nExpires: never\r\n\r\n".utf8))
        #expect(String(decoding: body, as: UTF8.self) == "Wikipedia")
        #expect(decoder.isFinished)
    }

    @Test func chunkedDecoderRejectsGarbage() {
        var decoder = ChunkedDecoder()
        #expect(throws: CosError.self) { try decoder.feed(Array("zz\r\n".utf8)) }
        var overlong = ChunkedDecoder()
        #expect(throws: CosError.self) { try overlong.feed(Array("2\r\nabc\r\n".utf8)) }
    }

    @Test func bodyUntilCloseWithoutFraming() async throws {
        let reader = ScriptedReader(["HTTP/1.1 200 OK\r\n\r\nhel", "lo"])
        let response = try await HTTPResponse(reader: reader)
        #expect(String(decoding: try await response.readBody(), as: UTF8.self) == "hello")
    }

    @Test func notFoundIsReadAsAResponse() async throws {
        let reader = ScriptedReader(["HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n"])
        let response = try await HTTPResponse(reader: reader)
        #expect(response.head.status == 404)
        #expect(try await response.readBody().isEmpty)
    }

    @Test func connectionClosedMidBodyThrows() async throws {
        let reader = ScriptedReader(["HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc"])
        let response = try await HTTPResponse(reader: reader)
        await #expect(throws: CosError.connectionClosed) { try await response.readBody() }

        let chunked = try await HTTPResponse(reader: ScriptedReader(["HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nab"]))
        await #expect(throws: CosError.connectionClosed) { try await chunked.readBody() }
    }

    @Test func closedBeforeHeadThrows() async {
        await #expect(throws: CosError.connectionClosed) { try await HTTPResponse(reader: ScriptedReader([])) }
    }
}
