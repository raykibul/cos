import CosKit
import SwiftUI

/// "On your tailnet" on the add-host screen: every machine the saved hosts
/// can see, the ones running cos with a Connect button.
struct TailnetSection: View {
    @Environment(AppModel.self) private var app
    /// Called after Connect has saved and switched to a machine.
    var connected: () -> Void
    @State private var showOthers = false
    @State private var error: String?

    var body: some View {
        let finder = app.tailnet
        let rows = finder.listing.rows(saved: app.hosts.hosts)
        Section {
            ForEach(rows.mine) { row in
                TailnetPeerRow(row: row) { connect(row.peer) }
            }
            if rows.mine.isEmpty {
                emptyRow(finder)
            }
            if !rows.others.isEmpty {
                Button {
                    withAnimation { showOthers.toggle() }
                } label: {
                    Label(
                        showOthers ? "Hide other devices" : "Show \(rows.others.count) other \(rows.others.count == 1 ? "device" : "devices")",
                        systemImage: showOthers ? "chevron.up" : "chevron.down"
                    )
                    .labelStyle(.tight)
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
                }
                if showOthers {
                    ForEach(rows.others) { row in
                        TailnetPeerRow(row: row, note: row.peer.tagged ? "Tagged" : "Another user") { connect(row.peer) }
                    }
                }
            }
        } header: {
            TailnetHeader(title: "On your tailnet", via: finder.listing.via, loading: finder.isLoading)
        } footer: {
            TailnetNotes(listing: finder.listing, error: error)
        }
    }

    @ViewBuilder
    private func emptyRow(_ finder: TailnetFinder) -> some View {
        if !finder.hasLoaded {
            HStack(spacing: 10) {
                ProgressView()
                Text("Asking your hosts…").foregroundStyle(Palette.ink3)
            }
            .font(.subheadline)
        } else if finder.listing.sources.isEmpty {
            Text("None of your hosts can list the tailnet right now. Pull down to try again.")
                .font(.subheadline)
                .foregroundStyle(Palette.ink3)
        } else {
            Text("No other machines on your tailnet.")
                .font(.subheadline)
                .foregroundStyle(Palette.ink3)
        }
    }

    private func connect(_ peer: TailnetPeer) {
        do {
            try app.connect(to: peer)
            connected()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// "Found on your tailnet" on the hosts tab: machines running cos that
/// aren't saved yet. Nothing at all when there are none.
struct FoundOnTailnetSection: View {
    @Environment(AppModel.self) private var app
    @State private var error: String?

    var body: some View {
        let rows = app.tailnet.listing.newHosts(saved: app.hosts.hosts)
        if !rows.isEmpty {
            Section {
                ForEach(rows) { row in
                    TailnetPeerRow(row: row) {
                        do {
                            try app.connect(to: row.peer)
                        } catch {
                            self.error = error.localizedDescription
                        }
                    }
                }
            } header: {
                TailnetHeader(title: "Found on your tailnet", via: app.tailnet.listing.via, loading: false)
            } footer: {
                if let error { Text(error).foregroundStyle(Palette.red) }
            }
        }
    }
}

/// A section title with "via <host>" under it.
struct TailnetHeader: View {
    var title: String
    var via: String?
    var loading: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(title)
                if loading { ProgressView().controlSize(.mini) }
            }
            if let via {
                Text(via)
                    .font(.caption)
                    .textCase(nil)
                    .foregroundStyle(Palette.ink3)
            }
        }
    }
}

/// Why some hosts didn't add to the list, quietly.
struct TailnetNotes: View {
    var listing: TailnetListing
    var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error {
                Text(error).foregroundStyle(Palette.red)
            }
            ForEach(listing.outdated, id: \.self) { host in
                Text("Update cos on \(host) to list your tailnet here.")
            }
            ForEach(listing.unavailable, id: \.host) { item in
                Text("\(item.host) can't list the tailnet: \(item.reason)")
            }
        }
    }
}

/// One machine: its OS, name, `os · 100.x`, and what it is to you.
struct TailnetPeerRow: View {
    var row: TailnetRow
    /// Said instead of "Online" for another user's or a tagged machine.
    var note: String?
    var connect: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: osSymbol)
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(row.peer.online ? Palette.ink2 : Palette.ink3)
                .frame(width: 34, height: 34)
                .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(row.peer.name)
                        .font(.body.weight(.medium))
                        .foregroundStyle(row.peer.online ? Palette.ink : Palette.ink2)
                        .lineLimit(1)
                    chip
                }
                if !row.subtitle.isEmpty {
                    Text(row.subtitle)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(Palette.ink3)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            if row.state == .cosRunning {
                Button("Connect", action: connect)
                    .font(.subheadline.weight(.semibold))
                    .buttonStyle(.borderedProminent)
                    .tint(Palette.claude)
                    .controlSize(.small)
                    .accessibilityLabel("Connect to \(row.peer.name)")
            }
        }
        .padding(.vertical, 1)
        .accessibilityElement(children: .combine)
    }

    private var chip: some View {
        let (label, color): (String, Color) = switch row.state {
        case .added: ("Added", Palette.ink3)
        case .cosRunning: ("cos running", Palette.green)
        case .online: (note ?? "Online", Palette.blue)
        case .offline(let lastSeen): (lastSeen.map { "Offline · seen \(shortAgo($0))" } ?? "Offline", Palette.ink3)
        }
        return Text(label)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(color.opacity(0.13), in: Capsule())
            .lineLimit(1)
            .fixedSize()
    }

    private var osSymbol: String {
        switch row.peer.os.lowercased() {
        case "macos": "laptopcomputer"
        case "ios": row.peer.hostname.localizedCaseInsensitiveContains("ipad") ? "ipad" : "iphone"
        case "linux", "freebsd", "openbsd": "server.rack"
        case "windows": "pc"
        case "android": "smartphone"
        case "tvos": "appletv"
        default: "desktopcomputer"
        }
    }
}
