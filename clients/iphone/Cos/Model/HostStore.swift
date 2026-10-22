import CosKit
import Foundation
import Observation
import SwiftUI

/// A Mac or server running cos, reached over Tailscale. Nothing secret:
/// Tailscale's identity and the host's device approval are the only auth.
struct Host: Codable, Equatable, Hashable, Identifiable, Sendable {
    var id: UUID
    /// What you call it; defaults to the first label of its name.
    var name: String
    /// `host:port` as `CosEndpoint` writes it (`my-mac:7700`, `[fd7a::1]:7700`).
    var address: String

    var endpoint: CosEndpoint? { try? CosEndpoint.parse(address: address) }

    /// `studio.tail1234.ts.net:7700` → `studio`; an IP address stays whole.
    static func defaultName(for endpoint: CosEndpoint) -> String {
        guard case .tcp(let host, _) = endpoint else { return endpoint.description }
        if host.contains(":") || host.allSatisfy({ $0.isNumber || $0 == "." }) { return host }
        return String(host.split(separator: ".").first ?? Substring(host))
    }
}

/// The saved hosts and the one in use, kept in `UserDefaults` as JSON.
@MainActor
@Observable
final class HostStore {
    private(set) var hosts: [Host] = []
    private(set) var currentId: UUID?

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let key: String

    init(defaults: UserDefaults = .standard, key: String = "cos.hosts") {
        self.defaults = defaults
        self.key = key
        load()
    }

    var current: Host? {
        hosts.first { $0.id == currentId } ?? hosts.first
    }

    /// Adds a host from what was typed and makes it current. The address is
    /// read as the CLI's `--host` reads it; a host already saved under that
    /// address is selected (and renamed if a name was given) instead.
    @discardableResult
    func add(name: String, address: String) throws -> Host {
        let endpoint = try CosEndpoint.parse(address: address)
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if let existing = hosts.first(where: { $0.address.caseInsensitiveCompare(endpoint.description) == .orderedSame }) {
            if !trimmed.isEmpty { rename(existing.id, to: trimmed) }
            select(existing.id)
            return hosts.first { $0.id == existing.id } ?? existing
        }
        let host = Host(id: UUID(), name: trimmed.isEmpty ? Host.defaultName(for: endpoint) : trimmed, address: endpoint.description)
        hosts.append(host)
        currentId = host.id
        save()
        return host
    }

    func rename(_ id: UUID, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let index = hosts.firstIndex(where: { $0.id == id }) else { return }
        hosts[index].name = trimmed
        save()
    }

    /// Removes a host. If it was current, the first one left becomes current.
    func remove(_ id: UUID) {
        hosts.removeAll { $0.id == id }
        if currentId == id { currentId = hosts.first?.id }
        save()
    }

    func select(_ id: UUID) {
        guard hosts.contains(where: { $0.id == id }), currentId != id else { return }
        currentId = id
        save()
    }

    func move(from source: IndexSet, to destination: Int) {
        hosts.move(fromOffsets: source, toOffset: destination)
        save()
    }

    // MARK: Storage

    private struct Saved: Codable {
        var hosts: [Host]
        var current: UUID?
    }

    private func load() {
        guard let data = defaults.data(forKey: key), let saved = try? JSONDecoder().decode(Saved.self, from: data) else { return }
        hosts = saved.hosts
        currentId = saved.current ?? saved.hosts.first?.id
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(Saved(hosts: hosts, current: currentId)) else { return }
        defaults.set(data, forKey: key)
    }
}
