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

    @Test func sessionsItStartedAndAgentsThatFinishedAreCards() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()

        let spawned = chat.rows.compactMap { row -> ToolCall? in
            if case .spawned(_, let tool) = row { tool } else { nil }
        }
        #expect(spawned.map(\.agentId) == [7, 8])
        #expect(spawned.compactMap { chat.spawnedAgent($0)?.title } == ["Fix the session timeout", "Add left-pad to the formatter"])
        // From an older daemon, without the id: matched by title.
        #expect(chat.spawnedAgent(ToolCall(name: "mcp__cos__agent_spawn", summary: "Fix the session timeout"))?.id == 7)
        #expect(chat.spawnedAgent(ToolCall(name: "mcp__cos__agent_spawn", summary: "Something else")) == nil)

        // The task notification is a card, never your message.
        let tasks = chat.rows.compactMap { row -> TaskNotification? in
            if case .task(_, let note) = row { note } else { nil }
        }
        #expect(tasks.map(\.title) == ["Check the 0.2 milestone"])
        #expect(tasks.first?.durationMs == 94_000)
        #expect(!chat.rows.contains { row in
            if case .user(let message, _) = row { message.text.contains("<task-notification>") } else { false }
        })
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
        guard case .user(let message, _) = chat.rows.last else {
            Issue.record("the last row is your message: \(String(describing: chat.rows.last))")
            return
        }
        #expect(message.text == "What's left before 0.2?")
        #expect(chat.rows.count == before + 1)
    }

    @Test func yourMessagesGetTicks() async {
        let session = await liveSession(FakeCos(replyDelay: .milliseconds(300)))
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()

        // The chief answered everything you wrote before: two blue ticks.
        let earlier = chat.rows.compactMap { row -> Receipt? in
            if case .user(_, let receipt) = row { receipt } else { nil }
        }
        #expect(!earlier.isEmpty)
        #expect(earlier.allSatisfy { $0 == .seen })

        await chat.send("Status?")
        // Claude has it and the chief is working on it: typing, and seen.
        _ = await eventually { chat.isTyping }
        #expect(chat.isTyping)
        guard case .user(let message, .seen) = chat.rows.last else {
            Issue.record("your message is seen while the chief works on it: \(String(describing: chat.rows.last))")
            return
        }

        // Its reply comes in: no longer typing, and the info has each time.
        let tick = session.transcriptTicks[40] ?? 0
        _ = await eventually { (session.transcriptTicks[40] ?? 0) > tick + 1 && !chat.isTyping }
        await chat.read()
        #expect(!chat.isTyping)
        let times = chat.receiptTimes(for: message)
        #expect(times.receipt == .seen)
        #expect(times.sentMs != nil)
        #expect(times.seenMs != nil)
        #expect(times.deliveredMs == message.tsMs)
    }

    @Test func aPendingMessageShowsItsTicks() {
        var message = PendingMessage(id: 1, text: "Hi", afterN: 0, state: .sending)
        #expect(message.receipt == .sending)
        message.state = .sent
        #expect(message.receipt == .sent)
        message.state = .failed("no")
        #expect(message.receipt == nil)
    }

    @Test func whatTheChiefIsDoingReadsAsOneLine() {
        #expect(ChiefChat.describe(ToolCall(name: "Bash", summary: "cargo test -p cos")) == "cargo test -p cos")
        #expect(ChiefChat.describe(ToolCall(name: "Read", summary: "src/app.ts")) == "Read src/app.ts")
        #expect(ChiefChat.describe(ToolCall(name: "mcp__cos__agent_spawn", summary: "Fix it")) == "Started an agent")
        #expect(ChiefChat.describe(ToolCall(name: "TodoWrite")) == "TodoWrite")
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
