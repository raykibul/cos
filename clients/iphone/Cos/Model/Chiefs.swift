import CosKit
import Foundation
import Observation

/// One row of the chiefs list: a project's Chief of Staff, as a chat.
struct ChiefSummary: Identifiable, Equatable {
    var project: Project
    var staff: Staff?
    /// Its agent, once it has started.
    var agent: Agent?
    /// What in this project waits on you: approvals, questions, plans,
    /// trust dialogs and logins.
    var waiting: Int
    /// Something happened since you last opened its chat.
    var unread: Bool

    var id: Int { project.id }

    var hasStarted: Bool { staff?.hasStarted ?? false }

    /// When it last did something, for the list's time and order.
    var lastActivity: Date? { agent?.lastActivityAt }

    /// The agents it started that are working.
    var working: Int { staff?.workingAgents ?? 0 }

    /// The chief itself is mid-turn.
    var isTyping: Bool {
        guard let state = staff?.state ?? agent?.state else { return false }
        return state == .working || state == .starting
    }

    /// The line under its name: "2 sessions working…" while its agents
    /// work (as a chat app shows "typing…"), else what it last said.
    var preview: String {
        if working > 0 { return working == 1 ? "1 session working…" : "\(working) sessions working…" }
        if isTyping { return "working…" }
        guard hasStarted else { return "Not started · a message starts it" }
        if let message = agent?.lastMessage.map(Self.plain), !message.isEmpty { return message }
        return staff?.stateLabel ?? ""
    }

    /// Whether `preview` is live activity rather than a message.
    var previewIsActivity: Bool { working > 0 || isTyping }

    /// Markdown flattened to one readable line: no markers, no line breaks.
    static func plain(_ markdown: String) -> String {
        var lines: [String] = []
        var inFence = false
        for raw in markdown.split(separator: "\n", omittingEmptySubsequences: true) {
            var line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                inFence.toggle()
                continue
            }
            if inFence { continue }
            while let first = line.first, "#>-*+".contains(first) {
                line.removeFirst()
                line = line.trimmingCharacters(in: .whitespaces)
            }
            line = line.replacingOccurrences(of: "**", with: "")
                .replacingOccurrences(of: "__", with: "")
                .replacingOccurrences(of: "`", with: "")
            if !line.isEmpty { lines.append(line) }
        }
        return lines.joined(separator: " ")
    }
}

/// The chips above the chiefs list.
enum ChiefFilter: String, CaseIterable, Identifiable {
    case all, needsYou, working

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: "All"
        case .needsYou: "Needs you"
        case .working: "Working"
        }
    }

    func includes(_ chief: ChiefSummary) -> Bool {
        switch self {
        case .all: true
        case .needsYou: chief.waiting > 0
        case .working: chief.working > 0 || chief.isTyping
        }
    }
}

extension HostSession {
    /// Every project's Chief of Staff, the latest active first; chiefs that
    /// never started go last, by name.
    var chiefs: [ChiefSummary] {
        let waitingByProject = Dictionary(grouping: waiting.compactMap { projectId(of: $0) }, by: { $0 }).mapValues(\.count)
        return projects.map { project -> ChiefSummary in
            let staff = staffs[project.id]
            let chiefAgent = self.agent(id: staff?.agentId)
            return ChiefSummary(
                project: project, staff: staff, agent: chiefAgent,
                waiting: waitingByProject[project.id] ?? 0,
                unread: readMarks.isUnread(projectId: project.id, activityMs: chiefAgent?.lastActivityMs)
            )
        }
        .sorted { a, b in
            switch (a.lastActivity, b.lastActivity) {
            case let (x?, y?) where x != y: return x > y
            case (_?, nil): return true
            case (nil, _?): return false
            default: return a.project.name.localizedStandardCompare(b.project.name) == .orderedAscending
            }
        }
    }

    /// The project a waiting item belongs to, by its workspace (or, for the
    /// chief's own approvals, by its chief).
    func projectId(of item: WaitingItem) -> Int? {
        switch item {
        case .approval(let approval):
            let workspace = approval.workspaceId ?? self.agent(id: approval.agentId)?.workspaceId
            return project(ofWorkspace: workspace)?.id ?? staffProject(ofAgent: approval.agentId)
        case .question(let question):
            if let runId = question.runId, let asking = self.run(id: runId) { return project(ofWorkspace: asking.workspaceId)?.id }
            guard let agentId = question.agentId else { return nil }
            return project(ofWorkspace: self.agent(id: agentId)?.workspaceId)?.id
        case .plan(let run):
            return project(ofWorkspace: run.workspaceId)?.id
        case .trust(let agent), .login(let agent):
            return project(ofWorkspace: agent.workspaceId)?.id ?? staffProject(ofAgent: agent.id)
        }
    }

    /// The project an agent works in: its workspace's, or the one it is
    /// Chief of Staff of.
    func projectId(ofAgent id: Int) -> Int? {
        staffProject(ofAgent: id) ?? project(ofWorkspace: self.agent(id: id)?.workspaceId)?.id
    }

    /// The terminals you can switch between from an agent's: its project's
    /// Chief of Staff, then the project's sessions that are alive (and the
    /// agent itself, whatever its state).
    func terminalPeers(of id: Int) -> [Agent] {
        guard let projectId = projectId(ofAgent: id) else { return self.agent(id: id).map { [$0] } ?? [] }
        var peers: [Agent] = []
        if let chief = self.agent(id: staffs[projectId]?.agentId) { peers.append(chief) }
        peers += agents(inProject: projectId).filter { $0.state.isAlive || $0.id == id }
        return peers
    }

    /// A project's agents, newest first, without its Chief of Staff.
    func agents(inProject id: Int) -> [Agent] {
        guard let project = project(id: id) else { return [] }
        let workspaces = Set(project.workspaces.map(\.id))
        return agents.filter { agent in
            agent.role != .staff && agent.workspaceId.map(workspaces.contains) == true
        }
    }

    /// A project's runs, newest first.
    func runs(inProject id: Int) -> [Run] {
        guard let project = project(id: id) else { return [] }
        let workspaces = Set(project.workspaces.map(\.id))
        return runs.filter { workspaces.contains($0.workspaceId) }
    }
}

/// When you last opened each chief's chat on this phone, per host, so the
/// list can mark chats with something new. Kept in `UserDefaults`.
@MainActor
@Observable
final class ReadMarks {
    /// Project id to the time (ms) you last had its chat open.
    private(set) var marks: [Int: Int64]
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let key: String

    init(hostId: UUID, defaults: UserDefaults = .standard) {
        self.defaults = defaults
        key = "cos.readMarks.\(hostId.uuidString)"
        var marks: [Int: Int64] = [:]
        for (project, value) in defaults.dictionary(forKey: key) ?? [:] {
            if let id = Int(project), let ms = (value as? NSNumber)?.int64Value { marks[id] = ms }
        }
        self.marks = marks
    }

    /// Its chat was opened, or is open, now.
    func markRead(projectId: Int, at ms: Int64 = Int64(Date().timeIntervalSince1970 * 1000)) {
        guard ms > marks[projectId] ?? 0 else { return }
        marks[projectId] = ms
        var stored: [String: Int64] = [:]
        for (id, ms) in marks { stored[String(id)] = ms }
        defaults.set(stored, forKey: key)
    }

    /// Its chief did something after you last looked. A chat never opened
    /// on this phone isn't marked: everything would be.
    func isUnread(projectId: Int, activityMs: Int64?) -> Bool {
        guard let read = marks[projectId], let activityMs else { return false }
        return activityMs > read
    }
}
