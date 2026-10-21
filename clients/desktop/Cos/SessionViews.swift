import CosKit
import SwiftUI

/// External sessions grouped by where they run: a cos workspace, a project's
/// own checkout, or anywhere else.
struct SessionGroup: Identifiable {
    var id: String
    var title: String
    var sessions: [ExternalSession]

    @MainActor
    static func groups(_ monitor: DaemonMonitor) -> [SessionGroup] {
        var groups: [String: SessionGroup] = [:]
        var order: [String] = []
        for session in monitor.sessions.sorted(by: { $0.displayName.localizedStandardCompare($1.displayName) == .orderedAscending }) {
            let key: String
            let title: String
            if let id = session.workspaceId, let workspace = monitor.workspace(id: id) {
                key = "w\(id)"
                title = [monitor.project(id: workspace.projectId)?.name, workspace.title].compactMap { $0 }.joined(separator: " › ")
            } else if let id = session.projectId, let project = monitor.project(id: id) {
                key = "p\(id)"
                title = project.name
            } else {
                key = "other"
                title = "Other folders"
            }
            if groups[key] == nil {
                groups[key] = SessionGroup(id: key, title: title, sessions: [])
                order.append(key)
            }
            groups[key]!.sessions.append(session)
        }
        // cos's own folders first, everything else last.
        return order.sorted { ($0 == "other" ? 1 : 0, $0) < ($1 == "other" ? 1 : 0, $1) }.map { groups[$0]! }
    }
}

struct SessionStatusDot: View {
    let session: ExternalSession

    var body: some View {
        Circle()
            .fill(session.isBusy ? Color.blue : Color.secondary.opacity(0.5))
            .frame(width: 7, height: 7)
            .help(session.isBusy ? "Busy" : "Idle")
    }
}

/// A Claude session cos didn't start. Read-only: cos can only show it.
struct ExternalSessionDetail: View {
    let session: ExternalSession
    let monitor: DaemonMonitor

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(session.displayName).font(.largeTitle.weight(.semibold))
                        Text(session.isBusy ? "Busy" : "Idle")
                            .font(.caption.weight(.medium))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background((session.isBusy ? Color.blue : Color.secondary).opacity(0.15), in: Capsule())
                    }
                    Text("A Claude session running outside cos. cos can see it but not control it.")
                        .foregroundStyle(.secondary)
                }

                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 16, verticalSpacing: 8) {
                    row("Folder") {
                        Text((session.cwd as NSString).abbreviatingWithTildeInPath)
                            .monospaced()
                            .textSelection(.enabled)
                            .help(session.cwd)
                    }
                    if let id = session.workspaceId, let workspace = monitor.workspace(id: id) {
                        row("Workspace") { Text(workspace.title) }
                    } else if let id = session.projectId, let project = monitor.project(id: id) {
                        row("Project") { Text(project.name) }
                    }
                    if let sessionId = session.sessionId {
                        row("Session") { Text(sessionId).monospaced().textSelection(.enabled) }
                    }
                    row("Process") { Text(String(session.pid)).monospaced() }
                    if let version = session.version {
                        row("Claude") { Text(version) }
                    }
                    if let ms = session.statusUpdatedMs {
                        row("Updated") { Text(Date(unixMilliseconds: ms), format: .relative(presentation: .named)) }
                    }
                }

                HStack {
                    Button("Reveal in Finder", systemImage: "folder") { Files.reveal(session.cwd) }
                    Button("Open in Terminal", systemImage: "terminal") { Files.openInTerminal(session.cwd) }
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(session.displayName)
    }

    private func row(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        GridRow {
            Text(label)
                .foregroundStyle(.secondary)
                .gridColumnAlignment(.trailing)
            content()
        }
    }
}
