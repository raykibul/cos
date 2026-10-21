import AppKit
import CosKit
import SwiftUI

/// One run: its state, the plan or board, questions, the report, limits,
/// a box to message the chief, and what its agents asked to do.
struct RunPage: View {
    let runId: Int
    let monitor: DaemonMonitor
    let window: WindowState

    private var board: Board? { monitor.boards[runId] }

    var body: some View {
        Group {
            if let run = monitor.run(id: runId) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        RunHeader(run: run, workspace: board?.workspace ?? monitor.workspace(id: run.workspaceId), monitor: monitor, window: window)
                        RunStateBanner(run: run, board: board, monitor: monitor, window: window)
                        ForEach(monitor.questions(forRun: runId)) { question in
                            QuestionCard(question: question, monitor: monitor, window: window, showsRun: false)
                        }
                        if run.state == .done || run.state == .reporting || run.reportPath != nil {
                            ReportSection(run: run, report: monitor.reports[runId], home: monitor.status?.home)
                        }
                        if let board {
                            if run.state == .awaitingPlanApproval {
                                PlanReview(board: board, monitor: monitor, window: window)
                            } else if !board.tasks.isEmpty {
                                TaskBoard(board: board, monitor: monitor, window: window)
                            }
                        } else if let error = monitor.boardErrors[runId] {
                            Text(error).foregroundStyle(.red).textSelection(.enabled)
                        } else {
                            ProgressView("Loading the board…")
                                .frame(maxWidth: .infinity, minHeight: 80)
                        }
                        if !run.state.isFinished {
                            MessageChiefBox(run: run, monitor: monitor)
                        }
                        LimitsRow(run: run, monitor: monitor, window: window)
                        RunActivity(run: run, monitor: monitor, window: window)
                    }
                    .padding(24)
                    .frame(maxWidth: 980, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .navigationTitle(run.goal)
            } else if !monitor.runsSupported {
                ContentUnavailableView("This cos daemon doesn't do runs yet", systemImage: "list.bullet.clipboard")
            } else if let error = monitor.boardErrors[runId] {
                ContentUnavailableView("No such run", systemImage: "list.bullet.clipboard", description: Text(error))
            } else {
                ProgressView()
            }
        }
        .onAppear { monitor.watchRun(runId) }
        .onDisappear { monitor.unwatchRun(runId) }
    }
}

// MARK: Header

private struct RunHeader: View {
    let run: Run
    let workspace: Workspace?
    let monitor: DaemonMonitor
    let window: WindowState

    private var canChange: Bool { monitor.connection == .running }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(run.goal)
                        .font(.title2.weight(.semibold))
                        .lineLimit(3)
                        .textSelection(.enabled)
                    RunStateBadge(state: run.state)
                }
                HStack(spacing: 6) {
                    if let workspace {
                        Button(workspace.title) { window.selection = .workspace(workspace.id) }
                            .buttonStyle(.link)
                        Text("·")
                    }
                    Text("Run \(run.id)")
                    Text("·")
                    Text("started \(run.createdAt.formatted(.relative(presentation: .named)))")
                    if run.state.isFinished {
                        Text("·")
                        Text("ended \(run.updatedAt.formatted(.relative(presentation: .named)))")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            if let chief = run.chiefAgentId {
                Button("Chief's Terminal", systemImage: "terminal") { window.selection = .agent(chief) }
                    .help("The chief is a Claude Code session like any other: watch it or type to it.")
            }
            if !run.state.isFinished {
                Button("Cancel Run…", systemImage: "xmark.circle") { window.cancellingRun = run }
                    .disabled(!canChange)
            }
        }
    }
}

