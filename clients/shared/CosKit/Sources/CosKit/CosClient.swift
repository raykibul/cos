import Foundation

/// Talks to the daemon's HTTP API on its Unix socket. Every call opens its
/// own connection, so a client can be shared freely.
public struct CosClient: Sendable {
    public let socketPath: String

    public init(socketPath: String = CosClient.defaultSocketPath()) {
        self.socketPath = socketPath
    }

    /// The daemon's home: `$COS_HOME` if set, otherwise `~/.cos`. This is the
    /// same rule the daemon uses, so a test daemon is found the same way.
    public static func defaultHome(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        userHome: String = NSHomeDirectory()
    ) -> String {
        if let home = environment["COS_HOME"], !home.isEmpty { return home }
        return (userHome as NSString).appendingPathComponent(".cos")
    }

    public static func defaultSocketPath(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        userHome: String = NSHomeDirectory()
    ) -> String {
        (defaultHome(environment: environment, userHome: userHome) as NSString).appendingPathComponent("cos.sock")
    }

    public func status() async throws -> Status {
        try await getJSON("/v1/status")
    }

    /// Up to `limit` events after sequence number `after`, oldest first.
    public func events(after: Int = 0, limit: Int = 100) async throws -> [LoggedEvent] {
        let page: EventsPage = try await getJSON("/v1/events?after=\(after)&limit=\(limit)")
        return page.events
    }

    /// Every event after `after`, then new ones as they happen. The stream
    /// never ends quietly: when the daemon closes it (it is stopping) it
    /// throws `.connectionClosed`. It does not reconnect; `EventFollower` does.
    public func stream(after: Int) -> AsyncThrowingStream<LoggedEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    // The daemon prefers Last-Event-ID over `after`; send both.
                    let (connection, response) = try await request(
                        "GET",
                        "/v1/events/stream?after=\(after)",
                        headers: [("Accept", "text/event-stream"), ("Last-Event-ID", String(after))]
                    )
                    defer { connection.close() }
                    var parser = SSEParser()
                    while let chunk = try await response.nextBodyChunk() {
                        for message in parser.feed(chunk) {
                            continuation.yield(try decode(LoggedEvent.self, from: Array(message.data.utf8)))
                        }
                    }
                    continuation.finish(throwing: CosError.connectionClosed)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    // MARK: Projects and workspaces

    /// Every project, in the order they were added, each with its workspaces.
    public func projects() async throws -> [Project] {
        let page: ProjectsPage = try await getJSON("/v1/projects")
        return page.projects
    }

    /// Registers the repo at `path` (any folder inside it works). Returns the
    /// existing project if the repo is already registered.
    public func addProject(path: String) async throws -> Project {
        try await sendJSON("POST", "/v1/projects", body: ["path": path])
    }

    /// Unregisters a project; the repo itself is never touched. Throws
    /// `.conflict` while it still has workspaces.
    public func removeProject(id: Int) async throws {
        try await send("DELETE", "/v1/projects/\(id)")
    }

    /// `title` is optional; the daemon uses the workspace's name without one.
    public func createWorkspace(projectId: Int, base: WorkspaceBase = .origin, title: String? = nil) async throws -> Workspace {
        var body = ["base": base.rawValue]
        if let title, !title.isEmpty { body["title"] = title }
        return try await sendJSON("POST", "/v1/projects/\(projectId)/workspaces", body: body)
    }

    public func renameWorkspace(id: Int, title: String) async throws -> Workspace {
        try await sendJSON("PATCH", "/v1/workspaces/\(id)", body: ["title": title])
    }

    /// Deletes the workspace's folder and keeps its branch. Throws
    /// `.conflict` if it has uncommitted changes.
    public func removeWorkspace(id: Int) async throws {
        try await send("DELETE", "/v1/workspaces/\(id)")
    }

    // MARK: Plumbing

    private func getJSON<T: Decodable>(_ path: String) async throws -> T {
        let (connection, response) = try await request("GET", path)
        defer { connection.close() }
        return try decode(T.self, from: try await response.readBody())
    }

    private func sendJSON<T: Decodable>(_ method: String, _ path: String, body: [String: String]) async throws -> T {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let (connection, response) = try await request(method, path, json: try encoder.encode(body))
        defer { connection.close() }
        return try decode(T.self, from: try await response.readBody())
    }

    private func send(_ method: String, _ path: String) async throws {
        let (connection, response) = try await request(method, path)
        defer { connection.close() }
        _ = try await response.readBody()
    }

    /// Sends a request and reads the response head. Non-2xx answers become
    /// `CosError`s carrying the daemon's message.
    private func request(
        _ method: String,
        _ path: String,
        headers: [(String, String)] = [],
        json body: Data? = nil
    ) async throws -> (SocketConnection, HTTPResponse) {
        let connection = SocketConnection(unixPath: socketPath)
        do {
            try await connection.open()
            try await connection.send(httpRequest(method: method, path: path, headers: headers, json: body.map { Array($0) }))
            let response = try await HTTPResponse(reader: connection)
            guard (200..<300).contains(response.head.status) else {
                let body = (try? await response.readBody()) ?? []
                throw CosError.fromResponse(status: response.head.status, body: body, path: path)
            }
            return (connection, response)
        } catch {
            connection.close()
            throw error
        }
    }
}

func decode<T: Decodable>(_ type: T.Type, from bytes: [UInt8]) throws -> T {
    do {
        return try JSONDecoder.cos().decode(type, from: Data(bytes))
    } catch {
        throw CosError.badResponse("could not decode \(T.self): \(error)")
    }
}
