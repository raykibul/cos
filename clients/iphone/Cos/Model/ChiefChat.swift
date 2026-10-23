import CosKit
import Foundation
import Observation

/// A message you sent that the transcript doesn't show yet.
struct PendingMessage: Identifiable, Equatable, Sendable {
    enum State: Equatable, Sendable {
        case sending
        case sent
        case failed(String)
    }

    var id: Int
    var text: String
    /// The transcript's `next` when it was sent: only later messages can be it.
    var afterN: Int
    var state: State
    /// When you pressed send.
    var createdMs: Int64 = nowMs()
    /// When cos took it.
    var sentMs: Int64?

    /// Its ticks, until it fails.
    var receipt: Receipt? {
        switch state {
        case .sending: .sending
        case .sent: .sent
        case .failed: nil
        }
    }
}

/// The ticks on what you sent, as a messaging app has them.
enum Receipt: Hashable, Sendable {
    /// The phone is sending it.
    case sending
    /// cos has it. While the chief is mid-task it waits in cos's inbox and
    /// goes in when the chief pauses.
    case sent
    /// It is in Claude's conversation.
    case delivered
    /// Claude has started on it: its next entry is Claude's own, or it is
    /// working right after your message.
    case seen
}

/// When each tick of a message happened, for its info sheet.
struct ReceiptTimes: Equatable {
    /// Known only for what you sent from this phone, while the app runs.
    var sentMs: Int64?
    var deliveredMs: Int64
    var seenMs: Int64?
    var receipt: Receipt
}

/// What the chief is doing while it works: its latest tool call, and since when.
struct ChiefActivity: Equatable {
    var text: String
    var sinceMs: Int64
}

func nowMs() -> Int64 {
    Int64(Date().timeIntervalSince1970 * 1000)
}

/// One row of the chat.
enum ChatRow: Identifiable, Equatable {
    /// What you typed, with its ticks.
    case user(TranscriptMessage, Receipt)
    /// What the chief said, as markdown.
    case reply(TranscriptMessage)
    /// One tool call, as a compact row.
    case tool(TranscriptMessage)
    /// cos's `agent_spawn`: a session the chief started, as a card that opens it.
    case spawned(TranscriptMessage, ToolCall)
    /// Claude's `<task-notification>`: a subagent (or background command)
    /// the chief ran finished.
    case task(TranscriptMessage, TaskNotification)
    /// A `[cos · N updates]` wakeup: news from its agents.
    case updates(TranscriptMessage, count: Int)
    case pending(PendingMessage)

    var id: String {
        switch self {
        case .user(let message, _), .reply(let message), .tool(let message), .spawned(let message, _), .task(let message, _),
            .updates(let message, _):
            "m\(message.n)"
        case .pending(let pending): "p\(pending.id)"
        }
    }
}

/// A project's Chief of Staff as a chat: its transcript, read with
/// `after=next` as `agent.transcript` events arrive, what you sent until the
/// transcript shows it, and the composer.
@MainActor
@Observable
final class ChiefChat {
    let projectId: Int
    @ObservationIgnored private let session: HostSession

    private(set) var messages: [TranscriptMessage] = []
    private(set) var next = 0
    private(set) var pending: [PendingMessage] = []
    /// When cos took each message you sent from here, by transcript `n`.
    private(set) var sentAt: [Int: Int64] = [:]
    /// What its agents reported, newest first.
    private(set) var inbox: [StaffItem] = []
    private(set) var isLoading = false
    private(set) var error: String?
    var draft = ""

    /// The agent whose transcript `messages` holds.
    @ObservationIgnored private var agentId: Int?
    @ObservationIgnored private var reading = false
    @ObservationIgnored private var readAgain = false
    @ObservationIgnored private var pendingIds = 0

    init(projectId: Int, session: HostSession) {
        self.projectId = projectId
        self.session = session
    }

    var staff: Staff? { session.staffs[projectId] }
    var project: Project? { session.project(id: projectId) }