/// What the run is doing, in a sentence, with progress or the next step.
private struct RunStateBanner: View {
    let run: Run
    let board: Board?
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        switch run.state {
        case .planning:
            banner(.purple, spinner: true, "The chief is reading the repo and writing a plan.",
                   "Nothing in the workspace changes until you approve the plan. You can message the chief below.")
        case .awaitingPlanApproval:
            banner(.orange, systemImage: "list.bullet.clipboard.fill",
                   "The chief proposes \(Format.count(board?.tasks.count ?? run.counts?.total ?? 0, "task")).",
                   "Read the plan below. Approving it starts the workers; to change it, message the chief instead.") {
                ApprovePlanButton(run: run, monitor: monitor, window: window)
            }
        case .running:
            VStack(alignment: .leading, spacing: 8) {
                RunProgress(run: run)
                if let counts = run.counts, counts.blocked > 0 {
                    Label("\(Format.count(counts.blocked, "task")) blocked. The chief decides what to do next; you can also retry, fail or cancel it below.", systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.orange)
                }
            }
        case .reporting:
            banner(.teal, spinner: true, "Every task is finished. The chief is writing the report.", nil)
        case .done:
            banner(.green, systemImage: "checkmark.circle.fill", "Done. \(run.counts.map(\.summary) ?? "")", nil)
        case .failed:
            banner(.red, systemImage: "xmark.octagon.fill", "The run failed.", run.reason)
        case .cancelled:
            banner(.secondary, systemImage: "slash.circle", "The run was cancelled.", run.reason)
        default:
            banner(.secondary, systemImage: "questionmark.circle", "The run is \(run.state.title.lowercased()).", nil)
        }
    }

    private func banner(
        _ tint: Color, spinner: Bool = false, systemImage: String? = nil, _ title: String, _ detail: String?,
        @ViewBuilder actions: () -> some View = { EmptyView() }
    ) -> some View {
        HStack(spacing: 12) {
            if spinner {
                ProgressView().controlSize(.small)
            } else if let systemImage {
                Image(systemName: systemImage).font(.title2).foregroundStyle(tint)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).fontWeight(.semibold)
                if let detail, !detail.isEmpty {
                    Text(detail)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 12)
            actions()
        }
        .padding(12)
        .background(tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct ApprovePlanButton: View {
    let run: Run
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        Button("Approve Plan") {
            window.perform("Couldn't approve the plan") { try await monitor.approvePlan(runId: run.id) }
        }
        .buttonStyle(.borderedProminent)
        .disabled(monitor.connection != .running)
    }
}

// MARK: Plan and board

/// The proposed plan, in full, then Approve.
private struct PlanReview: View {
    let board: Board
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("The plan").font(.headline)
            ForEach(board.tasks) { task in
                TaskCard(task: task, board: board, monitor: monitor, window: window, planning: true)
            }
            HStack {
                Text("Approving starts a worker on each task without dependencies, in its own worktree. Each task is checked, reviewed, then merged into \(board.workspace?.branch ?? "the workspace's branch").")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 16)
                ApprovePlanButton(run: board.run, monitor: monitor, window: window)
            }
            .padding(.top, 4)
        }
    }
}

private struct TaskBoard: View {
    let board: Board
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Tasks").font(.headline)
            ForEach(board.tasks) { task in
                TaskCard(task: task, board: board, monitor: monitor, window: window, planning: false)
            }
        }
    }
}

/// One task: what it is, where it stands, and what the checks and the
/// reviewer said.
private struct TaskCard: View {
    let task: RunTask
    let board: Board
    let monitor: DaemonMonitor
    let window: WindowState
    let planning: Bool

    @State private var showsInstructions = false
    @State private var showsTestOutput = false

    private var number: Int { (board.tasks.firstIndex { $0.id == task.id } ?? task.position) + 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("\(number)")
                    .font(.callout.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 18, alignment: .trailing)
                Text(task.title)
                    .font(.body.weight(.semibold))
                    .lineLimit(2)
                if task.retries > 0 {
                    Text("retried")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .help("Each task gets one retry.")
                }
                Spacer()
                if !planning { TaskStateBadge(state: task.state) }
                if !planning { actions }
            }

