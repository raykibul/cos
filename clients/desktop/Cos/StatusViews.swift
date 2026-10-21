import AppKit
import CosKit
import SwiftUI

extension ConnectionState {
    var title: String {
        switch self {
        case .connecting: "Connecting to cos…"
        case .running: "cos is running"
        case .notRunning: "cos is not running"
        case .failed: "Can't read cos"
        }
    }

    var symbol: String {
        switch self {
        case .connecting: "person.crop.circle.badge.clock"
        case .running: "person.crop.circle.badge.checkmark"
        case .notRunning: "person.crop.circle.badge.xmark"
        case .failed: "person.crop.circle.badge.exclamationmark"
        }
    }

    var color: Color {
        switch self {
        case .connecting: .secondary
        case .running: .green
        case .notRunning: .red
        case .failed: .orange
        }
    }
}

/// "cos is running" with pid, uptime and event count underneath.
struct StatusSummary: View {
    let monitor: DaemonMonitor
    var large = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Circle()
                    .fill(monitor.connection.color)
                    .frame(width: large ? 10 : 8, height: large ? 10 : 8)
                Text(monitor.connection.title)
                    .font(large ? .title2.weight(.semibold) : .headline)
            }
            Group {
                if let status = monitor.status {
                    // Ticks every second so the uptime stays current.
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Text("pid \(String(status.pid)) · up \(Format.uptime(from: status.startedAt, to: context.date)) · \(Format.count(status.events, "event"))")
                    }
                    if large {
                        Text("Version \(status.version) · \(status.home)")
                    }
                } else if case .failed(let message) = monitor.connection {
                    Text(message)
                    Text("Trying again automatically.")
                }
            }
            .font(large ? .callout : .caption)
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
        }
    }
}

/// How to start the daemon. The app doesn't start it itself yet.
struct StartHelp: View {
    let socketPath: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("To start it, run this in Terminal:")
            CommandText(command: Format.command("cos daemon install"))
            Text("This starts cos now and at every login. To try it without installing, run \(Text(Format.command("cos daemon")).font(.body.monospaced())) and keep the Terminal window open.")
                .fixedSize(horizontal: false, vertical: true)
            Text("Waiting for \(socketPath). This updates by itself once cos starts.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A shell command with a copy button.
struct CommandText: View {
    let command: String

    var body: some View {
        HStack {
            Text(command)
                .font(.body.monospaced())
                .textSelection(.enabled)
            Spacer(minLength: 8)
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(command, forType: .string)
            } label: {
                Image(systemName: "doc.on.doc")
            }
            .buttonStyle(.borderless)
            .help("Copy")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
    }
}

enum Format {
    /// `2h 5m`, `4m 10s`, `12s`.
    static func uptime(from start: Date, to now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince(start)))
        let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60)
        if days > 0 { return "\(days)d \(hours)h" }
        if hours > 0 { return "\(hours)h \(minutes)m" }
        if minutes > 0 { return "\(minutes)m \(seconds % 60)s" }
        return "\(seconds)s"
    }

    static func count(_ n: Int, _ noun: String) -> String {
        "\(n) \(noun)\(n == 1 ? "" : "s")"
    }

    private static let clock = Date.VerbatimFormatStyle(
        format: "\(hour: .twoDigits(clock: .twentyFourHour, hourCycle: .zeroBased)):\(minute: .twoDigits):\(second: .twoDigits).\(secondFraction: .fractional(3))",
        timeZone: .current,
        calendar: .current
    )
    private static let day = Date.VerbatimFormatStyle(
        format: "\(year: .defaultDigits)-\(month: .twoDigits)-\(day: .twoDigits)",
        timeZone: .current,
        calendar: .current
    )

    /// Local time; with the date as well when it isn't today.
    static func time(_ date: Date) -> String {
        let clock = date.formatted(clock)
        return Calendar.current.isDateInToday(date) ? clock : "\(date.formatted(day)) \(clock)"
    }

    /// Prefixes `COS_HOME=…` when the app was pointed at another home, so
    /// the command starts the daemon this app is looking for.
    static func command(_ command: String) -> String {
        guard let home = ProcessInfo.processInfo.environment["COS_HOME"], !home.isEmpty else { return command }
        return "COS_HOME='\(home)' \(command)"
    }
}
