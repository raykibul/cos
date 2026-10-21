/// One server-sent event.
public struct SSEMessage: Equatable, Sendable {
    public var id: String?
    public var event: String?
    public var data: String
}

/// Splits a `text/event-stream` body into messages, following the WHATWG
/// rules the daemon relies on: `data:` lines joined with newlines, `id:`,
/// `event:`, comment lines starting with `:` (keep-alives) ignored, and a
/// blank line ending each message. Input can arrive split anywhere.
public struct SSEParser {
    private var line: [UInt8] = []
    /// The previous byte was a CR, so a following LF belongs to the same line end.
    private var afterCR = false
    private var id: String?
    private var event: String?
    private var data: [String] = []

    public init() {}

    public mutating func feed(_ bytes: [UInt8]) -> [SSEMessage] {
        var messages: [SSEMessage] = []
        for byte in bytes {
            if afterCR {
                afterCR = false
                if byte == 10 { continue }
            }
            switch byte {
            case 10, 13:
                afterCR = byte == 13
                if let message = endLine() { messages.append(message) }
            default:
                line.append(byte)
            }
        }
        return messages
    }

    private mutating func endLine() -> SSEMessage? {
        defer { line.removeAll(keepingCapacity: true) }
        if line.isEmpty {
            return dispatch()
        }
        let text = String(decoding: line, as: UTF8.self)
        if text.hasPrefix(":") { return nil }
        let field: Substring
        var value: Substring
        if let colon = text.firstIndex(of: ":") {
            field = text[..<colon]
            value = text[text.index(after: colon)...]
            if value.hasPrefix(" ") { value = value.dropFirst() }
        } else {
            field = Substring(text)
            value = ""
        }
        switch field {
        case "data": data.append(String(value))
        case "id": id = String(value)
        case "event": event = String(value)
        default: break // `retry` and unknown fields: the client has its own backoff.
        }
        return nil
    }

    private mutating func dispatch() -> SSEMessage? {
        defer {
            id = nil
            event = nil
            data = []
        }
        // A message without data, such as a keep-alive, isn't delivered.
        guard !data.isEmpty else { return nil }
        return SSEMessage(id: id, event: event, data: data.joined(separator: "\n"))
    }
}
