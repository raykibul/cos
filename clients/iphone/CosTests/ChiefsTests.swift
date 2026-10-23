import CosKit
import Foundation
import Testing
@testable import Cos

@MainActor
@Suite struct ChiefsTests {
    @Test func listsEveryChiefLatestFirst() async {
        let session = await liveSession()
        defer { session.stop() }

        let chiefs = session.chiefs

        // harbor's chief never started, so it goes last.
        #expect(chiefs.map(\.project.name) == ["atlas", "opal", "harbor"])
        // atlas: the approval, the run's question and the plan; harbor: a
        // trust dialog and a login.
        #expect(chiefs.map(\.waiting) == [3, 0, 2])
        #expect(chiefs.map(\.waiting).reduce(0, +) == session.waitingCount)
        #expect(chiefs[0].preview == "1 session working…")
        #expect(chiefs[0].previewIsActivity)
        #expect(chiefs[1].preview == "The nightly dependency check passed. Nothing to update.")
        #expect(!chiefs[1].previewIsActivity)
        #expect(chiefs[2].preview.hasPrefix("Not started"))
        #expect(!chiefs[2].hasStarted)
    }

    @Test func filtersByWhatNeedsYouAndWhatWorks() async {
        let session = await liveSession()
        defer { session.stop() }

        let chiefs = session.chiefs

        #expect(chiefs.filter(ChiefFilter.all.includes).count == 3)
        #expect(chiefs.filter(ChiefFilter.needsYou.includes).map(\.project.name) == ["atlas", "harbor"])
        #expect(chiefs.filter(ChiefFilter.working.includes).map(\.project.name) == ["atlas"])
    }

    @Test func eachWaitingItemBelongsToItsProject() async {
        let session = await liveSession()
        defer { session.stop() }

        let projects = Dictionary(uniqueKeysWithValues: session.waiting.map { ($0.id, session.projectId(of: $0)) })

        #expect(projects["approval-12"] == 1)
        #expect(projects["question-5"] == 1)
        #expect(projects["plan-4"] == 1)
        #expect(projects["trust-12"] == 2)
        #expect(projects["login-11"] == 2)
    }

    @Test func aProjectsSessionsAndRuns() async {
        let session = await liveSession()
        defer { session.stop() }

        let sessions = session.agents(inProject: 1).map(\.id)
        #expect(Set(sessions).isSuperset(of: [7, 8, 13, 14]))
        #expect(!sessions.contains(40))
        #expect(!sessions.contains(11))
        #expect(Set(session.runs(inProject: 1).map(\.id)) == [2, 3, 4])
        #expect(session.runs(inProject: 2).map(\.id) == [1])
        #expect(session.agents(inProject: 99).isEmpty)
    }

    @Test func aChatOpenedOnThisPhoneShowsWhatIsNew() {
        let defaults = scratchDefaults()
        let host = UUID()
        let marks = ReadMarks(hostId: host, defaults: defaults)

        // Never opened here: not marked, or every chat would be.
        #expect(!marks.isUnread(projectId: 1, activityMs: 200))

        marks.markRead(projectId: 1, at: 100)
        #expect(marks.isUnread(projectId: 1, activityMs: 200))
        #expect(!marks.isUnread(projectId: 1, activityMs: 100))
        #expect(!marks.isUnread(projectId: 1, activityMs: nil))

        marks.markRead(projectId: 1, at: 300)
        marks.markRead(projectId: 1, at: 250)
        #expect(!marks.isUnread(projectId: 1, activityMs: 200))

        // Kept per host, across launches.
        #expect(ReadMarks(hostId: host, defaults: defaults).marks == [1: 300])
        #expect(ReadMarks(hostId: UUID(), defaults: defaults).marks.isEmpty)
    }

    @Test func previewsAreOnePlainLine() {
        let markdown = """
        ## Done

        **porto** is waiting on `left-pad`.

        ```
        npm install left-pad
        ```
        - one more thing
        """
        #expect(ChiefSummary.plain(markdown) == "Done porto is waiting on left-pad. one more thing")
    }

    @Test func routesOpenInTheirTabs() {
        let navigator = Navigator()

        navigator.open(.chief(projectId: 1))
        navigator.open(.chiefInfo(projectId: 1, tab: .sessions))
        #expect(navigator.tab == .chiefs)
        #expect(navigator.chiefs == [.chief(projectId: 1), .chiefInfo(projectId: 1, tab: .sessions)])

        navigator.open(.run(id: 3))
        #expect(navigator.tab == .runs)
        #expect(navigator.runs == [.run(id: 3)])

        navigator.apply(screen: "waiting")
        #expect(navigator.tab == .chiefs)
        #expect(navigator.chiefs == [.waiting])
    }
}
