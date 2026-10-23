import CosKit
import SwiftUI

/// A session the chief started (cos's `agent_spawn`), as a card in the
/// chat: its title, workspace, model and live state, with Terminal and
/// Session buttons that open it.
struct SpawnedCard: View {
    var tool: ToolCall
    /// `nil` until the session list has it, or from an older daemon that
    /// doesn't say which agent it was.
    var agent: Agent?
    var time: Int64
    @Environment(AppModel.self) private var app
    @Environment(Navigator.self) private var navigator

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                if let agent { navigator.open(.terminal(agentId: agent.id)) }
            } label: {
                HStack(alignment: .top, spacing: 10) {
                    CardIcon(symbol: "terminal", color: Palette.blue)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(time > 0 ? "SESSION STARTED · \(shortTime(time))" : "SESSION STARTED")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(Palette.ink3)
                        Text(agent?.title ?? (tool.summary.isEmpty ? "A new session" : tool.summary))
                            .font(.headline)
                            .foregroundStyle(Palette.ink)
                            .multilineTextAlignment(.leading)
                        if let place {
                            Text(place)
                                .font(.footnote)
                                .foregroundStyle(Palette.ink2)
                                .lineLimit(1)
                        }
                        if let agent {
                            HStack(spacing: 6) {
                                StateDot(color: agent.state.style.color)
                                Text(stateLine(agent))
                                    .lineLimit(1)
                            }
                            .font(.footnote)
                            .foregroundStyle(agent.state == .working ? Palette.blue : Palette.ink2)
                            .padding(.top, 3)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(agent == nil)

            if let agent {
                Divider()
                HStack(spacing: 0) {
                    CardButton(title: "Terminal", symbol: "apple.terminal", bold: true) {
                        navigator.open(.terminal(agentId: agent.id))
                    }
                    Divider().frame(height: 42)
                    CardButton(title: "Session", symbol: nil) {
                        navigator.open(.agent(id: agent.id))
                    }
                }
            }
        }
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
        .padding(.trailing, 44)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Session started: \(agent?.title ?? tool.summary)")
    }

    /// "quizclub/kyoto · sonnet".
    private var place: String? {
        guard let agent else { return nil }
        let workspace = app.session?.workspace(id: agent.workspaceId)
        let parts = [workspace?.branch, agent.model].compactMap { $0 }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private func stateLine(_ agent: Agent) -> String {
        var line = agent.state.style.label.lowercased()
        if let message = agent.lastMessage.map(ChiefSummary.plain), !message.isEmpty, agent.state != .working {
            line += " · \(message)"
        }
        return line
    }
}

/// A subagent (or background command) the chief ran inside its own Claude
/// session finished: Claude's `<task-notification>`, as a card with the
/// first lines of its result. Tapping it opens the whole report.
struct TaskCard: View {
    var note: TaskNotification
    var time: Int64
    /// Fills the composer with a reply about this report.
    var reply: (String) -> Void
    @State private var reading = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                reading = true
            } label: {
                HStack(alignment: .top, spacing: 10) {
                    CardIcon(symbol: note.succeeded ? "checkmark" : "xmark", color: note.succeeded ? Palette.green : Palette.red)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(header)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(Palette.ink3)
                        Text(note.title)
                            .font(.headline)
                            .foregroundStyle(Palette.ink)
                            .multilineTextAlignment(.leading)
                        if !preview.isEmpty {
                            Text(preview)
                                .font(.subheadline)
                                .foregroundStyle(Palette.ink2)
                                .lineLimit(3)
                                .multilineTextAlignment(.leading)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if !note.result.isEmpty {
                Divider()
                CardButton(title: "Read the report", symbol: nil, bold: true) { reading = true }
            }
        }
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
        .padding(.trailing, 44)
        .frame(maxWidth: .infinity, alignment: .leading)
        .sheet(isPresented: $reading) {
            TaskReport(note: note, time: time) {
                reading = false
                reply("About “\(note.title)”: ")
            }
        }
    }

    /// "AGENT FINISHED · 1m 34s".
    private var header: String {
        let what = note.isAgent ? "AGENT" : "TASK"
        let how = note.succeeded ? "FINISHED" : note.status.uppercased()
        var parts = ["\(what) \(how)"]
        if let duration = note.durationMs { parts.append(durationText(duration)) }
        if time > 0 { parts.append(shortTime(time)) }
        return parts.joined(separator: " · ")
    }

    private var preview: String { ChiefSummary.plain(note.result) }
}

/// A finished subagent's whole report, as a sheet.
struct TaskReport: View {
    var note: TaskNotification
    var time: Int64
    var reply: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    Label(statusLine, systemImage: note.succeeded ? "checkmark" : "xmark")
                        .labelStyle(.tight)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(note.succeeded ? Palette.green : Palette.red)
                    Text(note.title)
                        .font(.title2.weight(.bold))
                        .foregroundStyle(Palette.ink)
                    Text(note.isAgent
                        ? "An agent the Chief of Staff ran inside its own session. It has no terminal of its own."
                        : "A background command the Chief of Staff ran.")
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink2)
                    if note.result.isEmpty {
                        Text(note.summary)
                            .font(.body)
                            .foregroundStyle(Palette.ink)
                    } else {
                        MarkdownView(markdown: note.result)
                            .foregroundStyle(Palette.ink)
                            .textSelection(.enabled)
                            .padding(.top, 6)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
            }
            .pageBackground()
            .safeAreaInset(edge: .bottom, spacing: 0) {
                Button(action: reply) {
                    Label("Reply to the chief about it", systemImage: "arrowshape.turn.up.left")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 48)
                }
                .buttonStyle(.borderedProminent)
                .tint(Palette.claude)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .background(Palette.page)
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                if !note.result.isEmpty {
                    ToolbarItem(placement: .primaryAction) {
                        Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = note.result }
                    }
                }
            }
        }
        .presentationDetents([.large])
    }

    private var statusLine: String {
        var parts = [note.succeeded ? "Finished" : note.status.capitalized]
        if time > 0 { parts.append("at \(shortTime(time))") }
        if let duration = note.durationMs { parts.append("took \(durationText(duration))") }
        return parts.joined(separator: " ")
    }
}

/// The rounded icon on a card.
struct CardIcon: View {
    var symbol: String
    var color: Color

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 16, weight: .semibold))
            .foregroundStyle(color)
            .frame(width: 34, height: 34)
            .background(color.opacity(0.16), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// A full-width button along a card's bottom edge.
struct CardButton: View {
    var title: String
    var symbol: String?
    var bold = false
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let symbol { Image(systemName: symbol) }
                Text(title)
            }
            .font(.body.weight(bold ? .semibold : .regular))
            .foregroundStyle(Palette.claude)
            .frame(maxWidth: .infinity, minHeight: 42)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// "18:53".
func shortTime(_ ms: Int64) -> String {
    Date(unixMilliseconds: ms).formatted(date: .omitted, time: .shortened)
}

/// "45s", "1m 34s", "2h 5m".
func durationText(_ ms: Int64) -> String {
    let seconds = max(0, Int(ms / 1000))
    if seconds < 60 { return "\(seconds)s" }
    let minutes = seconds / 60
    if minutes < 60 { return seconds % 60 == 0 ? "\(minutes)m" : "\(minutes)m \(seconds % 60)s" }
    return minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes / 60)h \(minutes % 60)m"
}
