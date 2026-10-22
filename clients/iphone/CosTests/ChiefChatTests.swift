import CosKit
import Foundation
import Testing
@testable import Cos

@MainActor
@Suite struct ChiefChatTests {
    @Test func readsTheTranscriptAsRows() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)

        await chat.load()

        #expect(chat.error == nil)
        #expect(chat.chiefAgentId == 40)
        guard case .user = chat.rows.first else {
            Issue.record("the chat starts with what you asked: \(chat.rows)")
            return
        }
        #expect(chat.rows.contains { if case .tool = $0 { true } else { false } })
        #expect(chat.rows.contains { if case .updates(_, let count) = $0 { count == 2 } else { false } })
        #expect(chat.rows.contains { if case .reply = $0 { true } else { false } })
    }

    @Test func aSentMessageShowsUntilTheTranscriptHasIt() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()
        let before = chat.rows.count

        chat.draft = "  What's left before 0.2?  "
        await chat.send()

        #expect(chat.draft.isEmpty)
        #expect(chat.pending.isEmpty)
        guard case .user(let message) = chat.rows.last else {
            Issue.record("the last row is your message: \(String(describing: chat.rows.last))")
            return
        }
        #expect(message.text == "What's left before 0.2?")
        #expect(chat.rows.count == before + 1)
    }

    @Test func theFirstMessageStartsTheChief() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 2, session: session)
        await chat.load()
        #expect(chat.staff?.hasStarted == false)
        #expect(chat.rows.isEmpty)

        await chat.send("Look at the billing export")

        #expect(chat.staff?.hasStarted == true)
        #expect(chat.rows.count == 1)
    }

    @Test func theChiefsReplyArrivesLive() async {
        let session = await liveSession(FakeCos(replyDelay: .milliseconds(50)))
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()

        await chat.send("Status?")
        // The view reads again on each `agent.transcript` tick.
        let tick = session.transcriptTicks[40] ?? 0
        _ = await eventually { (session.transcriptTicks[40] ?? 0) > tick + 1 }
        await chat.read()

        guard case .reply(let reply) = chat.rows.last else {
            Issue.record("the chief answered: \(String(describing: chat.rows.last))")
            return
        }
        #expect(reply.text.contains("porto"))
    }

    @Test func aFailedSendCanBeRetriedOrDiscarded() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()

        await fake.setRefusal(.deviceDenied)
        await chat.send("Hello")
        guard case .failed = chat.pending.first?.state else {
            Issue.record("the message failed: \(chat.pending)")
            return
        }
        #expect(chat.rows.last.map { if case .pending = $0 { true } else { false } } == true)

        await fake.setRefusal(nil)
        await chat.retry(chat.pending[0])
        #expect(chat.pending.isEmpty)

        await fake.setRefusal(.deviceDenied)
        await chat.send("Again")
        chat.discard(chat.pending[0])
        #expect(chat.pending.isEmpty)
    }

    @Test func agentsTheChiefStartedThatWaitOnYou() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)

        let waiting = chat.agentsWaiting
        #expect(waiting.map(\.agent.id) == [8])
        #expect(waiting.first?.approval?.id == 12)
    }
}
