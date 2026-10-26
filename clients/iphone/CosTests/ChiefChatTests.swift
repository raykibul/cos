import CosKit
import Foundation
import Testing
@testable import Cos

@MainActor
@Suite struct ChiefChatTests {
    @Test func readsTheTranscriptAsRows() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)

        await chat.load()

        #expect(chat.error == nil)
        #expect(chat.agentId == 40)
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
        let chat = Chat(projectId: 1, session: session)
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
        let chat = Chat(projectId: 1, session: session)
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
        let chat = Chat(projectId: 1, session: session)
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
        #expect(Chat.describe(ToolCall(name: "Bash", summary: "cargo test -p cos")) == "cargo test -p cos")
        #expect(Chat.describe(ToolCall(name: "Read", summary: "src/app.ts")) == "Read src/app.ts")
        #expect(Chat.describe(ToolCall(name: "mcp__cos__agent_spawn", summary: "Fix it")) == "Started an agent")
        #expect(Chat.describe(ToolCall(name: "TodoWrite")) == "TodoWrite")
    }

    @Test func theFirstMessageStartsTheChief() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(projectId: 2, session: session)
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
        let chat = Chat(projectId: 1, session: session)
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
        let chat = Chat(projectId: 1, session: session)
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
        let chat = Chat(projectId: 1, session: session)

        let waiting = chat.agentsWaiting
        #expect(waiting.map(\.agent.id) == [8])
        #expect(waiting.first?.approval?.id == 12)
    }

    // MARK: Attachments

    @Test func aFileUploadsWhenPickedAndGoesWithTheMessage() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)
        await chat.load()
        #expect(!chat.canSend)

        await chat.attach(name: "crash notes.txt", contentType: "text/plain", data: Data("panic at main.rs:12".utf8))

        #expect(chat.files.map(\.name) == ["crash-notes.txt"])
        #expect(chat.files.first?.attachment != nil)
        // A file alone is enough to send.
        #expect(chat.canSend)
        await chat.send()

        #expect(chat.files.isEmpty)
        #expect(chat.pending.isEmpty)
        guard case .user(let message, _) = chat.rows.last else {
            Issue.record("the last row is your message: \(String(describing: chat.rows.last))")
            return
        }
        #expect(message.text.isEmpty)
        #expect(message.attachments.map(\.name) == ["crash-notes.txt"])
        #expect(message.attachments.first?.sizeLabel == "19 B")
        #expect(await fake.calls.contains("POST /v1/projects/1/staff/attachments"))
    }

    @Test func aPictureYouSentShowsInTheChat() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)
        await chat.load()

        guard case .user(let first, _) = chat.rows.first else {
            Issue.record("the chat starts with what you asked: \(chat.rows)")
            return
        }
        #expect(first.attachments.map(\.name) == ["login-expired.png", "auth.log"])
        let picture = await chat.image(first.attachments[0])
        #expect(picture == FakeData.loginShot)
        // Not a picture: nothing to fetch.
        #expect(await chat.image(first.attachments[1]) == nil)
    }

    @Test func aFileTooLargeIsRefusedOnTheComposer() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)
        await chat.load()

        await chat.attach(name: "dump.bin", contentType: "application/octet-stream", data: Data(count: Attachment.maxBytes + 1))

        guard case .failed(let reason) = chat.files.first?.state else {
            Issue.record("the file was refused: \(chat.files)")
            return
        }
        #expect(reason.contains("32 MB"))
        #expect(!chat.canSend)
        chat.removeFile(chat.files[0].id)
        #expect(chat.files.isEmpty)
    }

    @Test func aFailedMessageKeepsItsFilesForRetry() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)
        await chat.load()
        await chat.attach(name: "trace.log", contentType: "text/plain", data: Data("boom".utf8))
        chat.draft = "Look at this"

        await fake.setRefusal(.deviceDenied)
        await chat.send()
        #expect(chat.pending.first?.attachments.map(\.name) == ["trace.log"])

        await fake.setRefusal(nil)
        await chat.retry(chat.pending[0])
        #expect(chat.pending.isEmpty)
        guard case .user(let message, _) = chat.rows.last else {
            Issue.record("the last row is your message: \(String(describing: chat.rows.last))")
            return
        }
        #expect(message.text == "Look at this")
        #expect(message.attachments.map(\.name) == ["trace.log"])
    }
}

@MainActor
@Suite struct SessionChatTests {
    @Test func readsASessionsTranscriptWithTheChiefsPromptFirst() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(agentId: 7, session: session)

        await chat.load()

