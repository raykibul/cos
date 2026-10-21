import Foundation

public enum CosError: Error, Equatable, Sendable {
    /// Nothing answers on the socket: it doesn't exist, or a crashed daemon
    /// left it behind. Expected whenever the daemon is stopped.
    case notRunning(socket: String)
    /// The daemon closed the connection, for example because it is stopping.
    case connectionClosed
    /// 404: an unknown project or workspace id. The message is the daemon's.
    case notFound(String)
    /// 409: for example removing a project that still has workspaces, or a
    /// workspace with uncommitted changes. The message is the daemon's.
    case conflict(String)
    /// Any other non-2xx answer, such as 400 when git refuses. The message
    /// is the daemon's, written to be shown as-is.
    case http(status: Int, message: String)
    /// The answer wasn't valid HTTP or JSON.
    case badResponse(String)
    /// Any other socket error, such as a permission problem.
    case transport(String)

    /// The error for a non-2xx answer. The daemon sends `{"error": "..."}`;
    /// anything else (an older daemon without the route, say) gets a
    /// generic message.
    static func fromResponse(status: Int, body: [UInt8], path: String) -> CosError {
        struct ErrorBody: Decodable { var error: String }
        let message: String
        if let decoded = try? JSONDecoder().decode(ErrorBody.self, from: Data(body)) {
            message = decoded.error
        } else {
            let text = String(decoding: body, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            message = text.isEmpty ? "The cos daemon answered \(status) for \(path)." : text
        }
        switch status {
        case 404: return .notFound(message)
        case 409: return .conflict(message)
        default: return .http(status: status, message: message)
        }
    }
}

extension CosError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .notRunning(let socket): "The cos daemon is not running (nothing answers on \(socket))."
        case .connectionClosed: "The cos daemon closed the connection."
        case .notFound(let message), .conflict(let message), .http(_, let message): message
        case .badResponse(let detail): "Unexpected answer from the cos daemon: \(detail)"
        case .transport(let detail): "Could not talk to the cos daemon: \(detail)"
        }
    }
}
