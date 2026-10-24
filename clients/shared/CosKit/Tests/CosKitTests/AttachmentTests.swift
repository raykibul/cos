import Foundation
import Testing
@testable import CosKit

@Suite struct AttachmentTests {
    @Test func uploadsRawBytesAndNamesThemInMessages() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/projects/1/staff/attachments?name=Screen%20Shot%201.png"):
                .json(201, #"{"name":"Screen-Shot-1.png","path":"/w/chief/.cos/attachments/Screen-Shot-1.png","size":4,"content_type":"image/png"}"#)
            case ("POST", "/v1/projects/1/staff/message"), ("POST", "/v1/agents/7/input"): .empty(202)
            case ("GET", "/v1/agents/14/attachments/Screen-Shot-1.png"): .json(200, "png!")
            default: .json(404, #"{"error":"unknown route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        let saved = try await client.uploadAttachment(
            to: .staff(projectId: 1), name: "Screen Shot 1.png", contentType: "image/png", data: Data("png!".utf8)
        )
        #expect(saved == Attachment(
            name: "Screen-Shot-1.png", path: "/w/chief/.cos/attachments/Screen-Shot-1.png", size: 4, contentType: "image/png"
        ))
        let upload = try #require(server.requests.first { $0.path.contains("/attachments?") })
        #expect(upload.body == Array("png!".utf8))
        #expect(upload.headers["content-type"] == "image/png")

        try await client.messageStaff(projectId: 1, text: "", attachments: [saved.name])
        try await client.sendInput(agentId: 7, text: "see", attachments: ["a.txt"])
        #expect(try await client.attachmentData(agentId: 14, name: saved.name) == Data("png!".utf8))

        let message = try #require(server.requests.first { $0.path.hasSuffix("/staff/message") })
        #expect(message.value == .object(["text": .string(""), "attachments": .array([.string("Screen-Shot-1.png")])]))
        let input = try #require(server.requests.first { $0.path.hasSuffix("/input") })
        #expect(input.value == .object(["text": .string("see"), "attachments": .array([.string("a.txt")])]))
    }

    @Test func refusesFilesOverTheLimitBeforeSending() async throws {
        let client = CosClient(socketPath: "/nonexistent.sock")
        await #expect(throws: CosError.self) {
            try await client.uploadAttachment(
                to: .agent(id: 1), name: "big.bin", contentType: "application/octet-stream",
                data: Data(count: Attachment.maxBytes + 1)
            )
        }
    }

    @Test func transcriptsGiveTheFilesBack() throws {
        let json = #"{"n":1,"role":"user","kind":"text","text":"Fix this","ts_ms":0,"attachments":[{"name":"shot.png","path":"/w/.cos/attachments/shot.png","content_type":"image/png","size_label":"245 KB"},{"path":"/w/.cos/attachments/notes.md"},7]}"#
        let message = try JSONDecoder.cos().decode(TranscriptMessage.self, from: Data(json.utf8))
        #expect(message.text == "Fix this")
        #expect(message.attachments.map(\.name) == ["shot.png", "notes.md"])
        #expect(message.attachments[0].isImage)
        #expect(message.attachments[1].contentType == "application/octet-stream")
        let plain = try JSONDecoder.cos().decode(TranscriptMessage.self, from: Data(#"{"n":2,"role":"user","text":"hi"}"#.utf8))
        #expect(plain.attachments.isEmpty)
    }

    @Test func sizesReadLikeTheDaemonWritesThem() {
        #expect(Attachment.sizeLabel(812) == "812 B")
        #expect(Attachment.sizeLabel(250_000) == "245 KB")
        #expect(Attachment.sizeLabel(3_500_000) == "3.3 MB")
    }
}
