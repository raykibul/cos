import CosKit
import SwiftUI

@main
struct CosApp: App {
    @State private var monitor: DaemonMonitor
    /// Shared by the window and the menu bar panel, so the panel can open
    /// the window on an agent.
    @State private var window = WindowState()

    init() {
        // Honours $COS_HOME, so the app can be pointed at a test daemon.
        let monitor = DaemonMonitor(client: CosClient())
        monitor.start()
        _monitor = State(initialValue: monitor)
    }

    var body: some Scene {
        // First, so it opens at launch. Closing it leaves the app running in
        // the menu bar; the Dock icon or the panel brings it back.
        Window("cos", id: MainWindow.id) {
            MainWindow(monitor: monitor, window: window)
        }
        .defaultSize(width: 1000, height: 640)

        MenuBarExtra {
            MenuBarContent(monitor: monitor, window: window)
        } label: {
            // Badged while something waits on you.
            Image(systemName: monitor.waitingCount > 0 ? "person.crop.circle.badge.exclamationmark.fill" : monitor.connection.symbol)
                .accessibilityLabel(monitor.waitingCount > 0 ? "\(monitor.waitingCount) waiting on you" : monitor.connection.title)
        }
        .menuBarExtraStyle(.window)
    }
}
