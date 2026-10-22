import Foundation

/// Something that should reach you even when the app isn't on screen: an
/// approval, a question, a plan to review. Today nothing is sent, because
/// push needs an Apple Developer account; see "Push, later" in docs/iphone.md.
struct CosNotification: Equatable, Sendable {
    enum Kind: String, Sendable {
        case approval, question, plan, login, trust
    }

    var kind: Kind
    var title: String
    var body: String
    /// Groups notifications per host, as APNs' `thread-id` would.
    var thread: String
}

/// The seam push plugs into. `HostSession` reports what newly waits on you
/// while it follows a host; an implementation decides what to do with it.
///
/// Adding push later (docs/iphone.md):
/// - `start()` asks for permission and calls
///   `UIApplication.registerForRemoteNotifications()`;
/// - the device token goes to the daemon (a new route, for example
///   `PUT /v1/push/token`), which keeps it per approved device;
/// - the daemon sends the APNs push itself, so `notify` stays a no-op for
///   anything the daemon already pushed, or shows a local banner when the
///   app is open.
protocol Notifications: Sendable {
    /// Called once when the app starts.
    func start() async
    /// Something new waits on you on `host`.
    func notify(_ notification: CosNotification) async
    /// What waits on you changed; for the app icon's badge.
    func setBadge(_ count: Int) async
}

/// No push yet: updates arrive only while the app is open.
struct NoNotifications: Notifications {
    func start() async {}
    func notify(_ notification: CosNotification) async {}
    func setBadge(_ count: Int) async {}
}
