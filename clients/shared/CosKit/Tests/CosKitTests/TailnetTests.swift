import Foundation
import Testing
@testable import CosKit

/// `GET /v1/tailnet/peers`, as the daemon answers it over the tailnet.
@Suite struct TailnetTests {
    /// The shape in docs/daemon.md, asked by the phone (`is_you`).
    static let page = #"""
    {"peers": [
      {"id": "nBOX", "name": "build-box", "hostname": "build-box", "dns_name": "build-box.tail0000.ts.net",
       "os": "linux", "ips": ["100.100.2.2", "fd7a:115c:a1e0::2:2"], "online": true, "last_seen_ms": null,
       "mine": true, "tagged": false, "cos": "running", "address": "100.100.2.2:7700", "is_you": false},
      {"id": "nPHONE", "name": "phone", "hostname": "My Phone", "dns_name": "phone.tail0000.ts.net",
       "os": "iOS", "ips": ["100.100.2.1"], "online": true, "last_seen_ms": null,
       "mine": true, "tagged": false, "cos": "not_running", "address": "100.100.2.1:7700", "is_you": true},
      {"id": "nPAD", "name": "ipad", "hostname": "iPad", "dns_name": "", "os": "iOS", "ips": ["100.100.2.3"],
       "online": false, "last_seen_ms": 1792596501100, "mine": true, "tagged": false, "cos": "unknown",
       "address": "100.100.2.3:7700", "is_you": false}
    ],
    "self_name": "my-mac",
    "self": {"name": "my-mac", "dns_name": "my-mac.tail0000.ts.net", "os": "macOS", "address": "100.100.1.1:7700"},
    "reason": null}
    """#

    @Test func decodesThePage() throws {
        let page = try JSONDecoder.cos().decode(TailnetPeers.self, from: Data(Self.page.utf8))
        #expect(page.peers.map(\.name) == ["build-box", "phone", "ipad"])
        #expect(page.selfName == "my-mac")
        #expect(page.host == TailnetHost(name: "my-mac", dnsName: "my-mac.tail0000.ts.net", os: "macOS", address: "100.100.1.1:7700"))
        #expect(page.hostName == "my-mac")
        #expect(page.reason == nil)

        let box = page.peers[0]
        #expect(box.id == "nBOX")
        #expect(box.cos == .running)
        #expect(box.mine && box.online && !box.tagged && !box.isYou)
        #expect(box.ips == ["100.100.2.2", "fd7a:115c:a1e0::2:2"])
        #expect(box.displayIP == "100.100.2.2")
        #expect(box.address == "100.100.2.2:7700")
        #expect(page.peers[1].isYou)
        #expect(page.peers[1].cos == .notRunning)
        let ipad = page.peers[2]
        #expect(!ipad.online)
        #expect(abs((ipad.lastSeen?.timeIntervalSince1970 ?? 0) - 1_792_596_501.1) < 1e-3)
    }

    @Test func decodesLeniently() throws {
        // An older daemon: no `self`, no `is_you`. A peer without an id is
        // dropped, odd types fall back, and a new cos state stays readable.
        let json = #"""
        {"peers": [
          {"name": "no-id"},
          {"id": "n1", "hostname": "Box", "online": "yes", "cos": "warming_up", "ips": ["100.1.1.1", 7],
           "last_seen_ms": "nope", "extra": {"x": 1}},
          {"id": "n2", "name": "", "hostname": "", "address": "100.100.2.9:7700"}
        ], "self_name": "my-mac"}
        """#
        let page = try JSONDecoder.cos().decode(TailnetPeers.self, from: Data(json.utf8))
        #expect(page.peers.map(\.id) == ["n1", "n2"])
        #expect(page.host == nil)
        #expect(page.hostName == "my-mac")
        let box = page.peers[0]
        #expect(box.name == "Box")
        #expect(!box.online && !box.mine && !box.isYou)
        #expect(box.cos == PeerCos(rawValue: "warming_up"))
        #expect(box.ips == ["100.1.1.1"])
        #expect(box.lastSeen == nil)
        #expect(page.peers[1].name == "n2")
        #expect(page.peers[1].displayIP == "100.100.2.9")

        let empty = try JSONDecoder.cos().decode(TailnetPeers.self, from: Data(#"{"reason":"Tailscale is not logged in"}"#.utf8))
        #expect(empty.peers.isEmpty)
        #expect(empty.reason == "Tailscale is not logged in")
        #expect(empty.hostName == nil)
    }

    @Test func roundTrips() throws {
        let page = try JSONDecoder.cos().decode(TailnetPeers.self, from: Data(Self.page.utf8))
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let again = try JSONDecoder.cos().decode(TailnetPeers.self, from: encoder.encode(page))
        #expect(again == page)
    }

    @Test func asksTheRouteOverTCP() async throws {
        let server = try FakeDaemon(tcp: ()) { request in
            switch request.path {
            case "/v1/tailnet/peers", "/v1/tailnet/peers?port=7701": .json(200, Self.page)
            default: .json(404, #"{"error":"no such route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(endpoint: server.endpoint)

        #expect(try await client.tailnetPeers().peers.count == 3)
        #expect(try await client.tailnetPeers(port: 7701).host?.name == "my-mac")
        #expect(server.requests.map(\.path) == ["/v1/tailnet/peers", "/v1/tailnet/peers?port=7701"])
    }

    @Test func anOlderDaemonKeepsItsPeersToItself() async throws {
        let server = try FakeDaemon(tcp: ()) { _ in
            .json(403, #"{"error":"devices and remote access can only be managed on the host (`cos devices`, `cos remote`)","code":"host_only"}"#)
        }
        defer { server.stop() }

        await #expect {
            try await CosClient(endpoint: server.endpoint).tailnetPeers()
        } throws: { error in
            (error as? CosError)?.peersNotShared == true
        }
        #expect(!CosError.refused(Refusal(status: 403, code: .devicePending, message: "")).peersNotShared)
    }
}
