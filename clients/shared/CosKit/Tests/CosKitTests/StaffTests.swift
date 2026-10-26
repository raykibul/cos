import Foundation
import Testing
@testable import CosKit

/// Shapes from docs/chief-of-staff.md, docs/agents-api.md and "Environment"
/// in docs/daemon.md.
enum StaffSamples {
    static let staff = #"{"project_id":1,"agent_id":14,"state":"idle","model":"haiku","brief":"Ship 0.2.","worktree":"/w/chief","branch":"atlas/chief","working_agents":2,"waiting_on_you":1}"#
    static let identified = #"{"project_id":1,"agent_id":14,"state":"idle","brief":"Ship 0.2.","identity":"You are Ada.","paused":true,"updates_waiting":"3"}"#
    static let memory = #"{"notes":[{"name":"user-preferences","description":"How I like PRs","body":"- Short replies.","created_ms":1792611174811,"updated_ms":1792611274811},{"name":"","body":"no name"},{"name":"release-process","description":"How we tag","body":"Tag from main.","created_ms":1,"updated_ms":2}]}"#
    static let notStarted = #"{"project_id":2,"agent_id":null,"state":null,"model":null,"brief":"","worktree":null,"branch":null,"working_agents":0,"waiting_on_you":0}"#
    static let inbox = #"{"items":[{"id":7,"project_id":1,"kind":"agent","agent_id":15,"text":"Agent 15 \"Add a README line\" finished a turn\nDone: added it.","created_ms":1792611174811,"delivered_ms":null},{"id":8,"project_id":1,"kind":"message","agent_id":null,"text":"Ship it","created_ms":1792611274811,"delivered_ms":1792611275000},{"oops":true}]}"#
    static let transcript = #"""
    {"messages":[
      {"n":2,"role":"assistant","kind":"tool","tool":{"name":"mcp__cos__agent_spawn","summary":"{\"prompt\":\"…\"}","is_error":false},"ts_ms":1792611174900},
      {"n":1,"role":"user","kind":"text","text":"Fix the login bug","ts_ms":1792611174811},
      {"n":3,"role":"user","kind":"update","text":"[cos · 2 updates]\n### Agent 15 finished\nDone.","ts_ms":1792611180000},
      {"n":4,"role":"user","text":"[cos · 1 updates]\nlegacy","ts_ms":1792611190000},
      {"n":5,"role":"assistant","kind":"thinking_v2","text":"newer kind","ts_ms":1792611200000},
      {"role":"assistant","text":"no n"}
    ],"next":5}
    """#
    static let env = #"{"variables":{"CLAUDE_CONFIG_DIR":"~/.claude-personal","N":3},"global":{"CLAUDE_CONFIG_DIR":"~/.claude","API":"sk-abcdefghijk"},"effective":{"CLAUDE_CONFIG_DIR":"~/.claude-personal","API":"sk-abcdefghijk","N":"3"}}"#
    /// An agent with every field the newer daemon adds, and states this
    /// client doesn't know yet.
    static let newerAgent = #"{"id":"16","workspace_id":9,"run_id":3,"task_id":7,"role":"auditor","title":"","state":"daydreaming","user_settings":false,"rows":"40","started_ms":1792611174811,"spawned_by":14,"transcript_path":"/t.jsonl","env_names":["CLAUDE_CONFIG_DIR",4],"cwd":"/tasks/t7","brand_new_field":{"x":1}}"#
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.cos().decode(type, from: Data(json.utf8))
}

@Suite struct StaffModelTests {
    @Test func decodesStaff() throws {
        let staff = try decode(Staff.self, StaffSamples.staff)
        #expect(staff.agentId == 14)
        #expect(staff.state == .idle)
        #expect(staff.model == "haiku")
        #expect(staff.workingAgents == 2)
        #expect(staff.waitingOnYou == 1)
        #expect(staff.stateLabel == "Ready for your next message")

        let fresh = try decode(Staff.self, StaffSamples.notStarted)
        #expect(!fresh.hasStarted)
        #expect(fresh.state == nil)
        #expect(fresh.model == nil)
        #expect(fresh.stateLabel == "Not started")
    }

    @Test func decodesTheIdentityAndPause() throws {
        let staff = try decode(Staff.self, StaffSamples.identified)
        #expect(staff.identity == "You are Ada.")
        #expect(staff.paused)
        #expect(staff.updatesWaiting == 3)
        // An older daemon has none of them.
        let older = try decode(Staff.self, StaffSamples.staff)
        #expect(older.identity == "")
        #expect(!older.paused)
        #expect(older.updatesWaiting == 0)
    }

    @Test func decodesMemoryNotesByNameSkippingOnesWithoutAName() throws {
        let memory = try decode(StaffMemory.self, StaffSamples.memory)
        #expect(memory.notes.map(\.name) == ["release-process", "user-preferences"])
        let note = memory.notes[1]
        #expect(note.id == "user-preferences")
        #expect(note.description == "How I like PRs")
        #expect(note.body == "- Short replies.")
        #expect(note.updatedMs == 1_792_611_274_811)
    }

    @Test func checksANoteAsTheDaemonDoes() {
        #expect(StaffNote.nameProblem("release-process") == nil)
        #expect(StaffNote.nameProblem("a") == nil)
        #expect(StaffNote.nameProblem("") == "Give it a name.")
        #expect(StaffNote.nameProblem("Release") != nil)
        #expect(StaffNote.nameProblem("-release") != nil)
        #expect(StaffNote.nameProblem("release-") != nil)
        #expect(StaffNote.nameProblem("has space") != nil)
        #expect(StaffNote.nameProblem(String(repeating: "a", count: 65)) == "Keep the name to 64 characters.")

        #expect(StaffNote.oneLine("  How we tag\n  releases ") == "How we tag releases")
        #expect(StaffNote.descriptionProblem(" \n ") != nil)
        #expect(StaffNote.descriptionProblem(String(repeating: "x", count: 201)) == "Keep it to 200 characters.")
        #expect(StaffNote.descriptionProblem("How we tag") == nil)

        #expect(StaffNote.bodyProblem("  ") == "Write what the chief should know.")
        #expect(StaffNote.bodyProblem(String(repeating: "é", count: 4097)) != nil) // 8,194 bytes
        #expect(StaffNote.bodyProblem(String(repeating: "x", count: 8192)) == nil)
    }

    @Test func decodesTheInboxNewestFirstSkippingOddItems() throws {
        let inbox = try decode(StaffInbox.self, StaffSamples.inbox)
        #expect(inbox.items.map(\.id) == [8, 7])
        let item = inbox.items[1]
        #expect(item.kind == "agent")
        #expect(item.heading == "Agent 15 \"Add a README line\" finished a turn")
        #expect(item.body == "Done: added it.")
    }

    @Test func decodesTranscriptsLeniently() throws {
        let transcript = try decode(Transcript.self, StaffSamples.transcript)
        #expect(transcript.messages.map(\.n) == [1, 2, 3, 4, 5])
        #expect(transcript.next == 5)
        #expect(transcript.messages[0].role == .user)
        let tool = try #require(transcript.messages[1].tool)
        #expect(tool.title == "Started an agent")
        #expect(tool.isCosTool)
        #expect(transcript.messages[2].kind == .update)
        #expect(transcript.messages[2].updateCount == 2)
        #expect(transcript.messages[2].updateBody == "### Agent 15 finished\nDone.")
        // An untagged wakeup still reads as one; an unknown kind stays as it is.
        #expect(transcript.messages[3].kind == .update)
        #expect(transcript.messages[4].kind == MessageKind(rawValue: "thinking_v2"))
    }

    @Test func toolCallsNameTheAgentTheyActedOn() throws {
        let json = #"{"messages":[{"n":1,"role":"assistant","kind":"tool","tool":{"name":"mcp__cos__agent_spawn","summary":"Fix the flaky test","is_error":false,"agent_id":17},"ts_ms":1},{"n":2,"role":"assistant","kind":"tool","tool":{"name":"Bash","summary":"ls"},"ts_ms":2}],"next":2}"#
        let transcript = try decode(Transcript.self, json)
        let spawn = try #require(transcript.messages[0].tool)
        #expect(spawn.isSpawn)
        #expect(spawn.agentId == 17)
        #expect(transcript.messages[1].tool?.agentId == nil)
        #expect(transcript.messages[1].tool?.isSpawn == false)
    }

    @Test func taskNotificationsReadAsFinishedAgents() throws {
        let text = """
        <task-notification>
        <task-id>a066345f791fd4714</task-id>
        <tool-use-id>toolu_01N</tool-use-id>
        <output-file>/tmp/tasks/a066345f791fd4714.output</output-file>
        <status>completed</status>
        <summary>Agent "Update on open PRs" finished</summary>
        <note>A task-notification fires each time this agent stops.</note>
        <result>**PR status for raykibul (44 open)**

