import Foundation

/// Runs, the board, questions, limits and reports ("Runs, tasks and the
/// board" in docs/daemon.md, and docs/orchestration.md). Calls throw
/// `CosError`s whose descriptions are the daemon's messages. A daemon
/// without one of the newer routes answers `.notFound`.
extension CosClient {
    /// Every run, newest first, with task counts.
    public func runs() async throws -> [Run] {
        let page: RunsPage = try await getJSON("/v1/runs")
        return page.runs
    }

    /// Starts a run (`planning`). Throws `.conflict` if the workspace already
    /// has an unfinished run, `.notFound` for an unknown workspace.
    public func createRun(_ request: NewRun) async throws -> Run {
        try await sendJSON("POST", "/v1/runs", body: request)
    }

    public func board(runId: Int) async throws -> Board {
        try await getJSON("/v1/runs/\(runId)")
    }

    /// Approves the waiting plan. Throws `.conflict` if no plan is waiting.
    public func approvePlan(runId: Int) async throws -> Board {
        try await sendJSON("POST", "/v1/runs/\(runId)/approve-plan")
    }

    /// Cancels the run and its unfinished tasks. Throws `.conflict` once finished.
    public func cancelRun(id: Int, reason: String? = nil) async throws -> Board {
        if let reason = reason.nonEmpty {
            return try await sendJSON("POST", "/v1/runs/\(id)/cancel", body: ["reason": reason])
        }
        return try await sendJSON("POST", "/v1/runs/\(id)/cancel")
    }

    /// Moves a task: `ready` to retry a blocked task, `failed`, `cancelled`.
    /// Blocking, failing and retrying need a reason (400 without one).
    public func updateTask(id: Int, state: TaskState, reason: String? = nil) async throws -> Board {
        var body = ["state": state.rawValue]
        if let reason = reason.nonEmpty { body["reason"] = reason }
        return try await sendJSON("PATCH", "/v1/tasks/\(id)", body: body)
    }

    /// Questions from chiefs, by default only the open ones.
    public func questions(state: QuestionState? = .open) async throws -> [Question] {
        let path = "/v1/questions" + (state.map { "?state=\($0.rawValue)" } ?? "")
        let page: QuestionsPage = try await getJSON(path)
        return page.questions
    }

    /// Answers a question; the answer reaches the chief through its inbox.
    public func answer(questionId: Int, _ answer: String) async throws -> Question {
        try await sendJSON("POST", "/v1/questions/\(questionId)/answer", body: ["answer": answer])
    }

    /// A message to the run's chief, delivered through its inbox (202).
    public func messageChief(runId: Int, text: String) async throws {
        try await send("POST", "/v1/runs/\(runId)/message", body: ["text": text])
    }

    /// The report's markdown. Throws `.notFound` before it is written.
    public func report(runId: Int) async throws -> String {
        let report: RunReport = try await getJSON("/v1/runs/\(runId)/report")
        return report.markdown
    }

    /// The run's preset, cap, and how many approvals the chief has given.
    public func limits(runId: Int) async throws -> RunLimits {
        try await getJSON("/v1/runs/\(runId)/limits")
    }

    public func setLimits(runId: Int, preset: ApprovalPreset, approvalCap: Int) async throws -> RunLimits {
        struct Body: Encodable {
            var preset: String
            var approval_cap: Int
        }
        return try await sendJSON("PUT", "/v1/runs/\(runId)/limits", body: Body(preset: preset.rawValue, approval_cap: approvalCap))
    }
}
