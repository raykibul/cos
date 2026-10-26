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
    /// The files it names, as the transcript will show them.
    var attachments: [AttachedFile] = []
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

/// A file on the composer. It uploads into the chief's worktree as soon as
/// you pick it, so sending only names it.
struct DraftFile: Identifiable, Equatable, Sendable {
    enum State: Equatable, Sendable {
        case uploading
        case ready(Attachment)
        case failed(String)
    }

    var id: Int
    var name: String
    var contentType: String
    var size: Int
    /// The picture, for its thumbnail (images only).
    var preview: Data?
    var state: State

    var attachment: Attachment? {
        if case .ready(let attachment) = state { attachment } else { nil }
    }

    var isUploading: Bool { state == .uploading }
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
    /// A session's first prompt, from the Chief of Staff that started it,
    /// without the line cos appends.
    case fromChief(TranscriptMessage, prompt: String)
    /// What the chief (or a session's Claude) said, as markdown.
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
    /// What a `!` command you ran in Claude's shell mode printed, under it.
    case shellOutput(TranscriptMessage, ToolCall)
    case pending(PendingMessage)

    var id: String {
        switch self {
        case .user(let message, _), .fromChief(let message, _), .reply(let message), .tool(let message), .spawned(let message, _),
            .task(let message, _), .updates(let message, _), .shellOutput(let message, _):
            "m\(message.n)"
        case .pending(let pending): "p\(pending.id)"
        }
    }
}

/// Whose conversation a chat is.
enum ChatTarget: Hashable, Sendable {
    /// A project's Chief of Staff, which may not have started yet.
    case chief(projectId: Int)
    /// One session: any agent but a chief.
    case session(agentId: Int)
}

/// A conversation, like a messaging app's, read from an agent's transcript
/// with `after=next` as `agent.transcript` events arrive: a project's Chief
/// of Staff, or one session. It keeps what you sent until the transcript
/// shows it, and the composer: to the chief through `…/staff/message`
/// (which starts it if needed), to a session through its input (resuming it
/// first when it has exited).
@MainActor
@Observable
final class Chat {
    let target: ChatTarget
    @ObservationIgnored private let session: HostSession

    private(set) var messages: [TranscriptMessage] = []
    private(set) var next = 0
    private(set) var pending: [PendingMessage] = []
    /// When cos took each message you sent from here, by transcript `n`.
    private(set) var sentAt: [Int: Int64] = [:]
    /// What the chief's agents reported, newest first.
    private(set) var inbox: [StaffItem] = []
    private(set) var isLoading = false
    private(set) var error: String?
    var draft = ""
    /// The message the composer answers, quoted on top of it.
    var replyingTo: ChatReply?
    /// The files on the composer, in the order you picked them.
    private(set) var files: [DraftFile] = []
    /// Pictures you sent, by attachment name (names are unique in the
    /// agent's folder): picked here, or fetched from the daemon.
    private(set) var images: [String: Data] = [:]

    /// The agent whose transcript `messages` holds.
    @ObservationIgnored private var readingAgentId: Int?
    @ObservationIgnored private var reading = false
    @ObservationIgnored private var readAgain = false
    @ObservationIgnored private var pendingIds = 0
    @ObservationIgnored private var fileIds = 0

    init(projectId: Int, session: HostSession) {
        target = .chief(projectId: projectId)
        self.session = session
    }

    init(agentId: Int, session: HostSession) {
        target = .session(agentId: agentId)
        self.session = session
    }

    var isChief: Bool {
        if case .chief = target { true } else { false }
    }

    /// The chief's project, or the one the session works in.
    var projectId: Int? {
        switch target {
        case .chief(let projectId): projectId
        case .session(let agentId): session.projectId(ofAgent: agentId)
        }
    }

    /// The chief, for a chief's chat.
    var staff: Staff? {
        if case .chief(let projectId) = target { session.staffs[projectId] } else { nil }
    }

    var project: Project? { projectId.flatMap(session.project(id:)) }

    /// The agent whose conversation this is: the chief's, once it has
    /// started, or the session.
    var agentId: Int? {
        switch target {
        case .chief: staff?.agentId
        case .session(let agentId): agentId
        }
    }

    var agent: Agent? { session.agent(id: agentId) }

