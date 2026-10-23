import CosKit
import Foundation
import Observation

/// One agent: its state and last message from the session, a message box,
/// and a plain-text snapshot of its terminal when asked.
@MainActor
@Observable
final class AgentDetail {
    let agentId: Int
    @ObservationIgnored private let session: HostSession

    var draft = ""
    private(set) var screen: AgentScreen?
    private(set) var isSending = false
    private(set) var isLoadingScreen = false
    private(set) var error: String?

    init(agentId: Int, session: HostSession) {
        self.agentId = agentId
        self.session = session
    }

    var agent: Agent? { session.agent(id: agentId) }
    var approval: Approval? { session.pendingApproval(forAgent: agentId) }

    /// Types the draft into the agent and presses Return. A stopped agent is
    /// resumed first, so the message reaches the same session.
    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        isSending = true
        defer { isSending = false }
        do {
            if agent?.state.canResume == true {
                try await session.resume(agentId: agentId)
            }
            try await session.send(agentId: agentId, text: text)
            draft = ""
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// The terminal as plain text, as it is now.
    func loadScreen() async {
        isLoadingScreen = true
        defer { isLoadingScreen = false }
        do {
            screen = try await session.service.screen(agentId: agentId)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func perform(_ work: @escaping (HostSession) async throws -> Void) async {
        do {
            try await work(session)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}
