import CosKit
import Foundation
import Testing
@testable import Cos

@MainActor
@Suite struct HostSessionTests {
    @Test func loadsEverythingAndListsWhatWaitsOnYou() async {
        let session = await liveSession()
        defer { session.stop() }

        #expect(session.connection == .live)
        #expect(session.projects.map(\.name) == ["atlas", "harbor", "opal"])
        #expect(session.staffs.keys.sorted() == [1, 2, 3])
        #expect(session.waiting.map(\.id) == ["approval-12", "trust-12", "login-11"])
        #expect(session.waitingCount == 3)
    }

    @Test func decidingAnApprovalTakesItOffTheList() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let approval = try #require(session.approvals.first { $0.id == 12 })

        try await session.decide(approval, .allow)

        #expect(!session.waiting.contains { $0.id == "approval-12" })
        #expect(await eventually { session.agent(id: 8)?.state == .working })
    }

    @Test func eventsRefreshTheLists() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }

        await fake.setAgentState(7, .needsTrust)

        #expect(await eventually { session.waiting.contains { $0.id == "trust-7" } })
    }

    @Test func newItemsNotifyAndSetTheBadge() async {
        let fake = FakeCos(replyDelay: nil)
        let notes = RecordingNotifications()
        let session = await liveSession(fake, notifications: notes)
        defer { session.stop() }
        _ = await eventually { session.waitingCount == 3 }

        await fake.setAgentState(7, .needsTrust)
        _ = await eventually { session.waitingCount == 4 }
        try? await Task.sleep(for: .milliseconds(100))

        // What already waited at launch isn't announced; the new one is.
        let notified = await notes.notified
        #expect(notified.map(\.kind) == [.trust])
        #expect(notified.first?.title.contains("Fix the session timeout") == true)
        #expect(await notes.badges.last == 4)
    }

    @Test func aPendingDeviceIsRefusedThenLetInOnceApproved() async {
        let fake = FakeCos(refusal: .devicePending, replyDelay: nil)
        let session = HostSession(host: testHost(), service: fake, backoff: quickBackoff)
        session.start()
        defer { session.stop() }

        #expect(await eventually {
            if case .refused(let refusal) = session.connection { return refusal.code == .devicePending }
            return false
        })
        if case .refused(let refusal) = session.connection {
            #expect(refusal.message.contains("cos devices approve 3"))
        }

        // Approved on the host: the follower's retry gets in by itself.
        await fake.setRefusal(nil)
        #expect(await eventually { session.connection.isLive && session.hasLoaded })
    }

    @Test func anUnreachableHostSaysSo() async {
        let session = HostSession(host: testHost(), service: FakeCos(unreachable: true, replyDelay: nil), backoff: quickBackoff)
        session.start()
        defer { session.stop() }

        #expect(await eventually {
            if case .unreachable(let message) = session.connection { return message.contains("Tailscale") }
            return false
        })
    }

    @Test func stoppingKeepsWhatLoaded() async {
        let session = await liveSession()
        session.stop()

        #expect(!session.isFollowing)
        #expect(session.projects.count == 3)
        session.start()
        #expect(session.isFollowing)
        session.stop()
    }

    @Test func lookups() async {
        let session = await liveSession()
        defer { session.stop() }

        #expect(session.workspaceName(4) == "porto")
        #expect(session.workspaceName(5) == "Faster search")
        #expect(session.project(ofWorkspace: 9)?.name == "harbor")
        #expect(session.pendingApproval(forAgent: 8)?.id == 12)
        #expect(session.agentsStarted(byProject: 1).map(\.id).contains(8))
    }
}

@MainActor
@Suite struct SessionFolderTests {
    @Test func startsAnotherSessionBesideOne() async throws {
        let session = await liveSession()
        defer { session.stop() }

        let agent = try await session.startSession(workspaceId: 3, prompt: "  Add a test for the TTL  ")

        #expect(agent.workspaceId == 3)
        #expect(session.agent(id: agent.id) != nil)
        #expect(session.agents(inWorkspace: 3).map(\.id).contains(7))
        #expect(session.sessionsSharingFolder(with: agent).map(\.id).sorted() == [7, 13])
    }

    @Test func removingARunningSessionStopsItFirstAndKeepsASharedFolder() async throws {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let agent = try #require(session.agent(id: 7))

        try await session.removeSession(agent, withFolder: false, timeout: .seconds(2))

        let calls = await fake.calls
        #expect(calls.contains("POST /v1/agents/7/stop"))
        #expect(calls.contains("DELETE /v1/agents/7"))
        #expect(!calls.contains("DELETE /v1/workspaces/3"))
        #expect(session.agent(id: 7) == nil)
        #expect(session.agent(id: 13) != nil)
    }

    @Test func removingTheLastSessionCanTakeItsFolder() async throws {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let agent = try #require(session.agent(id: 12))
        #expect(session.sessionsSharingFolder(with: agent).isEmpty)

        try await session.removeSession(agent, withFolder: true, timeout: .seconds(2))

        #expect(await fake.calls.contains("DELETE /v1/workspaces/9"))
        #expect(session.agent(id: 12) == nil)
        #expect(await eventually { session.workspace(id: 9) == nil })
    }

    @Test func aFolderWithUncommittedChangesStays() async throws {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        // Porto has two sessions; take one away so the other is the last.
        try await session.removeSession(try #require(session.agent(id: 14)), withFolder: false)
        let agent = try #require(session.agent(id: 8))

        await #expect(throws: CosError.conflict("porto has uncommitted changes. Commit or discard them first.")) {
            try await session.removeSession(agent, withFolder: true, timeout: .seconds(2))
        }
        // It was stopped on the way, and stays, as does its folder.
        #expect(session.agent(id: 8)?.state.isAlive == false)
        #expect(session.workspace(id: 4) != nil)

        // Unticking removes just the session.
        try await session.removeSession(agent, withFolder: false)
        #expect(session.agent(id: 8) == nil)
    }

    @Test func aRemovedSessionsScreensGo() {
        let navigator = Navigator()
        navigator.chiefs = [.chief(projectId: 1), .workspace(id: 3), .session(agentId: 7), .terminal(agentId: 7), .session(agentId: 13)]
        navigator.forget(agentId: 7, workspaceId: nil)
        #expect(navigator.chiefs == [.chief(projectId: 1), .workspace(id: 3), .session(agentId: 13)])
        navigator.forget(agentId: 13, workspaceId: 3)
        #expect(navigator.chiefs == [.chief(projectId: 1)])
    }
}
