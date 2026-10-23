import CosKit
import SwiftUI

/// The home tab: every project's Chief of Staff as a chat, the latest
/// active first, like a messaging app's chat list. What waits on you is a
/// pinned row on top; a chief's workspaces and sessions are in its chat's
/// ⋯ menu, or a swipe away here.
struct ChiefsView: View {
    var session: HostSession
    @Environment(Navigator.self) private var navigator
    @State private var filter: ChiefFilter = .all
    @State private var search = ""

    var body: some View {
        let all = session.chiefs
        let chiefs = all.filter { filter.includes($0) && matches($0) }
        let waiting = session.waiting
        List {
            if !all.isEmpty {
                FilterChips(selection: $filter, needsYou: all.filter { $0.waiting > 0 }.count)
                    .listRowInsets(EdgeInsets(top: 2, leading: 16, bottom: 8, trailing: 16))
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
            }
            if !waiting.isEmpty, search.isEmpty, filter != .working {
                Button {
                    navigator.chiefs.append(.waiting)
                } label: {
                    WaitingRow(items: waiting)
                }
                .listRowBackground(Color.clear)
            }
            ForEach(chiefs) { chief in
                Button {
                    navigator.chiefs.append(.chief(projectId: chief.id))
                } label: {
                    ChiefListRow(chief: chief)
                }
                .listRowBackground(Color.clear)
                .swipeActions(edge: .trailing) {
                    Button("Workspaces", systemImage: "arrow.triangle.branch") {
                        navigator.chiefs.append(.chiefInfo(projectId: chief.id))
                    }
                    .tint(Palette.ink2)
                }
                .contextMenu {
                    Button("Open chat", systemImage: "bubble.left") { navigator.chiefs.append(.chief(projectId: chief.id)) }
                    Button("Workspaces and sessions", systemImage: "arrow.triangle.branch") {
                        navigator.chiefs.append(.chiefInfo(projectId: chief.id))
                    }
                    if let agentId = chief.agent?.id {
                        Button("Chief's screen", systemImage: "terminal") { navigator.chiefs.append(.screen(agentId: agentId)) }
                    }
                }
            }
            if all.isEmpty {
                ContentUnavailableView("No projects", systemImage: "folder", description: Text("Add one on the host: cos project add <path>"))
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            } else if chiefs.isEmpty {
                Text(emptyText)
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink3)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
        .pageBackground()
        .searchable(text: $search, prompt: "Search chiefs")
        .refreshable { await session.refresh() }
        .navigationTitle("Chiefs")
    }

    private func matches(_ chief: ChiefSummary) -> Bool {
        let query = search.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return true }
        return chief.project.name.localizedCaseInsensitiveContains(query) || chief.preview.localizedCaseInsensitiveContains(query)
    }

    private var emptyText: String {
        if !search.trimmingCharacters(in: .whitespaces).isEmpty { return "No chief matches “\(search)”." }
        switch filter {
        case .all: return "No chiefs."
        case .needsYou: return "Nothing waits on you."
        case .working: return "No chief is working right now."
        }
    }
}

/// All · Needs you · Working.
struct FilterChips: View {
    @Binding var selection: ChiefFilter
    var needsYou: Int

    var body: some View {
        HStack(spacing: 8) {
            ForEach(ChiefFilter.allCases) { filter in
                let selected = filter == selection
                Button {
                    selection = filter
                } label: {
                    HStack(spacing: 5) {
                        Text(filter.title)
                        if filter == .needsYou, needsYou > 0 {
                            Text("\(needsYou)").fontWeight(.bold).monospacedDigit()
                        }
                    }
                    .font(.subheadline.weight(selected ? .semibold : .medium))
                    .foregroundStyle(selected ? Palette.ink : Palette.ink2)
                    .padding(.horizontal, 14)
                    .frame(height: 32)
                    .background(selected ? Palette.claude.opacity(0.16) : Palette.subtle, in: Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
            Spacer(minLength: 0)
        }
    }
}

/// One chief: its project's avatar, name, when it last did something, what
/// it last said (or "2 sessions working…"), and what waits on you.
struct ChiefListRow: View {
    var chief: ChiefSummary

    var body: some View {
        HStack(spacing: 12) {
            ProjectAvatar(name: chief.project.name, size: 52)
                .opacity(chief.hasStarted ? 1 : 0.55)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(chief.project.name)
                        .font(.headline)
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    if let time = chief.lastActivity {
                        Text(chatTime(time))
                            .font(.subheadline.weight(highlighted ? .semibold : .regular))
                            .foregroundStyle(highlighted ? Palette.claude : Palette.ink3)
                    }
                }
                HStack(alignment: .top, spacing: 8) {
                    Text(chief.preview)
                        .font(.subheadline)
                        .foregroundStyle(chief.previewIsActivity ? Palette.blue : Palette.ink2)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if chief.waiting > 0 {
                        CountBadge(count: chief.waiting)
                            .accessibilityLabel("\(chief.waiting) waiting on you")
                    } else if chief.unread {
                        Circle()
                            .fill(Palette.claude)
                            .frame(width: 11, height: 11)
                            .padding(.top, 4)
                            .accessibilityLabel("New")
                    }
                }
            }
        }
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }

    private var highlighted: Bool { chief.waiting > 0 || chief.unread }
}

/// The pinned row: everything blocked on you, in one place.
struct WaitingRow: View {
    var items: [WaitingItem]

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "tray.full.fill")
                .font(.system(size: 21, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 52, height: 52)
                .background(Palette.claude, in: Circle())
            VStack(alignment: .leading, spacing: 3) {
                Text("Waiting on you")
                    .font(.headline)
                    .foregroundStyle(Palette.ink)
                Text(summary)
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(2)
            }
            Spacer(minLength: 6)
            CountBadge(count: items.count)
        }
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }

    /// "1 approval · 1 question · 2 logins".
    private var summary: String {
        var counts: [(String, String, Int)] = [
            ("approval", "approvals", 0), ("question", "questions", 0), ("plan", "plans", 0), ("trust dialog", "trust dialogs", 0),
            ("login", "logins", 0),
        ]
        for item in items {
            switch item {
            case .approval: counts[0].2 += 1
            case .question: counts[1].2 += 1
            case .plan: counts[2].2 += 1
            case .trust: counts[3].2 += 1
            case .login: counts[4].2 += 1
            }
        }
        return counts.filter { $0.2 > 0 }.map { "\($0.2) \($0.2 == 1 ? $0.0 : $0.1)" }.joined(separator: " · ")
    }
}
