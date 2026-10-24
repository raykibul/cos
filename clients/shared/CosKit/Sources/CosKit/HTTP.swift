import Foundation

// Just enough HTTP/1.1 for the daemon's API: requests with an optional JSON
// body, and responses framed by Content-Length, chunked encoding or
// connection close.

/// Something that hands out the bytes of a connection as they arrive.
protocol ByteReader: Sendable {
    /// The next bytes, or `nil` once the other side has closed.
    func read() async throws -> [UInt8]?
}

func httpRequest(
    method: String,
    path: String,
    host: String = "cos",
    headers: [(String, String)] = [],
    json body: [UInt8]? = nil,
    contentType: String = "application/json"
) -> [UInt8] {
    var text = "\(method) \(path) HTTP/1.1\r\nHost: \(host)\r\n"
    for (name, value) in headers {
        text += "\(name): \(value)\r\n"
    }
    if let body {
        text += "Content-Type: \(contentType)\r\nContent-Length: \(body.count)\r\n"
    }
    // One request per connection keeps the client simple.
    text += "Connection: close\r\n\r\n"
    return Array(text.utf8) + (body ?? [])
}

struct HTTPResponseHead: Equatable {
    var status: Int
    /// Header names in lower case.
    var headers: [String: String]

    /// Parses and removes the head from the front of `buffer`, leaving the
    /// start of the body. Returns `nil` if the head isn't complete yet.
    static func parse(_ buffer: inout [UInt8]) throws -> HTTPResponseHead? {
        guard let end = buffer.firstRange(of: [13, 10, 13, 10]) else { return nil }
        let text = String(decoding: buffer[..<end.lowerBound], as: UTF8.self)
        buffer.removeSubrange(..<end.upperBound)

        var lines = text.components(separatedBy: "\r\n")
        let statusLine = lines.removeFirst().split(separator: " ", maxSplits: 2)
        guard statusLine.count >= 2, statusLine[0].hasPrefix("HTTP/1."), let status = Int(statusLine[1]) else {
            throw CosError.badResponse("bad status line")
        }
        var headers: [String: String] = [:]
        for line in lines {
            guard let colon = line.firstIndex(of: ":") else { throw CosError.badResponse("bad header line") }
            let name = line[..<colon].lowercased()
            headers[name] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        }
        return HTTPResponseHead(status: status, headers: headers)
    }
}

/// Decodes a `Transfer-Encoding: chunked` body fed in arbitrary pieces.
struct ChunkedDecoder {
    private enum State: Equatable {
        case size
        case data(remaining: Int)
        case dataEnd
        case trailer
        case done
    }

    private var buffer: [UInt8] = []
    private var state = State.size

    var isFinished: Bool { state == .done }

    /// Feeds more raw bytes and returns the body bytes they complete.
    mutating func feed(_ input: [UInt8]) throws -> [UInt8] {
        buffer += input
        var output: [UInt8] = []
        while true {
            switch state {
            case .size:
                guard let line = takeLine() else { return output }
                // Ignore chunk extensions after ';'.
                let hex = line.split(separator: ";").first.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? ""
                guard let size = Int(hex, radix: 16), size >= 0 else {
                    throw CosError.badResponse("bad chunk size \(line)")
                }
                state = size == 0 ? .trailer : .data(remaining: size)
            case .data(let remaining):
                guard !buffer.isEmpty else { return output }
                let count = min(remaining, buffer.count)
                output += buffer[..<count]
                buffer.removeFirst(count)
                state = count == remaining ? .dataEnd : .data(remaining: remaining - count)
            case .dataEnd:
                guard let line = takeLine() else { return output }
                guard line.isEmpty else { throw CosError.badResponse("chunk is longer than its size") }
                state = .size
            case .trailer:
                guard let line = takeLine() else { return output }
                if line.isEmpty { state = .done }
            case .done:
                return output
            }
        }
    }

    private mutating func takeLine() -> String? {
        guard let end = buffer.firstRange(of: [13, 10]) else { return nil }
        let line = String(decoding: buffer[..<end.lowerBound], as: UTF8.self)
        buffer.removeSubrange(..<end.upperBound)
        return line
    }
}

/// A response whose body is read piece by piece, so a never-ending SSE
/// stream works the same way as a short JSON answer.
final class HTTPResponse {
    let head: HTTPResponseHead
    private let reader: ByteReader
    private var pending: [UInt8]
    private var framing: Framing

    private enum Framing {
        case length(remaining: Int)
        case chunked(ChunkedDecoder)
        case untilClose
    }

    /// Reads the head from `reader`.
    init(reader: ByteReader) async throws {
        var buffer: [UInt8] = []
        var head: HTTPResponseHead?
        while head == nil {
            guard let bytes = try await reader.read() else { throw CosError.connectionClosed }
            buffer += bytes
            head = try HTTPResponseHead.parse(&buffer)
        }
        self.head = head!
        self.reader = reader
        self.pending = buffer
        if self.head.status == 204 || self.head.status == 304 {
            // These never have a body, whatever the headers say.
            framing = .length(remaining: 0)
        } else if self.head.headers["transfer-encoding"]?.lowercased().contains("chunked") == true {
            framing = .chunked(ChunkedDecoder())
        } else if let length = self.head.headers["content-length"].flatMap({ Int($0) }) {
            framing = .length(remaining: length)
        } else {
            framing = .untilClose
        }
    }

    /// The next piece of the body, or `nil` once it is complete.
    func nextBodyChunk() async throws -> [UInt8]? {
        while true {
            var input = pending
            pending = []
            if input.isEmpty {
                if case .length(0) = framing { return nil }
                if case .chunked(let decoder) = framing, decoder.isFinished { return nil }
                guard let bytes = try await reader.read() else {
                    if case .untilClose = framing { return nil }
                    throw CosError.connectionClosed
                }
                input = bytes
            }
            switch framing {
            case .length(let remaining):
                let body = Array(input.prefix(remaining))
                framing = .length(remaining: remaining - body.count)
                if !body.isEmpty { return body }
            case .chunked(var decoder):
                let body = try decoder.feed(input)
                framing = .chunked(decoder)
                if !body.isEmpty { return body }
            case .untilClose:
                return input
            }
        }
    }

    func readBody() async throws -> [UInt8] {
        var body: [UInt8] = []
        while let chunk = try await nextBodyChunk() {
            body += chunk
        }
        return body
    }
}