    /// Loads the chief (for a chief's chat), the transcript from the start,
    /// and the chief's inbox.
    func load() async {
        isLoading = messages.isEmpty
        defer { isLoading = false }
        if case .chief(let projectId) = target {
            do {
                let staff = try await session.service.staff(projectId: projectId)
                session.setStaff(staff)
                error = nil
            } catch {
                if session.staffs[projectId] == nil { self.error = error.localizedDescription }
            }
        }
        await read()
        if case .chief(let projectId) = target {
            inbox = (try? await session.service.staffInbox(projectId: projectId)) ?? inbox
        }
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
        guard let agentId else { return }
        if agentId != readingAgentId {
            // A new chief agent (it started for the first time): start over.
            readingAgentId = agentId
            messages = []
            next = 0
        }
        do {
            let page = try await session.service.transcript(agentId: agentId, after: next)
            if page.next < next {
                // A shorter transcript started over (a new session, as after
                // /clear): read it from the top.
                let fresh = try await session.service.transcript(agentId: agentId, after: 0)
                messages = fresh.messages
                next = fresh.next
            } else {
                messages = Transcript.merge(messages, page.messages)
                next = max(next, page.next)
            }
            reconcile()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    // MARK: Keeping in sync

    /// How often a chat on screen reads its transcript again, in case an
    /// event was missed (a stream that died quietly).
    static let resyncInterval: Duration = .seconds(15)
    /// How long after its agent goes idle a chat reads again: Claude can
    /// write a turn's last reply after its Stop hook, which an older daemon
    /// never logs.
    static let idleRecheck: Duration = .milliseconds(2500)

    /// Reads what's new every `interval` until cancelled (the chat leaves
    /// the screen). Each read asks only for what follows `next`.
    func keepInSync(every interval: Duration = Chat.resyncInterval) async {
        while !Task.isCancelled {
            try? await Task.sleep(for: interval)
            if Task.isCancelled { return }
            await read()
        }
    }

    /// Reads again a moment after the agent went idle.
    func agentWentIdle(after delay: Duration = Chat.idleRecheck) async {
        try? await Task.sleep(for: delay)
        if !Task.isCancelled { await read() }
    }

    /// Why the composer can't send to a session right now, as the desktop
    /// words it: typing goes through its terminal, where a prompt would
    /// catch the text. `nil` when it can, including when it has stopped (a
    /// message resumes it).
    var blocked: String? {
        guard !isChief, let agent else { return nil }
        if let approval = session.pendingApproval(forAgent: agent.id), approval.isForYou { return "Answer the request above first" }
        switch agent.state {
        case .waitingApproval: return "Answer the request above first"
        case .needsTrust: return "Trust the folder first"
        case .needsLogin: return "Log in to Claude first"
        case .starting: return "Claude is starting…"
        default: return nil
        }
    }

    /// What the empty message box says.
    var placeholder: String {
        agent?.state.canResume == true && !isChief ? "Resume it with a message" : "Message"
    }

    /// There is text or a file to send, and no file is still uploading.
    var canSend: Bool {
        guard blocked == nil, !files.contains(where: \.isUploading) else { return false }
        return !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || files.contains { $0.attachment != nil }
    }

    /// Sends the draft and its files, with the quote it replies to,
    /// starting the chief if needed. Files that failed to upload stay on
    /// the composer.
    func send() async {
        guard canSend else { return }
        let text = ChatReply.compose(draft.trimmingCharacters(in: .whitespacesAndNewlines), replyingTo: replyingTo)
        let attachments = files.compactMap(\.attachment).map(\.attached)
        draft = ""
        replyingTo = nil
        files.removeAll { $0.attachment != nil }
        await send(text, attachments: attachments)
    }

    // MARK: Replies

    /// Who "them" is in this chat: "the Chief of Staff", or "Claude".
    var theirName: String { isChief ? "the Chief of Staff" : "Claude" }

    /// Quotes a message on top of the composer: theirs, or one of yours
    /// (what you wrote, without the quote it replied to).
    func reply(to message: TranscriptMessage) {
        if message.role == .user {
            let text = ChatReply.parse(message.text)?.body ?? message.text
            replyingTo = ChatReply(from: .me, text: text)
        } else {
            replyingTo = ChatReply(from: .them, text: message.text)
        }
    }

    /// "Replying to the Chief of Staff", "Replying to yourself".
    var replyLabel: String? {
        guard let replyingTo else { return nil }
        return "Replying to \(replyingTo.from == .me ? "yourself" : theirName)"
    }

    func send(_ text: String, attachments: [AttachedFile] = []) async {
        pendingIds += 1
        let id = pendingIds
        pending.append(PendingMessage(id: id, text: text, attachments: attachments, afterN: next, state: .sending))
        do {
            switch target {
            case .chief(let projectId):
                try await session.service.messageStaff(projectId: projectId, text: text, attachments: attachments.map(\.name))
            case .session(let agentId):
                if agent?.state.canResume == true {
                    try await session.resume(agentId: agentId)
                }
                try await session.service.sendInput(agentId: agentId, text: text, attachments: attachments.map(\.name))
            }
            update(id) {
                $0.state = .sent
                $0.sentMs = nowMs()
            }
            if case .chief(let projectId) = target {
                session.refreshStaff()
                if let staff = try? await session.service.staff(projectId: projectId) { session.setStaff(staff) }
            }
            await read()
        } catch {
            update(id) { $0.state = .failed(error.localizedDescription) }
        }
    }

    /// Sends a failed message again.
    func retry(_ message: PendingMessage) async {
        pending.removeAll { $0.id == message.id }
        await send(message.text, attachments: message.attachments)
    }

    func discard(_ message: PendingMessage) {
        pending.removeAll { $0.id == message.id }
    }

    private func update(_ id: Int, _ change: (inout PendingMessage) -> Void) {
        if let index = pending.firstIndex(where: { $0.id == id }) { change(&pending[index]) }
    }

    // MARK: Files

    /// Where files on the composer go: the chief's worktree, or the
    /// session's folder.
    private var attachTarget: AttachTarget {
        switch target {
        case .chief(let projectId): .staff(projectId: projectId)
        case .session(let agentId): .agent(id: agentId)
        }
    }

    /// Puts a file on the composer and uploads it.
    func attach(name: String, contentType: String, data: Data) async {
        guard files.count < Attachment.maxPerMessage else {
            refuse(name: name, contentType: contentType, size: data.count, reason: "A message takes \(Attachment.maxPerMessage) files at most")
            return
        }
        guard data.count <= Attachment.maxBytes else {
            refuse(name: name, contentType: contentType, size: data.count, reason: "Over 32 MB, the most cos takes")
            return
        }
        fileIds += 1
        let id = fileIds
        let preview = Attachment.isImage(contentType) ? data : nil
        files.append(DraftFile(id: id, name: name, contentType: contentType, size: data.count, preview: preview, state: .uploading))
        do {
            let attachment = try await session.service.uploadAttachment(to: attachTarget, name: name, contentType: contentType, data: data)
            if let preview { images[attachment.name] = preview }
            updateFile(id) {
                $0.name = attachment.name
                $0.state = .ready(attachment)
            }
        } catch {
            updateFile(id) { $0.state = .failed(error.localizedDescription) }
        }
    }

    /// A file the composer can't take, as a chip that says why.
    func refuse(name: String, contentType: String, size: Int, reason: String) {
        fileIds += 1
        files.append(DraftFile(id: fileIds, name: name, contentType: contentType, size: size, state: .failed(reason)))
    }

    func removeFile(_ id: Int) {
        files.removeAll { $0.id == id }
    }

    private func updateFile(_ id: Int, _ change: (inout DraftFile) -> Void) {
        if let index = files.firstIndex(where: { $0.id == id }) { change(&files[index]) }
    }

    /// A picture you sent, to show in its bubble: the one you picked, or
    /// fetched from the agent's folder once.
    func image(_ file: AttachedFile) async -> Data? {
        if let data = images[file.name] { return data }
        guard file.isImage, let agentId,
            let data = try? await session.service.attachmentData(agentId: agentId, name: file.name)
        else { return nil }
        images[file.name] = data
        return data
    }

    /// Drops what you sent once the transcript shows it: each pending
    /// message matches one later user message with the same text and files,
    /// or a wakeup that carried it (text typed while the chief worked).
    private func reconcile() {
        guard !pending.isEmpty else { return }
        var used = Set<Int>()
        pending = pending.filter { item in
            if case .failed = item.state { return true }
            let text = Self.normalize(item.text)
            let names = item.attachments.map(\.name)
            let match = messages.first { message in
                guard message.n > item.afterN, !used.contains(message.n), message.role == .user else { return false }
                if message.kind == .update {
                    // Typed in later, with the paths of its files.
                    return Self.normalize(message.text).contains(text) && item.attachments.allSatisfy { message.text.contains($0.path) }
                }
                return Self.normalize(message.text) == text && message.attachments.map(\.name) == names
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
            case (.user, .text) where !isChief && message.spawnedPrompt != nil:
                rows.append(.fromChief(message, prompt: message.spawnedPrompt ?? ""))
            case (.user, .text):
                if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !message.attachments.isEmpty {
                    let seen = answered[message.n] != nil || (typing && message.n == lastTyped)
                    rows.append(.user(message, seen ? .seen : .delivered))
                }
            case (.user, .tool) where message.tool?.isShellOutput == true:
                if let tool = message.tool { rows.append(.shellOutput(message, tool)) }
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

    /// The chief (or the session) is mid-turn: the chat shows it typing.
    var isTyping: Bool {
        guard let state = agent?.state ?? staff?.state else { return false }
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
        guard case .chief(let projectId) = target else { return nil }
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

    /// What waits on you, as messages at the end: for the chief, the agents
    /// it started that wait on an approval, a trust dialog or a login; for
    /// a session, the session itself.
    var agentsWaiting: [(agent: Agent, approval: Approval?)] {
        let agents: [Agent]
        switch target {
        case .chief(let projectId): agents = session.agentsStarted(byProject: projectId)
        case .session: agents = agent.map { [$0] } ?? []
        }
        return agents.compactMap { agent in
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
