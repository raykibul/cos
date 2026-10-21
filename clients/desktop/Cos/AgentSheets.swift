import CosKit
import SwiftUI

/// Prompt, optional title and model → `POST /v1/workspaces/{id}/agents`.
struct NewAgentSheet: View {
    let workspace: Workspace
    let monitor: DaemonMonitor
    let onCreated: (Agent) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var prompt = ""
    @State private var title = ""
    @State private var model = AgentModel.standard
    @State private var starting = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("New Agent in \(workspace.title)").font(.headline)

            VStack(alignment: .leading, spacing: 4) {
                Text("Prompt")
                TextEditor(text: $prompt)
                    .font(.body)
                    .frame(minHeight: 120)
                    .scrollContentBackground(.hidden)
                    .padding(6)
                    .background(.background, in: RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.quaternary))
                Text("What Claude should do. Leave it empty to start Claude waiting for you to type in its terminal.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Title")
                TextField("Title", text: $title, prompt: Text("Optional"))
                    .labelsHidden()
                Text("Without one, the agent is named after the start of the prompt.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Model")
                Picker("Model", selection: $model) {
                    ForEach(AgentModel.allCases) { model in
                        Text(model.title).tag(model)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }

            if let error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }

            HStack {
                if starting {
                    ProgressView().controlSize(.small)
                    Text("Starting Claude…").foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Start Agent") { start() }
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.borderedProminent)
                    .disabled(starting)
            }
        }
        .padding(20)
        .frame(width: 480)
    }

    private func start() {
        starting = true
        error = nil
        let request = NewAgent(prompt: prompt, title: title, model: model.value)
        Task {
            do {
                let agent = try await monitor.createAgent(workspaceId: workspace.id, request)
                onCreated(agent)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            starting = false
        }
    }
}

enum AgentModel: String, CaseIterable, Identifiable {
    case standard, sonnet, opus, haiku

    var id: String { rawValue }
    var title: String { self == .standard ? "Default" : rawValue.capitalized }
    /// What goes in `model`; `nil` lets Claude use its default.
    var value: String? { self == .standard ? nil : rawValue }
}
