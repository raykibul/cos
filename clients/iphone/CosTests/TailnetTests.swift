import CosKit
import Foundation
import Testing
@testable import Cos

private func peer(
    _ name: String, _ ip: String, cos: PeerCos = .notRunning, online: Bool = true, lastSeen: Date? = nil,
    mine: Bool = true, isYou: Bool = false
) -> TailnetPeer {
    TailnetPeer(
        id: "n\(name)", name: name, dnsName: "\(name).tail0000.ts.net", os: "linux", ips: [ip], online: online,
        lastSeen: lastSeen, mine: mine, cos: cos, address: "\(ip):7700", isYou: isYou
    )
}

private func host(_ name: String, _ address: String) -> Host {
    Host(id: UUID(), name: name, address: address)
}

@Suite struct TailnetListingTests {
    @Test func mergesByNodeIdAndRunningFromAnyHostWins() {
        let studio = host("studio", "studio:7700")
        let box = host("build-box", "100.101.102.110:7700")
        let old = host("old", "old:7700")
        let away = host("away", "away:7700")
        let earlier = Date(timeIntervalSince1970: 1_000)
        let later = Date(timeIntervalSince1970: 2_000)

        let listing = TailnetListing.merge([
            (studio, .peers(TailnetPeers(peers: [
                peer("gpu-box", "100.1.1.1", cos: .notRunning),
                peer("ipad", "100.1.1.2", cos: .unknown, online: false, lastSeen: earlier),
                peer("alpha", "100.1.1.3", cos: .notRunning),
            ]))),
            (box, .peers(TailnetPeers(peers: [
                peer("gpu-box", "100.1.1.1", cos: .running),
                peer("ipad", "100.1.1.2", cos: .unknown, online: false, lastSeen: later),
                peer("zed", "100.1.1.4", cos: .running),
            ]))),
            (old, .failed(.refused(Refusal(status: 403, code: .hostOnly, message: "host only")))),
            (away, .failed(.unreachable(address: "away:7700", detail: "no route"))),
        ])

        #expect(listing.peers.map(\.name) == ["gpu-box", "zed", "alpha", "ipad"])
        #expect(listing.peers[0].cos == .running)
        #expect(listing.peers[3].lastSeen == later)
        #expect(listing.sources == ["studio", "build-box"])
        #expect(listing.via == "via studio and build-box")
        #expect(listing.outdated == ["old"])
    }

    @Test func aHostWithoutTailscaleSaysWhyAndAddsNothing() {
        let listing = TailnetListing.merge([
            (host("studio", "studio:7700"), .peers(TailnetPeers(peers: [], reason: "Tailscale is not logged in"))),
        ])
        #expect(listing.peers.isEmpty)
        #expect(listing.sources.isEmpty)
        #expect(listing.via == nil)
        #expect(listing.unavailable.map(\.host) == ["studio"])
        #expect(listing.unavailable.map(\.reason) == ["Tailscale is not logged in"])
    }

    @Test func hidesThePhoneAndKeepsOtherUsersApart() {
        let listing = TailnetListing.merge([
            (host("studio", "studio:7700"), .peers(TailnetPeers(peers: [
                peer("gpu-box", "100.1.1.1", cos: .running),
                peer("iphone", "100.1.1.9", isYou: true),
                peer("ci", "100.1.1.5", cos: .unknown, mine: false),
            ]))),
        ])
        let rows = listing.rows(saved: [])
        #expect(rows.mine.map(\.peer.name) == ["gpu-box"])
        #expect(rows.others.map(\.peer.name) == ["ci"])
    }

    @Test func detectsWhatIsAlreadyAdded() {
        let byIP = host("Box", "100.1.1.1:7700")
        let byDNS = host("Studio", "studio.tail0000.ts.net:7700")
        let byName = host("mini", "10.0.0.5:7700")
        let saved = [byIP, byDNS, byName]

        #expect(TailnetListing.savedHost(for: peer("box", "100.1.1.1"), in: saved) == byIP)
        #expect(TailnetListing.savedHost(for: peer("studio", "100.1.1.2"), in: saved) == byDNS)
        #expect(TailnetListing.savedHost(for: peer("MINI", "100.1.1.3"), in: saved) == byName)
        #expect(TailnetListing.savedHost(for: peer("other", "100.1.1.4"), in: saved) == nil)
        // Saved by its short name, found by its MagicDNS label.
        #expect(TailnetListing.savedHost(for: peer("lab", "100.1.1.6"), in: [host("work", "lab:7700")]) != nil)
    }

