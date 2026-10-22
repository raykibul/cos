import Foundation

// Environment variables for agents ("Environment" in docs/daemon.md): a
// global list for every project, and each project's own, which wins. Values
// may be secrets, so apps mask them until asked.

/// Name → value.
public typealias EnvVars = [String: String]

/// `GET /v1/env`: the global list.
public struct GlobalEnv: Codable, Equatable, Sendable {
    public var variables: EnvVars

    public init(variables: EnvVars) {
        self.variables = variables
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        variables = c.vars(.variables) ?? [:]
    }
}

/// `GET /v1/projects/{id}/env`.
public struct ProjectEnv: Codable, Equatable, Sendable {
    /// The project's own variables.
    public var variables: EnvVars
    /// The list for all projects.
    public var global: EnvVars
    /// What its agents get: `global` with `variables` on top.
    public var effective: EnvVars

    public init(variables: EnvVars, global: EnvVars, effective: EnvVars? = nil) {
        self.variables = variables
        self.global = global
        self.effective = effective ?? global.merging(variables) { _, project in project }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        variables = c.vars(.variables) ?? [:]
        global = c.vars(.global) ?? [:]
        effective = c.vars(.effective) ?? global.merging(variables) { _, project in project }
    }

    /// Whether the project's own value replaces a global one.
    public func overrides(_ name: String) -> Bool {
        variables[name] != nil && global[name] != nil
    }
}

/// Names sorted for lists, with `CLAUDE_CONFIG_DIR` first: it picks the
/// Claude account.
public func sortedEnvNames<S: Sequence>(_ names: S) -> [String] where S.Element == String {
    names.sorted { a, b in
        if a == b { return false }
        if a == "CLAUDE_CONFIG_DIR" { return true }
        if b == "CLAUDE_CONFIG_DIR" { return false }
        return a < b
    }
}

/// A value as lists show it: its first three characters and its length
/// (`sk-…  (108 chars)`), as the CLI masks it. Short values show none of
/// themselves.
public func maskedEnvValue(_ value: String) -> String {
    if value.isEmpty { return "(empty)" }
    if value.count < 8 { return "•••  (\(value.count) chars)" }
    return "\(value.prefix(3))…  (\(value.count) chars)"
}

extension KeyedDecodingContainer {
    /// `{"NAME": "value"}`; a `[{name, value}]` list is read too, and
    /// numbers and booleans become strings. `nil` when missing.
    func vars(_ key: Key) -> EnvVars? {
        guard let value = optional(JSONValue.self, key) else { return nil }
        var out: EnvVars = [:]
        func put(_ name: String, _ value: JSONValue?) {
            guard !name.isEmpty else { return }
            switch value {
            case .string(let text)?: out[name] = text
            case .number(let number)?: out[name] = number.rounded() == number ? String(Int64(number)) : String(number)
            case .bool(let flag)?: out[name] = String(flag)
            default: break
            }
        }
        switch value {
        case .object(let object):
            for (name, item) in object { put(name, item) }
        case .array(let items):
            for item in items {
                if case .string(let name)? = item["name"] { put(name, item["value"]) }
            }
        default:
            return nil
        }
        return out
    }
}
