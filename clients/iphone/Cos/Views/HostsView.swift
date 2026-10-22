import CosKit
import SwiftUI

/// The Macs and servers this iPhone knows: pick one, rename, remove, add.
struct HostsView: View {
    @Environment(AppModel.self) private var app
    @Environment(Navigator.self) private var navigator
    @State private var renaming: Host?

    var body: some View {
        List {
            Section {
                ForEach(app.hosts.hosts) { host in
                    Button {
                        app.selectHost(host.id)
                    } label: {
                        HostRow(host: host, isCurrent: host.id == app.hosts.current?.id, connection: host.id == app.session?.host.id ? app.session?.connection : nil)
                    }
                    .foregroundStyle(Palette.ink)
                    .swipeActions(edge: .trailing) {
                        Button("Remove", systemImage: "trash", role: .destructive) { app.removeHost(host.id) }
                        Button("Rename", systemImage: "pencil") { renaming = host }
                            .tint(Palette.ink2)
                    }
                    .contextMenu {
                        Button("Rename", systemImage: "pencil") { renaming = host }
                        Button("Remove", systemImage: "trash", role: .destructive) { app.removeHost(host.id) }
                    }
                }
                .onMove { app.hosts.move(from: $0, to: $1) }
                Button {
                    navigator.addingHost = true
                } label: {
                    Label("Add a Mac or server", systemImage: "plus")
                }
            } footer: {
                Text("Tap a host to switch to it. Swipe to rename or remove it.")
            }

            FoundOnTailnetSection()

            if let session = app.session {
                Section("Connected to \(app.hosts.current?.name ?? session.host.name)") {
                    LabeledContent("Address", value: session.host.address)
                    LabeledContent("Status", value: statusText(session.connection))
                    if let status = session.status {
                        LabeledContent("cos version", value: status.version)
                        LabeledContent("Events", value: "\(status.lastSeq)")
                    }
                }
            }

            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("To reach a host over Tailscale")
                        .font(.subheadline.weight(.semibold))
                    step("1", "Install Tailscale on this iPhone and log in as the host's owner.")
                    step("2", "On the host, check that remote access listens: `cos remote status`.")
                    step("3", "Add it here by its Tailscale name or `100.x` address.")
                    step("4", "Approve this iPhone on the host: `cos devices approve <n>`.")
                }
                .padding(.vertical, 4)
            } footer: {
                Text("Plain HTTP inside Tailscale's WireGuard tunnel, on port 7700 unless you give another. Nothing secret is stored on this iPhone.")
            }
        }
        .pageBackground()
        .navigationTitle("Hosts")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { EditButton() }
        }
        .refreshable { await app.refreshTailnet() }
        .task { await app.refreshTailnet() }
        .sheet(isPresented: Bindable(navigator).addingHost) { AddHostSheet() }
        .sheet(item: $renaming) { host in RenameHostSheet(host: host) }
    }

    private func step(_ number: String, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(number)
                .font(.caption.weight(.bold).monospacedDigit())
                .foregroundStyle(Palette.ink3)
            Text(inlineMarkdown(text, base: .subheadline))
                .font(.subheadline)
                .foregroundStyle(Palette.ink2)
        }
    }

    private func statusText(_ connection: Connection) -> String {
        switch connection {
        case .live: "Connected"
        case .connecting: "Connecting…"
        case .refused(let refusal): refusal.code == .devicePending ? "Waiting for approval" : "Refused (\(refusal.code.rawValue))"
        case .unreachable: "Can't reach it"
        case .failed: "Trouble"
        }
    }
}

struct HostRow: View {
    var host: Host
    var isCurrent: Bool
    var connection: Connection?

