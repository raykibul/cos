import CosKit
import Foundation
import Observation

/// One run's board and report, reloaded whenever the session says a run
/// may have changed.
@MainActor
@Observable
final class RunDetail {
    let runId: Int
    @ObservationIgnored private let session: HostSession

    private(set) var board: Board?
    /// The report's markdown, once written.
    private(set) var report: String?
    private(set) var error: String?
    private(set) var isWorking = false

    init(runId: Int, session: HostSession) {
        self.runId = runId
        self.session = session
    }

    var run: Run? { board?.run ?? session.run(id: runId) }

    /// Questions this run's chief is waiting on.
    var questions: [Question] { session.questions.filter { $0.runId == runId } }

    func load() async {
        do {
            let board = try await session.service.board(runId: runId)
            self.board = board
            error = nil
            if board.run.state == .done || board.run.state == .reporting || board.run.reportPath != nil {
                report = try? await session.service.report(runId: runId)
            }
        } catch {
            self.error = error.localizedDescription
        }
    }

    func approvePlan() async {
        await act { self.board = try await self.session.approvePlan(runId: self.runId) }
    }

    func cancel() async {
        await act { self.board = try await self.session.cancelRun(id: self.runId) }
    }

    private func act(_ work: @escaping () async throws -> Void) async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await work()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

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