        3 need you.</result>
        <usage>total_tokens: 51234
        tool_uses: 12
        duration_ms: 241000</usage>
        </task-notification>
        """
        let encoded = try JSONEncoder().encode(["messages": [["n": "1", "role": "user", "kind": "text", "text": text]]])
        let transcript = try JSONDecoder.cos().decode(Transcript.self, from: encoded)
        let message = try #require(transcript.messages.first)
        #expect(message.kind == .task)
        let task = try #require(message.taskNotification)
        #expect(task.taskId == "a066345f791fd4714")
        #expect(task.title == "Update on open PRs")
        #expect(task.isAgent)
        #expect(task.succeeded)
        #expect(task.result == "**PR status for raykibul (44 open)**\n\n3 need you.")
        #expect(task.durationMs == 241_000)

        // A background command, and a bare one with no tags.
        let command = try #require(TaskNotification("<task-notification><status>failed</status><summary>Background command \"npm test\" failed (exit code 1)</summary></task-notification>"))
        #expect(command.title == "npm test")
        #expect(!command.isAgent)
        #expect(!command.succeeded)
        #expect(command.result.isEmpty)
        #expect(command.durationMs == nil)
        #expect(TaskNotification("<task-notification></task-notification>")?.title == "Background task")

        // Not one: ordinary text, or Claude's own (assistant) words about one.
        #expect(TaskNotification("I got a <task-notification>") == nil)
        #expect(TranscriptMessage(n: 2, role: .assistant, kind: .text, text: text).taskNotification == nil)
    }

    @Test func readsTheChiefsPromptWithoutCosLine() {
        let prompt = "Fix the flaky login test.\n\nRun it ten times.\n\n[cos] The Chief of Staff of atlas started you. When you finish, end with a short summary of what you did and anything left open."
        #expect(TranscriptMessage(n: 1, role: .user, kind: .text, text: prompt).spawnedPrompt == "Fix the flaky login test.\n\nRun it ten times.")
        #expect(TranscriptMessage(n: 1, role: .user, kind: .text, text: "Fix it").spawnedPrompt == nil)
        #expect(TranscriptMessage(n: 2, role: .assistant, kind: .text, text: prompt).spawnedPrompt == nil)
    }

    @Test func mergesPagesByNumber() {
        let first = [TranscriptMessage(n: 1, role: .user, kind: .text, text: "hi"), TranscriptMessage(n: 2, role: .assistant, kind: .tool, tool: ToolCall(name: "Bash"))]
        let second = [TranscriptMessage(n: 2, role: .assistant, kind: .tool, tool: ToolCall(name: "Bash", isError: true)), TranscriptMessage(n: 3, role: .assistant, kind: .text, text: "done")]
        let merged = Transcript.merge(first, second)
        #expect(merged.map(\.n) == [1, 2, 3])
        #expect(merged[1].tool?.isError == true)
    }

    @Test func toolTitles() {
        #expect(ToolCall(name: "Bash").title == "Bash")
        #expect(ToolCall(name: "mcp__cos__brief_update").title == "Updated its brief")
        #expect(ToolCall(name: "mcp__cos__memory_write").title == "Saved a note")
        #expect(ToolCall(name: "mcp__cos__new_thing").title == "new thing")
        #expect(ToolCall(name: "mcp__github__create_issue").title == "create issue (github)")
    }

    @Test func decodesEnvironment() throws {
        let env = try decode(ProjectEnv.self, StaffSamples.env)
        #expect(env.variables == ["CLAUDE_CONFIG_DIR": "~/.claude-personal", "N": "3"])
        #expect(env.overrides("CLAUDE_CONFIG_DIR"))
        #expect(!env.overrides("API"))
        #expect(sortedEnvNames(env.effective.keys) == ["CLAUDE_CONFIG_DIR", "API", "N"])
        #expect(maskedEnvValue("sk-abcdefghijk") == "sk-…  (14 chars)")
        #expect(maskedEnvValue("short") == "•••  (5 chars)")

        // Without `effective`, it's worked out.
        let worked = try decode(ProjectEnv.self, #"{"variables":{"A":"p"},"global":{"A":"g","B":"g"}}"#)
        #expect(worked.effective == ["A": "p", "B": "g"])
        // A list of {name, value} is read too.
        let listed = try decode(GlobalEnv.self, #"{"variables":[{"name":"A","value":"1"},{"name":"B"}]}"#)
        #expect(listed.variables == ["A": "1"])
    }

    @Test func decodesANewerAgentWithoutFailing() throws {
        let agent = try decode(Agent.self, StaffSamples.newerAgent)
        #expect(agent.id == 16)
        #expect(agent.title == "Agent 16")
        #expect(agent.state == AgentState(rawValue: "daydreaming"))
        #expect(agent.state.isAlive)
        #expect(agent.role == AgentRole(rawValue: "auditor"))
        #expect(agent.rows == 40)
        #expect(agent.userSettings == false)
        #expect(agent.spawnedBy == 14)
        #expect(agent.envNames == ["CLAUDE_CONFIG_DIR"])
        #expect(agent.cwd == "/tasks/t7")
    }

    @Test func needsLoginWaitsOnYou() throws {
        let agent = try decode(Agent.self, #"{"id":3,"workspace_id":1,"title":"x","state":"needs_login","started_ms":0}"#)
        #expect(agent.state == .needsLogin)
        #expect(agent.state.needsYou)
        #expect(agent.state.isAlive)
    }

    @Test func oneOddRowNeverHidesTheRest() throws {
        let agents = try decode(AgentsPage.self, #"{"agents":[{"id":1,"state":"idle","title":"a","started_ms":0},{"title":"no id"},7,{"id":2,"state":"working"}]}"#)
        #expect(agents.agents.map(\.id) == [1, 2])
        let approvals = try decode(ApprovalsPage.self, #"{"approvals":[{"id":1,"agent_id":2,"tool_name":"Bash","state":"weird","created_ms":0,"tier":5},{"id":2}]}"#)
        #expect(approvals.approvals.map(\.id) == [1])
        #expect(approvals.approvals[0].state == ApprovalState(rawValue: "weird"))
        #expect(approvals.approvals[0].tier == nil)
        let projects = try decode(ProjectsPage.self, #"{"projects":[{"id":1,"name":"atlas","workspaces":[{"id":"4","branch":"atlas/oslo"},{"nope":1}]},{"name":"no id"}]}"#)
        #expect(projects.projects.count == 1)
        #expect(projects.projects[0].workspaces.map(\.id) == [4])
        #expect(projects.projects[0].workspaces[0].city == "oslo")
    }
}

@Suite struct StaffClientTests {
    @Test func sendsTheStaffTranscriptAndEnvRoutes() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("GET", "/v1/projects/1/staff"), ("POST", "/v1/projects/1/staff/start"), ("PUT", "/v1/projects/1/staff"):
                .json(200, StaffSamples.staff)
            case ("POST", "/v1/projects/1/staff/message"): .empty(202)
            case ("GET", "/v1/projects/1/staff/inbox"): .json(200, StaffSamples.inbox)
            case ("GET", "/v1/agents/14/transcript?after=4"): .json(200, StaffSamples.transcript)
            case ("GET", "/v1/env"): .json(200, #"{"variables":{"A":"1"}}"#)
            case ("GET", "/v1/projects/1/env"): .json(200, StaffSamples.env)
            case ("PUT", "/v1/projects/1/env/CLAUDE_CONFIG_DIR"), ("DELETE", "/v1/env/A"): .json(200, #"{"variables":{}}"#)
            default: .json(404, #"{"error":"unknown route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.staff(projectId: 1).agentId == 14)
        #expect(try await client.startStaff(projectId: 1).state == .idle)
        _ = try await client.setStaffModel(projectId: 1, model: "  ")
        try await client.messageStaff(projectId: 1, text: "What's left?")
        #expect(try await client.staffInbox(projectId: 1).count == 2)
        #expect(try await client.transcript(agentId: 14, after: 4).next == 5)
        #expect(try await client.globalEnv() == ["A": "1"])
        #expect(try await client.projectEnv(projectId: 1).global["API"] == "sk-abcdefghijk")
        try await client.setEnv(projectId: 1, name: "CLAUDE_CONFIG_DIR", value: "~/.claude-work")
        try await client.unsetEnv(projectId: nil, name: "A")

        let requests = server.requests
        #expect(requests.first { $0.method == "PUT" && $0.path == "/v1/projects/1/staff" }?.value == .object(["model": .null]))
        #expect(requests.first { $0.path.hasSuffix("/staff/message") }?.json == ["text": "What's left?"])
        #expect(requests.first { $0.path.hasSuffix("/env/CLAUDE_CONFIG_DIR") }?.json == ["value": "~/.claude-work"])
    }
}

@Suite struct StaffMemoryClientTests {
    @Test func sendsTheBriefIdentityMemoryAndNewSessionRoutes() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("PUT", "/v1/projects/1/staff"), ("POST", "/v1/projects/1/staff/new-session"): .json(200, StaffSamples.identified)
            case ("GET", "/v1/projects/1/staff/memory"): .json(200, StaffSamples.memory)
            case ("PUT", "/v1/projects/1/staff/memory/release-process"):
                .json(200, #"{"name":"release-process","description":"How we tag","body":"Tag from main.","created_ms":1,"updated_ms":3}"#)
            case ("DELETE", "/v1/projects/1/staff/memory/release-process"): .empty(204)
            case ("DELETE", "/v1/projects/1/staff/memory/gone"): .json(404, #"{"error":"there is no note gone"}"#)
            default: .json(404, #"{"error":"unknown route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.updateStaff(projectId: 1, brief: "Ship 0.2.").identity == "You are Ada.")
        _ = try await client.updateStaff(projectId: 1, identity: "You are Ada.")
        #expect(try await client.newStaffSession(projectId: 1).agentId == 14)
        #expect(try await client.staffMemory(projectId: 1).count == 2)
        let saved = try await client.saveStaffNote(projectId: 1, name: "release-process", description: "How we tag", body: "Tag from main.")
        #expect(saved.updatedMs == 3)
        try await client.deleteStaffNote(projectId: 1, name: "release-process")
        await #expect(throws: CosError.self) { try await client.deleteStaffNote(projectId: 1, name: "gone") }

        let puts = server.requests.filter { $0.method == "PUT" && $0.path == "/v1/projects/1/staff" }
        // Only the field given goes, so the other stays as it is.
        #expect(puts.map(\.json) == [["brief": "Ship 0.2."], ["identity": "You are Ada."]])
        #expect(server.requests.first { $0.path.hasSuffix("/memory/release-process") && $0.method == "PUT" }?.json
            == ["description": "How we tag", "body": "Tag from main."])
    }
}
