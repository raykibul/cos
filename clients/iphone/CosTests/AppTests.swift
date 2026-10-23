import CosKit
import Foundation
import Testing
@testable import Cos

@Suite struct LaunchTests {
    @Test func readsTheFakeDataFlags() {
        let launch = Launch.from(["Cos", "-fakeData", "-screen", "chief", "-fakeRefusal", "device_pending"])
        #expect(launch.fakeData)
        #expect(launch.screen == "chief")
        #expect(launch.fakeRefusal == "device_pending")
        #expect(!launch.fakeUnreachable)
        #expect(!launch.noHosts)
    }

    @Test func aFlagWithoutItsValueIsIgnored() {
        let launch = Launch.from(["Cos", "-screen", "-fakeUnreachable"])
        #expect(launch.screen == nil)
        #expect(launch.fakeUnreachable)
        #expect(Launch.from(["Cos"]) == Launch())
    }
}

@MainActor
@Suite struct HostStoreTests {
    @Test func addsAHostByItsTailscaleName() throws {
        let store = HostStore(defaults: scratchDefaults())
        let host = try store.add(name: "", address: " my-mac ")

        #expect(host.name == "my-mac")
        #expect(host.address == "my-mac:7700")
        #expect(store.current == host)
    }

    @Test func addsAHostByItsAddress() throws {
        let store = HostStore(defaults: scratchDefaults())
        let host = try store.add(name: "Build box", address: "100.101.102.103:7701")

        #expect(host.name == "Build box")
        #expect(host.endpoint == .tcp(host: "100.101.102.103", port: 7701))
    }

    @Test func refusesWhatIsNotAnAddress() {
        let store = HostStore(defaults: scratchDefaults())
        #expect(throws: CosError.self) { try store.add(name: "", address: "https://my-mac") }
        #expect(throws: CosError.self) { try store.add(name: "", address: "my mac") }
        #expect(store.hosts.isEmpty)
    }

    @Test func theSameAddressTwiceIsOneHost() throws {
        let store = HostStore(defaults: scratchDefaults())
        let first = try store.add(name: "", address: "studio")
        try store.add(name: "", address: "other")
        let again = try store.add(name: "Studio", address: "studio:7700")

        #expect(store.hosts.count == 2)
        #expect(again.id == first.id)
        #expect(again.name == "Studio")
        #expect(store.current?.id == first.id)
    }

    @Test func renamesRemovesAndRemembers() throws {
        let defaults = scratchDefaults()
        let store = HostStore(defaults: defaults)
        let a = try store.add(name: "a", address: "a")
        let b = try store.add(name: "b", address: "b")

        store.rename(a.id, to: "  Studio  ")
        store.rename(b.id, to: "   ")
        store.select(a.id)

        let reopened = HostStore(defaults: defaults)
        #expect(reopened.hosts.map(\.name) == ["Studio", "b"])
        #expect(reopened.current?.id == a.id)

        reopened.remove(a.id)
        #expect(reopened.current?.id == b.id)
        reopened.remove(b.id)
        #expect(reopened.current == nil)
    }
}

@MainActor
@Suite struct AppModelTests {
    private func model(_ store: HostStore) -> AppModel {
        AppModel(hosts: store) { _ in FakeCos(replyDelay: nil) }
    }

    @Test func noHostsMeansNoSession() {
        let app = model(HostStore(defaults: scratchDefaults()))
        #expect(app.session == nil)
    }

    @Test func followsOnlyInTheForeground() throws {
        let store = HostStore(defaults: scratchDefaults())
        try store.add(name: "", address: "studio")
        let app = model(store)
        let session = try #require(app.session)
        #expect(!session.isFollowing)

        app.setActive(true)
        #expect(session.isFollowing)
        app.setActive(false)
        #expect(!session.isFollowing)
    }

    @Test func switchingHostsStartsANewSession() throws {
        let store = HostStore(defaults: scratchDefaults())
        try store.add(name: "", address: "studio")
        let app = model(store)
        app.setActive(true)
        let first = try #require(app.session)

        let other = try app.addHost(name: "", address: "build-box")

        #expect(app.session?.host.id == other.id)
        #expect(!first.isFollowing)
        #expect(app.session?.isFollowing == true)

        // Renaming keeps the session: it still follows the same address.
        let renamed = app.session
        app.renameHost(other.id, to: "Build box")
        #expect(app.hosts.current?.name == "Build box")
        #expect(app.session === renamed)
        app.removeHost(other.id)
        #expect(app.session?.host.address == "studio:7700")
        app.setActive(false)
    }
}

@MainActor
@Suite struct DetailTests {
    @Test func messagingAnExitedAgentResumesItFirst() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let detail = AgentDetail(agentId: 14, session: session)
        #expect(detail.agent?.state.canResume == true)

        detail.draft = "Carry on"
        await detail.send()

        #expect(detail.error == nil)
        #expect(detail.draft.isEmpty)
        let calls = await fake.calls
        let resume = calls.firstIndex(of: "POST /v1/agents/14/resume")
        let input = calls.firstIndex(of: "POST /v1/agents/14/input")
        #expect(resume != nil && input != nil && resume! < input!)
    }

    @Test func theScreenLoadsOnRequest() async {
        let session = await liveSession()
        defer { session.stop() }
        let detail = AgentDetail(agentId: 7, session: session)
        #expect(detail.screen == nil)

        await detail.loadScreen()

        #expect(detail.screen?.text.isEmpty == false)
    }
}

@Suite struct MarkdownTests {
    @Test func splitsBlocks() {
        let blocks = MarkdownBlock.parse("""
        # Title

        A paragraph
        that wraps.

        - one
          continued
        2. two
        > quoted
        ```
        let x = 1
        ```
        ---
        """)
        #expect(blocks == [
            .heading(level: 1, text: "Title"),
            .paragraph("A paragraph\nthat wraps."),
            .item(marker: "•", text: "one continued", depth: 0),
            .item(marker: "2.", text: "two", depth: 0),
            .quote("quoted"),
            .code("let x = 1"),
            .rule,
        ])
    }

    @Test func anUnclosedFenceKeepsItsCode() {
        #expect(MarkdownBlock.parse("```\nnpm test") == [.code("npm test")])
    }

    @Test func inlineCodeLosesItsIntentButKeepsItsText() {
        let string = inlineMarkdown("run `cos status` now", base: .body)
        #expect(String(string.characters) == "run cos status now")
        #expect(!string.runs.contains { $0.inlinePresentationIntent?.contains(.code) == true })
    }
}
