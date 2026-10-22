import CosKit
import SwiftUI

/// Every run, active first, with progress.
struct RunsView: View {
    var session: HostSession
    @State private var newRun = false

    var body: some View {
        let active = session.runs.filter { !$0.state.isFinished }
        let finished = session.runs.filter(\.state.isFinished)
        List {
            if session.runs.isEmpty {
                ContentUnavailableView("No runs yet", systemImage: "list.bullet.clipboard", description: Text("Start one with a goal: a chief plans it, workers do it, and you get a report."))
                    .listRowBackground(Color.clear)
            }
            if !active.isEmpty {
                Section("Active") {
                    ForEach(active) { run in
                        NavigationLink(value: Route.run(id: run.id)) { RunRow(run: run, place: place(run)) }
                    }
                }
            }
            if !finished.isEmpty {
                Section("Finished") {
                    ForEach(finished) { run in
                        NavigationLink(value: Route.run(id: run.id)) { RunRow(run: run, place: place(run)) }
                    }
                }
            }
        }
        .pageBackground()
        .refreshable { await session.refresh() }
        .navigationTitle("Runs")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("New run", systemImage: "plus") { newRun = true }
            }
        }
        .sheet(isPresented: $newRun) { NewRunSheet(session: session, workspaceId: nil) }
    }

    private func place(_ run: Run) -> String {
        [session.project(ofWorkspace: run.workspaceId)?.name, session.workspaceName(run.workspaceId)].compactMap { $0 }.joined(separator: " · ")
    }
}

struct RunRow: View {
    var run: Run
    var place: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(run.goal)
                    .font(.body.weight(.medium))
                    .lineLimit(2)
                Spacer(minLength: 6)
                StateBadge(style: run.state.style)
            }
            if let counts = run.counts, counts.total > 0, !run.state.isFinished || run.state == .done {
                HStack(spacing: 8) {
                    ProgressView(value: counts.progress)
                        .tint(run.state == .done ? Palette.green : Palette.blue)
                    Text("\(counts.merged)/\(counts.total)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(Palette.ink3)
                }
            }
            HStack(spacing: 6) {
                if let place, !place.isEmpty { Text(place) }
                Text(shortAgo(run.updatedAt))
                if let blocked = run.counts?.blocked, blocked > 0 {
                    Text("\(blocked) blocked").foregroundStyle(Palette.amber)
                }
            }
            .font(.caption)
            .foregroundStyle(Palette.ink3)
        }
        .padding(.vertical, 3)
    }
}

/// A run: its goal, the plan to approve, the board, questions, the report,
/// and Cancel.
struct RunView: View {
    var session: HostSession
    var runId: Int
    @State private var detail: RunDetail
    @State private var confirmCancel = false
    @State private var questionError: String?

    init(session: HostSession, runId: Int) {
        self.session = session
        self.runId = runId
        _detail = State(initialValue: RunDetail(runId: runId, session: session))
    }

