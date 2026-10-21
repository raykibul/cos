//! What the chief is told: its role (`--append-system-prompt`), its first
//! prompt, its MCP server, and the wakeups the daemon types into it.

use std::fmt::Write;
use std::path::Path;

use cos_core::chief::cut;
use cos_core::{Board, InboxItem, Run};
use cos_harness::ClaudeOptions;
use serde_json::json;

/// The name of the chief's MCP server, so its tools are `mcp__cos__…`.
pub(crate) const SERVER: &str = "cos";
/// The allow rule that lets the chief call every tool of its server without
/// asking: cos checks each call itself.
pub(crate) const ALLOW_TOOLS: &str = "mcp__cos";
const GOAL_IN_BRIEF: usize = 300;

/// The chief's role, appended to Claude's system prompt. Run details go in
/// the first prompt, so a resumed chief gets the same role.
pub(crate) const ROLE: &str = "\
You are the chief of staff for a cos run. cos is a daemon that runs coding agents for a user. \
The user gave the run one goal. Your job is judgment: understand the goal, plan it as small tasks, \
react to what happens, and report. cos does the bookkeeping: it starts one worker per task in its \
own git worktree, checks that the worker only changed the task's files, runs the task's test \
command, has a reviewer read the diff, and merges.

Rules:
- Never edit, create or delete files yourself, and never run commands that change anything. \
Explore read-only (Read, Grep, Glob, and read-only shell commands such as ls or git log).
- Act through your cos tools (mcp__cos__*): board_get, board_propose, task_update, task_send, \
ask_user, approval_decide and report_write.

Planning:
- Explore just enough to plan well, then call board_propose once with the whole plan.
- Each task is one small change that can be reviewed and merged on its own. Give it a short unique \
title, instructions a worker can follow without asking (what to change, where, and how to know \
it's done), and the exact files it may change (paths or globs relative to the repo root). Two \
tasks that may run at the same time must not share a file; use depends_on (other tasks' titles) \
to order them instead.
- Give each task a test_command when the repo has a quick, relevant test (for example \
`cargo test -p foo`); leave it out when there is none.
- If board_propose returns problems, fix all of them and propose again.
- After proposing, stop. The user approves the plan; cos then runs the tasks.

Wakeups:
- cos types a wakeup message into this session when something needs you: tasks that merged, \
blocked or failed, approvals routed to you, the user's answers and messages, and the run reaching \
the report. Each wakeup starts with a short brief of the run. Your earlier context may be \
compacted, so trust the brief and board_get over memory.
- A blocked task: read its reason (board_get), then task_update with retry (each task gets one \
retry, so say in the reason what the worker should do differently), fail, or cancel. Ask the user \
with ask_user when the choice needs their judgment.
- An approval routed to you: allow it only if the task really needs it and it stays inside the \
task's worktree; otherwise deny. Give a one-line reason. cos enforces the user's limits whatever you \
decide.
- ask_user returns at once; the answer comes in a later wakeup. Don't wait in a loop.
- When the run reaches the report, call report_write once with a short markdown report: what was \
done (task by task), what wasn't and why, and anything the user should check.
- End your turn after acting on a wakeup. Don't poll.";

/// The first prompt, typed when the chief starts.
pub(crate) fn first_prompt(run: &Run, workspace: &Path, branch: &str) -> String {
    format!(
        "You are the chief of cos run {id}.\n\
         Goal: {goal}\n\
         Workspace: {path} (branch {branch}); every task branches from and merges back into it.\n\
         Explore the repository read-only, then call board_propose with your plan. After proposing, stop and wait.",
        id = run.id,
        goal = run.goal,
        path = workspace.display(),
    )
}

/// The MCP config for a chief: one server, `cos`, that runs
/// `<cos> mcp --agent <id>` against this daemon.
pub(crate) fn mcp_config(cos: &Path, home: &Path, agent_id: i64) -> String {
    json!({
        "mcpServers": {
            SERVER: {
                "command": cos,
                "args": ["mcp", "--agent", agent_id.to_string()],
                "env": { "COS_HOME": home },
            }
        }
    })
    .to_string()
}

/// Gives a chief agent its role, its MCP server and the rule allowing its
/// tools. Called for every launch, so a resumed chief gets them too.
pub(crate) fn configure(claude: &mut ClaudeOptions, cos: &Path, home: &Path, agent_id: i64) {
    claude.append_system_prompt = Some(ROLE.into());
    claude.mcp_config = Some(mcp_config(cos, home, agent_id));
    if !claude.allowed_tools.iter().any(|tool| tool == ALLOW_TOOLS) {
        claude.allowed_tools.push(ALLOW_TOOLS.into());
    }
}

/// A wakeup: a brief of the run, then the items, oldest first.
pub(crate) fn wakeup(
    board: &Board,
    open_questions: usize,
    items: &[InboxItem],
    still_waiting: usize,
) -> String {
    let run = &board.run;
    let goal = cut(run.goal.lines().next().unwrap_or_default().trim(), GOAL_IN_BRIEF);
    let mut text = String::new();
    let _ = writeln!(text, "[cos wakeup] Run {} is {}. Goal: {goal}", run.id, run.state);
    let _ = writeln!(
        text,
        "Board: {}. Open questions to the user: {open_questions}.",
        board.counts.summary()
    );
    let _ = writeln!(text, "New, oldest first:");
    for (n, item) in items.iter().enumerate() {
        let _ = writeln!(text, "{}. [{}] {}", n + 1, item.kind.as_str(), item.text);
    }
    if still_waiting > 0 {
        let _ = writeln!(text, "({still_waiting} more waiting for the next wakeup.)");
    }
    text.push_str("Act on these with your cos tools (board_get for details), then end your turn.");
    text
}
