import CosKit
import SwiftUI

/// The ticks after a message's time: a clock while sending, one tick once
/// cos has it, two once Claude has it, two blue once Claude is on it.
struct ReceiptTicks: View {
    var receipt: Receipt

    var body: some View {
        Group {
            switch receipt {
            case .sending:
                Image(systemName: "clock")
                    .font(.system(size: 10, weight: .semibold))
            case .sent:
                TickShape(double: false)
                    .stroke(style: StrokeStyle(lineWidth: 1.6, lineCap: .round, lineJoin: .round))
                    .frame(width: 14, height: 10)
            case .delivered, .seen:
                TickShape(double: true)
                    .stroke(style: StrokeStyle(lineWidth: 1.6, lineCap: .round, lineJoin: .round))
                    .frame(width: 17, height: 10)
            }
        }
        .foregroundStyle(receipt == .seen ? Palette.seen : Palette.ink3)
        .accessibilityElement()
        .accessibilityLabel(receipt.label)
    }
}

extension Receipt {
    var label: String {
        switch self {
        case .sending: "Sending"
        case .sent: "Sent"
        case .delivered: "Delivered"
        case .seen: "Seen"
        }
    }

    /// What the tick means, for the info sheet.
    var meaning: String {
        switch self {
        case .sending: "On its way to your Mac."
        case .sent: "cos has it. It waits there while the chief is mid-task."
        case .delivered: "Claude has it."
        case .seen: "Claude is answering it."
        }
    }
}

/// One tick, or two side by side, as a messaging app draws them.
struct TickShape: Shape {
    var double: Bool

    func path(in rect: CGRect) -> Path {
        // Drawn on an 18×12 grid, scaled to the frame.
        let sx = rect.width / (double ? 18 : 13)
        let sy = rect.height / 12
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(x: rect.minX + x * sx, y: rect.minY + y * sy)
        }
        var path = Path()
        let left: CGFloat = double ? 1 : 2
        path.move(to: point(left, 6.5))
        path.addLine(to: point(left + 3, 9.5))
        path.addLine(to: point(left + 9, 2))
        if double {
            path.move(to: point(7.5, 9.5))
            path.addLine(to: point(8, 9.8))
            path.addLine(to: point(14, 2.3))
        }
        return path
    }
}

/// A message of yours and when each tick happened: long-press it, then Info.
struct MessageInfo: View {
    var message: TranscriptMessage
    var times: ReceiptTimes
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Bubble(outgoing: true, time: message.tsMs, receipt: times.receipt) {
                        Text(message.text)
                            .font(.body)
                            .foregroundStyle(Palette.ink)
                            .lineLimit(8)
                    }
                    .listRowBackground(ChatWallpaper())
                    .listRowInsets(EdgeInsets(top: 14, leading: 10, bottom: 14, trailing: 10))
                }
                Section("To the Chief of Staff") {
                    if let seen = times.seenMs {
                        step(.seen, at: seen, "The chief started on it: the next thing in its conversation is its own.")
                    } else if times.receipt == .seen {
                        step(.seen, at: nil, "The chief is working on it now.")
                    }
                    step(.delivered, at: times.deliveredMs, "In the chief's Claude session, as your words.")
                    if let sent = times.sentMs {
                        step(.sent, at: sent, sentDetail(sent))
                    }
                }
                Section("What the ticks mean") {
                    ForEach([Receipt.sending, .sent, .delivered, .seen], id: \.self) { receipt in
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            ReceiptTicks(receipt: receipt).frame(width: 22)
                            (Text(receipt.label).fontWeight(.semibold).foregroundStyle(Palette.ink) + Text(" · " + receipt.meaning))
                                .font(.subheadline)
                                .foregroundStyle(Palette.ink2)
                        }
                    }
                }
            }
            .navigationTitle("Message info")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func step(_ receipt: Receipt, at ms: Int64?, _ detail: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            ReceiptTicks(receipt: receipt).frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(receipt.label).font(.body.weight(.semibold))
                    Spacer()
                    if let ms, ms > 0 {
                        Text(Date(unixMilliseconds: ms).formatted(date: .omitted, time: .standard))
                            .font(.subheadline.monospacedDigit())
                            .foregroundStyle(Palette.ink2)
                    }
                }
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// "cos had it. The chief was busy, so it waited 31 s."
    private func sentDetail(_ sent: Int64) -> String {
        let waited = times.deliveredMs - sent
        guard waited >= 3000 else { return "cos had it." }
        return "cos had it. The chief was busy, so cos held it for \(durationText(waited)) and typed it in when the chief paused."
    }
}

/// The chief is working: moving dots, as a chat app shows someone typing,
/// and what it is doing right now. Tapping it opens the chief's terminal.
struct TypingBubble: View {
    var activity: ChiefActivity?
    var open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 4) {
                TypingDots()
                if let activity {
                    HStack(spacing: 5) {
                        Text(activity.text)
                            .font(.caption.monospaced())
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if activity.sinceMs > 0 {
                            TimelineView(.periodic(from: .now, by: 1)) { context in
                                Text("· " + durationText(max(0, Int64(context.date.timeIntervalSince1970 * 1000) - activity.sinceMs)))
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(Palette.ink3)
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.top, 9)
            .padding(.bottom, 7)
            .background(
                Palette.surface,
                in: UnevenRoundedRectangle(topLeadingRadius: 4, bottomLeadingRadius: 16, bottomTrailingRadius: 16, topTrailingRadius: 16, style: .continuous)
            )
            .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
            .frame(maxWidth: 520, alignment: .leading)
            .padding(.trailing, 44)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(activity.map { "The chief is working: \($0.text)" } ?? "The chief is working")
        .accessibilityHint("Opens its terminal")
    }
}

/// Three dots that rise in turn.
struct TypingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: reduceMotion)) { context in
            let t = context.date.timeIntervalSinceReferenceDate
            HStack(spacing: 4) {
                ForEach(0..<3, id: \.self) { index in
                    let lift = reduceMotion ? 0.5 : max(0, sin(t * 2 * .pi / 1.2 - Double(index) * 0.7))
                    Circle()
                        .fill(Palette.ink3)
                        .frame(width: 7, height: 7)
                        .opacity(0.35 + 0.65 * lift)
                        .offset(y: -3 * lift)
                }
            }
            .frame(height: 14)
        }
        .accessibilityHidden(true)
    }
}
