import Foundation

/// Everything an app asks the daemon, as a protocol, so an app can run
/// against an in-memory fake (screenshots, previews, view-model tests) as
/// well as a real `CosClient`. Device and listener management are left
/// out: they only work on the host's own socket. The tailnet's peers are
/// in.
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

    // The Chief of Staff and transcripts
    func staff(projectId: Int) async throws -> Staff
    func startStaff(projectId: Int) async throws -> Staff
    func messageStaff(projectId: Int, text: String) async throws
    /// A message naming files uploaded with `uploadAttachment(to: .staff…)`.
    func messageStaff(projectId: Int, text: String, attachments: [String]) async throws
    func staffInbox(projectId: Int) async throws -> [StaffItem]
    func transcript(agentId: Int, after: Int) async throws -> Transcript

    // Attachments ("Attachments" in docs/agents-api.md)
    func uploadAttachment(to target: AttachTarget, name: String, contentType: String, data: Data) async throws -> Attachment
    func attachmentData(agentId: Int, name: String) async throws -> Data

    // Environment
    func globalEnv() async throws -> EnvVars
    func projectEnv(projectId: Int) async throws -> ProjectEnv

    // The tailnet: approved devices may list it too.
    func tailnetPeers(port: Int?) async throws -> TailnetPeers
}

extension CosClient: CosService {}
