import Darwin
import Foundation
import os
@testable import CosKit

/// A tiny HTTP server on a Unix socket that answers like the daemon, from a
/// handler. It lets tests check the exact requests `CosClient` sends and how
/// it reads real responses.
final class FakeDaemon: Sendable {
    struct Request: Sendable {
        var method: String
        var path: String
        var headers: [String: String]
        var body: [UInt8]

        var json: [String: String]? {
            try? JSONDecoder().decode([String: String].self, from: Data(body))
        }

        /// The body as any JSON, for bodies with numbers.
        var value: JSONValue? {
            try? JSONDecoder().decode(JSONValue.self, from: Data(body))
        }
    }

    enum Reply: Sendable {
        case json(Int, String)
        case empty(Int)
        /// An SSE response: these messages, then the connection stays open
        /// until the server stops.
        case stream(String)
        /// An SSE response that ends properly after these messages.
        case streamThenEnd(String)
        /// An SSE response whose connection breaks after these messages,
        /// without the closing chunk: the daemon died, say.
        case streamThenDrop(String)
    }

    let socketPath: String
    private let listener: Int32
    private let handler: @Sendable (Request) -> Reply
    private let state = OSAllocatedUnfairLock(initialState: (requests: [Request](), stopped: false, open: [Int32]()))

    var requests: [Request] { state.withLock { $0.requests } }

    init(handler: @escaping @Sendable (Request) -> Reply) throws {
        // Short path: Unix socket paths are limited to 104 bytes.
        socketPath = NSTemporaryDirectory() + "cosfake-\(UUID().uuidString.prefix(8)).sock"
        self.handler = handler
        listener = socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            let path = Array(socketPath.utf8)
            precondition(path.count < buffer.count, "socket path too long")
            buffer.copyBytes(from: path)
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard bound == 0, listen(listener, 16) == 0 else { throw POSIXError(.EADDRINUSE) }
        Thread { [self] in acceptLoop() }.start()
    }

    func stop() {
        // Under the lock, so a connection can't close (and its number be
        // reused) between reading the list and shutting it down.
        state.withLock { state in
            state.stopped = true
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
            // Forget it before closing: once closed, the number can be reused
            // by another test's connection, which stop() must not shut down.
            state.withLock { $0.open.removeAll { $0 == client } }
            close(client)
        }
        guard let request = readRequest(client) else { return }
        state.withLock { $0.requests.append(request) }
        switch handler(request) {
        case .json(let status, let body):
            write(client, head(status, ["Content-Type: application/json", "Content-Length: \(body.utf8.count)"]) + body)
        case .empty(let status):
            write(client, head(status, []))
        case .stream(let messages):
            let chunk = String(messages.utf8.count, radix: 16) + "\r\n" + messages + "\r\n"
            write(client, head(200, ["Content-Type: text/event-stream", "Transfer-Encoding: chunked"]) + chunk)
            state.withLock { $0.open.append(client) }
            // Hold the stream open until stop() shuts the socket down.
            var byte: UInt8 = 0
            while read(client, &byte, 1) > 0 {}
        case .streamThenEnd(let messages):
            let chunk = messages.isEmpty ? "" : String(messages.utf8.count, radix: 16) + "\r\n" + messages + "\r\n"
            write(client, head(200, ["Content-Type: text/event-stream", "Transfer-Encoding: chunked"]) + chunk + "0\r\n\r\n")
        case .streamThenDrop(let messages):
            let chunk = messages.isEmpty ? "" : String(messages.utf8.count, radix: 16) + "\r\n" + messages + "\r\n"
            write(client, head(200, ["Content-Type: text/event-stream", "Transfer-Encoding: chunked"]) + chunk)
        }
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
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            let parts = line.split(separator: ":", maxSplits: 1)
            headers[parts[0].lowercased()] = parts[1].trimmingCharacters(in: .whitespaces)
        }
        var body = Array(buffer[end.upperBound...])
        let length = Int(headers["content-length"] ?? "0") ?? 0
        while body.count < length {
            let before = buffer.count
            if !fill() { return nil }
            body += buffer[before...]
        }
        return Request(method: String(start[0]), path: String(start[1]), headers: headers, body: body)
    }
}
