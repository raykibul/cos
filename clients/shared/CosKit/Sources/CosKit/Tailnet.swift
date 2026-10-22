import Foundation

// The other machines on a host's tailnet (`GET /v1/tailnet/peers`, see
// "Remote access" in docs/daemon.md), so an app can offer the ones running
// cos. An iPhone can't ask Tailscale itself, so it asks a host it already
// reaches. Decoded leniently: unknown fields are ignored, states stay open
// strings and an odd peer is skipped.

/// `GET /v1/tailnet/peers`.
public struct TailnetPeers: Codable, Equatable, Sendable {
    /// Running cos first, then online, then by name. Never the answering host.
    public var peers: [TailnetPeer]
    /// The answering host's MagicDNS name, once Tailscale has answered.
    public var selfName: String?
    /// The answering host. Older daemons leave it out.
    public var host: TailnetHost?
    /// Why there are no peers: Tailscale isn't usable on the host.
    public var reason: String?

    public init(peers: [TailnetPeer], selfName: String? = nil, host: TailnetHost? = nil, reason: String? = nil) {
        self.peers = peers
        self.selfName = selfName
        self.host = host
        self.reason = reason
    }

    private enum CodingKeys: String, CodingKey {
        case peers, selfName, host = "self", reason
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        peers = c.list(TailnetPeer.self, .peers)
        selfName = c.string(.selfName).nilIfBlank
        host = c.optional(TailnetHost.self, .host)
        reason = c.string(.reason).nilIfBlank
    }

    /// What to call the answering host: its name, else `self_name`.
    public var hostName: String? { (host?.name).nilIfBlank ?? selfName }
}

/// The host that answered.
public struct TailnetHost: Codable, Equatable, Hashable, Sendable {
    public var name: String
    /// The full MagicDNS name, such as `my-mac.tail0000.ts.net`; empty if none.
    public var dnsName: String
    public var os: String
    /// Its tailnet address and cos's port, such as `100.100.1.1:7700`.
    public var address: String

    public init(name: String, dnsName: String = "", os: String = "", address: String = "") {
        self.name = name
        self.dnsName = dnsName
        self.os = os
        self.address = address
    }

    private enum CodingKeys: String, CodingKey { case name, dnsName, os, address }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        guard let name = c.string(.name).nilIfBlank else {
            throw DecodingError.keyNotFound(CodingKeys.name, .init(codingPath: c.codingPath, debugDescription: "no name"))
        }
        self.name = name
        dnsName = c.string(.dnsName) ?? ""
        os = c.string(.os) ?? ""
        address = c.string(.address) ?? ""
    }
}

/// Another machine on the tailnet.
public struct TailnetPeer: Codable, Equatable, Hashable, Sendable, Identifiable {
    /// Tailscale's stable node ID.
    public var id: String
    /// The first label of its MagicDNS name, else its hostname.
    public var name: String
    public var hostname: String
    public var dnsName: String
    /// As Tailscale reports it: `macOS`, `linux`, `iOS`, `windows`, `android`…
    public var os: String
    public var ips: [String]
    public var online: Bool
    /// When it was last online, if it is offline and Tailscale knows.
    public var lastSeen: Date?
    /// The same Tailscale user as the host and not tagged: only these can
    /// be approved as devices, so only these are worth offering.
    public var mine: Bool
    public var tagged: Bool
    public var cos: PeerCos
    /// Where cos would listen on it, `100.x.y.z:7700`; empty if it has no address.
    public var address: String
    /// The device asking (over the tailnet), so an app can leave itself out.
    public var isYou: Bool

    public init(
        id: String, name: String, hostname: String? = nil, dnsName: String = "", os: String = "", ips: [String] = [],
        online: Bool = true, lastSeen: Date? = nil, mine: Bool = true, tagged: Bool = false, cos: PeerCos = .unknown,
        address: String = "", isYou: Bool = false
    ) {
        self.id = id
        self.name = name
        self.hostname = hostname ?? name
        self.dnsName = dnsName
        self.os = os
        self.ips = ips
        self.online = online
        self.lastSeen = lastSeen
        self.mine = mine
        self.tagged = tagged
        self.cos = cos
        self.address = address
        self.isYou = isYou
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, hostname, dnsName, os, ips, online, lastSeenMs, mine, tagged, cos, address, isYou
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        guard let id = c.string(.id).nilIfBlank else {
            throw DecodingError.keyNotFound(CodingKeys.id, .init(codingPath: c.codingPath, debugDescription: "no id"))
        }
        self.id = id
        hostname = c.string(.hostname) ?? ""
        name = c.string(.name).nilIfBlank ?? Optional(hostname).nilIfBlank ?? id
        dnsName = c.string(.dnsName) ?? ""
        os = c.string(.os) ?? ""
        ips = c.strings(.ips)
        online = c.optional(Bool.self, .online) ?? false
        lastSeen = c.int64(.lastSeenMs).map { Date(timeIntervalSince1970: Double($0) / 1000) }
        mine = c.optional(Bool.self, .mine) ?? false
        tagged = c.optional(Bool.self, .tagged) ?? false
        cos = c.optional(PeerCos.self, .cos) ?? .unknown
        address = c.string(.address) ?? ""
        isYou = c.optional(Bool.self, .isYou) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(name, forKey: .name)
        try c.encode(hostname, forKey: .hostname)
        try c.encode(dnsName, forKey: .dnsName)
        try c.encode(os, forKey: .os)
        try c.encode(ips, forKey: .ips)
        try c.encode(online, forKey: .online)
        try c.encodeIfPresent(lastSeen.map { Int64(($0.timeIntervalSince1970 * 1000).rounded()) }, forKey: .lastSeenMs)
        try c.encode(mine, forKey: .mine)
        try c.encode(tagged, forKey: .tagged)
        try c.encode(cos, forKey: .cos)
        try c.encode(address, forKey: .address)
        try c.encode(isYou, forKey: .isYou)
    }

    /// Its first tailnet IPv4 address (else its first address), for a
    /// subtitle; the host part of `address` when it lists no IPs.
    public var displayIP: String? {
        if let ip = ips.first(where: { !$0.contains(":") }) ?? ips.first { return ip }
        guard let endpoint = try? CosEndpoint.parse(address: address), case .tcp(let host, _) = endpoint else { return nil }
        return host
    }
}

/// Whether cos's tailnet listener answers on a peer. An open string, so a
/// state added later still decodes.
public struct PeerCos: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// Something accepted a connection on cos's port.
    public static let running = PeerCos(rawValue: "running")
    /// Refused, or no answer in time.
    public static let notRunning = PeerCos(rawValue: "not_running")
    /// Not probed: offline, or not this user's device.
    public static let unknown = PeerCos(rawValue: "unknown")
}

extension CosClient {
    /// The other machines on the daemon's tailnet, and whether cos answers
    /// on `port` on each (default: the daemon's own listener port). Over the
    /// tailnet the asking device is marked `isYou`. A daemon older than this
    /// route over the tailnet refuses it with `host_only`: see
    /// `CosError.peersNotShared`.
    public func tailnetPeers(port: Int? = nil) async throws -> TailnetPeers {
        try await getJSON(port.map { "/v1/tailnet/peers?port=\($0)" } ?? "/v1/tailnet/peers")
    }
}

extension CosError {
    /// An older daemon keeps its peer list to its own socket and refuses it
    /// over the tailnet: the list just isn't available from that host.
    public var peersNotShared: Bool { code == .hostOnly }
}
