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
        #expect(session.projects.map(\.name) == ["atlas", "harbor"])
        #expect(session.staffs.keys.sorted() == [1, 2])
        // The chief's approval (decided_by route "chief") is not yours.
        #expect(session.waiting.map(\.id) == ["approval-12", "question-5", "plan-4", "trust-12", "login-11"])
        #expect(session.waitingCount == 5)
    }

    @Test func decidingAnApprovalTakesItOffTheList() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let approval = try #require(session.approvals.first { $0.id == 12 })

        try await session.decide(approval, .allow)

        #expect(!session.waiting.contains { $0.id == "approval-12" })
        #expect(await eventually { session.agent(id: 8)?.state == .working })
    }

    @Test func approvingAPlanStartsTheRun() async throws {
        let session = await liveSession()
        defer { session.stop() }

        let board = try await session.approvePlan(runId: 4)

        #expect(board.run.state == .running)
        #expect(session.run(id: 4)?.state == .running)
        #expect(!session.waiting.contains { $0.id == "plan-4" })
    }

    @Test func answeringAQuestionClosesIt() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let question = try #require(session.questions.first)

        try await session.answer(question, "  At startup ")

        #expect(await eventually { session.questions.isEmpty })
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
        _ = await eventually { session.waitingCount == 5 }

        await fake.setAgentState(7, .needsTrust)
        _ = await eventually { session.waitingCount == 6 }
        try? await Task.sleep(for: .milliseconds(100))

        // What already waited at launch isn't announced; the new one is.
        let notified = await notes.notified
        #expect(notified.map(\.kind) == [.trust])
        #expect(notified.first?.title.contains("Fix the session timeout") == true)
        #expect(await notes.badges.last == 6)
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
        #expect(session.projects.count == 2)
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
