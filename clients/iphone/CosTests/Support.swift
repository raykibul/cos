import CosKit
import Foundation
@testable import Cos

/// Polls `condition` on the main actor until it holds or `timeout` passes.
@MainActor
func eventually(_ timeout: Duration = .seconds(5), _ condition: () -> Bool) async -> Bool {
    let clock = ContinuousClock()
    let deadline = clock.now + timeout
    while clock.now < deadline {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(20))
    }
    return condition()
}

/// A host for tests; the address is never dialled, the fake answers.
func testHost(_ name: String = "studio") -> Host {
    Host(id: UUID(), name: name, address: "\(name).tail0000.ts.net:7700")
}

/// Fast retries, so refusal tests don't wait seconds.
let quickBackoff = Backoff(initial: .milliseconds(20), maximum: .milliseconds(100))

/// A session on the fake daemon, already following and loaded.
@MainActor
func liveSession(
    _ fake: FakeCos = FakeCos(replyDelay: nil), notifications: any Notifications = NoNotifications()
) async -> HostSession {
    let host = testHost()
    let session = HostSession(
        host: host, service: fake, notifications: notifications, backoff: quickBackoff,
        readMarks: ReadMarks(hostId: host.id, defaults: scratchDefaults())
    )
    session.start()
    _ = await eventually { session.connection.isLive && session.hasLoaded && !session.staffs.isEmpty }
    return session
}

/// Notifications recorded instead of shown.
actor RecordingNotifications: Notifications {
    private(set) var notified: [CosNotification] = []
    private(set) var badges: [Int] = []

    func start() async {}
    func notify(_ notification: CosNotification) async { notified.append(notification) }
    func setBadge(_ count: Int) async { badges.append(count) }
}

/// A `UserDefaults` of its own, emptied, for one test.
func scratchDefaults() -> UserDefaults {
    let name = "cos.tests.\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: name)!
    defaults.removePersistentDomain(forName: name)
    return defaults
}
