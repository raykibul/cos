import Foundation

// Lenient decoding, as the desktop app's decoders do it. The daemon's JSON
// grows with each milestone, so unknown fields are ignored, states stay open
// strings, an optional field of the wrong type falls back to its default,
// and one odd row in a list never hides the rest. Only an object without
// its identity (an id, say) is refused.

/// One element of a list that may not decode; it then becomes `nil` and is
/// dropped, rather than failing the whole list.
struct Lenient<Value: Decodable>: Decodable {
    var value: Value?

    init(from decoder: Decoder) throws {
        value = try? Value(from: decoder)
    }
}

extension KeyedDecodingContainer {
    /// The list under `key`, skipping elements that don't decode. Missing
    /// or not a list: empty.
    func list<T: Decodable>(_ type: T.Type, _ key: Key) -> [T] {
        ((try? decodeIfPresent([Lenient<T>].self, forKey: key)) ?? nil)?.compactMap(\.value) ?? []
    }

    /// The value under `key`, or `nil` when it is missing, null or of the
    /// wrong type.
    func optional<T: Decodable>(_ type: T.Type, _ key: Key) -> T? {
        (try? decodeIfPresent(T.self, forKey: key)) ?? nil
    }

    /// A number that may come as a string, since ids drift between the two.
    func int(_ key: Key) -> Int? {
        if let value = optional(Int.self, key) { return value }
        if let value = optional(Double.self, key), value.rounded() == value, abs(value) < 1e15 { return Int(value) }
        if let text = optional(String.self, key) { return Int(text.trimmingCharacters(in: .whitespaces)) }
        return nil
    }

    func int64(_ key: Key) -> Int64? {
        if let value = optional(Int64.self, key) { return value }
        if let value = optional(Double.self, key), value.rounded() == value, abs(value) < 1e18 { return Int64(value) }
        if let text = optional(String.self, key) { return Int64(text.trimmingCharacters(in: .whitespaces)) }
        return nil
    }

    /// A required id: a number, or a numeric string.
    func id(_ key: Key) throws -> Int {
        guard let value = int(key) else {
            throw DecodingError.keyNotFound(key, .init(codingPath: codingPath, debugDescription: "no \(key.stringValue)"))
        }
        return value
    }

    /// Text under `key`; a number becomes its digits.
    func string(_ key: Key) -> String? {
        if let text = optional(String.self, key) { return text }
        if let number = int64(key) { return String(number) }
        return nil
    }

    /// A list of strings, dropping anything else.
    func strings(_ key: Key) -> [String] {
        list(String.self, key)
    }
}

extension Optional where Wrapped == String {
    /// `nil` for nil and blank strings; otherwise the text as it is.
    var nilIfBlank: String? {
        guard let self, !self.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return self
    }
}
