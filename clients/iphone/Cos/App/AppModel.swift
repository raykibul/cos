import CosKit
import Foundation
import Observation

/// What the app was launched with. `-fakeData` runs against an in-memory
/// daemon with invented data (screenshots, previews), like the desktop's
/// fake bridge; `-screen <name>` opens a screen; `-fakeRefusal <code>` and
/// `-fakeUnreachable` show those states.
struct Launch: Equatable, Sendable {
    var fakeData = false
    var screen: String?
    var fakeRefusal: String?
    var fakeUnreachable = false
    var noHosts = false

    static func from(_ arguments: [String]) -> Launch {
        func value(after flag: String) -> String? {
            guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else { return nil }
            let value = arguments[index + 1]
            return value.hasPrefix("-") ? nil : value
        }
        return Launch(
            fakeData: arguments.contains("-fakeData"),
            screen: value(after: "-screen"),
            fakeRefusal: value(after: "-fakeRefusal"),
            fakeUnreachable: arguments.contains("-fakeUnreachable"),
            noHosts: arguments.contains("-noHosts")
        )
    }
}

/// The app's root: the saved hosts, and a session following the current
/// one while the app is in the foreground.
@MainActor
@Observable
final class AppModel {
    let hosts: HostStore
    private(set) var session: HostSession?
    let launch: Launch

    @ObservationIgnored private let makeService: @MainActor (Host) -> any CosService
    @ObservationIgnored private let notifications: any Notifications
    @ObservationIgnored private var isActive = false

    init(
        hosts: HostStore, launch: Launch = Launch(), notifications: any Notifications = NoNotifications(),
        makeService: @escaping @MainActor (Host) -> any CosService
    ) {
        self.hosts = hosts
        self.launch = launch
        self.notifications = notifications
        self.makeService = makeService
        sync()
    }

    /// The real app: hosts in `UserDefaults`, a `CosClient` per host. With
    /// `-fakeData`, hosts live in a scratch suite and every host is the fake.
    static func live(arguments: [String] = ProcessInfo.processInfo.arguments) -> AppModel {
        let launch = Launch.from(arguments)
        guard launch.fakeData else {
            return AppModel(hosts: HostStore(), launch: launch) { host in
                CosClient(endpoint: host.endpoint ?? .tcp(host: host.address, port: CosEndpoint.defaultPort))
            }
        }
        let defaults = UserDefaults(suiteName: "cos.fake") ?? .standard
        defaults.removePersistentDomain(forName: "cos.fake")
        let store = HostStore(defaults: defaults)
        if !launch.noHosts {
            _ = try? store.add(name: "build-box", address: "100.101.102.110")
            _ = try? store.add(name: "studio", address: "studio.tail0000.ts.net")
        }
        return AppModel(hosts: store, launch: launch) { _ in
            FakeCos(refusal: launch.fakeRefusal.map(RefusalCode.init(rawValue:)), unreachable: launch.fakeUnreachable)
        }
    }

    // MARK: Hosts

    @discardableResult
    func addHost(name: String, address: String) throws -> Host {
        let host = try hosts.add(name: name, address: address)
        sync()
        return host
    }

    func renameHost(_ id: UUID, to name: String) {
        // The session keeps following the same address; screens read the
        // name from the store.
        hosts.rename(id, to: name)
    }

    func removeHost(_ id: UUID) {
        hosts.remove(id)
        sync()
    }

    func selectHost(_ id: UUID) {
        hosts.select(id)
        sync()
    }

    /// Follows the current host's events only while the app is in the
    /// foreground, and catches up when it comes back.
    func setActive(_ active: Bool) {
        isActive = active
        if active {
            session?.start()
        } else {
            session?.stop()
        }
    }

    /// A new session when the current host changed; the old one stops, so
    /// nothing from the previous host lingers.
    private func sync() {
        guard let host = hosts.current else {
            session?.stop()
            session = nil
            return
        }
        if session?.host.id == host.id, session?.host.address == host.address { return }
        session?.stop()
        let session = HostSession(host: host, service: makeService(host), notifications: notifications)
        self.session = session
        if isActive { session.start() }
    }
}
