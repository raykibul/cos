# ACP agents: Claude Code, opencode and Command Code (spec)

Status: proposed, not built. Section 11 lists the decisions to confirm, and section 12 lists the risks. Protocol details were checked on 28 Oct 2026 against four sources:
- the ACP site (protocol version 1, schema v1);
- opencode's source at its v1.19.5 release;
- Command Code's docs and its shipped npm bundle (`command-code@1.88.1`, closed source);
- the source of Claude's ACP adapter, `claude-agent-acp` 0.98.0 (commit `2f1f4d1`, built on `@anthropic-ai/claude-agent-sdk` 0.3.310).

Nothing was run live. Anything marked **(unverified)** needs a live check in milestone 2 or 5.

> **Changed on 28 Oct 2026:** Claude goes through ACP too. You rarely use Claude's terminal, so new Claude agents and chiefs run through `claude-agent-acp` (harness `claude`, section 3.10), with the same transcript, approvals and environment as every other ACP agent. Today's PTY and hooks path stays as the harness `claude-terminal`, for existing agents and for when you want the terminal. Retiring it is a later decision (section 11).

## 1. Context & problem

cos runs one kind of agent: Claude Code's own UI in a PTY the daemon owns, followed through per-session hooks ([claude-protocol.md](../claude-protocol.md)). You want to run opencode and Command Code agents next to it, in the same projects and workspaces, started by you or by a project's Chief of Staff, and shown in the same chat views.

