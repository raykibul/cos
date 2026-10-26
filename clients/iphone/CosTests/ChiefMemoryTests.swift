import CosKit
import Foundation
import Testing
@testable import Cos

@MainActor
@Suite struct ChiefMemoryTests {
    @Test func loadsTheNotesByName() async {
        let session = await liveSession()
        defer { session.stop() }
        let memory = ChiefMemory(projectId: 1, session: session)

        await memory.load()

        #expect(memory.supported)
        #expect(memory.notes?.map(\.name) == ["release-process", "test-suite", "user-preferences"])
        #expect(memory.staff?.identity.hasPrefix("You are Ada") == true)
    }

    @Test func savesTheBriefAndIdentity() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let memory = ChiefMemory(projectId: 1, session: session)

        try await memory.saveBrief("  Ship 0.3.  ")
        try await memory.saveIdentity("You are Grace.")

        #expect(session.staffs[1]?.brief == "Ship 0.3.")
        #expect(session.staffs[1]?.identity == "You are Grace.")
        await #expect(throws: CosError.self) {
            try await memory.saveBrief(String(repeating: "x", count: Staff.maxBriefBytes + 1))
        }
    }

    @Test func addsEditsDeletesAndUndoes() async throws {
        let session = await liveSession()
        defer { session.stop() }
        let memory = ChiefMemory(projectId: 1, session: session)
        await memory.load()

        try await memory.save(name: "deploys", description: "How we\n deploy", body: "  Fly, from main.  ")
        let added = try #require(memory.notes?.first { $0.name == "deploys" })
        #expect(added.description == "How we deploy")
        #expect(added.body == "Fly, from main.")

        await memory.delete(added)
        #expect(memory.notes?.contains { $0.name == "deploys" } == false)
        #expect(memory.removed?.name == "deploys")

        await memory.undo()
        #expect(memory.removed == nil)
        #expect(memory.notes?.contains { $0.name == "deploys" } == true)
    }

    @Test func aNewSessionStartsTheChatOver() async {
        let session = await liveSession()
        defer { session.stop() }
        let chat = ChiefChat(projectId: 1, session: session)
        await chat.load()
        #expect(!chat.rows.isEmpty)

        try? await ChiefMemory(projectId: 1, session: session).newSession()
        await chat.read()

        #expect(chat.rows.isEmpty)
    }
}
