import Foundation

/// A git repo registered with cos.
public struct Project: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var name: String
    /// The repo's main checkout.
    public var root: String
    /// `origin/HEAD`'s branch, or the checkout's current branch without a remote.
    public var baseBranch: String
    public var hasOrigin: Bool
    public var createdAt: Date
    public var workspaces: [Workspace]

    public init(id: Int, name: String, root: String, baseBranch: String, hasOrigin: Bool, createdAt: Date, workspaces: [Workspace] = []) {
        self.id = id
        self.name = name
        self.root = root
        self.baseBranch = baseBranch
        self.hasOrigin = hasOrigin
        self.createdAt = createdAt
        self.workspaces = workspaces
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, root, baseBranch, hasOrigin, createdAt, workspaces
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        name = try c.decode(String.self, forKey: .name)
        root = c.string(.root) ?? ""
        baseBranch = c.string(.baseBranch) ?? "main"
        hasOrigin = c.optional(Bool.self, .hasOrigin) ?? false
        createdAt = c.optional(Date.self, .createdAt) ?? Date(timeIntervalSince1970: 0)
        // Tolerate a project sent without its workspaces, or with an odd one.
        workspaces = c.list(Workspace.self, .workspaces)
    }
}

/// A git worktree of a project that cos created and owns. Agents run in it.
public struct Workspace: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var projectId: Int
    /// A city name such as `lisbon`; also the folder name.
    public var name: String
    /// Optional, set by people. `nil` without one: show `displayName`.
    public var title: String?
    /// The city it's named after: the last segment of its branch, as in
    /// `topflexi/lisbon`.
    public var city: String
    /// `<project slug>/<city>` (older workspaces: `cos/<name>`). Kept when
    /// the workspace is removed.
    public var branch: String
    public var path: String
    public var base: WorkspaceBase
    /// What it started from, such as `origin/main` or `HEAD`.
    public var baseRef: String
    public var baseCommit: String
    public var createdAt: Date

    public init(
        id: Int, projectId: Int, name: String, title: String? = nil, city: String? = nil, branch: String, path: String = "",
        base: WorkspaceBase = .origin, baseRef: String = "", baseCommit: String = "", createdAt: Date = Date(timeIntervalSince1970: 0)
    ) {
        self.id = id
        self.projectId = projectId
        self.name = name
        self.title = title.nilIfBlank
        self.city = city.nilIfBlank ?? Self.city(ofBranch: branch) ?? name
        self.branch = branch
        self.path = path
        self.base = base
        self.baseRef = baseRef
        self.baseCommit = baseCommit
        self.createdAt = createdAt
    }

    private enum CodingKeys: String, CodingKey {
        case id, projectId, name, title, city, branch, path, base, baseRef, baseCommit, createdAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        projectId = c.int(.projectId) ?? 0
        name = c.string(.name) ?? ""
        title = c.string(.title).nilIfBlank?.trimmingCharacters(in: .whitespacesAndNewlines)
        branch = c.string(.branch) ?? ""
        city = c.string(.city).nilIfBlank ?? Self.city(ofBranch: branch) ?? (name.isEmpty ? "workspace \(id)" : name)
        path = c.string(.path) ?? ""
        base = c.optional(WorkspaceBase.self, .base) ?? .origin
        baseRef = c.string(.baseRef) ?? ""
        baseCommit = c.string(.baseCommit) ?? ""
        createdAt = c.optional(Date.self, .createdAt) ?? Date(timeIntervalSince1970: 0)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(projectId, forKey: .projectId)
        try c.encode(name, forKey: .name)
        try c.encode(title, forKey: .title)
        try c.encode(city, forKey: .city)
        try c.encode(branch, forKey: .branch)
        try c.encode(path, forKey: .path)
        try c.encode(base, forKey: .base)
        try c.encode(baseRef, forKey: .baseRef)
        try c.encode(baseCommit, forKey: .baseCommit)
        try c.encode(createdAt, forKey: .createdAt)
    }

    /// What every client shows: its title, else its city.
    public var displayName: String { title ?? city }

    public var shortCommit: String { String(baseCommit.prefix(7)) }

    /// The last segment of a branch: `topflexi/lisbon-v1` → `lisbon-v1`.
    public static func city(ofBranch branch: String) -> String? {
        branch.split(separator: "/").last.map(String.init)
    }
}

/// Where a new workspace starts. A string rather than a closed enum, so a
/// base added by a newer daemon still decodes.
public struct WorkspaceBase: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }

    /// Fetch `origin/<base branch>` and start from exactly that commit.
    public static let origin = WorkspaceBase(rawValue: "origin")
    /// The main checkout's current `HEAD`, including unpushed commits.
    public static let head = WorkspaceBase(rawValue: "head")
}

/// `GET /v1/projects`.
public struct ProjectsPage: Codable, Equatable, Sendable {
    public var projects: [Project]

    public init(projects: [Project]) {
        self.projects = projects
    }

    public init(from decoder: Decoder) throws {
        projects = try decoder.container(keyedBy: CodingKeys.self).list(Project.self, .projects)
    }
}

extension EventKind {
    public static let projectAdded = "project.added"
    public static let projectRemoved = "project.removed"
    public static let workspaceCreated = "workspace.created"
    public static let workspaceRenamed = "workspace.renamed"
    public static let workspaceRemoved = "workspace.removed"
}

extension LoggedEvent {
    /// Whether this event may have changed the project list. Matches by
    /// prefix so kinds added later still trigger a refresh.
    public var changesProjects: Bool {
        kind.hasPrefix("project.") || kind.hasPrefix("workspace.")
    }
}
