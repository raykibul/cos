import AppKit
import CosKit
import SwiftUI

/// The panel under the menu bar icon.
struct MenuBarContent: View {
    let monitor: DaemonMonitor
    let window: WindowState
    @Environment(\.openWindow) private var openWindow

    private var recent: [LoggedEvent] { monitor.events.suffix(5).reversed() }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            StatusSummary(monitor: monitor)
            if monitor.connection == .notRunning {
                StartHelp(socketPath: monitor.client.socketPath)
                    .font(.callout)
            }

            if monitor.waitingCount > 0 {
                Divider()
                MenuBarWaiting(monitor: monitor, window: window) { item in
                    window.selection = item
                    openMainWindow()
                }
            }

            if !monitor.activeRuns.isEmpty {
                Divider()
                Text("Runs")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                ForEach(monitor.activeRuns.prefix(4)) { run in
                    Button {
                        window.selection = .run(run.id)
                        openMainWindow()
                    } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 6) {
                                Text(run.goal).fontWeight(.medium).lineLimit(1)
                                Spacer()
                                Text(run.state.title)
                                    .foregroundStyle(run.state.needsYou ? Color.orange : Color.secondary)
                            }
                            if let counts = run.counts, counts.total > 0, run.state != .awaitingPlanApproval {
                                ProgressView(value: counts.progress)
                                    .controlSize(.small)
                                    .tint(counts.blocked > 0 ? .orange : run.state.color)
                                Text(counts.summary)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .font(.callout)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
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
                Button("Open cos…") { openMainWindow() }
                .keyboardShortcut("o")
                Spacer()
                Button("Quit cos") { NSApp.terminate(nil) }
                    .keyboardShortcut("q")
            }
        }
        .padding(14)
        .frame(width: 360)
    }

    private func openMainWindow() {
        openWindow(id: MainWindow.id)
        // Opening from the panel doesn't activate the app; bring the window forward.
        NSApp.activate()
    }
}
