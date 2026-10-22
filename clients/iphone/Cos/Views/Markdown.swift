import SwiftUI

/// Markdown split into the blocks a chat or report shows: headings,
/// paragraphs, list items, quotes and code. Inline styles (bold, code,
/// links) are left to `AttributedString(markdown:)`, which only does inline
/// syntax well.
enum MarkdownBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    /// `marker` is `•` or the item's number, `1.`.
    case item(marker: String, text: String, depth: Int)
    case quote(String)
    case code(String)
    case rule

    static func parse(_ markdown: String) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var paragraph: [String] = []
        var code: [String]?

        func flush() {
            let text = paragraph.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty { blocks.append(.paragraph(text)) }
            paragraph = []
        }

        for raw in markdown.components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if var lines = code {
                if line.hasPrefix("```") {
                    blocks.append(.code(lines.joined(separator: "\n")))
                    code = nil
                } else {
                    lines.append(raw)
                    code = lines
                }
                continue
            }
            if line.hasPrefix("```") {
                flush()
                code = []
            } else if line.isEmpty {
                flush()
            } else if let heading = line.firstMatch(of: /^(#{1,6})\s+(.*)$/) {
                flush()
                blocks.append(.heading(level: heading.1.count, text: String(heading.2)))
            } else if line == "---" || line == "***" || line == "___" {
                flush()
                blocks.append(.rule)
            } else if let item = raw.firstMatch(of: /^(\s*)[-*+]\s+(.*)$/) {
                flush()
                blocks.append(.item(marker: "•", text: String(item.2), depth: item.1.count / 2))
            } else if let item = raw.firstMatch(of: /^(\s*)(\d+)[.)]\s+(.*)$/) {
                flush()
                blocks.append(.item(marker: "\(item.2).", text: String(item.3), depth: item.1.count / 2))
            } else if let quote = line.firstMatch(of: /^>\s?(.*)$/) {
                flush()
                blocks.append(.quote(String(quote.1)))
            } else if case .item(let marker, let text, let depth)? = blocks.last, paragraph.isEmpty, raw.hasPrefix("  ") {
                // A wrapped list item.
                blocks[blocks.count - 1] = .item(marker: marker, text: text + " " + line, depth: depth)
            } else {
                paragraph.append(line)
            }
        }
        if let lines = code { blocks.append(.code(lines.joined(separator: "\n"))) }
        flush()
        return blocks
    }
}

/// Inline markdown as an `AttributedString`; the text as it is when it
/// doesn't parse. `code` spans get a monospaced font a step smaller than
/// the text around them (`base`), on a subtle background.
func inlineMarkdown(_ text: String, base: Font = .body) -> AttributedString {
    let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace, failurePolicy: .returnPartiallyParsedIfPossible)
    guard var string = try? AttributedString(markdown: text, options: options) else { return AttributedString(text) }
    for run in string.runs {
        guard var intent = run.inlinePresentationIntent, intent.contains(.code) else { continue }
        // Text styles `.code` itself, at full size, over any font we set.
        intent.remove(.code)
        string[run.range].inlinePresentationIntent = intent
        string[run.range].font = codeFont(for: base)
        string[run.range].backgroundColor = Palette.subtle
    }
    return string
}

/// The monospaced font for inline code next to `font`.
func codeFont(for font: Font) -> Font {
    let style: Font.TextStyle = switch font {
    case .title3, .headline: .body
    case .body: .callout
    case .callout, .subheadline: .footnote
    case .footnote: .caption
    default: .caption2
    }
    return .system(style, design: .monospaced)
}

/// Renders markdown as blocks, in the body font.
struct MarkdownView: View {
    var markdown: String
    var font: Font = .body

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(MarkdownBlock.parse(markdown).enumerated()), id: \.offset) { _, block in
                view(for: block)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func view(for block: MarkdownBlock) -> some View {
        switch block {
        case .heading(let level, let text):
            Text(inlineMarkdown(text, base: .headline))
                .font(level <= 1 ? .title3.weight(.semibold) : level == 2 ? .headline : .subheadline.weight(.semibold))
                .padding(.top, level <= 2 ? 4 : 0)
        case .paragraph(let text):
            Text(inlineMarkdown(text, base: font)).font(font)
        case .item(let marker, let text, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(marker)
                    .font(font.monospacedDigit())
                    .foregroundStyle(Palette.ink2)
                    .frame(minWidth: 14, alignment: .trailing)
                Text(inlineMarkdown(text, base: font)).font(font)
            }
            .padding(.leading, CGFloat(depth) * 16)
        case .quote(let text):
            Text(inlineMarkdown(text, base: font))
                .font(font)
                .foregroundStyle(Palette.ink2)
                .padding(.leading, 10)
                .overlay(alignment: .leading) { Rectangle().fill(Palette.line).frame(width: 3) }
        case .code(let text):
            ScrollView(.horizontal, showsIndicators: false) {
                Text(text)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(10)
            }
            .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        case .rule:
            Divider()
        }
    }
}
