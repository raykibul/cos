import Foundation

// Files attached to a message ("Attachments" in docs/agents-api.md). A file
// is uploaded first, into the folder its agent works in; the message then
// names it in `attachments`, and the daemon types its path at the end so
// Claude can read it. Transcripts give the files back on your messages.

/// Where an upload goes: the folder an agent works in.
public enum AttachTarget: Hashable, Sendable {
    /// A project's Chief of Staff (its worktree is made if needed).
    case staff(projectId: Int)
    /// An agent's folder.
    case agent(id: Int)
    /// A workspace, for an agent you're about to start there.
    case workspace(id: Int)

    /// The upload route, without `?name=`.
    public var path: String {
        switch self {
        case .staff(let projectId): "/v1/projects/\(projectId)/staff/attachments"
        case .agent(let id): "/v1/agents/\(id)/attachments"
        case .workspace(let id): "/v1/workspaces/\(id)/attachments"
        }
    }
}

/// A file the daemon saved. Its `name` is what a message refers to.
public struct Attachment: Codable, Equatable, Hashable, Sendable {
    public var name: String
    public var path: String
    public var size: Int
    public var contentType: String

    public init(name: String, path: String, size: Int, contentType: String) {
        self.name = name
        self.path = path
        self.size = size
        self.contentType = contentType
    }

    private enum CodingKeys: String, CodingKey { case name, path, size, contentType }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = c.string(.path) ?? ""
        name = c.string(.name).nilIfBlank ?? (path as NSString).lastPathComponent
        size = c.int(.size) ?? 0
        contentType = c.string(.contentType).nilIfBlank ?? "application/octet-stream"
    }
}

/// A file one of your messages names, as the transcript reads it back.
public struct AttachedFile: Codable, Equatable, Hashable, Sendable, Identifiable {
    public var name: String
    public var path: String
    public var contentType: String
    /// As the message gives it: `245 KB`.
    public var sizeLabel: String

    public var id: String { path.isEmpty ? name : path }
    public var isImage: Bool { Attachment.isImage(contentType) }

    public init(name: String, path: String, contentType: String, sizeLabel: String) {
        self.name = name
        self.path = path
        self.contentType = contentType
        self.sizeLabel = sizeLabel
    }

    private enum CodingKeys: String, CodingKey { case name, path, contentType, sizeLabel }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = c.string(.path) ?? ""
        name = c.string(.name).nilIfBlank ?? (path as NSString).lastPathComponent
        contentType = c.string(.contentType).nilIfBlank ?? "application/octet-stream"
        sizeLabel = c.string(.sizeLabel) ?? ""
    }
}

extension Attachment {
    /// The largest file the daemon takes.
    public static let maxBytes = 32 * 1024 * 1024
    /// How many files one message may carry.
    public static let maxPerMessage = 20

    public static func isImage(_ contentType: String) -> Bool {
        ["image/png", "image/jpeg", "image/gif", "image/webp", "image/heic"].contains(contentType)
    }

    /// `812 B`, `245 KB`, `3.3 MB`, as the daemon writes it.
    public static func sizeLabel(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return "\((bytes + 1023) / 1024) KB" }
        return String(format: "%.1f MB", Double(bytes) / 1024 / 1024)
    }

    /// As a transcript would name it, for a message not in it yet.
    public var attached: AttachedFile {
        AttachedFile(name: name, path: path, contentType: contentType, sizeLabel: Self.sizeLabel(size))
    }
}

extension CosClient {
    /// Uploads `data` as `name` into the folder `target` works in (201).
    /// The daemon makes the name safe and unique, so use the answer's.
    public func uploadAttachment(
        to target: AttachTarget, name: String, contentType: String, data: Data
    ) async throws -> Attachment {
        guard data.count <= Attachment.maxBytes else {
            throw CosError.http(status: 413, message: "\(name) is over 32 MB, the most cos takes.")
        }
        let path = "\(target.path)?name=\(Self.queryValue(name))"
        let (connection, response) = try await request("POST", path, json: data, contentType: contentType)
        defer { connection.close() }
        return try decode(Attachment.self, from: try await response.readBody())
    }

    /// An agent's attachment, to show it (a picture you sent, say).
    public func attachmentData(agentId: Int, name: String) async throws -> Data {
        let (connection, response) = try await request(
            "GET", "/v1/agents/\(agentId)/attachments/\(Self.pathSegment(name))"
        )
        defer { connection.close() }
        return Data(try await response.readBody())
    }

    /// A message to the chief naming files uploaded for it (`.staff`).
    /// The text may be empty when there are files.
    public func messageStaff(projectId: Int, text: String, attachments: [String]) async throws {
        try await send(
            "POST", "/v1/projects/\(projectId)/staff/message",
            body: MessageBody(text: text, attachments: attachments)
        )
    }

    /// Types a message into an agent naming files uploaded to it (`.agent`).
    public func sendInput(agentId: Int, text: String, attachments: [String]) async throws {
        try await send("POST", "/v1/agents/\(agentId)/input", body: MessageBody(text: text, attachments: attachments))
    }

    static func queryValue(_ text: String) -> String {
        text.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-._~"))) ?? text
    }

    static func pathSegment(_ text: String) -> String {
        queryValue(text)
    }
}

private struct MessageBody: Encodable {
    var text: String
    var attachments: [String]

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(text, forKey: .text)
        if !attachments.isEmpty { try c.encode(attachments, forKey: .attachments) }
    }

    enum CodingKeys: String, CodingKey { case text, attachments }
}
