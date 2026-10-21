import CosKit
import SwiftUI

struct ProjectDetail: View {
    let project: Project
    let monitor: DaemonMonitor
    let window: WindowState

    private var canChange: Bool { monitor.connection == .running }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .firstTextBaseline) {
                    Text(project.name).font(.largeTitle.weight(.semibold))
                    Spacer()
                    Button("New Workspace…") { window.newWorkspaceIn = project }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canChange)
                }

                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 16, verticalSpacing: 8) {
                    DetailRow("Folder") { PathText(path: project.root) }
                    DetailRow("Base branch") { Text(project.baseBranch).monospaced() }
                    DetailRow("Remote") {
                        Text(project.hasOrigin ? "origin" : "No origin remote. New workspaces start from your checkout.")
                    }
                    DetailRow("Added") { Text(project.createdAt.formatted(date: .abbreviated, time: .shortened)) }
                }

                FolderButtons(path: project.root)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Workspaces").font(.headline)
                    if project.workspaces.isEmpty {
                        Text("No workspaces yet. A workspace is a separate checkout of this repo, on its own branch, where agents work.")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(project.workspaces) { workspace in
                        Button {
                            window.selection = .workspace(workspace.id)
                        } label: {
                            HStack {
                                Label(workspace.title, systemImage: "arrow.triangle.branch")
                                Spacer()
                                Text(workspace.branch).monospaced().foregroundStyle(.secondary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .padding(.vertical, 2)
                    }
                }

                Divider()
                Button("Remove Project…", role: .destructive) { window.removingProject = project }
                    .disabled(!canChange)
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(project.name)
    }
}

struct WorkspaceDetail: View {
    let workspace: Workspace
    let project: Project?
    let monitor: DaemonMonitor
    let window: WindowState

    private var canChange: Bool { monitor.connection == .running }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(workspace.title).font(.largeTitle.weight(.semibold))
                        if workspace.title != workspace.name {
                            Text(workspace.name).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("New Run…") { window.newRunIn = workspace }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canChange || !monitor.runsSupported || monitor.currentRun(inWorkspace: workspace.id) != nil)
                    Button("New Agent…") { window.newAgentIn = workspace }
                        .disabled(!canChange || !monitor.agentsSupported)
                    Button("Rename…") { window.renaming = workspace }
                        .disabled(!canChange)
                    Button("Remove…", role: .destructive) { window.removingWorkspace = workspace }
                        .disabled(!canChange)
                }

                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 16, verticalSpacing: 8) {
                    if let project {
                        DetailRow("Project") { Text(project.name) }
                    }
                    DetailRow("Branch") { Text(workspace.branch).monospaced().textSelection(.enabled) }
                    DetailRow("Base") {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(workspace.baseRef) at \(workspace.shortCommit)")
                                .monospaced()
                                .textSelection(.enabled)
                                .help(workspace.baseCommit)
                            Text(baseExplanation).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    DetailRow("Folder") { PathText(path: workspace.path) }
                    DetailRow("Created") { Text(workspace.createdAt.formatted(date: .abbreviated, time: .shortened)) }
                }

                FolderButtons(path: workspace.path)

                RunsSection(workspace: workspace, monitor: monitor, window: window)

                AgentsSection(workspace: workspace, monitor: monitor, window: window)
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(workspace.title)
    }

    private var baseExplanation: String {
        switch workspace.base {
        case .origin: "Started from the latest commit on origin."
        case .head: "Started from your checkout's HEAD."
        default: "Started from \(workspace.base.rawValue)."
        }
    }
}

private struct DetailRow<Content: View>: View {
    let label: String
    let content: Content

    init(_ label: String, @ViewBuilder content: () -> Content) {
        self.label = label
        self.content = content()
    }

    var body: some View {
        GridRow {
            Text(label)
                .foregroundStyle(.secondary)
                .gridColumnAlignment(.trailing)
            content
        }
    }
}

private struct PathText: View {
    let path: String

    var body: some View {
        Text((path as NSString).abbreviatingWithTildeInPath)
            .monospaced()
            .textSelection(.enabled)
            .help(path)
    }
}

private struct FolderButtons: View {
    let path: String

    var body: some View {
        HStack {
            Button("Reveal in Finder", systemImage: "folder") { Files.reveal(path) }
            Button("Open in Terminal", systemImage: "terminal") { Files.openInTerminal(path) }
        }
    }
}
