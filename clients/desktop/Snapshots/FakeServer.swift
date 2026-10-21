import Darwin
import Foundation
import os

/// A small daemon stand-in on a Unix socket, for the snapshot tool: the
/// agents API from docs/agents-api.md with canned data, and terminal
/// streams that replay a Claude Code screen. Keystrokes and resizes are
/// recorded, not acted on.
final class FakeServer: Sendable {
    struct Request: Sendable {
        var method: String
        var path: String
        var body: [UInt8]
    }

    enum Reply: Sendable {
        case json(Int, String)
        case empty(Int)
        /// SSE messages, then hold the stream open until the server stops.
        case stream(String)
        /// SSE messages, then end the stream.
        case streamThenEnd(String)
    }

    let socketPath: String
    private let listener: Int32
    private let handler: @Sendable (Request) -> Reply
    private let state = OSAllocatedUnfairLock(initialState: (requests: [Request](), open: [Int32]()))

    var requests: [Request] { state.withLock { $0.requests } }

    init(handler: @escaping @Sendable (Request) -> Reply) throws {
        socketPath = NSTemporaryDirectory() + "cos-snap-\(UUID().uuidString.prefix(8)).sock"
        self.handler = handler
        listener = socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: Array(socketPath.utf8)) }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard bound == 0, listen(listener, 16) == 0 else { throw POSIXError(.EADDRINUSE) }
        Thread { [self] in acceptLoop() }.start()
    }

    func stop() {
        state.withLock { state in
            for client in state.open { shutdown(client, SHUT_RDWR) }
        }
        shutdown(listener, SHUT_RDWR)
        close(listener)
        unlink(socketPath)
    }

    private func acceptLoop() {
        while true {
            let client = accept(listener, nil, nil)
            if client < 0 { return }
            Thread { [self] in serve(client) }.start()
        }
    }

    private func serve(_ client: Int32) {
        defer {
            state.withLock { $0.open.removeAll { $0 == client } }
            close(client)
        }
        guard let request = readRequest(client) else { return }
        state.withLock { $0.requests.append(request) }
        let sse = ["Content-Type: text/event-stream", "Transfer-Encoding: chunked"]
        switch handler(request) {
        case .json(let status, let body):
            write(client, head(status, ["Content-Type: application/json", "Content-Length: \(body.utf8.count)"]) + body)
        case .empty(let status):
            write(client, head(status, ["Content-Length: 0"]))
        case .stream(let messages):
            write(client, head(200, sse) + chunk(messages))
            state.withLock { $0.open.append(client) }
            var byte: UInt8 = 0
            while read(client, &byte, 1) > 0 {}
        case .streamThenEnd(let messages):
            write(client, head(200, sse) + chunk(messages) + "0\r\n\r\n")
        }
    }

    private func chunk(_ text: String) -> String {
        text.isEmpty ? "" : String(text.utf8.count, radix: 16) + "\r\n" + text + "\r\n"
    }

    private func head(_ status: Int, _ headers: [String]) -> String {
        "HTTP/1.1 \(status) X\r\n" + headers.map { $0 + "\r\n" }.joined() + "Connection: close\r\n\r\n"
    }

    private func write(_ client: Int32, _ text: String) {
        let bytes = Array(text.utf8)
        _ = bytes.withUnsafeBytes { Darwin.write(client, $0.baseAddress, $0.count) }
    }

    private func readRequest(_ client: Int32) -> Request? {
        var buffer: [UInt8] = []
        var chunk = [UInt8](repeating: 0, count: 4096)
        func fill() -> Bool {
            let count = read(client, &chunk, chunk.count)
            if count <= 0 { return false }
            buffer += chunk[..<count]
            return true
        }
        while buffer.firstRange(of: [13, 10, 13, 10]) == nil {
            if !fill() { return nil }
        }
        let end = buffer.firstRange(of: [13, 10, 13, 10])!
        let lines = String(decoding: buffer[..<end.lowerBound], as: UTF8.self).components(separatedBy: "\r\n")
        let start = lines[0].split(separator: " ")
        var length = 0
        for line in lines.dropFirst() where line.lowercased().hasPrefix("content-length:") {
            length = Int(line.split(separator: ":")[1].trimmingCharacters(in: .whitespaces)) ?? 0
        }
        while buffer.count - end.upperBound < length {
            if !fill() { return nil }
        }
        return Request(method: String(start[0]), path: String(start[1]), body: Array(buffer[end.upperBound...]))
    }
}