Both CLIs speak the [Agent Client Protocol](https://agentclientprotocol.com) (ACP) over stdio: `opencode acp` and `cmd acp`. Claude Code does too, through the official adapter `claude-agent-acp`, which runs Claude Code headless through Anthropic's Agent SDK (it's what Zed uses). ACP is JSON-RPC 2.0 between a client (cos) and an agent. The client starts sessions, sends prompts, gets the conversation back as `session/update` notifications, and answers the agent's `session/request_permission` calls. One ACP backend therefore covers all three and any other ACP agent.

What exists to build on, and where Claude is wired in today:

- **Nothing abstracts over agent kinds.** `cos-harness/src/lib.rs:3-4` says the shared surface (start, send, answer, interrupt, events) becomes a trait when Codex comes. The daemon's `Running.terminal` is `Arc<ClaudeTerminal>` (`crates/cos/src/daemon/agents.rs:288-305`), and every operation goes through it.
- **Launch:** `Inner::launch` (`agents.rs:776-849`) builds `ClaudeOptions` and `TerminalOptions` (`cos-harness/src/claude/terminal.rs:76-130`) and calls `ClaudeTerminal::spawn`. A Chief of Staff gets `--mcp-config` and `--allowedTools mcp__cos__*` from `staff::configure` (`daemon/staff/prompt.rs:89-109`) and `--append-system-prompt` from its record.
- **Events:** one `HookServer` at `<home>/hooks.sock` for every agent, routed by session id (`agents.rs:945-950`). `Inner::on_fired` (`daemon/agents/hooks.rs:60-145`) turns `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse` and `Stop` into states, approvals and `AgentSignal`s.
- **Approvals:** a `PermissionRequest` hook waits as `hooks::Waiting { hook: Hook, … }`. `Inner::answer` calls `Hook::answer(&Decision)` (`cos-harness/src/claude/hooks.rs:169-179`). `cos-policy` classifies requests by Claude tool name (`Bash`, `Read`, `Write`/`Edit`/`MultiEdit`, `Grep`/`Glob`/`LS`, `WebFetch`, `WebSearch`, `mcp__…`) and Claude input fields (`command`, `file_path`, `pattern`, `url`) (`cos-policy/src/classify.rs:30-56`).
- **Trust and login:** read from the screen before the first hook (`agents.rs:877-916`, `terminal.rs:400-470`).
- **Stop:** `end_session` (`agents.rs:968-986`) presses Esc, types `/exit`, then kills.
- **Transcript:** `daemon/transcript.rs` parses Claude's JSONL into `Message`s with tool results folded in, plus `context` from the newest reply's `usage`. `agents/transcripts.rs` decides when to log `agent.transcript` from hooks.
- **Shells** (`daemon/shells.rs`) borrow the PTY type: `ClaudeTerminal::spawn_program`, `TerminalView::Live` and `agents_api::terminal_stream`. They have no hooks, approvals or agent records.

Several places assume an agent has a terminal. The terminal, screen, keys, resize and trust routes all do. `agent_read {screen: true}` reads the screen (`staff/tools.rs:322-339`). The Mac app shows a `role: staff` agent's page as only its terminal (`clients/desktop/src/screens/AgentPage.tsx:125-130`) and opens the terminal for `needs_login`. The iPhone's session chat has a Transcript | Terminal switch.

## 2. Goals / non-goals

**Goals**

- One ACP backend that runs Claude Code (through `claude-agent-acp`), `opencode acp`, `cmd acp` and other ACP agents as cos agents, in any workspace, started from the apps, the CLI or a Chief of Staff's `agent_spawn`. A Chief of Staff itself can run on one too.
- Claude through ACP becomes the default for new agents and chiefs. Each project's environment (`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, API keys) still picks the Claude account it uses, as it does now (3.10).
- What has to work for ACP agents:
  - the **transcript**: the Mac app's and iPhone's chat, `agent_read`, and the context meter when the agent reports usage;
  - **approvals**, classified by cos-policy and answered by you;
  - the **states** `starting`, `working`, `waiting_approval`, `idle`, `needs_login`, `exited` and `interrupted`;
  - **sending messages** (attachments included), **interrupt**, **stop** and **resume**, including across daemon restarts.
- The PTY and hooks path keeps working exactly as today, as the harness `claude-terminal`: the same arguments, states and transcripts. Existing agents move to it in the migration, and the existing daemon tests pass unchanged.
- Per-project shells (`/v1/shells`, the Mac app's Ctrl-J tabs, the iPhone's shells) keep working, untouched (section 3.9).
- Nothing in a user's or repo's config can approve an ACP agent's request before cos sees it. Where cos can't fully prevent that, it detects it and shows it.

**Non-goals**

- A terminal view for ACP agents. They run on pipes, with no PTY and no screen reading.
- A backend per CLI: no opencode HTTP server client, no `cmd -p` driver.
- Removing the PTY and hooks path. That comes later, if `claude-terminal` stops being used (section 11).
- cos acting as the agent's file system or terminal (`fs/*`, `terminal/*` client methods). cos declares neither capability, so agents use their own tools (section 4 explains why, and when it might come back).
- Codex. Its `app-server` isn't ACP, but the trait in 3.1 is meant to fit it later.
- Choosing an agent's harness automatically from the task (jev or the chief can do that later).

## 3. Approach

```
                          cos daemon
                 ┌─────────────────────────────────────────────┐
 apps / CLI ───▶ │ AgentManager                                │
 chief tools     │   Running { session: Arc<dyn AgentSession> }│
                 │      │                       │              │
                 │  ClaudeAgent              AcpAgent          │
                 │  (PTY + hooks, as today)  (stdio JSON-RPC)  │
                 │      │  AgentEvent             │ AgentEvent │
                 │      └──────────┬──────────────┘            │
                 │         Inner::on_event(agent_id, event)    │
                 │   states · approvals (cos-policy) · inbox · │
                 │   transcript (Claude JSONL | cos JSONL)     │
                 └──────┬──────────────────────────┬───────────┘
                        │ PTY                      │ stdin/stdout
                  claude --session-id …      claude-agent-acp | opencode acp | cmd acp
                  (claude-terminal)          (claude, opencode, command-code)
                  hooks → hooks.sock         ◀── session/update, request_permission
                                             ──▶ initialize, session/new|resume|load,
                                                 session/prompt, session/cancel
```

- **cos-harness** gets a small backend seam (3.1): an `AgentSession` trait for one running agent and an `AgentEvent` enum that both backends produce. Claude's existing code goes behind it unchanged. The new `acp` module is an ACP client over stdio, with lenient types like `claude/protocol.rs`.
- **The daemon** keeps one `AgentManager`. `Running` holds `Arc<dyn AgentSession>` instead of `Arc<ClaudeTerminal>`. Both backends' events go through one `Inner::on_event`, so states, approvals, the inbox, `AgentSignal`s and the event log stay the same code.
- **ACP agents' transcripts** are written by cos from the `session/update` stream, as JSONL in the cos home. The existing transcript route reads them into the same `Message`s (3.3).
- **Harness profiles** are data, not code paths. Each profile holds the binary, its arguments, where to find it, how to pass the model, the permission rules and the system prompt, and the login hint for `claude`, `opencode` and `command-code` (3.8, 3.10). The protocol code is shared.

### 3.1 The backend seam

In `crates/cos-harness/src/agent.rs` (new), re-exported from the crate root:

```rust
/// One running agent, whatever drives it.
pub trait AgentSession: Send + Sync {
    fn pid(&self) -> Option<u32>;
    /// `Some(code)` once it exited (`None` inside: killed).
    fn exit_status(&self) -> Option<Option<i32>>;
    /// A message from you or the chief. Claude: one bracketed paste, then Return.
    /// ACP: one `session/prompt`, queued while a turn runs (3.2).
    fn send(&self, text: String) -> BoxFuture<'_, Result<(), HarnessError>>;
    /// Ends the running turn. Claude: Esc. ACP: `session/cancel`.
    fn interrupt(&self) -> BoxFuture<'_, Result<(), HarnessError>>;
    /// Ends the session, killing it after `timeout`. Claude: Esc if busy,
    /// `/exit`, kill. ACP: cancel if busy, close stdin, SIGTERM, then SIGKILL
    /// to its process group.
    fn stop(&self, busy: bool, timeout: Duration) -> BoxFuture<'_, ()>;
    fn kill(&self);
    /// The PTY, for backends that have one (`claude-terminal`). `None` for ACP.
    fn terminal(&self) -> Option<&ClaudeTerminal>;
}

/// What an agent did, the same for every backend.
pub enum AgentEvent {
    /// Ready for input: Claude's `SessionStart`, or ACP's session set up
    /// after `session/new`, `session/resume` or `session/load`.
    Started,
    /// A turn began: `UserPromptSubmit`, or a prompt sent.
    TurnStarted,
    ToolStarted { id: String, name: String, input: Value },
    ToolFinished { id: String, failed: bool },
    /// Needs an answer; `reply` answers it exactly once.
    PermissionAsked { reply: PermissionReply, tool: String, input: Value,
                      tool_use_id: Option<String>, cwd: Option<PathBuf> },
    /// A request stopped waiting without cos: answered in Claude's terminal,
    /// or the ACP turn was cancelled.
    PermissionWithdrawn { reply_id: u64 },
    TurnEnded { last_message: Option<String> },
    TranscriptPath(PathBuf),
    /// The transcript file grew. `turn_ended` asks for one late re-read
    /// (Claude writes the last reply after `Stop`).
    TranscriptGrew { turn_ended: bool },
    NeedsTrust,
    NeedsLogin { hint: Option<String> },
    Exited { code: Option<i32> },
}

/// Answers one permission request: a Claude hook, or an ACP request id.
pub struct PermissionReply { id: u64, inner: ReplyInner }
impl PermissionReply {
    pub fn answer(self, decision: &Decision);
}
```

- **`BoxFuture`** comes from the `futures` crate, already in `Cargo.lock` as a transitive dependency and added as a direct one (or written as `Pin<Box<dyn Future + Send + '_>>`). `async fn` in a trait isn't object-safe, and the daemon needs `dyn AgentSession`. That avoids adding `async-trait`.
- **`claude-terminal` behind it.** `ClaudeAgent` (in `claude/agent.rs`) wraps `ClaudeTerminal`. `send` is `submit_pasted`, `interrupt` writes `\x1b`, and `stop` is today's `end_session` moved from the daemon. Hooks still arrive on the shared `HookServer`, routed by session id in the daemon, because one socket serves every agent. The pure translation `Hook → AgentEvent` moves to `claude::hooks::event(&Hook)`. The daemon's `follow_hooks` and `watch_output` call `on_event` with the result. Session following on `/clear` (`follow_new_session`) and pairing a request with its `PreToolUse` (`pair`) stay where they are: Claude-specific routing in front of `on_event`. The screen checks for trust and login stay in `on_output` and emit `NeedsTrust` and `NeedsLogin`.
- **"Without changing its behaviour"** is the acceptance test for milestone 1. `TerminalOptions::args()` is byte for byte the same, and the hook settings, the order of state changes, approvals and `AgentSignal`s are unchanged. `agents/tests.rs` and `staff/tests.rs` pass without edits. A unit test feeds recorded hook sequences through both the old and new paths before the old one is deleted.
- **Shells keep `ClaudeTerminal::spawn_program`**. The type isn't renamed in this work, so `shells.rs` doesn't change.
- **Not in the trait:** `keys`, `resize`, `trust` and `screen` go through `terminal()`. A backend without one answers 409 (6.1).

### 3.2 Mapping ACP onto cos

**Lifecycle**

| cos | ACP | Notes |
|---|---|---|
| start | spawn `<binary> <args>` with stdin/stdout piped and stderr kept (the last 40 lines, as `ClaudeSession` does), in its own process group, in the agent's folder; then `initialize`; then `session/new {cwd, mcpServers}` | `initialize` sends `protocolVersion: 1`, `clientInfo: {name: "cos", version}` and `clientCapabilities: {fs: {readTextFile: false, writeTextFile: false}, terminal: false, auth: {terminal: true}}`. `auth.terminal` doesn't make cos run anything: it asks agents to list their terminal login commands, which become the login hint (3.7). cos declares nothing else (no `session.notices`, `session.compaction` or `elicitation`). A reply with a protocol version cos doesn't speak ends the agent as `exited`, with the reason in `last_message`. The agent's `agentCapabilities` (`loadSession`, `sessionCapabilities.resume`/`close`, `promptCapabilities`, `mcpCapabilities`) and `authMethods` are kept in memory. |
| set up the session | `session/set_config_option {sessionId, configId, value}` after `session/new`, `session/load` or `session/resume` answers | It sets the option of category `mode`, then the one of category `model` (3.7), from the `configOptions` the answer returned. opencode's mode is cos's own `cos` agent (3.4), and Command Code's is `default`. An agent without `configOptions` is left as it starts. |
| first prompt | `session/prompt {sessionId, prompt: [{type: "text", text}]}` once the session is set up | The text is the prompt, plus the attachments block and the chief's spawned line, exactly as for Claude. For agents with no system-prompt hook it's preceded by the instructions preamble (3.5). |
| send | `session/prompt` | One turn at a time per session. A message sent while a turn runs waits in a FIFO in the daemon (at most 20; 409 past that) and goes as the next prompt once the turn ends, as Claude's own queue does. It is written to the transcript when it's sent, so the apps' "Sent · it reads this when it's free" works as today. The queue is in memory: a daemon restart drops it, as it drops Claude's. |
| interrupt | `session/cancel {sessionId}` (a notification) | The agent must end the pending `session/prompt` with `stopReason: "cancelled"`. cos answers any permission request still open for that session with `{outcome: {outcome: "cancelled"}}` first, as the spec requires. A queued message is sent after the cancelled turn ends. |
| stop | cancel if a turn runs (wait up to 2 s for the prompt to end), `session/close` if offered, close stdin, SIGTERM to the group after 1 s, SIGKILL to the group at the stop timeout (10 s) | Closing stdin and then killing is how ACP's stdio transport ends a session. The process group matters: opencode starts LSP servers and MCP servers as children **(unverified for Command Code)**. |
| resume | spawn, `initialize`, then `session/resume {sessionId, cwd, mcpServers}` when `sessionCapabilities.resume` is offered and cos has the transcript; else `session/load` (same params) when `loadSession` is true | `session/resume` (stable since April 2026) doesn't replay, which suits cos, since it keeps the transcript itself. `session/load` replays the whole conversation as `session/update` before it answers. cos drops the replay, unless its file is missing and it uses the replay to rebuild it (3.3). If neither is offered, or both fail, cos starts `session/new` and the transcript gets a note line. The agent has lost its context, and the chief's preamble is sent again. opencode 1.19.5 and Command Code 1.88.1 both offer `loadSession` and `resume`. |
| requests cos didn't offer | `fs/*`, `terminal/*` and any other agent → client request | Answered with JSON-RPC error `-32601` (method not found) and logged at debug. opencode 1.19.5 calls `fs/write_text_file` after an edit you allowed, without checking the capability and without waiting for the reply. It has already written the file itself, so the error is harmless, and the fake agent does the same. |
| daemon restart | ACP children die with the daemon (they hold pipes to it) | As for Claude: running agents become `interrupted`, a daemon that stops normally stops them first, and Chiefs of Staff come back by themselves through resume. |

**Session ids.** cos keeps choosing `agents.session_id` (a UUID) for every agent. For ACP it names cos's conversation and its transcript file, and `by_session` routing ignores it. ACP's own session id, chosen by the agent, goes in a new column `agents.harness_session`. New session (the chief's `…/staff/new-session`) gives a new cos session id, so a new transcript file and `agent.transcript {next: 0}`, as for Claude. Resume keeps the cos session id. The ACP id changes only when neither `session/resume` nor `session/load` works.

**States**

| State | `claude-terminal` (today) | ACP (all three, Claude included) |
|---|---|---|
| `starting` | spawned, no hook yet | spawned, until the session is set up after `session/new`, `session/resume` or `session/load` |
| `needs_trust` | trust dialog on screen | never. ACP has no trust dialog, and `cmd acp` parses no flags, so Command Code's `-t` doesn't apply **(unverified that `cmd acp` never prompts for trust some other way)** |
| `needs_login` | login screen on screen | a call fails with ACP's auth-required error, `-32000`: `session/new` or `session/load` for Command Code, `session/prompt` for opencode (3.7) |
| `idle` | `SessionStart`, `Stop` | session ready, or `session/prompt` answered (any `stopReason`) |
| `working` | `UserPromptSubmit`, `PreToolUse`, `PostToolUse` | prompt sent, `tool_call`, or `tool_call_update` to `in_progress` |
| `waiting_approval` | a `PermissionRequest` open | a `session/request_permission` open |
| `exited` | the process ended | the process ended, or the protocol failed (bad version, stdout closed). The reason goes in `last_message`. |
| `interrupted` | running when the daemon stopped | the same |

`last_message` is the turn's last `agent_message_chunk` text, joined, cut as today. `stopReason` values other than `end_turn` and `cancelled` (`max_tokens`, `max_turn_requests`, `refusal`) still end in `idle`. They're written to the transcript's `turn` line and, for `refusal`, appended to `last_message` as "(the model refused)".

**Notifications** (`session/update`, by `sessionUpdate`)

| `sessionUpdate` | cos |
|---|---|
| `agent_message_chunk` | text appended to the current assistant message. It's flushed to the transcript at the next update of another kind, at turn end, or past 64 KiB. Text blocks only. Other content blocks are noted as `[image]` and so on. |
| `user_message_chunk` | only during `session/load` replay (cos writes your messages itself), ignored unless rebuilding (3.3) |
| `agent_thought_chunk` | ignored, as Claude's thinking is left out today |
| `tool_call` | `ToolStarted` and a transcript `tool` line, with the name normalised as in 3.4. The fields are `toolCallId`, `title`, `name` (optional), `kind`, `status`, `content`, `locations` and `rawInput`/`rawOutput`. |
| `tool_call_update` | status `completed`/`failed`: `ToolFinished` and a `tool_result` line. Only changed fields come, and they update the open call: title, a `rawInput` that arrives late, content. |
| `usage_update` (`used`, `size`, `cost`) | a transcript `usage` line, so the context meter works (3.3). Stable in ACP since June 2026, and both CLIs send it. |
| `current_mode_update`, `config_option_update` | checked against the mode cos set. If it moved (an agent's `switch_mode` tool, or anything else), cos sets it back and notes it in the transcript, because modes such as Command Code's `auto-accept`, `dont-ask` and `bypass` stop requests from reaching cos (3.4). The current model is recorded in the agent's `model`. |
| `plan` | ignored in v1 (a later "plan" card) |
| `available_commands_update`, `session_info_update` | ignored |
| anything else (`notice`, `compaction_update` and the unstable kinds, which cos doesn't ask for) | ignored with a debug log line, like `Event::Other` |

**Events in the log:** no new kinds. `agent.started`, `agent.state_changed`, `agent.exited`, `approval.requested`, `approval.decided` and `agent.transcript` come from `on_event` as for Claude. `agent.started` gains `harness`. Streamed chunks never go in the event log, just as terminal output doesn't.

### 3.3 Transcript for ACP agents

cos writes it. The agents' own stores are no good for this: opencode keeps sessions in its own database and Command Code in its own JSONL. Both are undocumented and change between releases, and the `session/update` stream already carries everything the chat shows.

**Where:** `<cos home>/transcripts/<agent id>/<session id>.jsonl` (folder 0700, file 0600). `transcript_path` is set when the agent launches rather than from a hook, so `GET /v1/agents/{id}/transcript`, `agent_read` and the transcript cache need no new lookup. Removing an agent removes its folder. Removing a project or workspace removes its agents' folders.

**Format:** one JSON object per line, appended only, with the format named on the first line. The Claude parser stays as it is. `transcript::Parser` reads the first line and picks the cos parser when it sees `"format": "cos-transcript"`. So the cache, `Transcript`, `Message`, `next`, `context` and every reader stay unchanged.

```jsonc
{"type":"session","format":"cos-transcript","v":1,"harness":"opencode","harness_session":"ses_…","ts_ms":…}
{"type":"user","text":"Add a README line\n\nAttached files (saved by cos; …):\n- /…/shot.png (image/png, 245 KB)","ts_ms":…}
{"type":"assistant","text":"I'll add it.","ts_ms":…}
{"type":"tool","id":"call_1","name":"Bash","input":{"command":"npm test"},"kind":"execute","title":"npm test","ts_ms":…}
{"type":"tool_result","id":"call_1","is_error":false,"agent_id":null,"unasked":false,"output":"…at most 8 KiB…","ts_ms":…}
{"type":"usage","used_tokens":41203,"window_tokens":200000,"cost":{"amount":0.12,"currency":"USD"},"ts_ms":…}
{"type":"turn","stop_reason":"end_turn","ts_ms":…}
{"type":"note","text":"opencode couldn't load the old session; this is a new one without its history.","ts_ms":…}
```

- **`user`** is exactly what cos sent: your message with its attachments block, a chief's `[cos · N updates]` wakeup, or the spawned line. The reader runs it through the same `push_user_text`, so `update` rows, `attachments`, and replies in the desktop's quote format come out as for Claude. The instructions preamble (3.5) is never written.
- **`tool`** carries the normalised name and input (3.4), so the apps' icons and summaries work (`Bash` gets the terminal icon and its command). cos's own MCP tools are written as `mcp__cos__agent_spawn` and so on, so the existing `SPAWN_TOOL`/`AGENT_TOOLS` handling and the spawned-agent card work. `kind` and `title` are the agent's, kept for the apps (6.1).
- **`tool_result`** carries `agent_id` for `agent_spawn`, which cos resolves when it writes the line (from the call's `rawOutput` or text content, `{"agent_id": N, …}`). `unasked` marks a call that ran without a permission request although cos asked the agent to ask (3.4).
- **`usage`** comes from ACP's `usage_update`: `used` becomes `used_tokens`, `size` becomes `window_tokens`, and `cost` is kept as is (not shown yet). The newest `usage` line becomes the transcript's `context`. opencode and Command Code both send it. The unstable `usage` on `session/prompt`'s answer isn't used. When an agent sends `used` without `size`, cos strips a `provider/` prefix from the model and asks `context_window`, so `anthropic/claude-sonnet-4-5` is known. Agents that report nothing get `context: null`, and the apps hide the meter, as they already do before a first reply.
- **Message numbers** stay stable, because the file is append-only and the parser numbers rows in file order. A `tool` waiting for its `tool_result` sits after `next`, as for Claude.
- **`agent.transcript`** is logged after each write, through the same `Note` scheduler (`agents/transcripts.rs`), with `TranscriptGrew` as the signal in place of hooks. It's logged at most once a second, with no late re-read needed (cos wrote the line itself).

**Resume rebuilds it** in one of three ways:

1. **Normal case:** the file is there. cos uses `session/resume`, which doesn't replay, and new turns append to the same file. With only `session/load`, cos drops the replayed updates (everything before `session/load` answers) and appends as before.
2. **The file is missing or empty** (deleted, or a backup restored without it): cos uses `session/load` and writes the replay into a new file. `user_message_chunk` becomes `user`, chunks become `assistant`, and tool calls become `tool`/`tool_result`, with `ts_ms: 0` (the apps show no time) and `"replayed": true` on the session line.
3. **Neither resume nor load works:** cos writes a `note` line, starts a new ACP session, and keeps appending to the same file, so the chat stays one conversation even though the model lost its context. `note` lines are left out of the chat in v1.

### 3.4 Approvals

**Request model.** A pure function in `cos-harness/src/acp/tools.rs`, `normalise(kind, title, raw_input, locations, content) -> Vec<(tool, input)>`, turns an ACP tool call into one or more cos-policy requests in the Claude vocabulary that cos-policy already knows. The daemon builds an `ApprovalRequest` for each, classifies them with `classify_with(…, &Filesystem)`, and keeps the strictest verdict. cos-policy's request model doesn't change. Its only change is the list of protected config paths (writes never delegated), which gains `opencode.json`, `opencode.jsonc`, `.opencode/` and `.commandcode/`, as `.claude/` and `.mcp.json` are now. The same function names the tool rows in the transcript.

**Claude's calls need no guessing.** `claude-agent-acp` puts Claude's own tool name in `_meta.claudeCode.toolName` on every tool call (`Bash`, `Edit`, `mcp__cos__agent_spawn`, …), and its `rawInput` is Claude's tool input. When that name is there, `normalise` passes the name and input through unchanged. So a Claude agent's requests are classified exactly as `claude-terminal`'s are today, and its tool rows carry the same names. The table below is for agents that don't send it.

| ACP `kind` | cos-policy tool | Input taken from | If it can't tell |
|---|---|---|---|
| `execute` | `Bash` | `rawInput.command` as a string. An array, or `command` plus `args`, is shell-quoted and joined. | no command: `Bash` with no `command`, which asks you |
| `read` | `Read` | every path in `locations[]` and `rawInput` (`file_path`, `filePath`, `path`) | no path: asks you |
| `edit` | `Edit` | every path in `locations[]`, in `diff` content (`path`) and in `rawInput` | no path: asks you |
| `delete` | `Write` | every path | no path: asks you |
| `move` | `Write` | source and destination (`locations[]`, `rawInput`) | as above |
| `search` | `Grep` | `rawInput` `path` and `pattern`, else `locations[]`, else the agent's folder | |
| `fetch` | `WebFetch` | `rawInput.url` | no URL: asks you |
| `switch_mode` | none | | always asks you ("wants to switch to mode X"), since modes can turn approvals off |
| `think`, `other`, missing | a cos MCP tool → `mcp__cos__<name>` (below); another recognisable MCP tool → `mcp__<server>__<tool>` (which asks you, as today) | the call's `name` when the agent sends it, else `title` and `rawInput` | anything else: asks you ("an ACP tool cos can't read"). opencode reports its `task` tool (a subagent) as `think`. |

- **`kind` is the agent's hint, not a fact.** The input's shape wins. A `command` in `rawInput` is classified as `Bash` whatever the kind, and a request is classified `allowed` only when its kind and its input agree. A misreported `think` that carries a command therefore can't slip into the `allowed` tier, which jev may one day approve on its own.
- **`title` is display text.** It's shown, never classified.
- **`cwd`** is `rawInput.cwd`/`workdir` if present **(unverified field names)**, else the agent's folder. The worktree root and `home` are as today (approvals.md, "Daemon wiring").
- **The stored approval** has `tool_name` and `tool_input` as classified, so the apps' "Can I run this?" for `Bash`, the summaries and jev work unchanged. The agent's own call is kept under `tool_input._acp` (`{kind, title, rawInput, locations}`), which cos-policy ignores. `tool_use_id` is the ACP `toolCallId`: no pairing guesswork, unlike Claude's hook.

**Answering.** ACP offers options, not allow/deny:

- **Allow** sends `{outcome: {outcome: "selected", optionId}}` with the option whose `kind` is `allow_once`.
- **Deny** sends the option whose `kind` is `reject_once`.
- **Never `allow_always` or `reject_always`.** They make the agent remember a rule, after which its next request never reaches cos. Command Code remembers "always" per tool name for the thread.
- **What the agents offer:** `claude-agent-acp` orders its options `allow_once`, `allow_always`, then `reject_once`. opencode 1.19.5 offers `allow_once`, `allow_always` and `reject_once`, and treats `cancelled` as a rejection. Command Code offers all four, and only the two `_once` options for requests it flags as risky (`_meta.risk`, kept with the approval).
- **No `allow_once` offered:** neither CLI does this, but another agent might. Allowing then gets 409 "<harness> only offers 'always allow' for this; deny it instead". Denying with no `reject_once` sends `cancelled`. Both are logged. Section 11 has this as a decision.
- **Your denial message.** ACP's answer has no message field. If you typed one, cos sends it as your next message once the turn ends (`The user denied "<summary>": <message>`). The default "The user denied this." isn't sent.
- **Withdrawn:** a cancel, or the session ending, settles open approvals as `settled` with `decided_by` null, as an exit does today. Nobody can answer in a terminal, so `decided_by: terminal` never happens for ACP agents.

**Keeping config from approving first.** ACP itself has no equivalent of Claude's `--setting-sources project`. Each profile forces "ask" where the agent allows it, and cos checks after the fact:

- **Claude** (`claude-agent-acp`): the same rules as today. The adapter loads the `user`, `project` and `local` settings by default, and a client can override that through `_meta.claudeCode.options.settingSources`. cos passes `["project"]` and `strictMcpConfig: true` when the agent's `user_settings` is false, exactly as `claude-terminal` passes `--setting-sources project --strict-mcp-config`. With `user_settings: true` (the default, and every chief), your own allow rules approve what they cover, as they do now. The permission mode is the session's `mode` option (`default`, `acceptEdits`, `plan`, `dontAsk`), set from `permission_mode` and kept there.

- **opencode.** It deep-merges its config layers. From lowest to highest: remote and org config, the global `~/.config/opencode/opencode.json`, `OPENCODE_CONFIG`, the repo's `opencode.json`, `.opencode/` folders and `OPENCODE_CONFIG_DIR`, then `OPENCODE_CONFIG_CONTENT`, then managed `/etc/opencode` and macOS MDM. `OPENCODE_PERMISSION` is merged into the top-level `permission` last. But an agent's own `permission` block beats the top-level one, so a repo's `.opencode/agents/build.md` could allow `bash` for opencode's default agent. Setting `OPENCODE_PERMISSION` alone is therefore not enough. The profile:
  - defines its own primary agent, `cos`, in `OPENCODE_CONFIG_CONTENT`. It has no `prompt` (which would replace opencode's base prompt, 3.5) and a `permission` block that gives every key cos cares about a plain string: `edit`, `bash`, `webfetch`, `websearch`, `external_directory` and `doom_loop` set to `ask`. A plain string replaces whatever pattern map a lower layer had for that key; a pattern map would be merged with the repo's patterns, and the last match wins.
  - selects that agent as the session's mode (`session/set_config_option`, category `mode`) and puts it back if it moves (3.2). A repo can't override it, because `OPENCODE_CONFIG_CONTENT` sits above project config.
  - sets `OPENCODE_PERMISSION` to the same rules for the top level.
  - leaves `read`, `glob`, `grep` and `list` at opencode's defaults (allowed, but reading `.env` asks), as Claude's manual mode doesn't ask for reads. Reads outside the folder ask through `external_directory`.

  `OPENCODE_DISABLE_PROJECT_CONFIG=1` would shut out repo config completely, but it also skips the repo's `AGENTS.md` and `CLAUDE.md`, so it's the alternative in section 11, not the default. Managed config (`/etc/opencode`, MDM) is an administrator's and can still win. The check below catches it.
- **Command Code** has no config-dir variable, and `cmd acp` parses no flags. Its `permissions.allow` lists are unioned across `~/.commandcode/settings.json`, the repo's `.commandcode/settings.json` and `.commandcode/settings.local.json`. A deny anywhere wins, but a repo can add allow rules, and no documented switch ignores project settings. The profile sets the mode to `default` (Command Code's other modes are `auto-accept`, `plan`, `dont-ask` and `bypass`) and keeps it there (3.2). Beyond that, cos relies on the check below and on cos-policy making writes to those files never delegated (milestone 4). `GET /v1/harnesses` says so for Command Code. Whether `cmd acp` applies the project allow lists at all is **(unverified)**.
- **The check:** a `tool_call` of kind `execute`, `edit`, `delete`, `move` or `fetch` that reaches `in_progress` or `completed` with no `request_permission` for its `toolCallId` ran without cos. That is the ACP counterpart of Claude's `permission_decision.source`, which the spike used to catch settings approving things. cos marks the transcript row `unasked: true`, counts it on the agent (`unasked_tools`, 6.1), and logs a warning. The apps show the count on the agent and a mark on the row. cos doesn't stop the agent: the action has already happened, and you decide what to do. A Claude agent with `user_settings: true` is left out of the check, because there your own allow rules approving things is the point, as with `claude-terminal` today.

### 3.5 System prompt per backend

| | `claude-terminal` | `claude` (ACP) | opencode | Command Code and other ACP agents |
|---|---|---|---|---|
| Chief's standing instructions, identity, brief and notes | `--append-system-prompt` (unchanged) | `systemPrompt: {mode: "append", content: [{type: "text", text}]}` on `session/new`, `session/resume` and `session/load`. It's an unstable ACP field that the adapter offers as `sessionCapabilities.systemPrompt`, and any agent that offers it gets the same. | written to `<cos home>/prompts/<agent id>.md` (0600) and listed in `instructions` in `OPENCODE_CONFIG_CONTENT`. opencode appends `instructions` files to its system prompt after its base prompt, and concatenates the arrays across config layers, so the repo's own instructions stay too. That an absolute path outside the repo is accepted is **(unverified)**. An agent `prompt` isn't used: opencode 1.19.5 puts it in place of the provider's base prompt, which would drop its coding instructions. | a preamble in front of the first prompt of each ACP session, `<cos-instructions>…</cos-instructions>`, then your text. It's sent again after a new ACP session, when neither resume nor load worked. Command Code has no system-prompt option that cos could find. |
| Spawned line (`[cos] The Chief of Staff of <project> started you…`) | appended to the first prompt | the same | the same | the same |

- **Preamble limits.** It's a user turn, not a system prompt, so the model may weigh it less, and compaction may summarise it away in a long session. cos doesn't write it to the transcript, so the chat starts with your first message. This is risk 12.4.
- **The prompt text** gets a per-backend line in place of "your cos tools (mcp__cos__*)", which is Claude's naming. ACP agents name MCP tools their own way (opencode as `cos_agent_spawn`, **(unverified)**). The line becomes "your cos tools (the `cos` MCP server)". Both Claude harnesses keep today's text, since their tools are still named `mcp__cos__*`.
- **A line about harnesses** joins the chief's prompt: which harnesses are installed (from 3.8) and that `agent_spawn` takes `harness`. It's added for every backend, Claude included, so this is the one change to Claude chiefs' prompt, and it changes no behaviour.
- **Changes apply from the next start or new session**, as today. The prompt file is rewritten at every launch.

### 3.6 MCP per backend

- **`claude-terminal`:** unchanged (`--mcp-config`, `--allowedTools mcp__cos__*`, `--strict-mcp-config` only without user settings).
- **`claude` (ACP):** the `cos` server goes in `mcpServers` like any ACP agent's, and `_meta.claudeCode.options.allowedTools: ["mcp__cos__*"]` pre-approves its tools, exactly as `--allowedTools` does now. The adapter merges client servers with any it was given. `strictMcpConfig` follows `user_settings`, as in 3.4.
- **ACP:** `session/new`, `session/resume` and `session/load` carry `mcpServers: [{name: "cos", command: <cos exe>, args: ["mcp", "--agent", "<id>"], env: [{name: "COS_HOME", value: <home>}]}]` for a Chief of Staff, and an empty list otherwise. Stdio MCP servers are the one transport every ACP agent must support. `cos mcp --agent` is already a plain stdio server, so it needs no change. `Launch.mcp_config` stays Claude's JSON. The ACP list is built at launch from the role, as `staff::configure` does now.
- **Pre-approval of cos's tools.** The daemon's tools route already checks every call (caller role, project), which is why Claude pre-approves `mcp__cos__*`. For ACP:
  1. Where a profile can allow tools by name, it allows them. opencode: `permission` rules for the `cos` server's tools **(unverified syntax)**.
  2. Otherwise cos answers `allow_once` itself, recorded as `allowed` by `rule` as in the old run wiring. It does so only for a Chief of Staff, and only when the request normalises to a known staff tool from server `cos`: the call's `name` (or else its title) in one of the forms `mcp__cos__X`, `cos_X`, `cos/X`, and `rawInput` keys that match that tool's schema. Anything else asks you. Misidentifying a tool is the risk here (12.5), so the match is strict and tested against recorded calls.
- **The user's own MCP servers:** an ACP agent loads whatever its own config lists, next to cos's. `user_settings: false` can't strip those for ACP in v1 (6.1). In Command Code, its own server wins when names clash (section 10).

### 3.7 Models, login and context

**Model.** `agents.model` stays one string, read per harness:
- **`claude-terminal`:** `--model`, as now.
- **`claude` (ACP):** the same aliases (`sonnet`, `opus[1m]`, …), set through the `model` config option like any ACP agent, so `context_window` keeps working on the names.
- **ACP:** the generic way is the session's config option of category `model`. cos calls `session/set_config_option {configId, value}` with the model once the session is up (3.2). opencode's value is `provider/model`, and Command Code's is one of the values its option lists. ACP removed `session/set_model` in June 2026, and neither `opencode acp` nor `cmd acp` takes a model flag.
- **What it runs:** after setting the option, cos records the option's `currentValue` as the agent's `model`, so the apps show what runs. A model the agent refuses leaves its default running, with a transcript `note`.
- **opencode fallback:** if an opencode version lacks `configOptions`, the profile puts `model` in `OPENCODE_CONFIG_CONTENT` instead.

A chief's model is per project as now. Changing a chief's harness clears it, since names don't carry over, except between `claude` and `claude-terminal`, which share Claude's names. `COS_STAFF_MODEL` applies to Claude chiefs only, on either harness.

**Login (`needs_login`).** ACP signals a missing login with error `-32000` (auth required). `claude-agent-acp` refuses sessions and turns with it when Claude Code has no credential (it checks with `claude auth status`), and it recreates its Claude process after a login elsewhere, so a login from a shell is picked up. Command Code 1.88.1 returns it from `session/new` and `session/load` ("Not authenticated. Run `cmd login`…"). opencode 1.19.5 returns it from `session/prompt`, when the provider refuses ("provider authentication required"), and its `authenticate` does nothing. Then:
- **The state** becomes `needs_login`. Agent JSON gets `login_hint`: the command from a `terminal` auth method the agent listed (Command Code's is `cmd login`, offered because cos declares `auth.terminal`), else the profile's (`claude auth login`, `opencode auth login`), else the `authMethods` names. The shell it opens in has the project's environment, so `claude auth login` logs in the account that project's `CLAUDE_CONFIG_DIR` names (3.10).
- **cos retries the call that failed,** every 15 s for 30 minutes, on the same process: `session/new` or `session/load`, or the prompt, which never ran. Logging in from a shell therefore carries on by itself, as a Claude agent carries on after its login screen. After 30 minutes it gives up and ends `exited`, with the reason. Resume tries again. If an agent doesn't re-read its credentials in a running process **(unverified for opencode and Command Code; the Claude adapter does)**, the retry respawns it instead.
- **cos never calls `authenticate`, and never runs a terminal login method itself.** Those open a browser or need a terminal, so they're yours to run, in a project shell (3.9).

API keys keep working through cos's environment: `cos env set ANTHROPIC_API_KEY=… --project P` or `COMMAND_CODE_API_KEY` (which overrides `~/.commandcode/auth.json`) reach the agent like any other variable. opencode also reads keys from its `auth.json` (`opencode auth login`) and from a repo's `.env`. An agent that exits before answering `initialize` ends `exited`, with the last stderr lines in `last_message`, so a missing provider shows its reason.

**Context meter:** see `usage` in 3.3.

### 3.8 Choosing a harness, and finding the binaries

**Names.** An agent's *harness* is what runs it: `claude` (Claude Code through `claude-agent-acp`, the default), `claude-terminal` (Claude Code in a PTY with hooks, as today), `opencode` or `command-code`. Its *backend* follows from that: `pty` for `claude-terminal`, `acp` for the rest. The migration gives every existing agent `claude-terminal`, so they keep resuming as they do now. Custom ACP agents come later (milestone 10) as harnesses named in settings.

**Where it's chosen**

| Who | How | Default |
|---|---|---|
| You, for an agent | `POST /v1/workspaces/{id}/agents {harness?}`, `cos agent new --harness opencode`, the New session composer's picker | `claude` |
| A Chief of Staff, for an agent it starts | `agent_spawn {harness?}` | the chief's own harness, so an opencode chief starts opencode agents unless it names another |
| You, for a project's chief | `PUT /v1/projects/{id}/staff {harness}`, `cos staff --harness …`, the chief's ⋯ menu | `claude` |

- **The `claude` default** applies from milestone 6. Until then the default is `claude-terminal`, so nothing changes for anyone who doesn't pick a harness.
- **A chief's harness** changes from its next start. A session can't move between different agents, so that start is a new session: a new transcript, the brief and notes carried over, its agents still its own. The route says so in its answer. The exception is moving between `claude-terminal` and `claude`: both keep Claude's own session in the same config dir, so the chief resumes the same conversation (3.10).
- **An agent's harness** is fixed for life (`agents.harness`), and resume uses the same one. The one exception is `POST /v1/agents/{id}/resume {harness: "claude"}`, which moves an exited `claude-terminal` agent onto ACP, keeping its Claude session (3.10).
- **An uninstalled harness** gets 400 from `POST …/agents` and `agent_spawn`, with the reason from `GET /v1/harnesses`.

**Profiles** (`crates/cos/src/daemon/harness.rs`, data):

| | `claude` | `opencode` | `command-code` |
|---|---|---|---|
| Binary | `which::claude_acp()`: `$COS_CLAUDE_ACP_BIN`, `claude-agent-acp` on `PATH`, then npm and Homebrew places as for opencode (npm package `@agentclientprotocol/claude-agent-acp`; Node needed unless a single-file release binary is used) | `which::opencode()`: `$COS_OPENCODE_BIN`, `opencode` on `PATH`, then `~/.opencode/bin`, `~/.local/bin`, `~/.npm-global/bin`, nvm's newest Node, `/usr/local/bin`, `/opt/homebrew/bin` | `which::command_code()`: `$COS_COMMAND_CODE_BIN`, `cmd` on `PATH`, then npm and Homebrew places as for opencode |
| Arguments | none | `acp` (`--cwd` isn't needed: cos sets the folder) | `acp` (it parses no flags, so `-t`, `--skip-onboarding` and `--model` don't apply) |
| Model | config option `model` (Claude aliases) | config option `model` (`provider/model`); `model` in `OPENCODE_CONFIG_CONTENT` as the fallback | config option `model` |
| Mode | from `permission_mode` (`default`, `acceptEdits`, `plan`, `dontAsk`) | `cos`, cos's own agent | `default` |
| Permissions | settings sources as today: all with `user_settings`, else `project` only (3.4) | the `cos` agent's `permission` block in `OPENCODE_CONFIG_CONTENT`, and `OPENCODE_PERMISSION` (3.4) | the mode only; repo allow lists still apply, which the check catches |
| System prompt | `systemPrompt` append | `instructions` file | preamble |
| cos tools | `allowedTools: ["mcp__cos__*"]` | `permission` allow rules in the `cos` agent **(unverified syntax for MCP tools)**, else cos answers | cos answers |
| Login hint | `claude auth login` in a project shell **(unverified subcommand; else `claude` then `/login`)**, or `CLAUDE_CODE_OAUTH_TOKEN` in cos's environment | `opencode auth login`, or a provider key in cos's environment | `cmd login` (its terminal auth method), or `COMMAND_CODE_API_KEY` |
| Usage | `usage_update` | `usage_update` | `usage_update` |

- **`GET /v1/harnesses`** lists each harness, with `available`, the binary, the version (`--version`, cached 60 s), `backend`, `terminal`, and `reason` when it's unavailable. The apps use it for the picker, and the chief's prompt names the installed ones.
- **The `cmd` name check.** `cmd` is a generic name, so `command-code` counts as available only when `cmd --version` looks like Command Code's **(unverified output)**.
- **Command Code serves one folder per process** (a second `cwd` gets `-32602`). cos runs one process per agent anyway.
- **`cos daemon install`** records `COS_CLAUDE_ACP_BIN`, `COS_OPENCODE_BIN` and `COS_COMMAND_CODE_BIN` when set, as it does `COS_CLAUDE_BIN`. `service_path` adds the folders of `claude-agent-acp`, `opencode` and `cmd` when they're found only in their install places.
- **The environment** is unchanged: daemon < global < project < cos's own, for every harness. The names cos sets itself (`CLAUDE_CODE_EXECUTABLE` for the Claude adapter, and `OPENCODE_CONFIG_CONTENT`, `OPENCODE_PERMISSION` and `OPENCODE_CONFIG` if used for opencode) join `reserved_env_names`, refused with a reason, so a project value can't loosen the permission rules. Claude's session markers are removed for ACP agents too (harmless, and they still matter when the agent shells out to `claude`).

### 3.9 Shells

Shells are independent of the agent backend and keep working per project:

- `daemon/shells.rs` uses `ClaudeTerminal::spawn_program`, `TerminalView::Live` and `agents_api::terminal_stream`. All three stay as they are: the seam in 3.1 wraps `ClaudeTerminal`, it doesn't replace it.
- A shell gets the project's environment from `Environment::effective`, not from any agent. Nothing in shells reads `agents.harness`.
- `/v1/shells` stays in memory, limited to 32, served on the socket and the tailnet.
- The one new link is in the apps: an ACP agent waiting on `needs_login` offers **Open a terminal**, which opens a project shell (the existing Ctrl-J tab on the Mac, the shell screen on the iPhone) in the agent's folder, with its `login_hint` ready to run.
- Milestone 3's tests include starting a shell in a project while an ACP agent runs there, with the project's environment, and milestone 7 repeats it with an ACP chief, to check the claim above.

### 3.10 Claude through ACP

**What runs.** The chain is `claude-agent-acp` (npm `@agentclientprotocol/claude-agent-acp`, 0.98.0 checked), then Anthropic's Agent SDK, then the Claude Code CLI, headless. cos sets `CLAUDE_CODE_EXECUTABLE`, which the adapter honours, to the `claude` it finds today (`which::claude()`, `COS_CLAUDE_BIN`). Agents therefore run your installed Claude Code, with its version and its login, not the copy the SDK bundles. One adapter process serves one agent.

**Environment and several Claude accounts.** These work as they do now:
- **cos sets the environment as it does today.** It starts each agent's adapter with the daemon's environment, then the global variables, then the project's, then its own, exactly as it starts `claude-terminal` agents now.
- **The adapter passes it on.** It hands its whole environment to the Claude process it starts (the SDK's `env` is `{...process.env, …}`), and reads `CLAUDE_CONFIG_DIR` itself when it looks for Claude's files.
- **So the project picks the account.** `cos env set CLAUDE_CONFIG_DIR=~/.claude-personal --project topflexi` makes topflexi's agents and chief use that account while other projects keep `~/.claude`. `CLAUDE_CODE_OAUTH_TOKEN` and `ANTHROPIC_API_KEY` work the same way.
- **Accounts never mix.** Each agent has its own adapter process, so two projects never share one. Variables are per project, as now. A running agent keeps the environment it started with, and stop and resume pick up a change.
- **Subscription logins work.** The adapter has a `--hide-claude-auth` flag that refuses claude.ai subscription billing. cos doesn't pass it.
- **Logging in** uses the same environment. `needs_login` opens a project shell, which has that project's `CLAUDE_CONFIG_DIR`, so `claude auth login` (or `claude` and `/login`) there logs in the right account. The adapter picks the login up without a restart (3.7).
- **One unchanged detail:** a `settings.json` `env` block in a config dir still applies on top of the process environment, as Claude Code does today.

**What cos passes** in `_meta.claudeCode.options` on `session/new`, `session/resume` and `session/load`:
- `settingSources`: `["user", "project", "local"]` with `user_settings` (the adapter's default), else `["project"]`;
- `strictMcpConfig` when `user_settings` is false;
- `allowedTools: ["mcp__cos__*"]` for a chief.

These are the same three things `claude-terminal` passes as flags today. On top, the session gets its mode from `permission_mode`, its model through the `model` option, and a chief's prompt as `systemPrompt` in `append` mode (3.5).

**Sessions and moving from `claude-terminal`.**
- **Resume:** the adapter offers `loadSession` and `session/resume`, and resumes from Claude's own JSONL in the config dir.
- **Session ids:** ACP's session id appears to be Claude's own session id. When the CLI never saved a session, the adapter starts a new one under the same id **(unverified)**.
- **Moving an agent or chief over:** that shared id is what lets a `claude-terminal` agent or chief move to `claude` without losing its conversation. cos resumes the Claude session through ACP (`session/resume` with its id, or `session/new` with `_meta.claudeCode.options.resume`). Then it rebuilds its own transcript from a `session/load` replay (3.3, case 2), so the chat shows the conversation from before the move.
- **Routes:** for agents, `POST /v1/agents/{id}/resume {harness: "claude"}`. For a chief, `PUT …/staff {harness: "claude"}` takes effect at its next start, which resumes rather than starting a new session.

**What stays the same as `claude-terminal`:**
- approvals classified by the exact Claude tool name and input (3.4);
- your own allow rules applying when `user_settings` is on;
- cos's MCP tools pre-approved for chiefs;
- models by Claude alias;
- the context meter, now exact, since `usage_update` gives the window size;
- the transcript and its `agent_spawn` cards, which keep their names;
- login through each project's `CLAUDE_CONFIG_DIR`.

**What changes:**
- **No terminal:** no watching or typing into Claude's UI, no `!` commands, no trust banner (headless Claude has no trust dialog), no `agent_read {screen: true}`. Slash commands the adapter forwards (`available_commands_update`) can still be typed as messages.
- **`AskUserQuestion` is off.** The adapter disables it unless the client supports ACP's form elicitation, which cos doesn't declare in v1. Claude asks in plain text instead, which reads fine in the chat.
- **The chief's prompt on resume.** The adapter keeps a session's system prompt across resumes, and a different one takes effect only at the next compaction. A brief or notes change therefore reaches a resumed chief later than with `--append-system-prompt`, and New session (already on the Identity & memory page) applies it at once. See risk 12.12.
- **`cos sessions`** may not list headless SDK sessions, if they don't write `sessions/<pid>.json` **(unverified)**. cos's own agents are listed by the apps from the database either way.

**Installing it.** `npm install -g @agentclientprotocol/claude-agent-acp`, which needs Node. The README shows a `@preview` channel published on every push to `main`. Whether a stable tag or single-file release binaries exist for the new package is **(unverified)**. `GET /v1/harnesses` reports the version, and cos refuses versions older than the one it was tested with.

## 4. Alternatives considered

| Option | Verdict |
|---|---|
| **One ACP backend, cos's own client** (chosen) | One protocol for opencode, Command Code and later agents. A small client (initialize, session/new, resume, load, set_config_option, prompt and cancel, request_permission, notifications) parsed leniently, like `claude/protocol.rs`: unknown updates are ignored, never errors. |
| The official Rust SDK (`agent-client-protocol`, with its types in `agent-client-protocol-schema` 1.13) | Not for now. It's typed from the schema, but cos uses a handful of messages and wants unknown variants tolerated rather than rejected, as for Claude. cos runs on tokio's multi-threaded runtime, and the SDK's connection types have used `!Send` futures that need a `LocalSet` **(unverified for the current version)**. Unstable parts sit behind cargo features that move (`set_model` was removed in June 2026). The schema crate can still check the recorded fixtures as a dev-dependency, and `schema/v1/schema.json` is the reference. |
| A backend per CLI (`opencode serve`'s HTTP API, `cmd -p`) | Rejected (your decision). It's two protocols to keep up with instead of one, and other agents get nothing. |
| Run the CLIs' TUIs in a PTY and read the screen | Rejected (your decision). Neither has per-session hooks like Claude's, so states and approvals would come from screen reading, which breaks on every UI change. |
| Read each CLI's own transcript store | Rejected. opencode's is a database and Command Code's an undocumented JSONL, both private and changing. The ACP stream already has the conversation, so cos records it. |
| Write ACP transcripts in Claude's JSONL shape | Rejected. It imitates an undocumented format and its quirks (`queued_command`, sidechains, `<synthetic>`) for no gain. A header-tagged cos format with the same `Message` output keeps readers unchanged. |
| Claude through ACP (`claude-agent-acp`) | **Chosen on 28 Oct 2026** as the default for Claude (3.10). It costs the terminal, which you rarely use, and adds a Node adapter between cos and Claude. In return, every agent goes through one pipeline, and Claude's tool names and inputs reach cos-policy unchanged. |
| Claude through ACP only, deleting the PTY path now | Not yet. Existing agents and chiefs resume through it, and it's the fallback if the adapter misbehaves. Section 11 has retiring it as a later decision. |
| cos speaking Claude's headless stream-json itself (`ClaudeSession`, as `cos try` does) instead of the adapter | Rejected. It would be a second protocol for one agent, and cos would have to track the Agent SDK's undocumented control messages itself. The adapter does that and is maintained by the ACP project. |
| cos as the agent's file system and terminal (`fs/*`, `terminal/*`) | Later, maybe. cos would see each command as structured data and could run it in a PTY, a terminal view for free. But it's a large surface (running and streaming commands, writes through cos), it changes where commands run, and it isn't needed for what you asked. |
| A daemon-side enum instead of a trait | Close. With two backends an enum is simpler, but you asked for a seam that a later Codex fits, and `dyn AgentSession` keeps the daemon from matching on backends everywhere. |

## 5. Affected components

| File | New / modified | Notes |
|---|---|---|
| `crates/cos-harness/src/agent.rs` | new | `AgentSession`, `AgentEvent`, `PermissionReply`. |
| `crates/cos-harness/src/claude/agent.rs` | new | `ClaudeAgent` over `ClaudeTerminal`; `end_session` moves here from the daemon. |
| `crates/cos-harness/src/claude/hooks.rs` | modified | `event(&Hook) -> Option<AgentEvent>`; `Hook` becomes a `PermissionReply` source. |
| `crates/cos-harness/src/acp/{mod,rpc,protocol,tools}.rs` | new | stdio JSON-RPC line codec, lenient message types, `AcpOptions`, `AcpAgent`, tool normalisation (3.4). |
| `crates/cos-harness/src/bin/cos-fake-acp/` | new | The fake ACP agent (section 8). Reuses the fake Claude's MCP client (`mcp.rs`), moved to a shared module. |
| `crates/cos-harness/tests/fixtures/acp/` | new | Recorded, anonymised `opencode acp` and `cmd acp` sessions. |
| `crates/cos-harness/src/lib.rs` | modified | Exports; the "trait comes later" note goes. |
| `crates/cos-harness/src/claude/mod.rs` | modified | `reserved_env_names` gains the opencode variables. |
| `crates/cos/src/daemon/agents.rs` | modified | `Running.session: Arc<dyn AgentSession>`; `launch` dispatches on harness; `on_event`; queue for ACP sends; `interrupt`; 409 `no_terminal`. |
| `crates/cos/src/daemon/agents/hooks.rs` | modified | Claude routing in front of `on_event`; state logic moves into `on_event`. |
| `crates/cos/src/daemon/agents/acp.rs` | new | Launch, resume and stop for ACP agents; the transcript writer; the "unasked" check; login retries. |
| `crates/cos/src/daemon/agents/approvals.rs` | modified | Builds requests from normalised ACP calls and keeps the strictest; option choice. |
| `crates/cos/src/daemon/agents/transcripts.rs` | modified | `TranscriptGrew` as a signal besides hooks. |
| `crates/cos/src/daemon/transcript.rs` | modified | Format detection by the first line; the cos-transcript parser; `provider/` prefix in `context_window`. |
| `crates/cos/src/daemon/harness.rs` | new | Profiles, `GET /v1/harnesses`, version probes. |
| `crates/cos/src/which.rs` | modified | `opencode()`, `command_code()`, their places, `service_path`. |
| `crates/cos/src/service/*` | modified | Records `COS_CLAUDE_ACP_BIN`, `COS_OPENCODE_BIN`, `COS_COMMAND_CODE_BIN`. |
| `crates/cos/src/daemon/agents_api.rs` | modified | `harness` on start; `POST …/interrupt`; `no_terminal` codes. |
| `crates/cos/src/daemon/staff.rs`, `staff/prompt.rs`, `staff/tools.rs`, `staff_api.rs` | modified | Chief's harness; prompt delivery per backend; `agent_spawn {harness}`; `workspaces_list` shows `harness`; `agent_read` without a screen. |
| `crates/cos/src/mcp.rs` | modified | `agent_spawn`'s schema gains `harness`. |
| `crates/cos-core/src/agents.rs`, `store.rs`, `staff.rs` | modified | One migration: `agents.harness` (existing rows `claude-terminal`; new agents default to `claude`), `agents.harness_session`, `agents.unasked_tools`, `staff.harness` (existing chiefs `claude-terminal`). |
| `crates/cos/src/agents_cmd.rs`, `staff_cmd.rs`, `try_cmd.rs` | modified | `--harness`; `cos try --acp opencode "…"` drives an ACP agent headless and prints its events. |
| `clients/desktop/src/…` | modified | Section 9. |
| `clients/shared/CosKit/…`, `clients/iphone/Cos/…` | modified | Section 9. |
| `docs/acp-protocol.md` | new (milestone 2) | What cos verified about ACP and each CLI, like `claude-protocol.md`. |
| `docs/agents-api.md`, `docs/daemon.md`, `docs/chief-of-staff.md`, `docs/approvals.md`, `docs/desktop.md`, `docs/iphone.md`, `docs/proposal.md` | modified | Contract and Progress, with each milestone. |
| `crates/cos-policy/src/paths.rs` | modified | `opencode.json(c)`, `.opencode/`, `.commandcode/` join the config paths that are never delegated to write. |

`daemon/shells.rs` doesn't change.

## 6. API / contract changes

### 6.1 Agents

**Agent JSON gains:**

```jsonc
{
  "harness": "opencode",          // "claude" | "claude-terminal" | "opencode" | "command-code"; "claude-terminal" for every agent from before
  "backend": "acp",               // "acp" | "pty"
  "has_terminal": false,          // the terminal, screen, keys, resize and trust routes work
  "harness_session": "ses_…",     // the agent's own session id (ACP); null for Claude
  "login_hint": "opencode auth login",  // with needs_login; null otherwise
  "unasked_tools": 0              // tool calls that ran without asking although cos asked the agent to ask
}
```

For ACP agents, `permission_mode` is `default` (anything else is 400 in v1). `user_settings` is kept but has no effect (3.6, section 11). `rows` and `cols` keep their defaults.

**Transcript `Message.tool` gains** optional `kind` and `title` (ACP only), for the apps' icons and full titles.

**Routes**

| Route | Change |
|---|---|
| `POST /v1/workspaces/{id}/agents` | Body gains `harness?`. 400 for an unknown or unavailable harness. |
| `POST /v1/agents/{id}/input` | ACP: sends, or queues while a turn runs; 409 with more than 20 waiting. |
| `POST /v1/agents/{id}/interrupt` | **New**, for both backends: ends the running turn (Claude: Esc; ACP: `session/cancel`). 202. 409 unless `working` or `waiting_approval`. |
| `GET …/terminal`, `GET …/screen`, `POST …/keys`, `…/resize`, `…/trust` | ACP: 409 `{"error": "agent 7 runs opencode, which has no terminal", "code": "no_terminal"}`. |
| `POST /v1/agents/{id}/stop`, `DELETE …` | Unchanged contract; 3.2 for how. |
| `POST /v1/agents/{id}/resume` | Body may be `{harness: "claude"}` to move an exited `claude-terminal` agent onto ACP with its Claude session (3.10). Any other change of harness is 400. Without a body it's unchanged. |
| `GET /v1/agents/{id}/transcript` | Unchanged contract; reads either format. |
| `GET /v1/harnesses` | **New**: `{harnesses: [{id, backend, available, terminal, binary, version, reason, note}]}`. `reason` says why it's unavailable. `note` is what you should know before using it, such as "repo allow lists in `.commandcode/settings.json` can approve tools without cos". On the socket and the tailnet. |

**Events:** `agent.started` gains `harness`. No new kinds.

**CLI:** `cos agent new … --harness H`, `cos staff [project] --harness H`, `cos harnesses`, `cos agent interrupt <id>`, `cos try --acp opencode|command-code "…"`.

### 6.2 Chief of Staff

- `PUT /v1/projects/{id}/staff` takes `harness`. `Staff` shows `harness`. The change applies from the next start, as a new session (3.8). The model is cleared when the harness changes.
- `agent_spawn` takes `harness?`, defaulting to the chief's own. `workspaces_list` shows each agent's `harness`.
- `agent_read {screen: true}` on an ACP agent returns `screen: null` with `"screen_note": "no terminal (opencode)"`.

### 6.3 Backward compatibility

- **Older apps, new daemon:** they ignore the new fields. An ACP agent's terminal routes answer 409, which they already show as "not running" or an error. The terminal toggle stays visible and empty in those apps. Clients decode agents leniently (CosKit, the desktop store), so nothing breaks.
- **New apps, older daemon:** no `harness` means `claude-terminal` and `has_terminal` true. `GET /v1/harnesses` answers 404, so the picker shows only Claude. `…/interrupt` answers 404, so the apps fall back to sending Esc through `keys`.
- **Database:** one migration. Existing agents and chiefs get `harness = 'claude-terminal'`, so they resume exactly as before. `unasked_tools` starts at 0. Only agents started after the default switch (milestone 6) are `claude`.
- **The CLI:** `cos agent new` without `--harness` follows the daemon's default, so after the switch it starts ACP Claude. `--harness claude-terminal` gives the terminal.
- **Claude transcripts** parse exactly as before. The format check only looks for the cos header.

## 7. Implementation plan (milestones)

Each milestone is one PR that passes `make test` and `make lint` (and `make ui-test ui-lint ui-typecheck` for UI work) by itself, and updates the docs it changes.

1. **The seam, Claude behind it, no behaviour change.** `AgentSession`, `AgentEvent`, `PermissionReply`, `ClaudeAgent`. `Running.session`, `on_event`, and `end_session` moved into the harness. Existing tests pass unedited. New unit tests cover `claude::hooks::event` and the recorded hook sequences.
2. **The ACP client in cos-harness.** `acp::{rpc, protocol, tools}`, `AcpAgent`, and `cos-fake-acp` (section 8). Harness tests against the fake: handshake, config options, prompt, chunks, tool calls, permission answers with `allow_once`/`reject_once`, cancel, resume, load with replay, auth-required, unoffered client methods, protocol errors, process-group stop. Also `cos try --acp`, and recorded `opencode acp` and `cmd acp` fixtures checked by unit tests. `docs/acp-protocol.md` holds what was verified, and section 12's unverified items are settled here where they can be. No daemon change.
3. **The daemon runs ACP agents.** The migration, `harness` on start, launch, the mode and model options (and setting the mode back), send with its queue, interrupt, stop and resume, states, `needs_login` with retries, the transcript writer and the cos-transcript reader with `context`, `agent.transcript`, `no_terminal` codes, `/v1/harnesses`, and the shell check from 3.9. The only ACP harness is a test-only `fake-acp` until milestone 5, so nothing changes for users yet. Until milestone 4, permission requests from ACP agents are answered `reject_once` and logged, so nothing runs unapproved.
4. **ACP approvals.** Normalisation into cos-policy requests (strictest wins, kind versus input), approvals with `_acp`, `allow_once`/`reject_once`, follow-up denial messages, the "unasked" check with its row mark and agent count, the new protected config paths in cos-policy, and jev in shadow mode on the mapped requests.
5. **Profiles: Claude, opencode and Command Code.** `which::claude_acp`, `which::opencode`, `which::command_code`, `COS_*_BIN`, `cos daemon install`. Also `CLAUDE_CODE_EXECUTABLE`, the `_meta.claudeCode.options` (settings sources, strict MCP), the permission config, modes, models, login hints, version probes, and the reserved env names. A live run per harness with a cheap model (haiku for Claude): start, a turn, an approval answered through cos, interrupt, stop, resume into the same session, and a daemon restart. For Claude, also two projects with different `CLAUDE_CONFIG_DIR`s, each running on its own account, and a login from a project shell picked up without a restart. `claude-terminal` stays the default.
6. **Claude through ACP as the default.** New agents and chiefs default to `claude`; existing ones stay `claude-terminal`. Also `resume {harness: "claude"}` and the chief's harness switch, both keeping the Claude session, with the transcript rebuilt from the replay. A live check: a `claude-terminal` agent moved to `claude` carries on its conversation, and so does a chief. This is a small PR on its own, so it can wait for a week of use of milestone 5.
7. **Chief of Staff on ACP.** `staff.harness`, the prompt per backend (`systemPrompt` append, instructions file or preamble), `mcpServers` with `cos mcp`, pre-approval of cos tools, `agent_spawn {harness}`, `workspaces_list`, `agent_read` without a screen, transcript names for cos tools (the spawned-agent card). A live check: a `claude` chief and an opencode chief each create a workspace and spawn an agent that commits a line. Milestones 6 and 7 can land in either order: until both have, a chief moved to `claude` gets its prompt and tools through 7.
8. **Mac app** (section 9). Against the fake bridge, then by hand against a sandbox daemon.
9. **iPhone and CosKit** (section 9).
10. **Later, separately:** retiring `claude-terminal` (the hooks, the hook socket, the screen checks for trust and login, the Claude JSONL parser), if it goes unused (section 11). Also custom ACP agents as named harnesses in settings (`PUT /v1/harnesses/{name} {command, args, env}`). Also plans as cards, image prompt blocks when `promptCapabilities.image`, ACP session modes for `plan`, and `user_settings: false` for ACP.

## 8. Test plan

All in Docker (`make test`, `make lint`, `make ui-*`), except the native Swift and Tauri builds and the live runs.

**`cos-fake-acp`** (`crates/cos-harness/src/bin/cos-fake-acp/`), beside `cos-fake-claude`. It speaks ACP on stdio, and what it does depends on the prompt, as the fake Claude's does:

- `echo` (default): streams `agent_message_chunk`s of `echo: <prompt>` in three chunks, then `end_turn`.
- `ask:<kind> <arg>`: a `tool_call` of that kind, then `session/request_permission` with `allow_once`, `allow_always`, `reject_once` and `reject_always`, then `completed` or `failed` from the answer. `ask-always-only:<kind>` offers only the `always` options.
- `run:<kind> <arg>`: a tool call that completes without asking, which should trip the "unasked" check.
- `mcp <tool> <json>`: calls the `cos` server from `mcpServers` over stdio, through the MCP client shared with the fake Claude, and reports the call as `tool_call`s named the way opencode names MCP tools.
- `usage <used> <size>`: sends a `usage_update`.
- `edit-write <path>`: asks for an edit and, once allowed, calls `fs/write_text_file` without waiting, as opencode does.
- `mode <id>`: sends `current_mode_update` to another mode, as if the agent switched by itself.
- `slow`: a turn that runs until cancelled, to test `session/cancel`.
- `crash`: exits mid-turn.
- `configOptions` with a `mode` and a `model` option on every session, recording what cos sets.
- `COS_FAKE_ACP_CLAUDE=1` plays `claude-agent-acp`:
  - tool calls carry `_meta.claudeCode.toolName` and Claude-shaped `rawInput`;
  - it offers `sessionCapabilities.systemPrompt`;
  - it records `_meta.claudeCode.options` and the `systemPrompt` it gets;
  - it resumes by session id from the fake Claude's transcript folder.
- Environment switches:
  - `COS_FAKE_ACP_NO_RESUME` and `COS_FAKE_ACP_NO_LOAD` take away `sessionCapabilities.resume` and `loadSession`;
  - `COS_FAKE_ACP_AUTH=<file>` answers `-32000` until the file exists, with `COS_FAKE_ACP_AUTH_ON=new|prompt` choosing where (Command Code's way or opencode's);
  - `COS_FAKE_ACP_VERSION` answers another protocol version;
  - `COS_FAKE_ACP_SESSIONS` is where sessions are kept for `session/load` replay;
  - `COS_FAKE_ACP_RECORD` writes its argv, environment, `initialize` params, `mcpServers` and config-option calls to a file for assertions.

**Harness** (milestone 2): every message pair above. Lenient parsing (unknown `sessionUpdate`, extra fields, non-JSON stdout lines skipped). Stdout closing mid-request. Cancel answering open permissions with `cancelled`. Stop killing a child the fake started (process group). Recorded `opencode acp` and `cmd acp` fixtures parsed without loss.

**Daemon** (milestones 3 to 7, `agents/tests.rs` and `staff/tests.rs` with `AgentConfig::for_tests` pointing the `fake-acp` test harness at the fake):

- start → `idle` → input → `working` → `idle`, with the transcript's user and assistant rows and `agent.transcript` logged;
- input while working is queued and sent after, in order; the 21st waiting gets 409;
- interrupt during `slow` → `idle`, and an open approval becomes `settled`;
- stop → `exited`; daemon shutdown → `interrupted`; resume → `session/resume`; with no resume capability → `session/load` with the replay not duplicated; with neither → a `note` and a new ACP session in the same file; with the transcript file deleted → `session/load` rebuilds it from the replay;
- the mode and model options are set after every `session/new`, `session/resume` and `session/load`, and a mode the agent switches by itself is set back;
- `fs/write_text_file` is answered `-32601` and the turn carries on;
- `needs_login` while the auth file is missing, both on `session/new` and on the prompt, then `idle` (or the prompt runs) once it exists, without a restart;
- approvals: each kind maps to the expected tool, tier and summary; a `think` call carrying a command isn't `allowed`; allow sends `allow_once` and deny `reject_once` (never `always`); always-only gets 409 on allow; a denial message arrives as the next prompt; `run:execute` sets `unasked` and `unasked_tools`;
- terminal, screen, keys, resize and trust answer 409 `no_terminal`, and `agent_read {screen: true}` answers with `screen_note`;
- Claude through ACP (the fake in its Claude mode):
  - requests are classified exactly as the same `Bash`/`Edit` requests from `claude-terminal`;
  - `user_settings: false` sends `settingSources: ["project"]` and `strictMcpConfig`;
  - the process gets the project's `CLAUDE_CONFIG_DIR` and cos's `CLAUDE_CODE_EXECUTABLE`;
  - `resume {harness: "claude"}` on an exited `claude-terminal` agent resumes its session id and rebuilds the chat from the replay;
  - the migration leaves existing agents on `claude-terminal`;
- the chief on `fake-acp`: `mcpServers` carries `cos mcp --agent <id>`, its cos tools are allowed without an approval, `agent_spawn` from it defaults to its harness, the spawned-agent card's `agent_id` is in the transcript, and the preamble or instructions file carries its brief and notes;
- shells: `POST /v1/shells {project_id}` in a project with a running `fake-acp` agent (and, from milestone 7, an ACP chief) starts, gets the project's variables, and streams;
- every existing Claude test unchanged.

**Apps:** Vitest against the fake bridge, which gains an opencode agent and chief: no terminal toggle, the login card opening a shell, the harness picker, the unasked badge, tool kind icons. CosKit and view-model tests on `FakeCos`.

**By hand, once per CLI:** milestone 5's live runs, the move from `claude-terminal` in milestone 6, the `claude` and opencode chiefs in milestone 7, and the Mac app and iPhone against a sandbox daemon (`COS_HOME=$PWD/.sandbox/home`).

## 9. Mac app and iPhone

Both decide from `has_terminal`, which is true when it's missing, never from the harness name.

**Mac app** (`clients/desktop`)
- **Agent page** (`AgentPage.tsx`): with no terminal, the `ViewSwitch`, the terminal button and Ctrl-Shift-J are hidden, and the page is the chat only. `terminalOnly` (`:128`) is true only for a Claude chief. An ACP chief's agent page shows its transcript, or links to its chief page.
- **`needs_login`** doesn't force a terminal open for ACP. It shows a card with `login_hint` and **Open a terminal**, which opens a project shell tab in the agent's folder (3.9).
- **Composer:** a **Stop** button while `working` or `waiting_approval` calls `…/interrupt` (Esc through `keys` on older daemons). It's useful for `claude-terminal` agents too.
- **New session sheet and chief ⋯ menu:** a harness picker from `GET /v1/harnesses`: Claude (the default), Claude with terminal, opencode, Command Code. Unavailable ones are disabled, with their reason. A model field has a per-harness placeholder (`sonnet`, `anthropic/claude-sonnet-4-5`, …). The chief's harness change warns that it starts a new session, except between the two Claude harnesses, which keep the conversation.
- **An exited `claude-terminal` session** gets "Resume without terminal" beside Resume, which is `resume {harness: "claude"}`.
- **Sidebar, chief header and session cards:** a small harness badge for non-Claude agents, and a warning mark with `unasked_tools` when it's above 0.
- **Tool rows:** the icon from `kind` when present, and `title` in the expanded row. Approvals need no change: `tool_name` is already `Bash`, `Edit` and so on.
- **Fake bridge:** an opencode agent and an opencode chief (no terminal, a context meter, an unasked row) and a Command Code agent in `needs_login`.

**iPhone** (`clients/iphone`, CosKit)
- **CosKit:** `Agent` gains `harness`, `backend`, `hasTerminal` (default true), `harnessSession`, `loginHint` and `unaskedTools`; `ToolCall` gains `kind` and `title`; `CosService` gains `harnesses()` and `interrupt(agentId:)`. Decoding stays lenient.
- **Session chat:** the Transcript | Terminal switch, the terminal buttons (`SessionChatView.swift:35,46,100,140`) and "Open its terminal" are hidden without a terminal. The empty state says "It shows here once opencode has written something."
- **Terminal screen:** the chips that switch between a chief and its live sessions leave out ACP agents.
- **Waiting on you:** a `needs_login` ACP agent shows `login_hint` and opens a shell on the machine.
- **New session and the chief's Identity & memory page:** the harness picker, as on the Mac.
- **Stop:** the same Stop while working.

## 10. Edge cases

| Case | What happens |
|---|---|
| The binary is missing | `GET /v1/harnesses` says why; starting is 400 with that reason. An agent whose binary vanished before a resume ends `exited` with the spawn error. |
| The agent exits before answering `initialize` | `exited`, with the last stderr lines in `last_message`. |
| Unknown protocol version in the answer | `exited`: "opencode speaks ACP version N; cos speaks 1." |
| Non-JSON on stdout (a stray log line) | Skipped, with a debug line, like a bad transcript line. |
| Stdout closes mid-turn | The turn ends, open approvals become `settled`, and the process is reaped: `exited` with its code. |
| A permission request for an unknown `toolCallId` | Classified from its own `toolCall` fields, as any request is. |
| A permission request after cos sent cancel | Answered `cancelled` at once and recorded as `settled`. |
| Two requests open at once | Each is its own approval. `waiting_approval` holds until both are answered. |
| Huge tool output | Kept to 8 KiB in `tool_result.output`, which the chat doesn't show in v1 anyway. |
| A message while `starting` or `needs_login` | Queued, and sent once the session is ready. A chief's inbox already waits for `idle`. |
| Resume fails (neither `session/resume` nor `session/load` works) | A `note`, a new ACP session, the preamble again, and the chief's brief and notes as they are now. |
| The daemon restarts while an approval waits | The approval becomes `settled` and the agent `interrupted`; a chief resumes. The agent may then re-ask, as a fresh request. |
| The workspace folder is removed while an agent runs | Refused, as today (409 while an agent runs). |
| Attachments | The same text block of paths. ACP image blocks come later. |
| The user's or the repo's opencode config allows `bash` | cos's own `cos` agent sets `bash` to `ask` in `OPENCODE_CONFIG_CONTENT`, above both (3.4). If managed config overrides it, the "unasked" check catches and shows it. |
| The agent writes an `AGENTS.md` or a config file | cos-policy already makes writes to `AGENTS.md`, `CLAUDE.md`, `.cos/` and `.env*` never delegated. `opencode.json(c)`, `.opencode/` and `.commandcode/` join that list in milestone 4, since a repo-level allow rule there would loosen the next session. |
| The agent switches its own mode | A `switch_mode` request asks you. A mode change cos didn't ask for (`current_mode_update`) is set back and noted. |
| Command Code has an MCP server named `cos` in its own config | Its own server wins a name clash, so a chief would get the wrong tools. cos checks the chief's first tool calls reach the daemon's tools route, and the profile notes the clash in `GET /v1/harnesses` **(unverified how a clash shows)**. |
| opencode calls `fs/write_text_file` after an edit | Answered `-32601`. The edit is already on disk. |
| Many ACP agents | Each is a process plus its MCP children, with no PTY. There's no new cap beyond what agents have today. |

## 11. Decisions needed from the user

The recommended option is first.

- [ ] **Field names:** `harness` (`claude`, `claude-terminal`, `opencode`, `command-code`) with a derived `backend` (`acp`, `pty`) and `has_terminal`, or a single `backend` field naming the CLI.
- [ ] **Claude's default:** `claude` (ACP) for new agents and chiefs after milestone 6, with existing ones left on `claude-terminal` until you move them; or move every exited `claude-terminal` agent and chief to `claude` automatically at its next resume.
- [ ] **Retiring `claude-terminal`:** keep it as the way to get Claude's terminal; or remove it (milestone 10) once nothing has used it for a while, which deletes the hooks, the hook socket, the screen checks and the Claude JSONL parser.
- [ ] **Installing the Claude adapter:** you install `@agentclientprotocol/claude-agent-acp` with npm, and cos finds it as it finds `claude`; or cos's installers (`install.sh`, `cos-agent` on npm) install it too.
- [ ] **`agent_spawn`'s default harness:** the chief's own, or always `claude`.
- [ ] **cos's own ACP client** (lenient, tokio), or the `agent-client-protocol` crate.
- [ ] **The transcript:** cos's header-tagged JSONL in `<cos home>/transcripts/` read into the same messages, or Claude-shaped JSONL.
- [ ] **No `allow_once` offered:** refuse allowing through cos (409, deny still works), or pick `allow_always` and accept that the agent stops asking for that kind of request.
- [ ] **Denial messages:** sent as your next message after the turn, or dropped (ACP has no field for them).
- [ ] **System prompt for agents without a hook:** the first-prompt preamble, or no standing instructions for those chiefs (Command Code chiefs not supported).
- [ ] **Unasked tool calls:** marked and counted (and shown), or the agent stopped on the first one.
- [ ] **Repo config for opencode:** cos's own `cos` agent with plain-string rules over the repo's config, so the repo's `AGENTS.md` and instructions still load; or `OPENCODE_DISABLE_PROJECT_CONFIG=1`, which shuts the repo's config out completely, `AGENTS.md` and `CLAUDE.md` included.
- [ ] **Command Code's repo allow lists:** ship it with the "unasked" check and a note in `GET /v1/harnesses`, or hold Command Code back until it can ignore project permissions.
- [ ] **Permission modes for ACP in v1:** `default` only, or also `plan` through ACP session modes where the agent offers one.
- [ ] **`user_settings` for ACP:** no effect in v1, documented, or left out of the API for ACP agents.
- [ ] **Reserved env names:** refuse `OPENCODE_CONFIG_CONTENT`, `OPENCODE_PERMISSION` (and `OPENCODE_CONFIG`) in cos's environment, or merge a user's value under cos's rules.
- [ ] **Custom ACP agents:** a later milestone (10), or in the first release.

## 12. Open questions and risks

1. **opencode's config layers (risk).** The precedence in 3.4 comes from opencode's source at 1.19.5, but one detail isn't proven: that a plain-string `permission` key in `OPENCODE_CONFIG_CONTENT` replaces a pattern map a repo set for that key, rather than merging with it. Milestone 5 tests each layer live: global, repo `opencode.json`, repo `.opencode/agents/*.md`, and agent-level blocks. Managed config (`/etc/opencode`, MDM) can always win. The "unasked" check is the backstop either way. `OPENCODE_CONFIG_DIR` adds a folder rather than replacing the default, so it can't isolate the user's config.
2. **opencode only asks for `ask` rules.** Rules that resolve to `allow` or `deny` never reach the client, which is why cos's agent sets `ask` for everything that matters. Reads are allowed by default (`.env` asks), as in Claude's manual mode. The `cos` agent's MCP allow rule syntax is **(unverified)**.
3. **Command Code (closed source).** cos depends on its ACP behaviour as observed in the 1.88.1 bundle, which can change in any release. Facts from that bundle: `cmd acp` parses no flags, offers `loadSession` and `resume`, returns `-32000` when logged out, sends `usage_update`, and has the modes `default`, `auto-accept`, `plan`, `dont-ask` and `bypass`. What remains is that repo allow lists can approve tools, which cos can only detect (see the Command Code decision in section 11). Still **(unverified)**: whether `cmd acp` applies those lists, whether it needs onboarding when used fresh, and how an MCP name clash shows. Milestone 5 pins the version it was checked with in `docs/acp-protocol.md`.
4. **The preamble isn't a system prompt.** Without a hook, a chief's instructions are a user turn that compaction may drop in a long session. Mitigations, to pick after watching a live chief: declare ACP's `session.compaction` client capability so the agent reports compactions (`compaction_update`), then re-send the preamble with the next prompt; or re-send it every N turns.
5. **Recognising cos's MCP tools in ACP calls.** Pre-approving them for a chief relies on matching names and inputs the agent reports in its own format. A false match would auto-allow a different tool. Mitigation: strict matching (server `cos`, a known tool name, input keys that match the schema), tests from recorded calls, and the tools route's own checks behind it.
6. **Usage depends on the agent.** `usage_update` is stable and both CLIs send it, but ACP doesn't require it. An agent that doesn't send it gets no context meter.
7. **`kind` and `rawInput` are agent-defined.** File and command fields differ per agent (`filePath` versus `file_path`). Normalisation needs per-agent field names, learned from fixtures. Anything unrecognised asks you, which is safe but noisy.
8. **Process trees.** opencode starts language servers and MCP servers. If the process-group kill misses one (setsid in a child), it outlives the agent. A test with the fake starting a child covers the common case.
9. **Protocol churn.** ACP is versioned (version 1, with a v2 schema in the works). cos relies only on stable methods (`session/resume` and `usage_update` became stable in 2026), each with a fallback, and nothing unstable. `session/set_model` was removed in June 2026, which shows the stable surface still moves. The fixtures are rechecked when the schema crate changes.
10. **No terminal means no escape hatch.** If an ACP agent hangs on something interactive, you see only its state and stderr, not a screen. That's the trade-off you chose. `GET /v1/agents/{id}` with `last_message` holding the stderr tail, and a project shell to run the CLI by hand, are the fallbacks.
11. **The model name per harness.** A chief's model doesn't carry over when its harness changes (except between the two Claude harnesses), so it's cleared. A per-harness default (`COS_STAFF_MODEL_OPENCODE`) can come later if needed.
12. **The Claude adapter is a large dependency that moves fast.** `claude-agent-acp` is about 14,000 lines in its main file, built on an Agent SDK that changes with Claude Code, and its README points at a preview channel. It needs Node. Behaviour cos relies on: `CLAUDE_CODE_EXECUTABLE`, `_meta.claudeCode.options`, `_meta.claudeCode.toolName`, the unstable `systemPrompt` field, and session ids matching Claude's. Mitigations: a minimum version checked by `GET /v1/harnesses`, recorded fixtures from the adapter, a live run per release of cos, and `claude-terminal` kept as the fallback. Two details to settle live: that a chief's changed brief really waits for compaction on resume (else cos starts a new session when its prompt changed), and how the adapter answers a `session/resume` for a Claude session it never saw (the move from `claude-terminal`).
