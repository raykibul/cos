import Foundation

/// The Chief of Staff, transcripts and environment routes
/// (docs/chief-of-staff.md, "Environment" in docs/daemon.md). Calls throw
/// `CosError`s whose descriptions are the daemon's messages. A daemon
/// without one of these routes answers `.notFound`.
extension CosClient {
    /// A project's Chief of Staff, even before it first starts.
    public func staff(projectId: Int) async throws -> Staff {
        try await getJSON("/v1/projects/\(projectId)/staff")
    }

    /// Sets the model it uses from its next start; `nil` or empty goes back
    /// to the default.
    public func setStaffModel(projectId: Int, model: String?) async throws -> Staff {
        struct Body: Encodable {
            var model: String?
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(model, forKey: .model) // an explicit null clears it
            }
            enum CodingKeys: String, CodingKey { case model }
        }
        return try await sendJSON("PUT", "/v1/projects/\(projectId)/staff", body: Body(model: model.nonEmpty))
    }

    /// Starts or resumes it; nothing if it's running.
    public func startStaff(projectId: Int) async throws -> Staff {
        try await sendJSON("POST", "/v1/projects/\(projectId)/staff/start")
    }

    /// A message to it (202). Starts it if needed; the text goes in as soon
    /// as it's idle.
    public func messageStaff(projectId: Int, text: String) async throws {
        try await send("POST", "/v1/projects/\(projectId)/staff/message", body: ["text": text])
    }

    /// The latest news its agents and runs reported, newest first. Your own
    /// messages are left out.
    public func staffInbox(projectId: Int) async throws -> [StaffItem] {
        let inbox: StaffInbox = try await getJSON("/v1/projects/\(projectId)/staff/inbox")
        return inbox.items
    }

    /// Any agent's chat, read from Claude's transcript: the messages after
    /// `after`, oldest first. Ask again with `after: transcript.next`.
    public func transcript(agentId: Int, after: Int = 0) async throws -> Transcript {
        try await getJSON("/v1/agents/\(agentId)/transcript?after=\(after)")
    }

    // MARK: Environment

    /// The variables every project's agents get.
    public func globalEnv() async throws -> EnvVars {
        let env: GlobalEnv = try await getJSON("/v1/env")
        return env.variables
    }

    /// A project's variables, the global ones, and their merge.
    public func projectEnv(projectId: Int) async throws -> ProjectEnv {
        try await getJSON("/v1/projects/\(projectId)/env")
    }

    /// Sets a variable globally (`projectId: nil`) or for a project. The
    /// daemon refuses names cos keeps (400, with the reason).
    public func setEnv(projectId: Int?, name: String, value: String) async throws {
        try await send("PUT", Self.envPath(projectId, name), body: ["value": value])
    }

    /// Removes a variable, whether or not it was set.
    public func unsetEnv(projectId: Int?, name: String) async throws {
        try await send("DELETE", Self.envPath(projectId, name))
    }

    private static func envPath(_ projectId: Int?, _ name: String) -> String {
        let escaped = name.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "/"))) ?? name
        return (projectId.map { "/v1/projects/\($0)/env/" } ?? "/v1/env/") + escaped
    }
}
