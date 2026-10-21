import Foundation

/// A run's report, as far as the app knows.
public enum RunReportState: Equatable, Sendable {
    /// The daemon has none yet (404), or doesn't serve reports.
    case notWritten
    case markdown(String)
    /// Fetching it failed; the daemon's message.
    case failed(String)
}

/// What the monitor keeps current for a run a page is showing.
@MainActor
final class RunWatch {
    var count = 1
    let board: CoalescedRefresh<Board>
    let limits: CoalescedRefresh<RunLimits>
    let report: CoalescedRefresh<String>

    init(board: CoalescedRefresh<Board>, limits: CoalescedRefresh<RunLimits>, report: CoalescedRefresh<String>) {
        self.board = board
        self.limits = limits
        self.report = report
    }
}

extension DaemonMonitor {
    func setUpRuns() {
        let client = client
        runsRefresh = CoalescedRefresh(delay: runsDebounce, fetch: { try await client.runs() }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let runs):
                self.runs = runs
                runsSupported = true
                runsError = nil
            case .failure(CosError.notFound):
                // An older daemon without runs.
                runsSupported = false
                runs = []
                runsError = nil
            case .failure(let error):
                runsError = error.localizedDescription
            }
        }
        questionsRefresh = CoalescedRefresh(delay: runsDebounce, fetch: { try await client.questions(state: .open) }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let questions):
                self.questions = questions.filter { $0.state == .open }.sorted { $0.createdMs < $1.createdMs }
                questionsSupported = true
            case .failure(CosError.notFound):
                questionsSupported = false
                questions = []
            case .failure:
                break
            }
        }
        allApprovalsRefresh = CoalescedRefresh(delay: runsDebounce, fetch: { try await client.approvals(state: nil) }) { [weak self] result in
            guard let self else { return }
            if case .success(let approvals) = result {
                allApprovals = approvals.sorted { $0.createdMs > $1.createdMs }
            }
        }
    }

    /// Reloads the run list and open questions, and every watched run.
    public func refreshRuns() {
        runsRefresh.request()
        questionsRefresh.request()
        for (id, watch) in watches {
            watch.board.request()
            watch.limits.request()
            if reports[id] != nil { watch.report.request() }
        }
        if !watches.isEmpty { allApprovalsRefresh.request() }
    }

    func applyRunEvent(_ event: LoggedEvent) {
        if event.changesRuns { runsRefresh.request() }
        if event.changesQuestions { questionsRefresh.request() }
        guard !watches.isEmpty else { return }
        let approval = event.kind.hasPrefix("approval.")
        // A run's agents starting fill in `chief_agent_id` and tasks' `agent_id`.
        let agentStarted = event.kind == EventKind.agentStarted
        let run = event.runNumber
        for (id, watch) in watches {
            let mine = run == nil || run == id
            if (event.changesRuns && mine) || agentStarted { watch.board.request() }
            if approval || (event.kind.hasPrefix("run.") && mine) { watch.limits.request() }
            if mine, event.kind == EventKind.runReportWritten || event.kind == EventKind.runStateChanged {
                watch.report.request()
            }
        }
        if approval { allApprovalsRefresh.request() }
    }

    // MARK: Watching a run

    /// Keeps a run's board, limits, report and approvals current until
    /// `unwatchRun`. Pages call it when they appear; calls are counted.
    public func watchRun(_ id: Int) {
        if let watch = watches[id] {
            watch.count += 1
            return
        }
        let client = client
        let watch = RunWatch(
            board: CoalescedRefresh(delay: runsDebounce, fetch: { try await client.board(runId: id) }) { [weak self] result in
                self?.applyBoard(id, result)
            },
            limits: CoalescedRefresh(delay: runsDebounce, fetch: { try await client.limits(runId: id) }) { [weak self] result in
                guard let self else { return }
                switch result {
                case .success(let limits):
                    self.limits[id] = limits
                    limitsSupported = true
                case .failure(CosError.notFound):
                    // No limits route, or no such run; the board says which.
                    limitsSupported = false
                case .failure:
                    break
                }
            },
            report: CoalescedRefresh(delay: runsDebounce, fetch: { try await client.report(runId: id) }) { [weak self] result in
                guard let self else { return }
                switch result {
                case .success(let markdown): reports[id] = .markdown(markdown)
                case .failure(CosError.notFound): reports[id] = .notWritten
                case .failure(let error): reports[id] = .failed(error.localizedDescription)
                }
            }
        )
        watches[id] = watch
        watch.board.request()
        watch.limits.request()
        allApprovalsRefresh.request()
    }

    public func unwatchRun(_ id: Int) {
        guard let watch = watches[id] else { return }
        watch.count -= 1
        if watch.count <= 0 { watches[id] = nil }
    }

    func applyBoard(_ id: Int, _ result: Result<Board, Error>) {
        switch result {
        case .success(let board):
            store(board)
        case .failure(let error):
            boardErrors[id] = error.localizedDescription
        }
    }

    /// Keeps a board and the run list in step, and fetches the report once
    /// there may be one.
    func store(_ board: Board) {
        let id = board.run.id
        boards[id] = board
        boardErrors[id] = nil
        var run = board.run
        run.counts = board.counts
        if let index = runs.firstIndex(where: { $0.id == id }) {
            runs[index] = run
        } else {
            runs.insert(run, at: 0)
        }
        let mayHaveReport = board.run.state == .done || board.run.state == .reporting || board.run.reportPath != nil
        if mayHaveReport, reports[id] == nil, let watch = watches[id] {
            watch.report.request()
        }
    }

    // MARK: Lookups

    /// The run with its counts: from its board when a page shows it.
    public func run(id: Int) -> Run? {
        if let board = boards[id] {
            var run = board.run
            run.counts = board.counts
            return run
        }
        return runs.first { $0.id == id }
    }

    /// A workspace's runs, newest first.
    public func runs(inWorkspace id: Int) -> [Run] {
        runs.filter { $0.workspaceId == id }
    }

    /// The workspace's unfinished run. There is at most one.
    public func currentRun(inWorkspace id: Int) -> Run? {
        runs.first { $0.workspaceId == id && !$0.state.isFinished }
    }

    /// Unfinished runs, newest first.
    public var activeRuns: [Run] {
        runs.filter { !$0.state.isFinished }
    }

    /// Runs whose plan waits for you, oldest first.
    public var plansAwaitingApproval: [Run] {
        runs.filter { $0.state == .awaitingPlanApproval }.sorted { $0.createdAt < $1.createdAt }
    }

    public func questions(forRun id: Int) -> [Question] {
        questions.filter { $0.runId == id }
    }

    /// Agents a run started: its chief, workers and reviewers.
    public func agents(forRun id: Int) -> [Agent] {
        agents.filter { $0.runNumber == id }
    }

    /// Every approval of a run's agents, newest first, including the ones
    /// rules allowed. Filled while a page watches the run.
    public func approvals(forRun id: Int) -> [Approval] {
        let ids = Set(agents(forRun: id).map(\.id))
        var seen = Set<Int>()
        // Pending ones come from the live list too, in case the full list lags.
        let pending = approvals.filter { ids.contains($0.agentId) }
        return (pending + allApprovals.filter { ids.contains($0.agentId) })
            .filter { seen.insert($0.id).inserted }
            .sorted { $0.createdMs > $1.createdMs }
    }

    /// How many things in this run wait on you: its plan, its questions,
    /// and approvals of its agents routed to you.
    public func waitingCount(forRun id: Int) -> Int {
        let ids = Set(agents(forRun: id).map(\.id))
        let plan = run(id: id)?.state == .awaitingPlanApproval ? 1 : 0
        return plan + questions(forRun: id).count + approvalsForYou.filter { ids.contains($0.agentId) }.count
    }

    // MARK: Actions

    // Like the agent actions, these apply the daemon's answer at once and
    // then refresh. They throw `CosError`s whose descriptions can be shown.

    @discardableResult
    public func createRun(_ request: NewRun) async throws -> Run {
        let run = try await client.createRun(request)
        runs.removeAll { $0.id == run.id }
        runs.insert(run, at: 0)
        runsRefresh.request()
        return run
    }

    @discardableResult
    public func approvePlan(runId: Int) async throws -> Board {
        let board = try await client.approvePlan(runId: runId)
        store(board)
        runsRefresh.request()
        return board
    }

    @discardableResult
    public func cancelRun(id: Int, reason: String? = nil) async throws -> Board {
        let board = try await client.cancelRun(id: id, reason: reason)
        store(board)
        runsRefresh.request()
        questionsRefresh.request()
        return board
    }

    /// Retry (`ready`), fail or cancel a task.
    @discardableResult
    public func updateTask(_ task: RunTask, to state: TaskState, reason: String? = nil) async throws -> Board {
        let board = try await client.updateTask(id: task.id, state: state, reason: reason)
        store(board)
        runsRefresh.request()
        return board
    }

    /// Answers a question. One answered elsewhere meanwhile (409) leaves the
    /// list anyway, and the error is rethrown.
    @discardableResult
    public func answer(_ question: Question, _ text: String) async throws -> Question {
        defer { questionsRefresh.request() }
        do {
            let answered = try await client.answer(questionId: question.id, text)
            questions.removeAll { $0.id == question.id }
            return answered
        } catch CosError.conflict(let message) {
            questions.removeAll { $0.id == question.id }
            throw CosError.conflict(message)
        }
    }

    public func messageChief(runId: Int, text: String) async throws {
        try await client.messageChief(runId: runId, text: text)
    }

    @discardableResult
    public func setLimits(runId: Int, preset: ApprovalPreset, approvalCap: Int) async throws -> RunLimits {
        let limits = try await client.setLimits(runId: runId, preset: preset, approvalCap: approvalCap)
        self.limits[runId] = limits
        watches[runId]?.board.request()
        runsRefresh.request()
        return limits
    }
}
