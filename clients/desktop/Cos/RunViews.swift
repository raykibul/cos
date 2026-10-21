import CosKit
import SwiftUI

// MARK: States

extension RunState {
    var title: String {
        switch self {
        case .planning: "Planning"
        case .awaitingPlanApproval: "Plan ready"
        case .running: "Running"
        case .reporting: "Writing report"
        case .done: "Done"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        default: rawValue.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    var color: Color {
        switch self {
        case .planning: .purple
        case .awaitingPlanApproval: .orange
        case .running: .blue
        case .reporting: .teal
        case .done: .green
        case .failed: .red
        default: .secondary
        }
    }

    /// Something is happening without you.
    var isBusy: Bool { self == .planning || self == .running || self == .reporting }
    var needsYou: Bool { self == .awaitingPlanApproval }

    var symbol: String {
        switch self {
        case .done: "checkmark.circle.fill"
        case .failed: "xmark.octagon.fill"
        case .cancelled: "slash.circle"
        case .awaitingPlanApproval: "list.bullet.clipboard.fill"
        default: "list.bullet.clipboard"
        }
    }
}

extension TaskState {
    var title: String {
        switch self {
        case .proposed: "Proposed"
        case .ready: "Ready"
        case .running: "Working"
        case .checking: "Checking"
        case .review: "In review"
        case .merged: "Merged"
        case .blocked: "Blocked"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        default: rawValue.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    var color: Color {
        switch self {
        case .ready: .teal
        case .running: .blue
        case .checking: .indigo
        case .review: .purple
        case .merged: .green
        case .blocked: .orange
        case .failed: .red
        default: .secondary
        }
    }

    var isBusy: Bool { self == .running || self == .checking || self == .review }
}

extension ApprovalPreset {
    var title: String {
        switch self {
        case .strict: "Strict"
        case .normal: "Normal"
        case .trusted: "Trusted"
        default: rawValue.capitalized
        }
    }

    /// One line on what the chief may approve for you.
    var explanation: String {
        switch self {
        case .strict: "The chief approves nothing. Every request the rules don't allow asks you."
        case .normal: "The chief may approve builds, scripts and edits inside a task's worktree. Network access and installs ask you."
        case .trusted: "The chief may also approve downloads, network reads and installs into the worktree."
        default: "A preset this app doesn't know yet."
        }
    }
}

/// A coloured capsule with a title: a spinner while busy, a mark when it needs you.
struct StateCapsule: View {
    let title: String
    let color: Color
    var busy = false
    var attention = false

    var body: some View {
        HStack(spacing: 5) {
            if busy {
                ProgressView().controlSize(.mini)
            } else if attention {
                Image(systemName: "exclamationmark.circle.fill")
            } else {
                Circle().fill(color).frame(width: 7, height: 7)
            }
            Text(title)
        }
        .font(.caption.weight(.medium))
        .foregroundStyle(attention ? Color.orange : Color.primary.opacity(0.8))
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(color.opacity(attention ? 0.18 : 0.13), in: Capsule())
        .fixedSize()
    }
}

struct RunStateBadge: View {
    let state: RunState
    var body: some View {
        StateCapsule(title: state.title, color: state.color, busy: state.isBusy, attention: state.needsYou)
    }
}

struct TaskStateBadge: View {
    let state: TaskState
    var body: some View {
        StateCapsule(title: state.title, color: state.color, busy: state.isBusy, attention: state.needsDecision)
    }
}

extension TaskCounts {
    /// `2 of 4 merged · 1 working · 1 blocked`.
    var summary: String {
        var parts = ["\(merged) of \(total) merged"]
        if active > 0 { parts.append("\(active) working") }
        if ready > 0 { parts.append("\(ready) ready") }
        if blocked > 0 { parts.append("\(blocked) blocked") }
        if failed > 0 { parts.append("\(failed) failed") }
        if cancelled > 0 { parts.append("\(cancelled) cancelled") }
        return parts.joined(separator: " · ")
    }
}

/// A run's progress: a bar and its counts.
struct RunProgress: View {
    let run: Run

    var body: some View {
        if let counts = run.counts, counts.total > 0 {
            VStack(alignment: .leading, spacing: 3) {
                ProgressView(value: counts.progress)
                    .tint(counts.blocked > 0 ? .orange : run.state.color)
                Text(run.state == .awaitingPlanApproval ? Format.count(counts.total, "task") + " proposed" : counts.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }
}

// MARK: Workspace page

/// The workspace's current run and past runs, and New Run.
struct RunsSection: View {
    let workspace: Workspace
    let monitor: DaemonMonitor
    let window: WindowState

    private var runs: [Run] { monitor.runs(inWorkspace: workspace.id) }
    private var current: Run? { monitor.currentRun(inWorkspace: workspace.id) }
    private var canStart: Bool { monitor.connection == .running && monitor.runsSupported && current == nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Runs").font(.headline)
                Spacer()
                Button("New Run…", systemImage: "plus") { window.newRunIn = workspace }
                    .disabled(!canStart)
                    .help(current != nil ? "This workspace already has a run going. A workspace runs one at a time, since every task merges into its branch." : "Give the chief a goal; it plans tasks for workers.")
            }
            if !monitor.runsSupported {
                Placeholder(
                    systemImage: "list.bullet.clipboard",
                    title: "This cos daemon doesn't do runs yet",
                    detail: "Update cos and restart the daemon to give a goal to a chief here."
                )
            } else if runs.isEmpty {
                Placeholder(
                    systemImage: "list.bullet.clipboard",
                    title: "No runs yet",
                    detail: "A run takes a goal. The chief plans it as tasks, you approve the plan, and workers do the tasks in their own worktrees; each is checked, reviewed and merged into this workspace's branch."
                )
            } else {
                if let current {
                    RunCard(run: current, waiting: monitor.waitingCount(forRun: current.id)) { window.selection = .run(current.id) }
                }
                let past = runs.filter(\.state.isFinished)
                if !past.isEmpty {
                    Text(current == nil ? "Past runs" : "Earlier")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.top, current == nil ? 0 : 4)
                    VStack(spacing: 0) {
                        ForEach(past) { run in
                            PastRunRow(run: run) { window.selection = .run(run.id) }
                            if run.id != past.last?.id { Divider() }
                        }
                    }
                    .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
                }
            }
            if let error = monitor.runsError, monitor.connection == .running {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        }
    }
}

/// The current run: goal, state and progress.
struct RunCard: View {
    let run: Run
    let waiting: Int
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(run.goal)
                        .font(.body.weight(.medium))
                        .lineLimit(2)
                    Spacer()
                    if waiting > 0 {
                        Text("\(waiting) waiting on you")
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.orange)
                    }
                    RunStateBadge(state: run.state)
                }
                RunProgress(run: run)
                HStack(spacing: 6) {
                    Text("Started \(run.createdAt.formatted(.relative(presentation: .named)))")
                    Text("·")
                    Text("\(run.preset.title) approvals")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(run.state.color.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(run.state.color.opacity(0.25)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct PastRunRow: View {
    let run: Run
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 8) {
                Image(systemName: run.state.symbol).foregroundStyle(run.state.color)
                Text(run.goal).lineLimit(1)
                Spacer()
                if let counts = run.counts, counts.total > 0 {
                    Text("\(counts.merged)/\(counts.total) merged")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                Text(run.state.title)
                    .font(.caption)
                    .foregroundStyle(run.state.color)
                Text(run.updatedAt, format: .relative(presentation: .named))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct Placeholder: View {
    let systemImage: String
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: systemImage)
                .font(.title)
                .foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, minHeight: 120)
        .padding()
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10))
    }
}

// MARK: Sheets

/// Goal, preset and cap → `POST /v1/runs`.
struct NewRunSheet: View {
    let workspace: Workspace
    let monitor: DaemonMonitor
    let onCreated: (Run) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var goal = ""
    @State private var preset = ApprovalPreset.normal
    @State private var cap = 20
    @State private var starting = false
    @State private var error: String?

    private var trimmed: String { goal.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("New Run in \(workspace.title)").font(.headline)

            VStack(alignment: .leading, spacing: 4) {
                Text("Goal")
                TextEditor(text: $goal)
                    .font(.body)
                    .frame(minHeight: 110)
                    .scrollContentBackground(.hidden)
                    .padding(6)
                    .background(.background, in: RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.quaternary))
                Text("What should be true when the run is done. The chief reads the repo and plans tasks; nothing changes until you approve the plan.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("What the chief may approve for you")
                PresetPicker(preset: $preset)
                Text("git push, secrets, and anything outside the worktree always ask you.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            CapStepper(cap: $cap)
                .disabled(preset == .strict)

            if let error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }

            HStack {
                if starting {
                    ProgressView().controlSize(.small)
                    Text("Starting the chief…").foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Start Run") { start() }
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.borderedProminent)
                    .disabled(starting || trimmed.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 500)
    }

    private func start() {
        starting = true
        error = nil
        let request = NewRun(workspaceId: workspace.id, goal: trimmed, preset: preset, approvalCap: cap)
        Task {
            do {
                let run = try await monitor.createRun(request)
                onCreated(run)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            starting = false
        }
    }
}

/// The three presets as radio buttons, each with its line of explanation.
struct PresetPicker: View {
    @Binding var preset: ApprovalPreset

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(ApprovalPreset.all) { option in
                Button {
                    preset = option
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Image(systemName: preset == option ? "largecircle.fill.circle" : "circle")
                            .foregroundStyle(preset == option ? Color.accentColor : .secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(option.title + (option == .normal ? " (default)" : ""))
                            Text(option.explanation)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }
}

struct CapStepper: View {
    @Binding var cap: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Stepper(value: $cap, in: 0...500) {
                Text("The chief may approve up to \(cap) requests")
                    .monospacedDigit()
            }
            Text("After that, everything asks you.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

/// Preset and cap of a running run → `PUT /v1/runs/{id}/limits`.
struct LimitsSheet: View {
    let run: Run
    let limits: RunLimits
    let monitor: DaemonMonitor

    @Environment(\.dismiss) private var dismiss
    @State private var preset: ApprovalPreset
    @State private var cap: Int
    @State private var saving = false
    @State private var error: String?

    init(run: Run, limits: RunLimits, monitor: DaemonMonitor) {
        self.run = run
        self.limits = limits
        self.monitor = monitor
        _preset = State(initialValue: limits.preset)
        _cap = State(initialValue: limits.approvalCap)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Approvals for this Run").font(.headline)
            PresetPicker(preset: $preset)
            CapStepper(cap: $cap)
            Text("The chief has approved \(limits.chiefApprovals) so far. Changes apply to requests from now on.")
                .font(.caption)
                .foregroundStyle(.secondary)
            if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(saving || (preset == limits.preset && cap == limits.approvalCap))
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    private func save() {
        saving = true
        error = nil
        Task {
            do {
                try await monitor.setLimits(runId: run.id, preset: preset, approvalCap: cap)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            saving = false
        }
    }
}

/// Retry, fail or cancel a task, with the reason the daemon keeps.
struct TaskAction: Identifiable {
    enum Kind { case retry, fail, cancel }
    let task: RunTask
    let kind: Kind
    var id: String { "\(task.id)-\(kind)" }

    var title: String {
        switch kind {
        case .retry: "Retry “\(task.title)”"
        case .fail: "Fail “\(task.title)”"
        case .cancel: "Cancel “\(task.title)”"
        }
    }

    var button: String {
        switch kind {
        case .retry: "Retry Task"
        case .fail: "Fail Task"
        case .cancel: "Cancel Task"
        }
    }

    var detail: String {
        switch kind {
        case .retry: "The task goes back to ready and a worker starts on it again with your reason. Each task gets one retry."
        case .fail: "The task stops for good, and tasks that depend on it are blocked."
        case .cancel: "The task stops for good, and tasks that depend on it are blocked. Its branch is kept if it has commits."
        }
    }

    var state: TaskState {
        switch kind {
        case .retry: .ready
        case .fail: .failed
        case .cancel: .cancelled
        }
    }

    var needsReason: Bool { kind != .cancel }
}

struct TaskActionSheet: View {
    let action: TaskAction
    let monitor: DaemonMonitor

    @Environment(\.dismiss) private var dismiss
    @State private var reason = ""
    @State private var working = false
    @State private var error: String?

    private var trimmed: String { reason.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(action.title).font(.headline).lineLimit(2)
            Text(action.detail)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 4) {
                TextField("Reason", text: $reason, prompt: Text(action.needsReason ? "Why (required)" : "Why (optional)"), axis: .vertical)
                    .lineLimit(2...5)
                Text(action.kind == .retry ? "The worker sees this." : "Kept as the task's last failure, for the chief and the report.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Don't", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(action.button, role: action.kind == .retry ? nil : .destructive) { perform() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(working || (action.needsReason && trimmed.isEmpty))
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private func perform() {
        working = true
        error = nil
        Task {
            do {
                try await monitor.updateTask(action.task, to: action.state, reason: trimmed)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }
}

/// Cancel a run, with an optional reason.
struct CancelRunSheet: View {
    let run: Run
    let monitor: DaemonMonitor

    @Environment(\.dismiss) private var dismiss
    @State private var reason = ""
    @State private var working = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Cancel this run?").font(.headline)
            Text(run.goal).lineLimit(3).foregroundStyle(.secondary)
            Text("cos stops the chief and every worker, and cancels the unfinished tasks. Merged tasks stay merged, and branches with commits are kept.")
                .fixedSize(horizontal: false, vertical: true)
            TextField("Reason", text: $reason, prompt: Text("Why (optional)"), axis: .vertical)
                .lineLimit(1...4)
            if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Keep Running", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Cancel Run", role: .destructive) { cancel() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(working)
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private func cancel() {
        working = true
        error = nil
        Task {
            do {
                try await monitor.cancelRun(id: run.id, reason: reason)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }
}

// MARK: Questions

/// A chief's question with its choices and a field for your own answer.
struct QuestionCard: View {
    let question: Question
    let monitor: DaemonMonitor
    let window: WindowState
    /// Shows which run asked; off on the run's own page.
    var showsRun = true

    @State private var answer = ""
    @State private var sending = false

    private var trimmed: String { answer.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "questionmark.bubble.fill").foregroundStyle(.orange)
                if showsRun, let id = question.runId {
                    Text("The chief of")
                    Button(monitor.run(id: id)?.goal ?? "run \(id)") { window.selection = .run(id) }
                        .buttonStyle(.link)
                        .lineLimit(1)
                    Text("asks")
                } else {
                    Text("The chief asks")
                }
                Text(question.createdAt, format: .relative(presentation: .named)).foregroundStyle(.secondary)
            }
            .font(.callout)
            Text(question.question)
                .fontWeight(.semibold)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if !question.choices.isEmpty {
                FlowRow(spacing: 6) {
                    ForEach(question.choices, id: \.self) { choice in
                        Button(choice) { send(choice) }
                            .disabled(sending)
                    }
                }
            }
            HStack(alignment: .bottom) {
                TextField("Answer", text: $answer, prompt: Text(question.choices.isEmpty ? "Your answer" : "Or answer in your own words"), axis: .vertical)
                    .lineLimit(1...4)
                    .onSubmit { if !trimmed.isEmpty { send(trimmed) } }
                Button("Send") { send(trimmed) }
                    .buttonStyle(.borderedProminent)
                    .disabled(sending || trimmed.isEmpty)
            }
        }
        .padding(12)
        .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }

    private func send(_ text: String) {
        sending = true
        window.perform("Couldn't answer the question") {
            defer { sending = false }
            do {
                try await monitor.answer(question, text)
            } catch CosError.conflict {
                // Answered elsewhere meanwhile; nothing left to do.
            }
        }
    }
}

/// Lays children out left to right, wrapping onto new lines.
struct FlowRow: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let width = rows.map { $0.width }.max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var items: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = [Row()]
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            if !rows[rows.count - 1].items.isEmpty, rows[rows.count - 1].width + spacing + size.width > width {
                rows.append(Row())
            }
            var row = rows[rows.count - 1]
            row.width += (row.items.isEmpty ? 0 : spacing) + size.width
            row.height = max(row.height, size.height)
            row.items.append(index)
            rows[rows.count - 1] = row
        }
        return rows.filter { !$0.items.isEmpty }
    }
}

// MARK: Approvals: who answers and why

extension Approval {
    /// Why it's where it is: "cos asks you because: …", "Routed to the
    /// chief: …", "Auto-allowed by rule: …". `nil` from a daemon without routes.
    var routeExplanation: String? {
        let reason = reason.flatMap { $0.isEmpty ? nil : $0 }
        switch route {
        case .auto?: return "Auto-allowed by rule" + (reason.map { ": \($0)" } ?? "")
        case .chief?: return "Routed to the chief" + (reason.map { ": \($0)" } ?? "")
        case .user?:
            if tier == .neverDelegated {
                return "cos always asks you: " + (reason ?? "this is never delegated")
            }
            return reason.map { "cos asks you because: \($0)" } ?? "cos asks you."
        case nil: return reason
        default: return reason
        }
    }

    var routeSymbol: String {
        switch route {
        case .auto?: "checkmark.shield"
        case .chief?: "person.badge.shield.checkmark"
        default: "person.fill.questionmark"
        }
    }

    /// `allowed by the chief`, `denied by you`, `pending`.
    var outcome: String {
        let who: String? = switch decidedBy {
        case "user"?: "you"
        case "chief"?: "the chief"
        case "rule"?: "a rule"
        case "terminal"?: "the terminal"
        case let other?: other
        case nil: nil
        }
        switch state {
        case .pending: return route == .chief ? "Waiting for the chief" : "Waiting for you"
        case .expired: return "Expired unanswered"
        case .settled: return "Settled" + (who.map { " in \($0)" } ?? "")
        default: return state.rawValue.capitalized + (who.map { " by \($0)" } ?? "")
        }
    }

    var outcomeColor: Color {
        switch state {
        case .pending: .orange
        case .allowed: .green
        case .denied, .expired: .red
        default: .secondary
        }
    }
}
