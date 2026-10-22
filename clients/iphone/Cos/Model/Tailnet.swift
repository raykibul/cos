import CosKit
import Foundation
import Observation

/// The machines on the user's tailnet, for the add-host screen and the
/// hosts tab. An iPhone can't ask Tailscale, so every saved host that
/// answers lists its peers (`GET /v1/tailnet/peers`) and the answers are
/// merged by Tailscale node ID.
struct TailnetListing: Equatable, Sendable {
    /// Merged, running cos first, then online, then by name. The phone itself
    /// (`isYou`) included; `rows` leaves it out.
    var peers: [TailnetPeer] = []
    /// The saved hosts whose lists these are, by the names they're saved under.
    var sources: [String] = []
    /// Saved hosts that answered `host_only`: an older cos, which keeps its
    /// list to itself.
    var outdated: [String] = []
    /// Saved hosts that answered but can't use Tailscale, with why.
    var unavailable: [(host: String, reason: String)] = []

    static func == (a: Self, b: Self) -> Bool {
        a.peers == b.peers && a.sources == b.sources && a.outdated == b.outdated
            && a.unavailable.map(\.host) == b.unavailable.map(\.host) && a.unavailable.map(\.reason) == b.unavailable.map(\.reason)
    }

    /// One saved host's answer.
    enum Answer: Sendable {
        case peers(TailnetPeers)
        case failed(CosError)
    }

    /// Merges what each saved host answered. Hosts that couldn't be reached
    /// or refused this device add nothing.
    static func merge(_ answers: [(host: Host, answer: Answer)]) -> TailnetListing {
        var listing = TailnetListing()
        var merged: [String: TailnetPeer] = [:]
        var order: [String] = []
        for (host, answer) in answers {
            switch answer {
            case .failed(let error):
                if error.peersNotShared { listing.outdated.append(host.name) }
            case .peers(let page):
                if let reason = page.reason, page.peers.isEmpty {
                    listing.unavailable.append((host.name, reason))
                    continue
                }
                listing.sources.append(host.name)
                for peer in page.peers {
                    if let known = merged[peer.id] {
                        merged[peer.id] = combine(known, peer)
                    } else {
                        merged[peer.id] = peer
                        order.append(peer.id)
                    }
                }
            }
        }
        listing.peers = order.compactMap { merged[$0] }.sorted(by: daemonOrder)
        return listing
    }

    /// The same machine as seen from two hosts: `running` from either wins,
    /// as does being online, being the phone, and the latest sighting.
    private static func combine(_ a: TailnetPeer, _ b: TailnetPeer) -> TailnetPeer {
        var peer = a
        if b.cos == .running, a.cos != .running {
            peer.cos = .running
            if !b.address.isEmpty { peer.address = b.address }
        } else if a.cos == .unknown, b.cos != .unknown {
            peer.cos = b.cos
        }
        peer.online = a.online || b.online
        peer.isYou = a.isYou || b.isYou
        peer.mine = a.mine || b.mine
        peer.lastSeen = peer.online ? nil : [a.lastSeen, b.lastSeen].compactMap { $0 }.max()
        if peer.address.isEmpty { peer.address = b.address }
        if peer.ips.isEmpty { peer.ips = b.ips }
        if peer.os.isEmpty { peer.os = b.os }
        if peer.dnsName.isEmpty { peer.dnsName = b.dnsName }
        return peer
    }

    /// The daemon's order: running cos first, then online, then by name.
    private static func daemonOrder(_ a: TailnetPeer, _ b: TailnetPeer) -> Bool {
        if (a.cos == .running) != (b.cos == .running) { return a.cos == .running }
        if a.online != b.online { return a.online }
        return a.name.localizedStandardCompare(b.name) == .orderedAscending
    }

    // MARK: What the screens show

    /// The rows to show against the saved hosts: never the phone itself, and
    /// other users' and tagged machines apart, behind a toggle.
    func rows(saved: [Host]) -> (mine: [TailnetRow], others: [TailnetRow]) {
        let rows = peers.filter { !$0.isYou }.map { TailnetRow(peer: $0, saved: Self.savedHost(for: $0, in: saved)) }
        return (rows.filter(\.peer.mine), rows.filter { !$0.peer.mine })
    }

    /// Machines running cos that aren't saved yet, for the hosts tab.
    func newHosts(saved: [Host]) -> [TailnetRow] {
        rows(saved: saved).mine.filter { $0.state == .cosRunning }
    }