            VStack(alignment: .leading, spacing: 6) {
                if planning || showsInstructions {
                    Text(task.instructions)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                } else if !task.instructions.isEmpty {
                    Button {
                        showsInstructions = true
                    } label: {
                        Text(task.instructions).lineLimit(1).foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("Show the instructions")
                }
                if !task.dependsOn.isEmpty {
                    line("After") {
                        Text(task.dependsOn.map { id in
                            let index = board.tasks.firstIndex { $0.id == id }
                            let other = index.map { board.tasks[$0] }
                            let mark = other?.state == .merged ? " ✓" : ""
                            return "\(index.map { "\($0 + 1). " } ?? "")\(other?.title ?? "task \(id)")\(mark)"
                        }.joined(separator: ", "))
                    }
                }
                line("Files") {
                    FlowRow(spacing: 4) {
                        ForEach(task.files, id: \.self) { file in
                            Text(file)
                                .font(.caption.monospaced())
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 4))
                        }
                    }
                }
                if let test = task.testCommand {
                    line("Tests") { Text(test).font(.callout.monospaced()).textSelection(.enabled) }
                }
                if let branch = task.branch, !planning {
                    line("Branch") { Text(branch).font(.callout.monospaced()).textSelection(.enabled) }
                }
                if let checks = task.checks { checksLine(checks) }
                if let review = task.review { reviewLine(review) }
            }
            .padding(.leading, 26)

            if let failure = task.lastFailure, !failure.isEmpty, !planning {
                Label {
                    Text(failure).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: task.state == .blocked || task.state == .failed ? "exclamationmark.triangle.fill" : "arrow.uturn.backward")
                }
                .font(.callout)
                .foregroundStyle(task.state == .failed ? Color.red : Color.orange)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background((task.state == .failed ? Color.red : Color.orange).opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                .padding(.leading, 26)
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
        .overlay(alignment: .leading) {
            // A coloured edge for the state, so the board reads at a glance.
            UnevenRoundedRectangle(topLeadingRadius: 10, bottomLeadingRadius: 10)
                .fill(planning ? Color.secondary.opacity(0.3) : task.state.color)
                .frame(width: 4)
        }
    }

    @ViewBuilder
    private var actions: some View {
        let canChange = monitor.connection == .running && !board.run.state.isFinished
        HStack(spacing: 6) {
            if let agent = task.agentId {
                Button {
                    window.selection = .agent(agent)
                } label: {
                    Image(systemName: "terminal")
                }
                .help("The worker's terminal")
            }
            Menu {
                if let agent = task.agentId {
                    Button("Open Worker's Terminal") { window.selection = .agent(agent) }
                    Divider()
                }
                Button("Retry…") { window.taskAction = TaskAction(task: task, kind: .retry) }
                    .disabled(!canChange || !task.canRetry)
                Button("Fail…") { window.taskAction = TaskAction(task: task, kind: .fail) }
                    .disabled(!canChange || !task.canFail)
                Button("Cancel Task…") { window.taskAction = TaskAction(task: task, kind: .cancel) }
                    .disabled(!canChange || !task.canCancel)
                if let worktree = task.worktree {
                    Divider()
                    Button("Reveal Worktree in Finder") { Files.reveal(worktree) }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
        }
    }

    private func line(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 48, alignment: .leading)
            content()
                .font(.callout)
        }
    }

    @ViewBuilder
    private func checksLine(_ checks: TaskChecks) -> some View {
        line("Checks") {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 12) {
                    if let scope = checks.scope {
                        Label(scope.ok ? "In scope" : "Out of scope", systemImage: scope.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                            .foregroundStyle(scope.ok ? .green : .red)
                    }
                    if let tests = checks.tests {
                        Button {
                            showsTestOutput.toggle()
                        } label: {
                            Label(testTitle(tests), systemImage: tests.skipped ? "minus.circle" : tests.passed ? "checkmark.circle.fill" : "xmark.circle.fill")
                                .foregroundStyle(tests.skipped ? Color.secondary : tests.passed ? .green : .red)
                        }
                        .buttonStyle(.plain)
                        .disabled(tests.outputTail?.isEmpty ?? true)
                        .help("Show the end of the test output")
                    }
                }
                if let scope = checks.scope, !scope.problems.isEmpty {
                    ForEach(scope.problems, id: \.self) { problem in
                        Text(problem).font(.caption.monospaced()).foregroundStyle(.red).textSelection(.enabled)
                    }
                }
                if showsTestOutput, let tail = checks.tests?.outputTail {
                    ScrollView(.horizontal) {
                        Text(tail)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .padding(8)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.black.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
                }
            }
        }
    }

    private func testTitle(_ tests: TestCheck) -> String {
        switch tests.status {
        case "passed": "Tests passed"
        case "failed": "Tests failed" + (tests.exitCode.map { " (exit \($0))" } ?? "")
        case "timed_out": "Tests timed out"
        case "skipped": "No test command"
        default: "Tests: \(tests.status.replacingOccurrences(of: "_", with: " "))"
        }
    }

    private func reviewLine(_ review: TaskReview) -> some View {
        line("Review") {
            VStack(alignment: .leading, spacing: 2) {
                Label(review.approved ? "Approved" : (review.verdict == "changes" ? "Changes asked" : review.verdict.capitalized),
                      systemImage: review.approved ? "checkmark.seal.fill" : "arrow.uturn.backward.circle.fill")
                    .foregroundStyle(review.approved ? .green : .orange)
                if let notes = review.notes, !notes.isEmpty {
                    Text(notes)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

// MARK: Report

private struct ReportSection: View {
    let run: Run
    let report: RunReportState?
    let home: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Report").font(.headline)
                Spacer()
                if case .markdown(let markdown)? = report {
                    Button("Copy", systemImage: "doc.on.doc") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(markdown, forType: .string)
                    }
                }
                if let url = run.reportURL(home: home) {
                    Button("Reveal in Finder", systemImage: "folder") { Files.reveal(url.path) }
                        .disabled(!FileManager.default.fileExists(atPath: url.path))
                }
            }
            switch report {
            case .markdown(let markdown)?:
                MarkdownView(markdown: markdown)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.background, in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.quaternary))
            case .failed(let message)?:
                Text(message).foregroundStyle(.red).textSelection(.enabled)
            case .notWritten?:
                Text(run.state == .done
                     ? "This cos daemon doesn't serve the report yet." + (run.reportPath.map { " It is at \($0)." } ?? "")
                     : "The chief hasn't written it yet.")
                    .foregroundStyle(.secondary)
            case nil:
                if run.state == .reporting {
                    Text("The chief hasn't written it yet.").foregroundStyle(.secondary)
                } else {
                    ProgressView().controlSize(.small)
                }
            }
        }
    }
}