    /// Its agent's id, once it has started.
    var chiefAgentId: Int? { staff?.agentId }

    /// Loads the chief, its transcript from the start, and its inbox.
    func load() async {
        isLoading = messages.isEmpty
        defer { isLoading = false }
        do {
            let staff = try await session.service.staff(projectId: projectId)
            session.setStaff(staff)
            error = nil
        } catch {
            if session.staffs[projectId] == nil { self.error = error.localizedDescription }
        }
        await read()
        inbox = (try? await session.service.staffInbox(projectId: projectId)) ?? inbox
    }

    /// Reads what's new in the transcript. Calls made while a read is under
    /// way fold into one more read.
    func read() async {
        if reading {
            readAgain = true
            return
        }
        reading = true
        defer { reading = false }
        repeat {
            readAgain = false
            await readOnce()
        } while readAgain
    }

    private func readOnce() async {
        guard let chief = chiefAgentId else { return }
        if chief != agentId {
            // A new chief agent (it started for the first time): start over.
            agentId = chief
            messages = []
            next = 0
        }
        do {
            let page = try await session.service.transcript(agentId: chief, after: next)
            messages = Transcript.merge(messages, page.messages)
            next = max(next, page.next)
            reconcile()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Sends the draft to the chief, starting it if needed.
    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        draft = ""
        await send(text)
    }

    func send(_ text: String) async {
        pendingIds += 1
        let id = pendingIds
        pending.append(PendingMessage(id: id, text: text, afterN: next, state: .sending))
        do {
            try await session.service.messageStaff(projectId: projectId, text: text)
            update(id) {
                $0.state = .sent
                $0.sentMs = nowMs()
            }
            session.refreshStaff()
            if let staff = try? await session.service.staff(projectId: projectId) { session.setStaff(staff) }
            await read()
        } catch {
            update(id) { $0.state = .failed(error.localizedDescription) }
        }
    }

    /// Sends a failed message again.
    func retry(_ message: PendingMessage) async {
        pending.removeAll { $0.id == message.id }
        await send(message.text)
    }

    func discard(_ message: PendingMessage) {
        pending.removeAll { $0.id == message.id }
    }

    private func update(_ id: Int, _ change: (inout PendingMessage) -> Void) {
        if let index = pending.firstIndex(where: { $0.id == id }) { change(&pending[index]) }
    }

    /// Drops what you sent once the transcript shows it: each pending
    /// message matches one later user message with the same text, or a
    /// wakeup that carried it (text typed while the chief worked).
    private func reconcile() {
        guard !pending.isEmpty else { return }
        var used = Set<Int>()
        pending = pending.filter { item in
            if case .failed = item.state { return true }
            let text = Self.normalize(item.text)
            let match = messages.first { message in
                message.n > item.afterN && !used.contains(message.n) && message.role == .user
                    && (message.kind == .update ? Self.normalize(message.text).contains(text) : Self.normalize(message.text) == text)
            }
            if let match {
                used.insert(match.n)
                if let sent = item.sentMs { sentAt[match.n] = sent }
                return false
            }
            return true
        }
    }

    private static func normalize(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    // MARK: Rows

    var rows: [ChatRow] {
        var rows: [ChatRow] = []
        let answered = answeredAt()
        let lastTyped = messages.last { $0.role == .user && $0.kind == .text }?.n
        let typing = isTyping
        for message in messages {
            switch (message.role, message.kind) {
            case (.user, .update):
                rows.append(.updates(message, count: message.updateCount))
            case (.user, .task), (.user, .text) where message.taskNotification != nil:
                if let note = message.taskNotification { rows.append(.task(message, note)) }
            case (.user, .text):
                if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    let seen = answered[message.n] != nil || (typing && message.n == lastTyped)
                    rows.append(.user(message, seen ? .seen : .delivered))
                }
            case (.user, _):
                // A tool result logged as a user turn, or something newer.
                continue
            case (_, .tool):
                if let tool = message.tool, tool.isSpawn, !tool.isError {
                    rows.append(.spawned(message, tool))
                } else {
                    rows.append(.tool(message))
                }
            default:
                if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { rows.append(.reply(message)) }
            }
        }
        rows += pending.map { .pending($0) }
        return rows
    }

    /// For each message in the transcript that is yours, when Claude's next
    /// entry came: the moment it had seen it.
    private func answeredAt() -> [Int: Int64] {
        var answered: [Int: Int64] = [:]
        var next: Int64?
        for message in messages.reversed() {
            if message.role == .user {
                if let next { answered[message.n] = next }
            } else {
                next = message.tsMs
            }
        }
        return answered
    }

    /// When each tick of one of your messages happened.
    func receiptTimes(for message: TranscriptMessage) -> ReceiptTimes {
        let seenMs = messages.first { $0.n > message.n && $0.role != .user }?.tsMs
        let lastTyped = messages.last { $0.role == .user && $0.kind == .text }?.n
        let seen = seenMs != nil || (isTyping && message.n == lastTyped)
        return ReceiptTimes(sentMs: sentAt[message.n], deliveredMs: message.tsMs, seenMs: seenMs, receipt: seen ? .seen : .delivered)
    }

    // MARK: Typing

    /// The chief is mid-turn: the chat shows it typing.
    var isTyping: Bool {
        guard let state = session.agent(id: chiefAgentId)?.state ?? staff?.state else { return false }
        return state == .working || state == .starting
    }

    /// While it works, its latest tool call ("cargo test", "Read
    /// src/app.ts"), unless it has written since.
    var activity: ChiefActivity? {
        guard isTyping else { return nil }
        // Tool results come back as user entries; skip them.
        guard let last = messages.last(where: { $0.role != .user || $0.kind == .text }), last.role != .user,
            let tool = last.tool, last.kind == .tool
        else { return nil }
        return ChiefActivity(text: Self.describe(tool), sinceMs: last.tsMs)
    }

    /// A tool call as one short line: Bash's command, else the tool and its argument.
    static func describe(_ tool: ToolCall) -> String {
        let summary = tool.summary.trimmingCharacters(in: .whitespacesAndNewlines)
        if tool.isCosTool { return tool.title }
        if summary.isEmpty { return tool.title }
        if tool.name == "Bash" { return summary }
        return "\(tool.title) \(summary)"
    }

    /// The session an `agent_spawn` started: by the id in its result, or,
    /// from an older daemon, the chief's agent with the same title.
    func spawnedAgent(_ tool: ToolCall) -> Agent? {
        if let id = tool.agentId { return session.agent(id: id) }
        return session.agentsStarted(byProject: projectId).first { $0.title == tool.summary }
    }

    /// The inbox items an update carried: reported after the previous update
    /// and by the time this one was typed in.
    func items(for update: TranscriptMessage) -> [StaffItem] {
        let updates = messages.filter { $0.kind == .update }
        guard let index = updates.firstIndex(where: { $0.n == update.n }), update.tsMs > 0 else { return [] }
        let since = index > 0 ? updates[index - 1].tsMs : 0
        return inbox
            .filter { $0.createdMs > since && $0.createdMs <= update.tsMs + 2000 }
            .sorted { $0.createdMs < $1.createdMs }
    }

    /// Agents it started that wait on you: approvals, trust dialogs, logins.
    var agentsWaiting: [(agent: Agent, approval: Approval?)] {
        session.agentsStarted(byProject: projectId).compactMap { agent in
            if let approval = session.pendingApproval(forAgent: agent.id), approval.isForYou { return (agent, approval) }
            if agent.state == .needsTrust || agent.state == .needsLogin { return (agent, nil) }
            return nil
        }
    }

    static let suggestions = [
        "Plan the next feature and split it across agents",
        "Review what every workspace is doing right now",
        "Collect the open questions from running agents",
    ]
}
