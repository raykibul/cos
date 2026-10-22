import Foundation

/// Everything an app asks the daemon, as a protocol, so an app can run
/// against an in-memory fake (screenshots, previews, view-model tests) as
/// well as a real `CosClient`. Device and listener management are left
/// out: they only work on the host's own socket.
public protocol CosService: EventSource {
    // Status and events (EventSource): status(), stream(after:)
    func events(after: Int, limit: Int) async throws -> [LoggedEvent]

    // Projects and workspaces
    func projects() async throws -> [Project]
    func createWorkspace(projectId: Int, base: WorkspaceBase, title: String?) async throws -> Workspace

    // Agents and approvals
    func agents(workspaceId: Int?, state: AgentState?) async throws -> [Agent]
    func agent(id: Int) async throws -> Agent
    func createAgent(workspaceId: Int, _ request: NewAgent) async throws -> Agent
    func sendInput(agentId: Int, text: String) async throws
    func trustAgent(id: Int) async throws
    func stopAgent(id: Int) async throws
    func resumeAgent(id: Int) async throws -> Agent
    func screen(agentId: Int) async throws -> AgentScreen
    func approvals(state: ApprovalState?) async throws -> [Approval]
    func decide(approvalId: Int, _ decision: ApprovalDecision, message: String?) async throws -> Approval

    // Runs
    func runs() async throws -> [Run]
    func createRun(_ request: NewRun) async throws -> Run
    func board(runId: Int) async throws -> Board
    func approvePlan(runId: Int) async throws -> Board
    func cancelRun(id: Int, reason: String?) async throws -> Board
    func questions(state: QuestionState?) async throws -> [Question]
    func answer(questionId: Int, _ answer: String) async throws -> Question
    func messageChief(runId: Int, text: String) async throws
    func report(runId: Int) async throws -> String

    // The Chief of Staff and transcripts
    func staff(projectId: Int) async throws -> Staff
    func startStaff(projectId: Int) async throws -> Staff
    func messageStaff(projectId: Int, text: String) async throws
    func staffInbox(projectId: Int) async throws -> [StaffItem]
    func transcript(agentId: Int, after: Int) async throws -> Transcript

    // Environment
    func globalEnv() async throws -> EnvVars
    func projectEnv(projectId: Int) async throws -> ProjectEnv
}

extension CosClient: CosService {}
