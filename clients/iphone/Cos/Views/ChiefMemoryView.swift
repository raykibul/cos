import CosKit
import SwiftUI

/// A chief's Identity & memory page: who it is (written by you, which it
/// can't change), its brief, and its memory notes to read, add, edit and
/// delete. Changes reach the chief from its next start, so the page offers
/// New session once it has started. Reached from the chat's ⋯ menu, the
/// info page and, before it starts, the chat's "Give it a brief first" card.
struct ChiefMemoryView: View {
    var session: HostSession
    @State private var memory: ChiefMemory
    @State private var editing: NoteEditing?
    @State private var confirmNewSession = false
    @State private var error: String?
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss

    init(session: HostSession, projectId: Int) {
        self.session = session
        _memory = State(initialValue: ChiefMemory(projectId: projectId, session: session))
    }

    var body: some View {
        let name = memory.project?.name ?? "this project"
        List {
            Section {
                Text("Who \(name)'s Chief of Staff is, and what it carries from one session to the next. It reads changes on its next start, or now if you start a new session.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 4, leading: 4, bottom: 4, trailing: 4))
            }

            ProseSection(
                title: "Identity", locked: true, saved: memory.staff?.identity ?? "", maxBytes: Staff.maxIdentityBytes,
                words: Staff.identityWords, minHeight: 180,
                placeholder: "Who is your Chief of Staff? For example: You are Ada, the Chief of Staff for \(name). You're calm, direct and careful. Talk to me like a senior engineer: short answers, no fluff. Never push to main.",
                note: "Who it is and how it works. It's at the top of the chief's prompt, it can't change it, and it lasts across sessions.",
                savedNote: "Saved. It takes this identity from its next start or new session."
            ) { try await memory.saveIdentity($0) }

            ProseSection(
                title: "Brief", locked: false, saved: memory.staff?.brief ?? "", maxBytes: Staff.maxBriefBytes, words: nil, minHeight: 110,
                placeholder: "What should it be working on, for whom, and what's open? For example: Ship 0.2: the login fix, left-pad and the release notes.",
                note: "It keeps this up to date itself, so it may rewrite what you put here.",
                savedNote: "Saved. It reads the new brief on its next start or new session."
            ) { try await memory.saveBrief($0) }

            notesSection

            if memory.staff?.hasStarted == true {
                Section {
                    Button("New session", systemImage: "plus.bubble") { confirmNewSession = true }
                } footer: {
                    Text("Starts a fresh conversation with the identity, brief and notes as they are now.")
                }
            }
        }
        .scrollDismissesKeyboard(.interactively)
        .pageBackground()
        .navigationTitle("Identity & memory")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Add note", systemImage: "plus") { editing = .add }
                    .disabled(!memory.supported || memory.notes == nil || memory.isFull)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let removed = memory.removed {
                UndoBar(text: "Deleted \(removed.name)") {
                    Task { await memory.undo() }
                }
                .task(id: removed.name) {
                    try? await Task.sleep(for: .seconds(6))
                    if memory.removed?.name == removed.name { memory.dismissUndo() }
                }
            }
        }
        .sheet(item: $editing) { editing in
            NoteSheet(memory: memory, editing: editing)
        }
        .confirmationDialog("Start a new session?", isPresented: $confirmNewSession, titleVisibility: .visible) {
            Button("New session") { newSession() }
        } message: {
            Text("The chief drops this conversation and starts afresh with its identity, brief and notes. The sessions it started stay its own.")
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
        .refreshable {
            await session.refresh()
            await memory.load()
        }
        .task { await memory.load() }
        .onChange(of: session.staffChanges[memory.projectId]) {
            Task { await memory.load() }
        }
    }

    @ViewBuilder
    private var notesSection: some View {
        Section {
            if !memory.supported {
                Text("This cos daemon is older than memory notes. Update cos and restart the daemon.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
            } else if let notes = memory.notes {
                if notes.isEmpty {
                    Text("No notes yet. The chief writes what lasts here (what you prefer, decisions and why, how the project works), and so can you.")
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink3)
                }
                ForEach(notes) { note in
                    NoteRow(note: note)
                        .swipeActions {
                            Button("Delete", systemImage: "trash", role: .destructive) { Task { await memory.delete(note) } }
                            Button("Edit", systemImage: "pencil") { editing = .edit(note) }
                                .tint(Palette.ink2)
                        }
                        .contextMenu {
                            Button("Edit", systemImage: "pencil") { editing = .edit(note) }
                            Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = note.body }
                            Button("Delete", systemImage: "trash", role: .destructive) { Task { await memory.delete(note) } }
                        }
                }
                if !memory.isFull {
                    Button("Add note", systemImage: "plus") { editing = .add }
                }
            } else if let error = memory.error {
                Text(error).font(.footnote).foregroundStyle(Palette.red)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
            if let error = memory.error, memory.notes != nil {
                Text(error).font(.footnote).foregroundStyle(Palette.red)
            }
        } header: {
            Text("Memory notes")
        } footer: {
            if memory.supported, let notes = memory.notes {
                Text(memory.isFull
                    ? "\(notes.count) of \(StaffNote.maxNotes) notes: delete one to add another."
                    : "Only each note's name and description go in its prompt; it reads a note when it needs it.")
            }
        }
    }

    /// Back to the chat if it's the screen below, else to it.
    private func openChat() {
        if navigator.chiefs.dropLast().last == Route.chief(projectId: memory.projectId) {
            dismiss()
        } else {
            navigator.open(.chief(projectId: memory.projectId))
        }
    }

    private func newSession() {
        Task {
            do {
                try await memory.newSession()
                openChat()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// A text the chief reads from its prompt, with a size limit and Save.
private struct ProseSection: View {
    var title: String
    /// Marked "Only you can change this".
    var locked: Bool
    var saved: String
    var maxBytes: Int
    /// Counts words against this many, a guide rather than a limit.
    var words: Int?
    var minHeight: CGFloat
    var placeholder: String
    var note: String
    var savedNote: String
    var save: (String) async throws -> Void

    @State private var draft: String?
    @State private var busy = false
    @State private var error: String?
    @State private var justSaved = false

    var body: some View {
        let text = draft ?? saved
        let bytes = text.trimmingCharacters(in: .whitespacesAndNewlines).utf8.count
        let tooLong = bytes > maxBytes
        let dirty = draft != nil && draft?.trimmingCharacters(in: .whitespacesAndNewlines) != saved
        Section {
            TextEditor(text: Binding(get: { draft ?? saved }, set: { draft = $0; justSaved = false }))
                .font(.body)
                .frame(minHeight: minHeight)
                .scrollContentBackground(.hidden)
                .overlay(alignment: .topLeading) {
                    if text.isEmpty {
                        Text(placeholder)
                            .font(.body)
                            .foregroundStyle(Palette.ink3)
                            .padding(.top, 8)
                            .padding(.leading, 5)
                            .allowsHitTesting(false)
                    }
                }
                .accessibilityLabel(title)
            if dirty || busy {
                HStack {
                    Button("Revert") {
                        draft = nil
                        error = nil
                    }
                    .buttonStyle(QuietButtonStyle())
                    Spacer()
                    Button(busy ? "Saving…" : "Save \(title.lowercased())") {
                        Task { await submit(text) }
                    }
                    .buttonStyle(ClaudeButtonStyle())
                    .disabled(tooLong || busy)
                }
            }
        } header: {
            HStack(spacing: 8) {
                Text(title)
                if locked {
                    Label("Only you can change this", systemImage: "lock.fill")
                        .labelStyle(.tight)
                        .font(.caption2.weight(.medium))
                        .textCase(nil)
                        .foregroundStyle(Palette.ink2)
                }
            }
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if let error {
                    Text(error).foregroundStyle(Palette.red)
                } else if tooLong {
                    Text("Too long: keep it under \(maxBytes.formatted()) bytes.").foregroundStyle(Palette.red)
                } else {
                    Text(justSaved ? savedNote : note)
                }
                Text(counts(text: text, bytes: bytes))
                    .monospacedDigit()
                    .foregroundStyle(tooLong ? Palette.red : Palette.ink3)
            }
        }
    }

    /// "312 / ~500 words · 1,840 / 8,192 bytes", or the bytes alone.
    private func counts(text: String, bytes: Int) -> String {
        let size = "\(bytes.formatted()) / \(maxBytes.formatted()) bytes"
        guard let words else { return size }
        return "\(wordCount(text).formatted()) / ~\(words) words · \(size)"
    }

    private func submit(_ text: String) async {
        busy = true
        defer { busy = false }
        do {
            try await save(text)
            draft = nil
            error = nil
            justSaved = true
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// A note: its name and description, unfolding to its body.
private struct NoteRow: View {
    var note: StaffNote
    @State private var open = false

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            Text(note.body)
                .font(.footnote.monospaced())
                .foregroundStyle(Palette.ink)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 4)
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(note.name)
                    .font(.subheadline.monospaced().weight(.medium))
                    .foregroundStyle(Palette.ink)
                Text(note.description)
                    .font(.footnote)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(open ? nil : 1)
                if note.updatedMs > 0 {
                    Text("Updated \(shortAgo(note.updatedMs)) · \(Attachment.sizeLabel(note.body.utf8.count))")
                        .font(.caption2)
                        .foregroundStyle(Palette.ink3)
                }
            }
            .padding(.vertical, 2)
        }
    }
}

/// Adding a note, or editing one.
enum NoteEditing: Identifiable {
    case add
    case edit(StaffNote)

    var id: String {
        switch self {
        case .add: "add"
        case .edit(let note): "edit-\(note.name)"
        }
    }

    var note: StaffNote? {
        if case .edit(let note) = self { note } else { nil }
    }
}

/// The sheet for a note: its name (fixed once it exists), a one-line
/// description, and the body, checked as the daemon checks them.
private struct NoteSheet: View {
    var memory: ChiefMemory
    var editing: NoteEditing
    @State private var name: String
    @State private var description: String
    @State private var bodyText: String
    @State private var tried = false
    @State private var busy = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    init(memory: ChiefMemory, editing: NoteEditing) {
        self.memory = memory
        self.editing = editing
        _name = State(initialValue: editing.note?.name ?? "")
        _description = State(initialValue: editing.note?.description ?? "")
        _bodyText = State(initialValue: editing.note?.body ?? "")
    }

    var body: some View {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let existing = editing.note
        let nameProblem = existing == nil ? StaffNote.nameProblem(trimmed) : nil
        let descriptionProblem = StaffNote.descriptionProblem(description)
        let bodyProblem = StaffNote.bodyProblem(bodyText)
        let replaces = existing == nil && (memory.notes ?? []).contains { $0.name == trimmed }
        let bytes = bodyText.trimmingCharacters(in: .whitespacesAndNewlines).utf8.count
        NavigationStack {
            Form {
                Section {
                    TextField("user-preferences", text: Binding(get: { name }, set: { name = $0.lowercased() }))
                        .font(.body.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(existing != nil)
                        .foregroundStyle(existing != nil ? Palette.ink2 : Palette.ink)
                } header: {
                    Text("Name")
                } footer: {
                    if tried, let nameProblem {
                        Text(nameProblem).foregroundStyle(Palette.red)
                    } else if replaces {
                        Text("There's already a note called \(trimmed); this replaces it.")
                    } else if existing == nil {
                        Text("Lowercase letters, digits and dashes, such as release-process.")
                    }
                }
                Section {
                    TextField("How I like PRs and replies", text: $description)
                } header: {
                    Text("Description")
                } footer: {
                    if tried, let descriptionProblem {
                        Text(descriptionProblem).foregroundStyle(Palette.red)
                    } else {
                        Text("One line: what the note holds. This goes in its prompt.")
                    }
                }
                Section {
                    TextEditor(text: $bodyText)
                        .font(.footnote.monospaced())
                        .frame(minHeight: 200)
                } header: {
                    Text("Note")
                } footer: {
                    if tried, let bodyProblem {
                        Text(bodyProblem).foregroundStyle(Palette.red)
                    } else {
                        Text("\(bytes.formatted()) / \(StaffNote.maxBodyBytes.formatted()) bytes").monospacedDigit()
                    }
                }
                if let error {
                    Section { Text(error).font(.footnote).foregroundStyle(Palette.red) }
                }
            }
            .navigationTitle(existing.map { "Edit \($0.name)" } ?? "Add a memory note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(existing != nil ? "Save" : replaces ? "Replace" : "Add") {
                        tried = true
                        guard nameProblem == nil, descriptionProblem == nil, bodyProblem == nil else { return }
                        Task { await submit(name: existing?.name ?? trimmed) }
                    }
                    .disabled(busy)
                }
            }
        }
        .interactiveDismissDisabled(busy)
    }

    private func submit(name: String) async {
        busy = true
        defer { busy = false }
        do {
            try await memory.save(name: name, description: description, body: bodyText)
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// "Deleted release-process · Undo", along the bottom for a few seconds.
struct UndoBar: View {
    var text: String
    var undo: () -> Void

    var body: some View {
        HStack {
            Text(text)
                .font(.subheadline)
                .lineLimit(1)
            Spacer()
            Button("Undo", action: undo)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Palette.claude)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}