    var body: some View {
        HStack(spacing: 12) {
            Text(String(host.name.prefix(1)).uppercased())
                .font(.headline)
                .foregroundStyle(isCurrent ? .white : Palette.ink2)
                .frame(width: 34, height: 34)
                .background(isCurrent ? Palette.claude : Palette.subtle, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text(host.name).font(.body.weight(.medium))
                Text(host.address)
                    .font(.caption.monospaced())
                    .foregroundStyle(Palette.ink3)
            }
            Spacer()
            if let connection {
                ConnectionDot(connection: connection)
            }
            if isCurrent {
                Image(systemName: "checkmark")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Palette.claude)
            }
        }
        .contentShape(Rectangle())
    }
}

struct AddHostSheet: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var address = ""
    @State private var error: String?

    var body: some View {
        let firstRun = app.hosts.hosts.isEmpty
        NavigationStack {
            Form {
                if firstRun {
                    Section {
                        Text(inlineMarkdown(
                            "Add your first machine by its Tailscale name (for example `my-mac`) or `100.x` address. After that, cos lists the rest of your tailnet here.",
                            base: .subheadline
                        ))
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink2)
                    }
                } else {
                    TailnetSection { dismiss() }
                }
                Section {
                    TextField("my-mac or 100.101.102.103", text: $address)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                        .font(.body.monospaced())
                        .onSubmit(add)
                } header: {
                    Text(firstRun ? "Address" : "Or by name or address")
                } footer: {
                    if let error {
                        Text(error).foregroundStyle(Palette.red)
                    } else {
                        Text("Its Tailscale name (MagicDNS) or tailnet address. Port 7700 unless you add one, as in my-mac:7701.")
                    }
                }
                Section("Name") {
                    TextField(placeholderName, text: $name)
                        .onSubmit(add)
                }
            }
            .pageBackground()
            .refreshable { if !firstRun { await app.refreshTailnet() } }
            .task { if !firstRun { await app.refreshTailnet() } }
            .navigationTitle("Add a host")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add", action: add).disabled(address.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .presentationDetents(firstRun ? [.medium, .large] : [.large])
    }

    private var placeholderName: String {
        (try? CosEndpoint.parse(address: address)).map(Host.defaultName(for:)) ?? "Optional"
    }

    private func add() {
        do {
            try app.addHost(name: name, address: address)
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct RenameHostSheet: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    var host: Host
    @State private var name = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(host.name, text: $name)
                        .onSubmit(save)
                } footer: {
                    Text(host.address).font(.caption.monospaced())
                }
            }
            .navigationTitle("Rename")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear { name = host.name }
        }
        .presentationDetents([.height(220)])
    }

    private func save() {
        app.renameHost(host.id, to: name)
        dismiss()
    }
}

/// The app icon's mark: a white "c" on Claude orange.
struct CosMark: View {
    var body: some View {
        GeometryReader { geometry in
            let side = min(geometry.size.width, geometry.size.height)
            RoundedRectangle(cornerRadius: side * 0.24, style: .continuous)
                .fill(Palette.claude)
                .overlay {
                    Circle()
                        .trim(from: 0.125, to: 0.875)
                        .stroke(.white, style: StrokeStyle(lineWidth: side * 0.125, lineCap: .round))
                        .padding(side * 0.27)
                }
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}

/// The first screen, before any host is saved.
struct WelcomeView: View {
    @Environment(Navigator.self) private var navigator

    var body: some View {
        VStack(spacing: 20) {
            Spacer()
            CosMark()
                .frame(width: 84, height: 84)
            Text("cos")
                .font(.largeTitle.weight(.semibold))
            Text("Your coding agents' chief of staff, from your pocket. Connect to a Mac or server running cos over Tailscale.")
                .font(.body)
                .foregroundStyle(Palette.ink2)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
            Spacer()
            Button {
                navigator.addingHost = true
            } label: {
                Label("Add a Mac or server", systemImage: "plus")
            }
            .buttonStyle(ClaudeButtonStyle())
            .padding(.horizontal, 32)
            Text("Remote access is on by default; `cos remote status` on the host says where it listens.")
                .font(.footnote)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
                .foregroundStyle(Palette.ink3)
                .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Palette.page)
        .sheet(isPresented: Bindable(navigator).addingHost) { AddHostSheet() }
    }
}
