import Darwin
import Foundation
import os
import Testing
@testable import CosKit

/// The tailnet side: addresses, HTTP and SSE over TCP, and refusals with codes.
@Suite struct AddressTests {
    @Test(arguments: [
        ("my-mac", "my-mac:7700"),
        ("my-mac:7701", "my-mac:7701"),
        ("  my-mac.tail1234.ts.net ", "my-mac.tail1234.ts.net:7700"),
        ("100.101.102.103", "100.101.102.103:7700"),
        ("100.101.102.103:7700", "100.101.102.103:7700"),
        ("fd7a:115c:a1e0::1", "[fd7a:115c:a1e0::1]:7700"),
        ("[fd7a:115c:a1e0::1]", "[fd7a:115c:a1e0::1]:7700"),
        ("[fd7a:115c:a1e0::1]:7701", "[fd7a:115c:a1e0::1]:7701"),
    ])
    func theSamePortDefaultAsTheCLI(typed: String, expected: String) throws {
        let endpoint = try CosEndpoint.parse(address: typed)
        #expect(endpoint.description == expected)
        #expect(endpoint.isRemote)
        #expect(endpoint.hostHeader == expected)
    }

    @Test func ipv6IsKeptWithoutBrackets() throws {
        #expect(try CosEndpoint.parse(address: "[fd7a::5]:7701") == .tcp(host: "fd7a::5", port: 7701))
    }

    @Test(arguments: ["", "   ", "http://my-mac:7700", "my-mac/v1", "my mac", "my-mac:0", "my-mac:99999", "my-mac:port", "[not-ip]:7700", "[fd7a::1", "a:b:c:7700x"])
    func refusesWhatIsNotAnAddress(typed: String) {
        #expect {
            try CosEndpoint.parse(address: typed)
        } throws: { error in
            guard case CosError.invalidAddress(let message) = error else { return false }
            return message.contains("not a host address")
        }
    }

    @Test func theSocketStaysThePlaceholderHost() {
        let unix = CosEndpoint.unix("/tmp/cos.sock")
        #expect(!unix.isRemote)
        #expect(unix.hostHeader == "cos")
        #expect(CosClient(socketPath: "/tmp/cos.sock").socketPath == "/tmp/cos.sock")
    }
}

@Suite struct TCPTests {
    static let workspaceWithoutTitle = #"{"id":9,"project_id":1,"name":"kyoto","title":null,"city":"kyoto","branch":"atlas/kyoto","path":"/w/kyoto","base":"head","base_ref":"HEAD","base_commit":"abc","created_at":"2026-10-22T09:00:00Z"}"#

