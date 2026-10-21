import CosKit
import SwiftUI

struct NewWorkspaceSheet: View {
    let project: Project
    let monitor: DaemonMonitor
    let onCreated: (Workspace) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var title = ""
    @State private var base: WorkspaceBase
    @State private var creating = false
    @State private var error: String?

    init(project: Project, monitor: DaemonMonitor, onCreated: @escaping (Workspace) -> Void) {
        self.project = project
        self.monitor = monitor
        self.onCreated = onCreated
        // Without an origin remote there is nothing to fetch.
        _base = State(initialValue: project.hasOrigin ? .origin : .head)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("New Workspace in \(project.name)").font(.headline)

            VStack(alignment: .leading, spacing: 4) {
                Text("Title")
                TextField("Title", text: $title, prompt: Text("Optional"))
                    .labelsHidden()
                Text("Leave it empty to use the workspace's name, such as lisbon.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Start from")
                BaseOption(
                    title: "Latest from origin",
                    detail: project.hasOrigin
                        ? "Fetches origin/\(project.baseBranch) and starts from exactly that commit."
                        : "This repo has no origin remote.",
                    isSelected: base == .origin
                ) { base = .origin }
                    .disabled(!project.hasOrigin)
                BaseOption(
                    title: "My current checkout (HEAD)",
                    detail: "Starts from your checkout's current commit, unpushed commits included; uncommitted changes aren't copied.",
                    isSelected: base == .head
                ) { base = .head }
            }

            if let error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }

            HStack {
                if creating {
                    ProgressView().controlSize(.small)
                    Text(base == .origin ? "Fetching origin…" : "Creating…").foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create Workspace") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(creating)
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private func create() {
        creating = true
        error = nil
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                let workspace = try await monitor.createWorkspace(projectId: project.id, base: base, title: title.isEmpty ? nil : title)
                onCreated(workspace)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            creating = false
        }
    }
}

/// A radio button with a line of explanation under it.
private struct BaseOption: View {
    let title: String
    let detail: String
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: isSelected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct RenameWorkspaceSheet: View {
    let workspace: Workspace
    let monitor: DaemonMonitor

    @Environment(\.dismiss) private var dismiss
    @State private var title: String
    @State private var saving = false
    @State private var error: String?

    init(workspace: Workspace, monitor: DaemonMonitor) {
        self.workspace = workspace
        self.monitor = monitor
        _title = State(initialValue: workspace.title)
    }

    private var trimmed: String { title.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Rename Workspace").font(.headline)
            VStack(alignment: .leading, spacing: 4) {
                TextField("Title", text: $title)
                Text("Only the title changes. The folder and the branch \(workspace.branch) keep their names.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Rename") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(saving || trimmed.isEmpty || trimmed == workspace.title)
            }
        }
        .padding(20)
        .frame(width: 380)
    }

    private func save() {
        saving = true
        error = nil
        Task {
            do {
                try await monitor.renameWorkspace(workspace, title: trimmed)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            saving = false
        }
    }
}
