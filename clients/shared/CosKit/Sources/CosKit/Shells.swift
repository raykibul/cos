import Foundation

// Shells ("Shells" in docs/agents-api.md): login shells the daemon runs in a
// folder, for you to type in. They run in a terminal like an agent's and
// stream the same way, but they aren't agents: no hooks, no events, nothing
// in the database. Approved devices open them over the tailnet too, as the
// daemon's user.

/// `GET /v1/shells`, `POST /v1/shells`.
public struct Shell: Codable, Equatable, Hashable, Sendable, Identifiable {
    public var id: Int
    /// The project whose environment it got, if any.
    public var projectId: Int?
    public var cwd: String
    public var pid: Int?
    public var startedMs: Int64
    public var exited: Bool
    public var exitCode: Int?

    public init(id: Int, projectId: Int? = nil, cwd: String, pid: Int? = nil, startedMs: Int64 = 0, exited: Bool = false, exitCode: Int? = nil) {
        self.id = id
        self.projectId = projectId
        self.cwd = cwd
        self.pid = pid
        self.startedMs = startedMs
        self.exited = exited
        self.exitCode = exitCode
    }

    private enum CodingKeys: String, CodingKey {
        case id, projectId, cwd, pid, startedMs, exited, exitCode
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        projectId = c.int(.projectId)
        cwd = c.string(.cwd) ?? ""
        pid = c.int(.pid)
        startedMs = c.int64(.startedMs) ?? 0
        exited = c.optional(Bool.self, .exited) ?? false
        exitCode = c.int(.exitCode)
    }

    /// The folder's name: `lisbon` for `/…/worktrees/atlas-1/lisbon`.
    public var folderName: String {
        let name = (cwd as NSString).lastPathComponent
        return name.isEmpty ? cwd : name
    }
}

/// `GET /v1/shells`.
public struct ShellsPage: Codable, Equatable, Sendable {
    public var shells: [Shell]

    public init(shells: [Shell]) {
        self.shells = shells
    }

    public init(from decoder: Decoder) throws {
        shells = try decoder.container(keyedBy: CodingKeys.self).list(Shell.self, .shells)
    }
}

/// A shell's terminal stream, keystrokes and size. `CosClient` is the real one.
public protocol ShellTerminalSource: Sendable {
    func shellTerminal(id: Int) -> AsyncThrowingStream<TerminalEvent, Error>
    func sendShellKeys(id: Int, _ bytes: [UInt8]) async throws
    func resizeShell(id: Int, rows: Int, cols: Int) async throws
}

/// A shell's terminal through the agents' terminal plumbing, so a
/// `TerminalSession` (and its `TerminalStream`) drives a shell as it drives
/// an agent: where they say `agentId`, it is the shell's id.
public struct ShellTerminal: TerminalSource, TerminalControl {
    public var shells: any ShellTerminalSource

    public init(shells: any ShellTerminalSource) {
        self.shells = shells
    }

    public func terminal(agentId shellId: Int) -> AsyncThrowingStream<TerminalEvent, Error> {
        shells.shellTerminal(id: shellId)
    }

    public func sendKeys(agentId shellId: Int, _ bytes: [UInt8]) async throws {
        try await shells.sendShellKeys(id: shellId, bytes)
    }

    public func resizeAgent(id: Int, rows: Int, cols: Int) async throws {
        try await shells.resizeShell(id: id, rows: rows, cols: cols)
    }
}

extension CosClient: ShellTerminalSource {
    /// The daemon's shells, the ones another app opened included.
    public func shells() async throws -> [Shell] {
        let page: ShellsPage = try await getJSON("/v1/shells")
        return page.shells
    }

    /// Starts a login shell in `cwd` (an absolute folder), with a
    /// project's environment when given one. 409 at 32 shells; an older
    /// daemon refuses it over the tailnet (403 `host_only`).
    public func startShell(cwd: String, projectId: Int? = nil, rows: Int? = nil, cols: Int? = nil) async throws -> Shell {
        struct Body: Encodable {
            var cwd: String
            var projectId: Int?
            var rows: Int?
            var cols: Int?
            enum CodingKeys: String, CodingKey {
                case cwd, rows, cols
                case projectId = "project_id"
            }
        }
        return try await sendJSON("POST", "/v1/shells", body: Body(cwd: cwd, projectId: projectId, rows: rows, cols: cols))
    }

    /// Kills it if it still runs, and forgets it.
    public func closeShell(id: Int) async throws {
        try await send("DELETE", "/v1/shells/\(id)")
    }

    public func shellTerminal(id: Int) -> AsyncThrowingStream<TerminalEvent, Error> {
        terminalStream("/v1/shells/\(id)/terminal")
    }

    public func sendShellKeys(id: Int, _ bytes: [UInt8]) async throws {
        try await send("POST", "/v1/shells/\(id)/keys", body: ["data": Data(bytes).base64EncodedString()])
    }

    public func resizeShell(id: Int, rows: Int, cols: Int) async throws {
        try await send("POST", "/v1/shells/\(id)/resize", body: ["rows": rows, "cols": cols])
    }
}