    @Test func eachRowSaysWhatItIs() {
        let saved = [host("studio", "studio:7700")]
        func state(_ peer: TailnetPeer) -> TailnetRow.State {
            TailnetRow(peer: peer, saved: TailnetListing.savedHost(for: peer, in: saved)).state
        }
        let seen = Date(timeIntervalSince1970: 5_000)
        #expect(state(peer("studio", "100.1.1.1", cos: .running)) == .added)
        #expect(state(peer("gpu", "100.1.1.2", cos: .running)) == .cosRunning)
        #expect(state(peer("mini", "100.1.1.3")) == .online)
        #expect(state(peer("ipad", "100.1.1.4", cos: .unknown, online: false, lastSeen: seen)) == .offline(lastSeen: seen))
        var nowhere = peer("odd", "100.1.1.5", cos: .running)
        nowhere.address = ""
        #expect(state(nowhere) == .online)
        #expect(TailnetRow(peer: peer("gpu", "100.1.1.2"), saved: nil).subtitle == "Linux · 100.1.1.2")
    }
}

@MainActor
@Suite struct TailnetFinderTests {
    private func fakeApp(oldStudio: Bool = false, unreachable: Set<String> = []) -> AppModel {
        let store = HostStore(defaults: scratchDefaults())
        _ = try? store.add(name: "build-box", address: "100.101.102.110")
        _ = try? store.add(name: "studio", address: "studio.tail0000.ts.net")
        return AppModel(hosts: store) { host in
            FakeCos(
                unreachable: unreachable.contains(host.name), replyDelay: nil, hostName: host.name,
                peersHostOnly: oldStudio && host.name == "studio"
            )
        }
    }

    @Test func listsTheFakeTailnetFromEveryHost() async {
        let app = fakeApp()
        await app.refreshTailnet()

        let listing = app.tailnet.listing
        #expect(app.tailnet.hasLoaded)
        #expect(listing.sources == ["build-box", "studio"])
        let rows = listing.rows(saved: app.hosts.hosts)
        #expect(rows.mine.map(\.peer.name) == ["build-box", "gpu-box", "studio", "mini", "ipad"])
        #expect(rows.mine.map(\.state).prefix(4) == [.added, .cosRunning, .added, .online])
        #expect(rows.others.map(\.peer.name) == ["ci-runner"])
        #expect(!rows.mine.contains { $0.peer.isYou })
        #expect(listing.newHosts(saved: app.hosts.hosts).map(\.peer.name) == ["gpu-box"])
    }

    @Test func connectAddsTheMachineAndSwitchesToIt() async throws {
        let app = fakeApp()
        await app.refreshTailnet()
        let gpu = try #require(app.tailnet.listing.newHosts(saved: app.hosts.hosts).first)

        let added = try app.connect(to: gpu.peer)

        #expect(added.name == "gpu-box")
        #expect(added.address == "100.101.102.120:7700")
        #expect(app.hosts.hosts.count == 3)
        #expect(app.hosts.current?.id == added.id)
        #expect(app.session?.host.id == added.id)
        #expect(app.tailnet.listing.newHosts(saved: app.hosts.hosts).isEmpty)

        // Connecting to one already saved switches to it, adding nothing.
        let studio = try #require(app.tailnet.listing.peers.first { $0.name == "studio" })
        let again = try app.connect(to: studio)
        #expect(again.name == "studio")
        #expect(app.hosts.hosts.count == 3)
        #expect(app.hosts.current?.id == again.id)
    }

    @Test func anOlderDaemonIsANoteNotAnError() async {
        let app = fakeApp(oldStudio: true)
        await app.refreshTailnet()

        #expect(app.tailnet.listing.sources == ["build-box"])
        #expect(app.tailnet.listing.outdated == ["studio"])
        #expect(app.tailnet.listing.newHosts(saved: app.hosts.hosts).map(\.peer.name) == ["gpu-box"])
    }

    @Test func unreachableHostsAreSkipped() async {
        let app = fakeApp(unreachable: ["build-box", "studio"])
        await app.refreshTailnet()

        #expect(app.tailnet.hasLoaded)
        #expect(app.tailnet.listing.peers.isEmpty)
        #expect(app.tailnet.listing.sources.isEmpty)
        #expect(app.tailnet.listing.outdated.isEmpty)
    }

    @Test func aHostThatHangsIsGivenUpOn() async throws {
        let finder = TailnetFinder(timeout: .milliseconds(100)) { _ in FakeCos(replyDelay: nil, peersDelay: .seconds(60)) }
        let clock = ContinuousClock()
        let started = clock.now
        await finder.refresh(hosts: [host("slow", "slow:7700")])
        #expect(clock.now - started < .seconds(3))
        #expect(finder.hasLoaded)
        #expect(finder.listing.peers.isEmpty)
    }
}
