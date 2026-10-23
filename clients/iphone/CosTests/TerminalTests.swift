import CosKit
import Foundation
import Testing
@testable import Cos

/// What a terminal view would have been told.
@MainActor
final class RecordingSink: TerminalSink {
    var resets: [CosKit.TerminalSize] = []
    var resizes: [CosKit.TerminalSize] = []
    var output: [UInt8] = []

    func terminalReset(size: CosKit.TerminalSize) {
        resets.append(size)
        output = []
    }

    func terminalFeed(_ bytes: [UInt8]) { output += bytes }
    func terminalResize(size: CosKit.TerminalSize) { resizes.append(size) }

    var text: String { String(decoding: output, as: UTF8.self) }
}

@MainActor
@Suite struct TerminalTests {
    @Test func theFakeStreamsAScreenEchoesKeysAndFitsTheView() async throws {
        let fake = FakeCos(replyDelay: nil)
        let terminal = TerminalSession(agentId: 7, source: fake, control: fake, backoff: quickBackoff, resizeDelay: .milliseconds(10))
        let sink = RecordingSink()
        terminal.sink = sink
        terminal.start()
        defer { terminal.stop() }

        #expect(await eventually { terminal.phase == .live })
        #expect(sink.resets.first?.cols == 48)
        #expect(sink.text.contains("Fix the session timeout"))

        // The view fits 40 columns: the agent is resized to match.
        terminal.viewSizeChanged(CosKit.TerminalSize(rows: 30, cols: 40))
        #expect(await eventually { terminal.ptySize == CosKit.TerminalSize(rows: 30, cols: 40) })
        #expect(sink.resizes.last == CosKit.TerminalSize(rows: 30, cols: 40))

        terminal.send(Array("y\r".utf8))
        #expect(await eventually { sink.text.hasSuffix("y\r\n") })
        #expect(await fake.keys[7] == Array("y\r".utf8))
    }

    @Test func peersAreTheChiefThenItsLiveSessions() async {
        let session = await liveSession()
        defer { session.stop() }

        #expect(session.projectId(ofAgent: 40) == 1)
        #expect(session.projectId(ofAgent: 8) == 1)
        #expect(session.projectId(ofAgent: 12) == 2)
        let peers = session.terminalPeers(of: 7).map(\.id)
        #expect(peers.first == 40)
        #expect(Set(peers).isSuperset(of: [7, 8, 13]))
        // An exited session is left out, unless it's the one open.
        #expect(!peers.contains(14))
        #expect(session.terminalPeers(of: 14).contains(14))
    }

    @Test func aTerminalOpensFromTheChatsTab() {
        let navigator = Navigator()
        navigator.apply(screen: "terminal")
        #expect(navigator.chiefs == [.chief(projectId: 1), .terminal(agentId: 7)])
        navigator.chiefs = []
        navigator.open(.terminal(agentId: 40))
        #expect(navigator.tab == .chiefs)
        #expect(navigator.chiefs == [.terminal(agentId: 40)])
    }
}
