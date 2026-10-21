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
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(Int.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        root = try container.decode(String.self, forKey: .root)
        baseBranch = try container.decode(String.self, forKey: .baseBranch)
        hasOrigin = try container.decode(Bool.self, forKey: .hasOrigin)
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        // Tolerate a project sent without its workspaces.
        workspaces = try container.decodeIfPresent([Workspace].self, forKey: .workspaces) ?? []
    }
}

/// A git worktree of a project that cos created and owns. Agents run in it.
public struct Workspace: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var projectId: Int
    /// A city name such as `lisbon`; also the folder and branch name.
    public var name: String
    /// Shown to people; starts as the name and can be renamed.
    public var title: String
    /// `cos/<name>`. Kept when the workspace is removed.
    public var branch: String
    public var path: String
    public var base: WorkspaceBase
    /// What it started from, such as `origin/main` or `HEAD`.
    public var baseRef: String
    public var baseCommit: String
    public var createdAt: Date

    public init(id: Int, projectId: Int, name: String, title: String, branch: String, path: String, base: WorkspaceBase, baseRef: String, baseCommit: String, createdAt: Date) {
        self.id = id
        self.projectId = projectId
        self.name = name
        self.title = title
        self.branch = branch
        self.path = path
        self.base = base
        self.baseRef = baseRef
        self.baseCommit = baseCommit
        self.createdAt = createdAt
    }

    public var shortCommit: String { String(baseCommit.prefix(7)) }
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
