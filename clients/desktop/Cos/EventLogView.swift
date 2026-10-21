import CosKit
import SwiftUI

/// Daemon status at the top, then the event log following the live stream.
struct EventLogView: View {
    let monitor: DaemonMonitor
    @State private var follow = true

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            StatusSummary(monitor: monitor, large: true)
                .padding()
            if monitor.connection == .notRunning {
                GroupBox {
                    StartHelp(socketPath: monitor.client.socketPath)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(4)
                }
                .padding([.horizontal, .bottom])
            }
            Divider()
            EventList(events: monitor.events, follow: follow)
        }
        .navigationTitle("Event Log")
        .toolbar {
            Toggle(isOn: $follow) {
                Label("Follow new events", systemImage: "arrow.down.to.line")
            }
            .help("Keep the newest event in view")
        }
    }
}

private struct EventList: View {
    let events: [LoggedEvent]
    let follow: Bool

    var body: some View {
        if events.isEmpty {
            ContentUnavailableView("No events yet", systemImage: "list.bullet.rectangle", description: Text("Events appear here as the daemon writes them."))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0, pinnedViews: .sectionHeaders) {
                        Section {
                            ForEach(Array(events.enumerated()), id: \.element.seq) { index, event in
                                EventRow(event: event)
                                    .background(index.isMultiple(of: 2) ? Color.clear : Color.primary.opacity(0.04))
                                    .id(event.seq)
                            }
                        } header: {
                            HeaderRow()
                        }
                    }
                    .textSelection(.enabled)
                }
                .onAppear { scrollToEnd(proxy) }
                .onChange(of: events.last?.seq) { scrollToEnd(proxy) }
                .onChange(of: follow) { scrollToEnd(proxy) }
            }
        }
    }

    private func scrollToEnd(_ proxy: ScrollViewProxy) {
        guard follow, let last = events.last else { return }
        proxy.scrollTo(last.seq, anchor: .bottom)
    }
}

private enum Column {
    static let seq: CGFloat = 56
    static let time: CGFloat = 200
    static let kind: CGFloat = 170
}

private struct HeaderRow: View {
    var body: some View {
        HStack(spacing: 12) {
            Text("Seq").frame(width: Column.seq, alignment: .trailing)
            Text("Time").frame(width: Column.time, alignment: .leading)
            Text("Kind").frame(width: Column.kind, alignment: .leading)
            Text("Data")
            Spacer()
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(.secondary)
        .padding(.horizontal)
        .padding(.vertical, 6)
        .background(.bar)
    }
}

private struct EventRow: View {
    let event: LoggedEvent

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(String(event.seq))
                .foregroundStyle(.secondary)
                .frame(width: Column.seq, alignment: .trailing)
            Text(Format.time(event.ts))
                .frame(width: Column.time, alignment: .leading)
            Text(event.kind)
                .fontWeight(.medium)
                .frame(width: Column.kind, alignment: .leading)
            Text(event.data.compact)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .help(event.data.compact)
            Spacer(minLength: 0)
        }
        .font(.callout.monospaced())
        .padding(.horizontal)
        .padding(.vertical, 4)
    }
}