    var body: some View {
        List {
            if let run = detail.run {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        StateBadge(style: run.state.style)
                        Text(run.goal)
                            .font(.title3.weight(.semibold))
                        Text([session.project(ofWorkspace: run.workspaceId)?.name, session.workspaceName(run.workspaceId), "preset \(run.preset.rawValue)"].compactMap { $0 }.joined(separator: " · "))
                            .font(.footnote)
                            .foregroundStyle(Palette.ink3)
                        if let reason = run.reason, !reason.isEmpty {
                            Text(reason)
                                .font(.footnote)
                                .foregroundStyle(run.state == .failed ? Palette.red : Palette.ink2)
                        }
                        if run.state == .awaitingPlanApproval {
                            Text("The chief proposed \(detail.board?.tasks.count ?? 0) tasks. Workers start once you approve.")
                                .font(.subheadline)
                                .foregroundStyle(Palette.ink2)
                                .padding(.top, 4)
                            Button("Approve plan") { Task { await detail.approvePlan() } }
                                .buttonStyle(ClaudeButtonStyle())
                                .disabled(detail.isWorking)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }

            if !detail.questions.isEmpty {
                Section {
                    ForEach(detail.questions) { question in
                        QuestionCard(session: session, question: question, error: $questionError)
                            .padding(.vertical, 4)
                    }
                }
            }

            if let board = detail.board, !board.tasks.isEmpty {
                Section {
                    ForEach(board.tasks) { task in
                        TaskRow(task: task, board: board)
                    }
                } header: {
                    HStack {
                        Text(detail.run?.state == .awaitingPlanApproval ? "Plan" : "Board")
                        Spacer()
                        Text(detail.run?.state == .awaitingPlanApproval ? "\(board.tasks.count) tasks" : "\(board.counts.merged) of \(board.counts.total) merged")
                            .textCase(nil)
                            .foregroundStyle(Palette.ink3)
                    }
                }
            }

            if let report = detail.report {
                Section("Report") {
                    MarkdownView(markdown: report, font: .subheadline)
                        .textSelection(.enabled)
                        .padding(.vertical, 6)
                }
            }

            if let error = detail.error ?? questionError {
                Section { Text(error).font(.footnote).foregroundStyle(Palette.red) }
            }

            if let run = detail.run, !run.state.isFinished {
                Section {
                    Button("Cancel run", role: .destructive) { confirmCancel = true }
                        .disabled(detail.isWorking)
                } footer: {
                    Text("Stops its chief and workers. Merged tasks stay merged.")
                }
            }
        }
        .pageBackground()
        .navigationTitle("Run \(runId)")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await detail.load() }
        .task(id: session.runTick) { await detail.load() }
        .confirmationDialog("Cancel this run?", isPresented: $confirmCancel, titleVisibility: .visible) {
            Button("Cancel run", role: .destructive) { Task { await detail.cancel() } }
            Button("Keep it running", role: .cancel) {}
        }
    }
}

struct TaskRow: View {
    var task: RunTask
    var board: Board
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: task.state.style.symbol)
                    .font(.footnote)
                    .foregroundStyle(task.state.style.color)
                Text(task.title)
                    .font(.subheadline.weight(.medium))
                Spacer(minLength: 6)
                Text(task.state.style.label)
                    .font(.caption)
                    .foregroundStyle(task.state.style.color)
            }
            if !dependencies.isEmpty {
                Text("after " + dependencies.joined(separator: ", "))
                    .font(.caption)
                    .foregroundStyle(Palette.ink3)
                    .padding(.leading, 22)
            }
            if let failure = task.lastFailure, !failure.isEmpty {
                Text(failure)
                    .font(.caption)
                    .foregroundStyle(task.state == .blocked ? Palette.amber : Palette.ink2)
                    .padding(.leading, 22)
            }
            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    if !task.instructions.isEmpty {
                        Text(task.instructions).font(.footnote).foregroundStyle(Palette.ink2)
                    }
                    if !task.files.isEmpty {
                        CodeBox(text: task.files.joined(separator: "\n"))
                    }
                    if let tests = task.checks?.tests {
                        Label("Tests \(tests.status)", systemImage: tests.passed ? "checkmark.circle" : "xmark.circle")
                            .font(.caption)
                            .foregroundStyle(tests.passed ? Palette.green : Palette.red)
                    }
                    if let review = task.review {
                        Label("Review: \(review.verdict)", systemImage: review.approved ? "hand.thumbsup" : "arrow.uturn.backward")
                            .font(.caption)
                            .foregroundStyle(review.approved ? Palette.green : Palette.amber)
                    }
                }
                .padding(.leading, 22)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture { withAnimation(.snappy) { expanded.toggle() } }
    }

    private var dependencies: [String] {
        task.dependsOn.compactMap { id in board.task(id: id)?.title }
    }
}