/// Markdown in blocks: headings, lists, code blocks and paragraphs, with
/// inline styling from `AttributedString(markdown:)`.
struct MarkdownView: View {
    let markdown: String

    enum Block: Hashable {
        case heading(Int, String)
        case item(String, String)
        case code(String)
        case paragraph(String)
        case rule
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.blocks(markdown).enumerated()), id: \.offset) { _, block in
                switch block {
                case .heading(let level, let text):
                    inline(text)
                        .font(level == 1 ? .title2.weight(.semibold) : level == 2 ? .title3.weight(.semibold) : .headline)
                        .padding(.top, level <= 2 ? 6 : 2)
                case .item(let marker, let text):
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(marker).foregroundStyle(.secondary).monospacedDigit()
                        inline(text).fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.leading, 8)
                case .code(let text):
                    Text(text)
                        .font(.callout.monospaced())
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
                case .paragraph(let text):
                    inline(text).fixedSize(horizontal: false, vertical: true)
                case .rule:
                    Divider()
                }
            }
        }
        .textSelection(.enabled)
    }

    private func inline(_ text: String) -> Text {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        if let attributed = try? AttributedString(markdown: text, options: options) {
            return Text(attributed)
        }
        return Text(text)
    }

    static func blocks(_ markdown: String) -> [Block] {
        var blocks: [Block] = []
        var paragraph: [String] = []
        var code: [String]?
        func flush() {
            if !paragraph.isEmpty { blocks.append(.paragraph(paragraph.joined(separator: " "))) }
            paragraph = []
        }
        for raw in markdown.components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                if let lines = code {
                    blocks.append(.code(lines.joined(separator: "\n")))
                    code = nil
                } else {
                    flush()
                    code = []
                }
                continue
            }
            if code != nil {
                code!.append(raw)
                continue
            }
            if line.isEmpty {
                flush()
            } else if let hashes = line.firstIndex(where: { $0 != "#" }), line.hasPrefix("#"), line[hashes] == " " {
                flush()
                blocks.append(.heading(line.distance(from: line.startIndex, to: hashes), String(line[hashes...]).trimmingCharacters(in: .whitespaces)))
            } else if line == "---" || line == "***" {
                flush()
                blocks.append(.rule)
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                flush()
                blocks.append(.item("•", String(line.dropFirst(2))))
            } else if let dot = line.firstIndex(of: "."), line[..<dot].allSatisfy(\.isNumber), !line[..<dot].isEmpty,
                      line[line.index(after: dot)...].hasPrefix(" ") {
                flush()
                blocks.append(.item(String(line[...dot]), String(line[line.index(dot, offsetBy: 2)...])))
            } else {
                paragraph.append(line)
            }
        }
        if let code { blocks.append(.code(code.joined(separator: "\n"))) }
        flush()
        return blocks
    }
}

// MARK: Messages and limits

private struct MessageChiefBox: View {
    let run: Run
    let monitor: DaemonMonitor

