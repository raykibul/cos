import Foundation

public enum CosError: Error, Equatable, Sendable {
    /// Nothing answers on the socket: it doesn't exist, or a crashed daemon
    /// left it behind. Expected whenever the daemon is stopped.
    case notRunning(socket: String)
    /// A daemon over the tailnet can't be reached: its name doesn't resolve,
    /// nothing listens on the port, or it doesn't answer. `detail` says
    /// which, written to be shown as-is.
    case unreachable(address: String, detail: String)
    /// A daemon over the tailnet refused this device, with a stable `code`
    /// (`device_pending`, `wrong_user`, …; see "Refusals" in docs/daemon.md).
    case refused(Refusal)
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
    /// What was typed isn't a host address (`CosEndpoint.parse`).
    case invalidAddress(String)

    /// The error for a non-2xx answer. The daemon sends `{"error": "..."}`,
    /// plus a `code` for tailnet refusals; anything else (an older daemon
    /// without the route, say) gets a generic message.
    static func fromResponse(status: Int, body: [UInt8], path: String) -> CosError {
        struct ErrorBody: Decodable {
            var error: String?
            var code: String?
        }
        let decoded = try? JSONDecoder().decode(ErrorBody.self, from: Data(body))
        let message: String
        if let text = decoded?.error {
            message = text
        } else {
            let text = String(decoding: body, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            message = text.isEmpty ? "The cos daemon answered \(status) for \(path)." : text
        }
        if let code = decoded?.code, !code.isEmpty {
            return .refused(Refusal(status: status, code: RefusalCode(rawValue: code), message: message))
        }
        switch status {
        case 404: return .notFound(message)
        case 409: return .conflict(message)
        default: return .http(status: status, message: message)
        }
    }

    /// The refusal's `code`, if the daemon sent one.
    public var code: RefusalCode? {
        if case .refused(let refusal) = self { refusal.code } else { nil }
    }

    /// The refusal, if this is one.
    public var refusal: Refusal? {
        if case .refused(let refusal) = self { refusal } else { nil }
    }

    /// The HTTP status, for answers the daemon gave.
    public var status: Int? {
        switch self {
        case .notFound: 404
        case .conflict: 409
        case .http(let status, _): status
        case .refused(let refusal): refusal.status
        default: nil
        }
    }

    /// The daemon couldn't be reached at all, rather than answering.
    public var isUnreachable: Bool {
        switch self {
        case .notRunning, .unreachable, .connectionClosed, .transport: true
        default: false
        }
    }
}

/// A tailnet refusal: the status (403, or 503 for `tailscale_unavailable`),
/// its stable code and the daemon's message, which names the command to run
/// on the host when there is one.
public struct Refusal: Equatable, Hashable, Sendable {
    public var status: Int
    public var code: RefusalCode
    public var message: String

    public init(status: Int, code: RefusalCode, message: String) {
        self.status = status
        self.code = code
        self.message = message
    }
}

/// The stable codes of "Refusals" in docs/daemon.md. An open string, so a
/// code added by a newer daemon still decodes (and shows its message).
public struct RefusalCode: RawRepresentable, Hashable, Sendable, CustomStringConvertible {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public var description: String { rawValue }

    /// Tailscale doesn't know the client address.
    public static let unknownPeer = RefusalCode(rawValue: "unknown_peer")
    /// The client is a tagged node, which has no user.
    public static let taggedDevice = RefusalCode(rawValue: "tagged_device")
    /// The client is logged in as a different Tailscale user.
    public static let wrongUser = RefusalCode(rawValue: "wrong_user")
    /// New device, not yet approved on the host.
    public static let devicePending = RefusalCode(rawValue: "device_pending")
    /// Denied on the host.
    public static let deviceDenied = RefusalCode(rawValue: "device_denied")
    /// Device or listener management, which only the host's socket may do.
    public static let hostOnly = RefusalCode(rawValue: "host_only")
    /// 503: `tailscale whois` failed on the host; try again.
    public static let tailscaleUnavailable = RefusalCode(rawValue: "tailscale_unavailable")

    /// Whether waiting and asking again can lift it without anyone doing
    /// anything here: approval on the host, or Tailscale recovering.
    public var mayClearByItself: Bool {
        self == .devicePending || self == .tailscaleUnavailable
    }
}

extension CosError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .notRunning(let socket): "The cos daemon is not running (nothing answers on \(socket))."
        case .unreachable(let address, let detail): "Can't reach \(address). \(detail)"
        case .refused(let refusal): refusal.message
        case .connectionClosed: "The cos daemon closed the connection."
        case .notFound(let message), .conflict(let message), .http(_, let message): message
        case .badResponse(let detail): "Unexpected answer from the cos daemon: \(detail)"
        case .transport(let detail): "Could not talk to the cos daemon: \(detail)"
        case .invalidAddress(let message): message
        }
    }
}
