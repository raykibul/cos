import Foundation
import Testing
@testable import CosKit

/// Payloads captured from the real daemon (`cos daemon`, milestone 3).
enum Samples {
    static let status = #"{"version":"0.1.0","pid":76227,"started_at":"2026-10-21T18:36:19.126787Z","home":"/Users/rakibul.islam/personal/apps/cos/.sandbox/app-home","events":1,"last_seq":1}"#
    static let eventsPage = #"{"events":[{"seq":1,"ts":"2026-10-21T18:36:19.126Z","kind":"daemon.started","data":{"pid":76227,"version":"0.1.0"}},{"seq":2,"ts":"2026-10-21T18:36:31.006Z","kind":"daemon.stopped","data":{"reason":"SIGTERM"}}]}"#
    /// The example in docs/daemon.md.
    static let docsEvent = #"{"seq":3,"ts":"2026-10-21T18:06:01.843Z","kind":"daemon.started","data":{"pid":58448,"version":"0.1.0"}}"#
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.cos().decode(type, from: Data(json.utf8))
}

@Suite struct ModelTests {
    @Test func decodesStatus() throws {
        let status = try decode(Status.self, Samples.status)
        #expect(status.version == "0.1.0")
        #expect(status.pid == 76227)
        #expect(status.home == "/Users/rakibul.islam/personal/apps/cos/.sandbox/app-home")
        #expect(status.events == 1)
        #expect(status.lastSeq == 1)
        #expect(abs(status.startedAt.timeIntervalSince1970 - 1_792_607_779.126787) < 1e-5)
    }

    @Test func decodesEventsPage() throws {
        let page = try decode(EventsPage.self, Samples.eventsPage)
        #expect(page.events.map(\.seq) == [1, 2])
        #expect(page.events[0].kind == EventKind.daemonStarted)
        #expect(page.events[0].data["pid"] == .number(76227))
        #expect(page.events[0].runId == nil)
        #expect(page.events[1].data == .object(["reason": .string("SIGTERM")]))
        #expect(abs(page.events[1].ts.timeIntervalSince1970 - 1_792_607_791.006) < 1e-5)
    }

    @Test func decodesDocsSampleAndRunId() throws {
        let event = try decode(LoggedEvent.self, Samples.docsEvent)
        #expect(event.seq == 3)
        #expect(event.data["version"] == .string("0.1.0"))

        let withRun = try decode(LoggedEvent.self, #"{"seq":9,"ts":"2026-10-21T18:06:01Z","kind":"run.started","run_id":"r1","data":null}"#)
        #expect(withRun.runId == "r1")
        #expect(withRun.data == .null)
    }

    @Test(arguments: [
        // From the Rust test in cos-core/src/events.rs: no fractional seconds.
        ("2026-10-21T16:26:40Z", 1_792_600_000.0),
        ("2026-10-21T16:26:40.5Z", 1_792_600_000.5),
        ("2026-10-21T16:26:40.123Z", 1_792_600_000.123),
        ("2026-10-21T16:26:40.123456Z", 1_792_600_000.123456),
        ("2026-10-21T16:26:40.123456789Z", 1_792_600_000.123456789),
        ("2026-10-21T18:26:40+02:00", 1_792_600_000.0),
    ])
    func parsesRFC3339(text: String, expected: Double) throws {
        let date = try #require(parseRFC3339(text))
        #expect(abs(date.timeIntervalSince1970 - expected) < 1e-5)
    }

    @Test(arguments: ["", "yesterday", "2026-10-21T16:26:40.Z", "2026-10-21"])
    func rejectsBadDates(text: String) {
        #expect(parseRFC3339(text) == nil)
    }

    @Test func aBadDateDoesNotLoseTheEvent() throws {
        // Lenient: the event still counts for resuming the stream.
        let event = try decode(LoggedEvent.self, #"{"seq":1,"ts":"soon","kind":"x","data":{}}"#)
        #expect(event.seq == 1)
        #expect(event.ts == Date(timeIntervalSince1970: 0))
        // Without its seq it is refused, and a page skips it.
        #expect(throws: DecodingError.self) { try decode(LoggedEvent.self, #"{"ts":"soon","kind":"x"}"#) }
        let page = try decode(EventsPage.self, #"{"events":[{"kind":"x"},{"seq":4,"kind":"y"}]}"#)
        #expect(page.events.map(\.seq) == [4])
    }

    @Test func compactData() throws {
        let page = try decode(EventsPage.self, Samples.eventsPage)
        #expect(page.events[0].data.compact == "pid=76227 version=0.1.0")
        #expect(page.events[1].data.compact == "reason=SIGTERM")

        let nested: JSONValue = .object([
            "files": .array([.string("a.rs"), .string("b.rs")]),
            "ok": .bool(true),
            "score": .number(0.5),
            "task": .object(["id": .number(2)]),
        ])
        #expect(nested.compact == #"files=["a.rs","b.rs"] ok=true score=0.5 task={"id":2}"#)
        #expect(JSONValue.null.compact == "null")
    }
}