        #expect(!chat.isChief)
        #expect(chat.agentId == 7)
        #expect(chat.projectId == 1)
        #expect(chat.staff == nil)
        guard case .fromChief(_, let prompt) = chat.rows.first else {
            Issue.record("the chat starts with the chief's prompt: \(String(describing: chat.rows.first))")
            return
        }
        #expect(prompt.hasPrefix("Sessions expire after five minutes"))
        #expect(!prompt.contains("[cos]"))
        #expect(chat.rows.contains { if case .tool = $0 { true } else { false } })
        // It's working: the chat shows it typing, with its latest tool call.
        #expect(chat.isTyping)
        #expect(chat.activity?.text == "npm test")
    }

    @Test func yourOwnSessionsPromptIsYourMessage() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(agentId: 13, session: session)
        await chat.load()

        guard case .user(let message, let receipt) = chat.rows.first else {
            Issue.record("your prompt is your message: \(String(describing: chat.rows.first))")
            return
        }
        #expect(message.text.hasPrefix("Draft the release notes"))
        #expect(receipt == .seen)
    }

    @Test func aMessageTypesIntoTheSession() async {
        let session = await liveSession(FakeCos(replyDelay: .milliseconds(50)))
        defer { session.stop() }
        let chat = Chat(agentId: 13, session: session)
        await chat.load()

        chat.draft = "Mention the session fix too."
        await chat.send()

        #expect(chat.pending.isEmpty)
        guard case .user(let message, _) = chat.rows.last else {
            Issue.record("the last row is your message: \(String(describing: chat.rows.last))")
            return
        }
        #expect(message.text == "Mention the session fix too.")
        // Its answer arrives live, from the transcript.
        let answered = await eventually {
            Task { await chat.read() }
            if case .reply(let reply) = chat.rows.last { return reply.text.hasPrefix("Done.") }
            return false
        }
        #expect(answered)
    }

    @Test func aStoppedSessionIsResumedByAMessage() async {
        let fake = FakeCos(replyDelay: nil)
        let session = await liveSession(fake)
        defer { session.stop() }
        let chat = Chat(agentId: 14, session: session)
        await chat.load()
        #expect(chat.placeholder == "Resume it with a message")

        chat.draft = "Write up the spike"
        await chat.send()

        let calls = await fake.calls
        #expect(calls.contains("POST /v1/agents/14/resume"))
        #expect(calls.last { $0.hasPrefix("POST /v1/agents/14/") } == "POST /v1/agents/14/input")
        #expect(chat.pending.isEmpty)
    }

    @Test func theComposerWaitsWhileAPromptWouldCatchTheText() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(agentId: 8, session: session)
        await chat.load()

        #expect(chat.blocked == "Answer the request above first")
        chat.draft = "hello"
        #expect(!chat.canSend)
        #expect(chat.agentsWaiting.map(\.agent.id) == [8])
        #expect(chat.agentsWaiting.first?.approval?.id == 12)

        let trust = Chat(agentId: 12, session: session)
        #expect(trust.blocked == "Trust the folder first")
        let login = Chat(agentId: 11, session: session)
        #expect(login.blocked == "Log in to Claude first")
    }
}

@MainActor
@Suite struct ReplyChatTests {
    @Test func aReplyCarriesItsQuoteAndShowsIt() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(projectId: 1, session: session)
        await chat.load()
        let theirs = try #require(chat.messages.last { $0.role == .assistant && $0.kind == .text })

        chat.reply(to: theirs)
        #expect(chat.replyLabel == "Replying to the Chief of Staff")
        chat.draft = "Allow it."
        await chat.send()

        #expect(chat.replyingTo == nil)
        guard case .user(let message, _) = chat.rows.last else {
            Issue.record("the last row is your reply: \(String(describing: chat.rows.last))")
            return
        }
        let parsed = try #require(ChatReply.parse(message.text))
        #expect(parsed.reply.from == .them)
        #expect(parsed.reply.text.hasPrefix("Good news from **lisbon**"))
        #expect(parsed.body == "Allow it.")

        // Replying to that reply quotes what you wrote, not its quote.
        chat.reply(to: message)
        #expect(chat.replyingTo == ChatReply(from: .me, text: "Allow it."))
        #expect(chat.replyLabel == "Replying to yourself")
    }

    @Test func inASessionTheyAreClaude() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = Chat(agentId: 13, session: session)
        await chat.load()
        if let last = chat.messages.last { chat.reply(to: last) }
        #expect(chat.replyLabel == "Replying to Claude")
    }
}
