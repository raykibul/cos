import Foundation

/// `GET /v1/status`.
public struct Status: Codable, Equatable, Sendable {
    public var version: String
    public var pid: Int
    public var startedAt: Date
    /// The daemon's home directory (`~/.cos` or `$COS_HOME`).
    public var home: String
    /// Number of stored events.
    public var events: Int
    /// Sequence number of the newest event, 0 if there are none.
    public var lastSeq: Int

    public init(version: String, pid: Int, startedAt: Date, home: String, events: Int, lastSeq: Int) {
        self.version = version
        self.pid = pid
        self.startedAt = startedAt
        self.home = home
        self.events = events
        self.lastSeq = lastSeq
    }

    private enum CodingKeys: String, CodingKey {
        case version, pid, startedAt, home, events, lastSeq
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = c.string(.version) ?? ""
        pid = c.int(.pid) ?? 0
        startedAt = c.optional(Date.self, .startedAt) ?? Date(timeIntervalSince1970: 0)
        home = c.string(.home) ?? ""
        events = c.int(.events) ?? 0
        // The one field a client relies on to resume the event stream.
        lastSeq = try c.id(.lastSeq)
    }
}

/// One entry of the daemon's event log.
public struct LoggedEvent: Codable, Equatable, Sendable, Identifiable {
    /// Position in the log, starting at 1. Never reused.
    public var seq: Int
    public var ts: Date
    /// Dotted name such as `daemon.started`.
    public var kind: String
    /// The run this event belongs to, if any.
    public var runId: String?
    public var data: JSONValue

    public var id: Int { seq }

    public init(seq: Int, ts: Date, kind: String, runId: String? = nil, data: JSONValue) {
        self.seq = seq
        self.ts = ts
        self.kind = kind
        self.runId = runId
        self.data = data
    }

    private enum CodingKeys: String, CodingKey {
        case seq, ts, kind, runId, data
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        seq = try c.id(.seq)
        ts = c.optional(Date.self, .ts) ?? Date(timeIntervalSince1970: 0)
        kind = c.string(.kind) ?? "unknown"
        runId = c.string(.runId)
        data = c.optional(JSONValue.self, .data) ?? .null
    }

    /// `data.agent_id`, `data.project_id` and the like, as numbers.
    public func number(_ key: String) -> Int? {
        switch data[key] {
        case .number(let value)?: return value.rounded() == value ? Int(value) : nil
        case .string(let text)?: return Int(text)
        default: return nil
        }
    }
}

/// `GET /v1/events`.
public struct EventsPage: Codable, Equatable, Sendable {
    public var events: [LoggedEvent]

    public init(events: [LoggedEvent]) {
        self.events = events
    }

    public init(from decoder: Decoder) throws {
        events = try decoder.container(keyedBy: CodingKeys.self).list(LoggedEvent.self, .events)
    }
}

/// Event kinds the daemon writes today. New kinds arrive with new milestones,
/// so code must not assume this list is complete.
public enum EventKind {
    public static let daemonStarted = "daemon.started"
    public static let daemonStopped = "daemon.stopped"
}

/// Any JSON value. Event `data` differs per kind, so it stays untyped here.
public enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }

    public subscript(key: String) -> JSONValue? {
        if case .object(let object) = self { object[key] } else { nil }
    }

    /// One-line text for lists: `pid=76227 version=0.1.0` for objects,
    /// with nested values as compact JSON.
    public var compact: String {
        if case .object(let object) = self {
            return object.keys.sorted().map { "\($0)=\(object[$0]!.inline)" }.joined(separator: " ")
        }
        return inline
    }

    private var inline: String {
        switch self {
        case .null: return "null"
        case .bool(let value): return String(value)
        case .number(let value):
            // Integers come back as Double; print 76227, not 76227.0.
            if value.rounded() == value, abs(value) < 1e15 { return String(Int64(value)) }
            return String(value)
        case .string(let value): return value
        case .array(let values): return "[" + values.map(\.json).joined(separator: ",") + "]"
        case .object: return json
        }
    }

    private var json: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        guard let data = try? encoder.encode(self) else { return "?" }
        return String(decoding: data, as: UTF8.self)
    }
}

extension JSONDecoder {
    /// A decoder for the daemon's JSON: snake_case keys and RFC 3339 dates.
    public static func cos() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let text = try container.decode(String.self)
            guard let date = parseRFC3339(text) else {
                throw DecodingError.dataCorruptedError(in: container, debugDescription: "not an RFC 3339 date: \(text)")
            }
            return date
        }
        return decoder
    }
}

/// Parses `2026-10-21T18:36:19Z`, `…19.126Z` and `…19.126787Z` alike.
/// The daemon writes as many fractional digits as it has, and Foundation's
/// ISO 8601 parsers are picky about the count, so the fraction is split off.
public func parseRFC3339(_ text: String) -> Date? {
    var base = Substring(text)
    var fraction = 0.0
    if let dot = text.firstIndex(of: ".") {
        let digits = text[text.index(after: dot)...].prefix(while: \.isNumber)
        guard !digits.isEmpty, let value = Double("0." + digits) else { return nil }
        fraction = value
        base = text[..<dot] + text[digits.endIndex...]
    }
    guard let whole = try? Date(String(base), strategy: .iso8601) else { return nil }
    return whole.addingTimeInterval(fraction)
}
