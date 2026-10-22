import Darwin
import Foundation

/// Where a `CosClient` finds its daemon: the Unix socket on this machine, or
/// another machine's tailnet listener over TCP (see "Remote access" in
/// docs/daemon.md). Plain HTTP either way: WireGuard encrypts the tailnet.
public enum CosEndpoint: Equatable, Hashable, Sendable, CustomStringConvertible {
    /// The daemon's socket, such as `~/.cos/cos.sock`.
    case unix(String)
    /// A host name or IP address (IPv6 without brackets) and a port.
    case tcp(host: String, port: Int)

    /// The port a daemon's tailnet listener uses unless told otherwise.
    public static let defaultPort = 7700

    /// A TCP endpoint from what a person typed, read as the CLI's `--host`
    /// reads it: `my-mac`, `my-mac:7701`, `100.101.102.103`,
    /// `fd7a:115c:a1e0::1` or `[fd7a:115c:a1e0::1]:7701`. The port defaults
    /// to 7700. Anything else, such as a URL or a path, throws
    /// `CosError.invalidAddress` with a message to show as-is.
    public static func parse(address typed: String) throws -> CosEndpoint {
        let address = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        func refuse(_ why: String) -> CosError {
            .invalidAddress("“\(address)” is not a host address (\(why)). Use a name or IP, like my-mac, my-mac:7700 or 100.101.102.103.")
        }
        if address.isEmpty { throw refuse("it's empty") }
        if address.contains("://") || address.contains("/") { throw refuse("leave out http:// and any path") }
        if address.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0) || CharacterSet.controlCharacters.contains($0) }) {
            throw refuse("it has spaces")
        }
        let full: String
        if hasPort(address) {
            full = address
        } else if address.contains(":"), !address.hasPrefix("[") {
            full = "[\(address)]:\(defaultPort)"
        } else {
            full = "\(address):\(defaultPort)"
        }
        guard let colon = full.lastIndex(of: ":") else { throw refuse("no host") }
        let host = String(full[..<colon])
        guard let port = Int(full[full.index(after: colon)...]), (1...65535).contains(port) else {
            throw refuse(full.hasSuffix(":0") ? "port 0" : "bad port")
        }
        if host.hasPrefix("[") {
            guard host.hasSuffix("]"), host.count > 2 else { throw refuse("not a name or IP address") }
            let ip = String(host.dropFirst().dropLast())
            guard isIPv6(ip) else { throw refuse("not a name or IP address") }
            return .tcp(host: ip, port: port)
        }
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._"))
        guard !host.isEmpty, host.unicodeScalars.allSatisfy({ $0.isASCII && allowed.contains($0) }) else {
            throw refuse("not a name or IP address")
        }
        return .tcp(host: host, port: port)
    }

    /// The socket's path, or `host:port` with IPv6 in brackets.
    public var description: String {
        switch self {
        case .unix(let path): path
        case .tcp(let host, let port): host.contains(":") ? "[\(host)]:\(port)" : "\(host):\(port)"
        }
    }

    /// Over the tailnet rather than this machine's socket.
    public var isRemote: Bool {
        if case .tcp = self { true } else { false }
    }

    /// The `Host` header: the address over TCP, a placeholder on the socket.
    var hostHeader: String {
        switch self {
        case .unix: "cos"
        case .tcp: description
        }
    }

    /// Whether `address` ends in `:port`, allowing for bracketed IPv6.
    private static func hasPort(_ address: String) -> Bool {
        guard let colon = address.lastIndex(of: ":") else { return false }
        let before = address[..<colon]
        let port = address[address.index(after: colon)...]
        guard !port.isEmpty, port.allSatisfy(\.isASCII), port.allSatisfy(\.isNumber) else { return false }
        return !before.contains(":") || (before.hasPrefix("[") && before.hasSuffix("]"))
    }

    private static func isIPv6(_ text: String) -> Bool {
        var address = in6_addr()
        return text.withCString { inet_pton(AF_INET6, $0, &address) } == 1
    }
}
