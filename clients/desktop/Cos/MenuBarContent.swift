import AppKit
import CosKit
import SwiftUI

/// The panel under the menu bar icon.
struct MenuBarContent: View {
    let monitor: DaemonMonitor
    @Environment(\.openWindow) private var openWindow

    private var recent: [LoggedEvent] { monitor.events.suffix(5).reversed() }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            StatusSummary(monitor: monitor)
            if monitor.connection == .notRunning {
                StartHelp(socketPath: monitor.client.socketPath)
                    .font(.callout)
            }

            Divider()

            Text("Recent events")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            if recent.isEmpty {
                Text("No events yet")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            ForEach(recent) { event in
                VStack(alignment: .leading, spacing: 1) {
                    HStack {
                        Text(event.kind).fontWeight(.medium)
                        Spacer()
                        Text(Format.time(event.ts)).foregroundStyle(.secondary).monospacedDigit()
                    }
                    Text(event.data.compact)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                .font(.callout)
            }

            Divider()

            HStack {
                Button("Open cos…") {
                    openWindow(id: MainWindow.id)
                    // A menu bar app isn't active by default; bring the window forward.
                    NSApp.activate()
                }
                .keyboardShortcut("o")
                Spacer()
                Button("Quit cos") { NSApp.terminate(nil) }
                    .keyboardShortcut("q")
            }
        }
        .padding(14)
        .frame(width: 340)
    }
}
