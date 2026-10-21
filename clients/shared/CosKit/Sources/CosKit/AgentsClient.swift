import Foundation

/// The agents API (docs/agents-api.md). Calls throw `CosError`s whose
/// descriptions are the daemon's messages, written to be shown as-is.
extension CosClient {
    /// Agents, newest first, optionally only one workspace's or one state's.
    public func agents(workspaceId: Int? = nil, state: AgentState? = nil) async throws -> [Agent] {
        var query: [String] = []
        if let workspaceId { query.append("workspace_id=\(workspaceId)") }
        if let state { query.append("state=\(Self.queryValue(state.rawValue))") }
        let path = "/v1/agents" + (query.isEmpty ? "" : "?" + query.joined(separator: "&"))
        let page: AgentsPage = try await getJSON(path)
        return page.agents
    }

    public func agent(id: Int) async throws -> Agent {
        try await getJSON("/v1/agents/\(id)")
    }

    /// Starts an agent (role `user`) in a workspace's folder.
    public func createAgent(workspaceId: Int, _ request: NewAgent = NewAgent()) async throws -> Agent {
        try await sendJSON("POST", "/v1/workspaces/\(workspaceId)/agents", body: request)
    }

    /// Types `text` and presses Return. Throws `.conflict` once it has exited.
    public func sendInput(agentId: Int, text: String) async throws {
        try await send("POST", "/v1/agents/\(agentId)/input", body: ["text": text])
    }

    /// Raw keystrokes, exactly as typed (Esc, arrows, Ctrl-C…).
    public func sendKeys(agentId: Int, _ bytes: [UInt8]) async throws {
        try await send("POST", "/v1/agents/\(agentId)/keys", body: ["data": Data(bytes).base64EncodedString()])
    }

    /// Resizes the agent's PTY. It has one size, and the last resize wins.
    public func resizeAgent(id: Int, rows: Int, cols: Int) async throws {
        try await send("POST", "/v1/agents/\(id)/resize", body: ["rows": rows, "cols": cols])
    }

    /// Answers Claude's trust dialog with yes. Throws `.conflict` unless the
    /// agent is `needs_trust`.
    public func trustAgent(id: Int) async throws {
        try await send("POST", "/v1/agents/\(id)/trust")
    }

    /// Sends `/exit`; the daemon kills the process after 10 s (202).
    public func stopAgent(id: Int) async throws {
        try await send("POST", "/v1/agents/\(id)/stop")
    }

    /// Starts `claude --resume` as the same agent. Only for `exited` or
    /// `interrupted` agents.
    public func resumeAgent(id: Int) async throws -> Agent {
        try await sendJSON("POST", "/v1/agents/\(id)/resume")
    }

    /// Forgets an exited agent. Throws `.conflict` while it is running.
    public func removeAgent(id: Int) async throws {
        try await send("DELETE", "/v1/agents/\(id)")
    }

    /// The current screen as plain text.
    public func screen(agentId: Int) async throws -> AgentScreen {
        try await getJSON("/v1/agents/\(agentId)/screen")
    }

    /// Approvals, by default only the pending ones.
    public func approvals(state: ApprovalState? = .pending) async throws -> [Approval] {
        let path = "/v1/approvals" + (state.map { "?state=\(Self.queryValue($0.rawValue))" } ?? "")
        let page: ApprovalsPage = try await getJSON(path)
        return page.approvals
    }

    /// Allows or denies a pending approval. Throws `.conflict` if it is no
    /// longer pending (answered in the terminal, say).
    public func decide(approvalId: Int, _ decision: ApprovalDecision, message: String? = nil) async throws -> Approval {
        var body = ["decision": decision.rawValue]
        if let message = message.nonEmpty { body["message"] = message }
        return try await sendJSON("POST", "/v1/approvals/\(approvalId)", body: body)
    }

    private static func queryValue(_ text: String) -> String {
        text.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed.subtracting(CharacterSet(charactersIn: "&=+"))) ?? text
    }
}