    @State private var text = ""
    @State private var sending = false
    @State private var note: String?
    @State private var unsupported = false

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Message the chief").font(.headline)
            if unsupported {
                Text("This cos daemon doesn't take messages for the chief yet. You can type into the chief's terminal instead.")
                    .foregroundStyle(.secondary)
            } else {
                HStack(alignment: .bottom) {
                    TextField("Message", text: $text, prompt: Text(run.state == .awaitingPlanApproval ? "Ask for changes to the plan, add a constraint…" : "Add a constraint, change course, ask how it's going…"), axis: .vertical)
                        .lineLimit(1...6)
                        .textFieldStyle(.roundedBorder)
                    Button("Send") { send() }
                        .disabled(sending || trimmed.isEmpty || monitor.connection != .running)
                }
                Text(note ?? "It reaches the chief the next time it's idle, with anything else waiting for it.")
                    .font(.caption)
                    .foregroundStyle(note?.hasPrefix("Couldn't") == true ? Color.red : Color.secondary)
            }
        }
    }

    private func send() {
        sending = true
        let message = trimmed
        Task {
            do {
                try await monitor.messageChief(runId: run.id, text: message)
                text = ""
                note = "Sent. The chief reads it the next time it's idle."
            } catch CosError.notFound {
                unsupported = true
            } catch {
                note = "Couldn't send it: \(error.localizedDescription)"
            }
            sending = false
        }
    }
}

private struct LimitsRow: View {
    let run: Run
    let monitor: DaemonMonitor
    let window: WindowState

    private var limits: RunLimits? { monitor.limits[run.id] }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text("Approvals").font(.headline)
                Spacer()
                if let limits, !run.state.isFinished {
                    Button("Change…") { window.editingLimits = LimitsEdit(run: run, limits: limits) }
                        .disabled(monitor.connection != .running)
                }
            }
            let preset = limits?.preset ?? run.preset
            let cap = limits?.approvalCap ?? run.approvalCap
            HStack(spacing: 6) {
                Text(preset.title).fontWeight(.medium)
                Text("·")
                if preset == .strict {
                    Text("the chief approves nothing for you")
                } else if let limits {
                    Text("the chief has approved \(limits.chiefApprovals) of up to \(cap)")
                        .monospacedDigit()
                    if limits.capReached {
                        Text("· cap reached, everything asks you").foregroundStyle(.orange)
                    }
                } else {
                    Text("the chief may approve up to \(cap)")
                }
            }
            .font(.callout)
            Text(preset.explanation + (monitor.limitsSupported ? "" : " This cos daemon doesn't report the chief's approvals yet."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

// MARK: Activity

/// What the run's agents asked to do, who answered, and why.
private struct RunActivity: View {
    let run: Run
    let monitor: DaemonMonitor
    let window: WindowState

    @State private var showsAll = false

    var body: some View {
        let approvals = monitor.approvals(forRun: run.id)
        let agents = monitor.agents(forRun: run.id)
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Activity").font(.headline)
                Spacer()
                if !agents.isEmpty {
                    Text(Format.count(agents.count, "agent")).font(.caption).foregroundStyle(.secondary)
                }
            }
            if approvals.isEmpty {
                Text(run.state.isFinished
                     ? "No permission requests from this run's agents."
                     : "No permission requests yet. Requests from the chief, workers and reviewers show up here, including the ones rules allowed.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                let auto = approvals.filter { $0.decidedBy == "rule" && $0.state == .allowed }.count
                let chief = approvals.filter { $0.decidedBy == "chief" && $0.state == .allowed }.count
                if auto + chief > 0 {
                    Text("Allowed for you: \(auto) by rule, \(chief) by the chief.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                VStack(spacing: 0) {
                    let shown = showsAll ? approvals : Array(approvals.prefix(12))
                    ForEach(shown) { approval in
                        ActivityRow(approval: approval, agent: monitor.agent(id: approval.agentId), monitor: monitor, window: window)
                        if approval.id != shown.last?.id { Divider() }
                    }
                }
                .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                if approvals.count > 12, !showsAll {
                    Button("Show all \(approvals.count)") { showsAll = true }.buttonStyle(.link)
                }
            }
        }
    }
}

private struct ActivityRow: View {
    let approval: Approval
    let agent: Agent?
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: approval.routeSymbol)
                .foregroundStyle(approval.outcomeColor)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(approval.detail)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                }
                HStack(spacing: 6) {
                    if let agent {
                        Button(agent.title) { window.selection = .agent(agent.id) }
                            .buttonStyle(.link)
                        Text("·")
                    }
                    Text(approval.outcome).foregroundStyle(approval.outcomeColor)
                    Text("·")
                    Text(approval.createdAt, format: .relative(presentation: .named))
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                if let why = approval.routeExplanation {
                    Text(why)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            if approval.state == .pending {
                ApprovalButtons(approval: approval, monitor: monitor, window: window, compact: true)
                    .help(approval.route == .chief ? "The chief decides this one; you can answer first." : "")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }
}
