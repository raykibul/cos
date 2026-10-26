import CosKit
import Foundation
import Observation

/// A chief's identity, brief and memory notes, for its Identity & memory
/// page ("Memory notes" in docs/chief-of-staff.md). The identity and brief
/// come with the chief's record; the notes are read while the page shows
/// and again on `staff.changed`. Every change is used from the chief's next
/// start or new session.
@MainActor
@Observable
final class ChiefMemory {
    let projectId: Int
    @ObservationIgnored private let session: HostSession

    /// `nil` until they first load.
    private(set) var notes: [StaffNote]?
    /// `false` on a daemon older than notes (404).
    private(set) var supported = true
    private(set) var error: String?
    /// A note just deleted, which Undo writes back.
    private(set) var removed: StaffNote?

    init(projectId: Int, session: HostSession) {
        self.projectId = projectId
        self.session = session
    }

    var staff: Staff? { session.staffs[projectId] }
    var project: Project? { session.project(id: projectId) }
    var isFull: Bool { (notes?.count ?? 0) >= StaffNote.maxNotes }

    func load() async {
        do {
            notes = try await session.service.staffMemory(projectId: projectId)
            supported = true
            error = nil
        } catch CosError.notFound {
            supported = false
        } catch {
            if notes == nil { self.error = error.localizedDescription }
        }
    }

    /// Saves the identity; the chief reads it from its next start.
    func saveIdentity(_ text: String) async throws {
        let staff = try await session.service.updateStaff(projectId: projectId, brief: nil, identity: text)
        session.setStaff(staff)
        try Self.check(staff.identity, wanted: text, what: "identities")
    }

    /// Saves the brief; the chief reads it from its next start.
    func saveBrief(_ text: String) async throws {
        let staff = try await session.service.updateStaff(projectId: projectId, brief: text, identity: nil)
        session.setStaff(staff)
        try Self.check(staff.brief, wanted: text, what: "briefs")
    }

    /// A daemon older than the field ignores it and answers the old text.
    private static func check(_ saved: String, wanted: String, what: String) throws {
        if saved != wanted.trimmingCharacters(in: .whitespacesAndNewlines) {
            throw CosError.http(status: 400, message: "This cos daemon is older than editable \(what), so it kept the old one. Update cos and restart the daemon.")
        }
    }

    /// Creates or replaces a note, with its description on one line.
    func save(name: String, description: String, body: String) async throws {
        let note = try await session.service.saveStaffNote(
            projectId: projectId, name: name, description: StaffNote.oneLine(description),
            body: body.trimmingCharacters(in: .whitespacesAndNewlines)
        )
        var list = (notes ?? []).filter { $0.name != note.name }
        list.append(note)
        notes = list.sorted { $0.name < $1.name }
        removed = nil
    }

    func delete(_ note: StaffNote) async {
        do {
            try await session.service.deleteStaffNote(projectId: projectId, name: note.name)
        } catch CosError.notFound {
            // Gone already.
        } catch {
            self.error = error.localizedDescription
            return
        }
        notes?.removeAll { $0.name == note.name }
        removed = note
        error = nil
    }

    /// Writes the note just deleted back.
    func undo() async {
        guard let note = removed else { return }
        removed = nil
        do {
            try await save(name: note.name, description: note.description, body: note.body)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func dismissUndo() {
        removed = nil
    }

    /// A fresh Claude session for the chief, with everything here as it is now.
    func newSession() async throws {
        let staff = try await session.service.newStaffSession(projectId: projectId)
        session.setStaff(staff)
        session.refreshStaff()
    }
}

/// How many words: runs of anything but white space.
func wordCount(_ text: String) -> Int {
    text.split(whereSeparator: \.isWhitespace).count
}
