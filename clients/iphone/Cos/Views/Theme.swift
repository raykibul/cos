import CosKit
import SwiftUI

/// The desktop's warm neutrals (clients/desktop/src/styles.css), light and
/// dark from the asset catalog. One accent, Claude orange, used sparingly;
/// amber for the ★ of a Chief of Staff.
enum Palette {
    static let page = Color("Page")
    static let surface = Color("Surface")
    static let subtle = Color("Subtle")
    static let line = Color("Line")
    static let ink = Color("Ink")
    static let ink2 = Color("Ink2")
    static let ink3 = Color("Ink3")
    static let claude = Color("Claude")
    static let blue = Color("ToneBlue")
    static let green = Color("ToneGreen")
    static let red = Color("ToneRed")
    static let purple = Color("TonePurple")
    static let teal = Color("ToneTeal")
    static let amber = Color("ToneAmber")
    /// Behind a chat's bubbles.
    static let wall = Color("Wall")
    static let wallDot = Color("WallDot")
    /// Your own bubbles.
    static let outgoing = Color("Outgoing")
    /// Two blue ticks: Claude has seen your message.
    static let seen = Color("Seen")
}

/// A state's words, symbol and colour.
struct StateStyle {
    var label: String
    var symbol: String
    var color: Color
}

extension AgentState {
    var style: StateStyle {
        switch self {
        case .starting: StateStyle(label: "Starting", symbol: "circle.dotted", color: Palette.ink3)
        case .working: StateStyle(label: "Working", symbol: "circle.fill", color: Palette.blue)
        case .idle: StateStyle(label: "Idle", symbol: "circle", color: Palette.green)
        case .waitingApproval: StateStyle(label: "Needs approval", symbol: "exclamationmark.circle.fill", color: Palette.claude)
        case .needsTrust: StateStyle(label: "Trust dialog", symbol: "lock.circle", color: Palette.claude)
        case .needsLogin: StateStyle(label: "Needs login", symbol: "person.crop.circle.badge.exclamationmark", color: Palette.claude)
        case .exited: StateStyle(label: "Exited", symbol: "stop.circle", color: Palette.ink3)
        case .interrupted: StateStyle(label: "Interrupted", symbol: "pause.circle", color: Palette.amber)
        default: StateStyle(label: rawValue.replacingOccurrences(of: "_", with: " ").capitalized, symbol: "circle.dashed", color: Palette.ink3)
        }
    }
}

extension RunState {
    var style: StateStyle {
        switch self {
        case .planning: StateStyle(label: "Planning", symbol: "pencil.and.list.clipboard", color: Palette.purple)
        case .awaitingPlanApproval: StateStyle(label: "Plan to review", symbol: "checklist", color: Palette.claude)
        case .running: StateStyle(label: "Running", symbol: "play.circle.fill", color: Palette.blue)
        case .reporting: StateStyle(label: "Reporting", symbol: "doc.text", color: Palette.teal)
        case .done: StateStyle(label: "Done", symbol: "checkmark.circle.fill", color: Palette.green)
        case .failed: StateStyle(label: "Failed", symbol: "xmark.octagon.fill", color: Palette.red)
        case .cancelled: StateStyle(label: "Cancelled", symbol: "slash.circle", color: Palette.ink3)
        default: StateStyle(label: rawValue.replacingOccurrences(of: "_", with: " ").capitalized, symbol: "circle.dashed", color: Palette.ink3)
        }
    }
}

extension TaskState {
    var style: StateStyle {
        switch self {
        case .proposed: StateStyle(label: "Proposed", symbol: "circle.dashed", color: Palette.ink3)
        case .ready: StateStyle(label: "Ready", symbol: "circle", color: Palette.ink2)
        case .running: StateStyle(label: "Running", symbol: "circle.fill", color: Palette.blue)
        case .checking: StateStyle(label: "Checking", symbol: "checklist.unchecked", color: Palette.teal)
        case .review: StateStyle(label: "In review", symbol: "eye.circle", color: Palette.purple)
        case .merged: StateStyle(label: "Merged", symbol: "checkmark.circle.fill", color: Palette.green)
        case .blocked: StateStyle(label: "Blocked", symbol: "exclamationmark.triangle.fill", color: Palette.amber)
        case .failed: StateStyle(label: "Failed", symbol: "xmark.circle.fill", color: Palette.red)
        case .cancelled: StateStyle(label: "Cancelled", symbol: "slash.circle", color: Palette.ink3)
        default: StateStyle(label: rawValue.replacingOccurrences(of: "_", with: " ").capitalized, symbol: "circle.dashed", color: Palette.ink3)
        }
    }
}

/// A small capsule with a state's symbol and words.
struct StateBadge: View {
    var style: StateStyle