    @Test func requestsAndStreamsOverTCP() async throws {
        let server = try FakeDaemon(tcp: ()) { request in
            switch request.path {
            case "/v1/status": .json(200, Samples.status)
            case "/v1/projects":
                .json(200, #"{"projects":[{"id":1,"name":"atlas","root":"/r","base_branch":"main","has_origin":true,"created_at":"2026-10-22T09:00:00Z","workspaces":[\#(Self.workspaceWithoutTitle)]}]}"#)
            case let path where path.hasPrefix("/v1/events/stream"):
                .streamThenEnd("id: 2\ndata: {\"seq\":2,\"ts\":\"2026-10-22T09:00:00Z\",\"kind\":\"agent.transcript\",\"data\":{\"agent_id\":40,\"next\":12}}\n\n")
            default: .json(404, #"{"error":"no such route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(endpoint: server.endpoint)

        #expect(try await client.status().lastSeq == 1)
        let projects = try await client.projects()
        let workspace = try #require(projects.first?.workspaces.first)
        #expect(workspace.title == nil)
        #expect(workspace.displayName == "kyoto")
        #expect(workspace.branch == "atlas/kyoto")

        var events: [LoggedEvent] = []
        // The stream never ends quietly: the daemon closing it is an error.
        await #expect(throws: CosError.connectionClosed) {
            for try await event in client.stream(after: 1) { events.append(event) }
        }
        #expect(events.map(\.kind) == ["agent.transcript"])
        #expect(events.first?.number("agent_id") == 40)
        #expect(events.first?.number("next") == 12)

        // Over TCP the Host header is the address, as the bridge sends it.
        #expect(server.requests.allSatisfy { $0.headers["host"] == server.endpoint.description })
    }

    @Test func refusalsCarryTheirCode() async throws {
        let server = try FakeDaemon(tcp: ()) { request in
            switch request.path {
            case "/v1/status":
                .json(403, #"{"error":"this device (phone) is waiting for approval. On my-mac, run `cos devices approve 2`","code":"device_pending"}"#)
            case "/v1/projects":
                .json(503, #"{"error":"tailscale whois failed; try again","code":"tailscale_unavailable"}"#)
            case "/v1/runs":
                .json(403, #"{"error":"something new","code":"brand_new_code"}"#)
            default:
                .json(404, #"{"error":"unknown agent 9"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(endpoint: server.endpoint)

        await #expect {
            try await client.status()
        } throws: { error in
            guard let error = error as? CosError, let refusal = error.refusal else { return false }
            return refusal.status == 403 && refusal.code == .devicePending && error.code == .devicePending
                && refusal.message.contains("cos devices approve 2") && error.localizedDescription == refusal.message
                && refusal.code.mayClearByItself
        }
        await #expect(throws: CosError.refused(Refusal(status: 503, code: .tailscaleUnavailable, message: "tailscale whois failed; try again"))) {
            try await client.projects()
        }
        // A code this client doesn't know is still a refusal.
        await #expect(throws: CosError.refused(Refusal(status: 403, code: RefusalCode(rawValue: "brand_new_code"), message: "something new"))) {
            try await client.runs()
        }
        // Without a code it's an ordinary answer.
        await #expect(throws: CosError.notFound("unknown agent 9")) { try await client.agent(id: 9) }
    }

    @Test func theFollowerRetriesARefusedDevice() async throws {
        let approved = Flag()
        let server = try FakeDaemon(tcp: ()) { request in
            if !approved.isSet {
                return .json(403, #"{"error":"waiting for approval","code":"device_pending"}"#)
            }
            if request.path == "/v1/status" { return .json(200, Samples.status) }
            return .stream("")
        }
        defer { server.stop() }
        let follower = EventFollower(source: CosClient(endpoint: server.endpoint), backoff: Backoff(initial: .milliseconds(10), maximum: .milliseconds(20)))
        var sawRefusal = false
        for await update in follower.updates() {
            switch update {
            case .disconnected(let error, _):
                if error.code == .devicePending {
                    sawRefusal = true
                    approved.set()
                }
            case .connected(let status):
                #expect(status.lastSeq == 1)
                #expect(sawRefusal)
                return
            default: break
            }
        }
        Issue.record("the follower ended")
    }

    @Test func aClosedPortIsUnreachable() async throws {
        // Take a free port and give it back, so nothing listens there. (Not
        // with a FakeDaemon: its accept thread would outlive the socket and
        // could take another test's listener when the number is reused.)
        let probe = socket(AF_INET, SOCK_STREAM, 0)
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                _ = bind(probe, $0, length)
                _ = getsockname(probe, $0, &length)
            }
        }
        close(probe)
        let endpoint = CosEndpoint.tcp(host: "127.0.0.1", port: Int(UInt16(bigEndian: address.sin_port)))
        await #expect {
            try await CosClient(endpoint: endpoint).status()
        } throws: { error in
            guard case CosError.unreachable(let address, let detail) = error else { return false }
            return address == endpoint.description && detail.contains("cos remote on")
        }
    }

    @Test func anUnknownNameIsUnreachable() async throws {
        let client = CosClient(endpoint: .tcp(host: "no-such-host.invalid", port: 7700))
        await #expect {
            try await client.status()
        } throws: { error in
            guard let error = error as? CosError, case .unreachable = error else { return false }
            return error.isUnreachable
        }
    }
}

/// A flag shared with a fake daemon's handler.
final class Flag: Sendable {
    private let value = OSAllocatedUnfairLockBox(false)
    var isSet: Bool { value.get() }
    func set() { value.set(true) }
}


final class OSAllocatedUnfairLockBox<Value: Sendable>: Sendable {
    private let lock: OSAllocatedUnfairLock<Value>
    init(_ value: Value) { lock = OSAllocatedUnfairLock(initialState: value) }
    func get() -> Value { lock.withLock { $0 } }
    func set(_ value: Value) { lock.withLock { $0 = value } }
}
