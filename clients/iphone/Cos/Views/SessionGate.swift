import CosKit
import SwiftUI

/// Shows a tab's content once the current host answers, and a full screen
/// for each way it can't: refused (with the daemon's code), unreachable, or
/// still connecting. A refused device is asked again every few seconds, so
/// approving it on the host lets the app in by itself.
struct SessionGate<Content: View>: View {
    @Environment(AppModel.self) private var app
    @ViewBuilder var content: (HostSession) -> Content

    var body: some View {
        if let session = app.session {
            gated(session)
                .toolbar { HostMenu() }
        } else {
            ContentUnavailableView("No host", systemImage: "desktopcomputer", description: Text("Add a Mac or server in Hosts."))
        }
    }

    @ViewBuilder
    private func gated(_ session: HostSession) -> some View {
        let name = app.hosts.current?.name ?? session.host.name
        switch session.connection {
        case .refused(let refusal):
            RefusalView(hostName: name, address: session.host.address, refusal: refusal) { session.reconnect() }
        case .unreachable(let message) where !session.hasLoaded:
            UnreachableView(hostName: name, address: session.host.address, message: message) { session.reconnect() }
        case .failed(let message) where !session.hasLoaded:
            UnreachableView(hostName: name, address: session.host.address, message: message) { session.reconnect() }
        case .connecting where !session.hasLoaded, .live where !session.hasLoaded:
            VStack(spacing: 14) {
                ProgressView()
                Text("Connecting to \(name)…")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
                Text(session.host.address)
                    .font(.footnote.monospaced())
                    .foregroundStyle(Palette.ink3)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Palette.page)
        default:
            content(session)
                .safeAreaInset(edge: .top, spacing: 0) {
                    OfflineBanner(connection: session.connection, hostName: name)
                }
        }
    }
}

/// A thin line above content loaded earlier, while the host can't be reached.
struct OfflineBanner: View {
    var connection: Connection
    var hostName: String

    var body: some View {
        if let text {
            Label(text, systemImage: "wifi.slash")
                .font(.footnote.weight(.medium))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(.bar)
        }
    }

    private var text: String? {
        switch connection {
        case .unreachable, .failed: "Can't reach \(hostName) · showing what was loaded"
        default: nil
        }
    }
}

/// What a refusal means and what to do about it.
struct RefusalView: View {
    var hostName: String
    var address: String
    var refusal: Refusal
    var retry: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 18) {
                Image(systemName: symbol)
                    .font(.system(size: 44, weight: .regular))
                    .foregroundStyle(refusal.code == .devicePending ? Palette.claude : Palette.ink2)
                    .padding(.top, 48)
                Text(title)
                    .font(.title2.weight(.semibold))
                    .multilineTextAlignment(.center)
                Text(explanation)
                    .font(.body)
                    .foregroundStyle(Palette.ink2)
                    .multilineTextAlignment(.center)
                if let command {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("On \(hostName), run:")
                            .font(.footnote)
                            .foregroundStyle(Palette.ink2)
                        CodeBox(text: command)
                        Button("Copy command", systemImage: "doc.on.doc") {
                            UIPasteboard.general.string = command
                        }
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(Palette.ink2)
                    }
                    .padding(.top, 4)
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text("The daemon said")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(Palette.ink3)
                    Text(inlineMarkdown(refusal.message, base: .footnote))
                        .font(.footnote)
                        .foregroundStyle(Palette.ink2)
                        .textSelection(.enabled)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))

                if refusal.code.mayClearByItself {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Checking again every few seconds…")
                            .font(.footnote)
                            .foregroundStyle(Palette.ink3)
                    }
                }
                Button("Try again now", systemImage: "arrow.clockwise", action: retry)
                    .buttonStyle(QuietButtonStyle())
                    .frame(maxWidth: 260)
                Text(address)
                    .font(.caption.monospaced())
                    .foregroundStyle(Palette.ink3)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 32)
        }
        .background(Palette.page)
    }

    /// The command the daemon names, between backticks.
    private var command: String? {
        if let match = refusal.message.firstMatch(of: /`([^`]+)`/) { return String(match.1) }
        if refusal.code == .devicePending { return "cos devices approve <this iPhone>" }
        return nil
    }

    private var symbol: String {
        switch refusal.code {
        case .devicePending: "hourglass"
        case .deviceDenied: "hand.raised"
        case .wrongUser: "person.crop.circle.badge.xmark"
        case .taggedDevice: "tag"
        case .unknownPeer: "questionmark.circle"
        case .tailscaleUnavailable: "arrow.triangle.2.circlepath"
        case .hostOnly: "lock"
        default: "exclamationmark.triangle"
        }
    }

    private var title: String {
        switch refusal.code {
        case .devicePending: "Approve this iPhone on \(hostName)"
        case .deviceDenied: "\(hostName) denied this iPhone"
        case .wrongUser: "Another Tailscale account"
        case .taggedDevice: "This device is tagged"
        case .unknownPeer: "\(hostName) doesn't know this device"
        case .tailscaleUnavailable: "Tailscale isn't answering on \(hostName)"
        case .hostOnly: "Only on \(hostName) itself"
        default: "\(hostName) refused this iPhone"
        }
    }

    private var explanation: String {
        switch refusal.code {
        case .devicePending:
            "cos lets in only devices you approve. This iPhone has asked; approve it on the host and the app carries on by itself."
        case .deviceDenied:
            "This iPhone was denied on the host. Remove it there to ask again, then approve it."
        case .wrongUser:
            "This iPhone is logged in to Tailscale as someone else. Log in as the host's owner in the Tailscale app."
        case .taggedDevice:
            "Tagged Tailscale devices have no user, so cos can't tell they're yours. Log this device in as yourself."
        case .unknownPeer:
            "Tailscale on the host doesn't know this address. Check that this iPhone is connected in the Tailscale app."
        case .tailscaleUnavailable:
            "The host couldn't ask Tailscale who this is. It usually passes; the app tries again."
        case .hostOnly:
            "Devices and remote access are managed on the host's own socket, so a device can't approve itself."
        default:
            "The daemon refused this device with “\(refusal.code.rawValue)”."
        }
    }
}

/// The host doesn't answer at all.
struct UnreachableView: View {
    var hostName: String
    var address: String
    var message: String
    var retry: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 18) {
                Image(systemName: "wifi.slash")
                    .font(.system(size: 44))
                    .foregroundStyle(Palette.ink2)
                    .padding(.top, 48)
                Text("Can't reach \(hostName)")
                    .font(.title2.weight(.semibold))
                Text(message)
                    .font(.body)
                    .foregroundStyle(Palette.ink2)
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
                VStack(alignment: .leading, spacing: 10) {
                    hint("1.circle", "Tailscale is connected on this iPhone and on \(hostName).")
                    hint("2.circle", "cos is running there: `cos status`")
                    hint("3.circle", "Remote access is on there: `cos remote status`")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Trying again every few seconds…")
                        .font(.footnote)
                        .foregroundStyle(Palette.ink3)
                }
                Button("Try again now", systemImage: "arrow.clockwise", action: retry)
                    .buttonStyle(QuietButtonStyle())
                    .frame(maxWidth: 260)
                Text(address)
                    .font(.caption.monospaced())
                    .foregroundStyle(Palette.ink3)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 32)
        }
        .background(Palette.page)
    }

    private func hint(_ symbol: String, _ text: String) -> some View {
        Label {
            Text(inlineMarkdown(text, base: .subheadline)).font(.subheadline)
        } icon: {
            Image(systemName: symbol).foregroundStyle(Palette.ink3)
        }
    }
}