    var body: some View {
        Label(style.label, systemImage: style.symbol)
            .font(.caption.weight(.medium))
            .labelStyle(.tight)
            .foregroundStyle(style.color)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(style.color.opacity(0.12), in: Capsule())
            .fixedSize()
    }
}

/// Icon and title close together, also inside a `List` row (whose default
/// label style lines icons up in a wide column).
struct TightLabelStyle: LabelStyle {
    var spacing: CGFloat = 5

    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: spacing) {
            configuration.icon
            configuration.title
        }
    }
}

extension LabelStyle where Self == TightLabelStyle {
    static var tight: TightLabelStyle { TightLabelStyle() }
}

/// A coloured dot for compact rows.
struct StateDot: View {
    var color: Color

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8)
    }
}

/// The ★ that marks a Chief of Staff.
struct ChiefStar: View {
    var size: CGFloat = 15

    var body: some View {
        Image(systemName: "star.fill")
            .font(.system(size: size, weight: .semibold))
            .foregroundStyle(Palette.amber)
            .accessibilityLabel("Chief of Staff")
    }
}

/// A prominent action in Claude orange, for the one thing a screen asks.
struct ClaudeButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background(Palette.claude.opacity(configuration.isPressed ? 0.8 : 1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .opacity(isEnabled ? 1 : 0.45)
    }
}

/// A quiet secondary action next to a `ClaudeButtonStyle` one.
struct QuietButtonStyle: ButtonStyle {
    var tint: Color = Palette.ink

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background(Palette.subtle.opacity(configuration.isPressed ? 0.6 : 1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// Monospaced text in a quiet box: commands, paths, terminal lines.
struct CodeBox: View {
    var text: String
    var lineLimit: Int? = nil

    var body: some View {
        Text(text)
            .font(.system(.footnote, design: .monospaced))
            .foregroundStyle(Palette.ink)
            .lineLimit(lineLimit)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

extension View {
    /// The warm page behind lists and scroll views.
    func pageBackground() -> some View {
        scrollContentBackground(.hidden).background(Palette.page)
    }
}

/// "just now", "5m ago", "3h ago", "2d ago".
func shortAgo(_ ms: Int64, now: Date = Date()) -> String {
    let minutes = max(0, Int((now.timeIntervalSince1970 * 1000 - Double(ms)) / 60_000))
    if minutes < 1 { return "just now" }
    if minutes < 60 { return "\(minutes)m ago" }
    let hours = minutes / 60
    if hours < 48 { return "\(hours)h ago" }
    return "\(hours / 24)d ago"
}

func shortAgo(_ date: Date, now: Date = Date()) -> String {
    shortAgo(Int64(date.timeIntervalSince1970 * 1000), now: now)
}

/// A project's round avatar: its initial on a colour picked from its name,
/// so a project keeps its colour across launches and hosts.
struct ProjectAvatar: View {
    var name: String
    var size: CGFloat = 52

    var body: some View {
        Text(initial)
            .font(.system(size: size * 0.4, weight: .bold, design: .rounded))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(color, in: Circle())
            .accessibilityHidden(true)
    }

    private var initial: String {
        name.first(where: \.isLetter).map { String($0).uppercased() } ?? "#"
    }

    private var color: Color {
        let colors = [Palette.blue, Palette.green, Palette.purple, Palette.teal, Palette.amber, Palette.red]
        // A stable hash: `hashValue` changes from launch to launch.
        let sum = name.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) & 0xFFFF }
        return colors[sum % colors.count]
    }
}

/// A chat's wallpaper: the warm wall with a quiet grid of dots.
struct ChatWallpaper: View {
    var body: some View {
        Canvas { context, size in
            let step: CGFloat = 20
            var y: CGFloat = step / 2
            while y < size.height {
                var x: CGFloat = step / 2
                while x < size.width {
                    context.fill(Path(ellipseIn: CGRect(x: x - 1.3, y: y - 1.3, width: 2.6, height: 2.6)), with: .color(Palette.wallDot))
                    x += step
                }
                y += step
            }
        }
        .background(Palette.wall)
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}

/// A count in a capsule: orange for what waits on you.
struct CountBadge: View {
    var count: Int
    var color: Color = Palette.claude

    var body: some View {
        Text("\(count)")
            .font(.caption.weight(.bold).monospacedDigit())
            .foregroundStyle(.white)
            .padding(.horizontal, 6)
            .frame(minWidth: 21, minHeight: 21)
            .background(color, in: Capsule())
    }
}

/// "18:53" today, "Yesterday", the weekday this week, else the date: how a
/// chat list dates its rows.
func chatTime(_ date: Date, now: Date = Date(), calendar: Calendar = .current) -> String {
    let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
    switch days {
    case ...0: return date.formatted(date: .omitted, time: .shortened)
    case 1: return "Yesterday"
    case 2..<7: return date.formatted(.dateTime.weekday(.abbreviated))
    default: return date.formatted(date: .numeric, time: .omitted)
    }
}