    /// "via build-box", "via build-box and studio".
    var via: String? {
        guard !sources.isEmpty else { return nil }
        return "via " + sources.formatted(.list(type: .and))
    }

    /// The saved host that is this peer, matched by address (its IPs, its
    /// MagicDNS name or `address`) or by name.
    static func savedHost(for peer: TailnetPeer, in hosts: [Host]) -> Host? {
        var names = Set([peer.name, peer.dnsName, peer.hostname].filter { !$0.isEmpty }.map { $0.lowercased() })
        names.formUnion(peer.ips.map { $0.lowercased() })
        if let ip = peer.displayIP { names.insert(ip.lowercased()) }
        return hosts.first { host in
            if !peer.address.isEmpty, host.address.caseInsensitiveCompare(peer.address) == .orderedSame { return true }
            if case .tcp(let address, _)? = host.endpoint, names.contains(address.lowercased()) { return true }
            return host.name.caseInsensitiveCompare(peer.name) == .orderedSame
        }
    }
}

/// A machine as a row: what it is and what the user can do with it.
struct TailnetRow: Identifiable, Equatable, Sendable {
    var peer: TailnetPeer
    /// The saved host it already is.
    var saved: Host?

    var id: String { peer.id }

    enum State: Equatable, Sendable {
        case added
        case cosRunning
        case online
        case offline(lastSeen: Date?)
    }

    var state: State {
        if saved != nil { return .added }
        if !peer.online { return .offline(lastSeen: peer.lastSeen) }
        // Connecting needs somewhere to connect to.
        if peer.cos == .running, !peer.address.isEmpty { return .cosRunning }
        return .online
    }

    /// `linux · 100.101.102.120`.
    var subtitle: String {
        [osName, peer.displayIP].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    private var osName: String? {
        switch peer.os.lowercased() {
        case "": nil
        case "linux": "Linux"
        case "windows": "Windows"
        case "android": "Android"
        case "ios": peer.hostname.localizedCaseInsensitiveContains("ipad") ? "iPadOS" : "iOS"
        default: peer.os
        }
    }
}

/// Fetches the tailnet from every saved host at once and keeps the merged
/// listing. A host that doesn't answer within `timeout` is skipped.
@MainActor
@Observable
final class TailnetFinder {
    private(set) var listing = TailnetListing()
    private(set) var isLoading = false
    /// At least one refresh has finished.
    private(set) var hasLoaded = false

    @ObservationIgnored private let makeService: @MainActor (Host) -> any CosService
    @ObservationIgnored private let timeout: Duration
    @ObservationIgnored private var generation = 0

    init(timeout: Duration = .seconds(5), makeService: @escaping @MainActor (Host) -> any CosService) {
        self.timeout = timeout
        self.makeService = makeService
    }

    /// Asks every host in `hosts`, in parallel. A refresh started meanwhile
    /// wins; this one's answer is dropped.
    func refresh(hosts: [Host]) async {
        generation += 1
        let mine = generation
        isLoading = true
        let services = hosts.map { ($0, makeService($0)) }
        let timeout = timeout
        let answers = await withTaskGroup(of: (Int, TailnetListing.Answer).self) { group in
            for (index, (_, service)) in services.enumerated() {
                group.addTask {
                    do {
                        let page = try await withTimeout(timeout) { try await service.tailnetPeers(port: nil) }
                        return (index, .peers(page))
                    } catch let error as CosError {
                        return (index, .failed(error))
                    } catch {
                        return (index, .failed(.transport(error.localizedDescription)))
                    }
                }
            }
            var answers: [(Int, TailnetListing.Answer)] = []
            for await answer in group { answers.append(answer) }
            return answers.sorted { $0.0 < $1.0 }.map { (services[$0.0].0, $0.1) }
        }
        guard mine == generation else { return }
        listing = TailnetListing.merge(answers)
        isLoading = false
        hasLoaded = true
    }
}

/// Runs `operation`, giving up with `CosError.unreachable` after `timeout`.
func withTimeout<T: Sendable>(_ timeout: Duration, _ operation: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await operation() }
        group.addTask {
            try await Task.sleep(for: timeout)
            throw CosError.unreachable(address: "", detail: "It didn't answer in time.")
        }
        defer { group.cancelAll() }
        guard let first = try await group.next() else { throw CancellationError() }
        return first
    }
}
