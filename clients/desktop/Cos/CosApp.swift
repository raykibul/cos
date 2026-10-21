import CosKit
import SwiftUI

@main
struct CosApp: App {
    @State private var monitor: DaemonMonitor

    init() {
        // Honours $COS_HOME, so the app can be pointed at a test daemon.
        let monitor = DaemonMonitor(client: CosClient())
        monitor.start()
        _monitor = State(initialValue: monitor)
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarContent(monitor: monitor)
        } label: {
            Image(systemName: monitor.connection.symbol)
                .accessibilityLabel(monitor.connection.title)
        }
        .menuBarExtraStyle(.window)

        Window("cos", id: MainWindow.id) {
            MainWindow(monitor: monitor)
        }
        .defaultSize(width: 1000, height: 640)
    }
}
