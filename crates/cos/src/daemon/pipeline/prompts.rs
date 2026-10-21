//! What the pipeline tells workers and reviewers. The rules go in
//! `--append-system-prompt`, the same for every task, so a resume passes
//! them again unchanged; the task itself is the first prompt.

use cos_core::{DiffSummary, Task, TaskChecks, TestsStatus};

/// The worker's rules.
pub(crate) const WORKER_SYSTEM: &str = "\
You are a worker agent started by cos, a chief of staff for coding agents. You do one task, \
alone, in your own git worktree (the current folder), on a branch cos made for the task. \
Nobody is watching to answer questions, so make reasonable choices yourself.

Rules:
- Change only the files listed for the task (paths or globs relative to the repository root). \
Any other change fails the task's scope check.
- Never touch .git/, .claude/, .codex/, .cos/, .env files or .mcp.json.
- Commit your work with git as you go, with clear commit messages. Leave nothing uncommitted \
or untracked when you finish: uncommitted work is thrown away. Don't push, switch branches, \
rebase or merge.
- If the task has a test command, run it and make it pass before you finish.
- Finish the whole task in this one turn.
- End your final message with a line that says exactly DONE, or STUCK: <why> if you can't \
finish the task.";

/// The reviewer's rules.
pub(crate) const REVIEWER_SYSTEM: &str = "\
You are a reviewer agent started by cos, a chief of staff for coding agents. You review one \
task's changes in its git worktree (the current folder). You are read-only: read files and use \
git log, git show and git diff, but don't edit anything or run commands that change anything. \
You are in plan mode only to keep you read-only: don't call ExitPlanMode and don't write a \
plan; write your review as your reply. Nobody is watching to answer questions.

Check that the change does what the task asks, stays within the task's files, is correct, \
is tested where it should be, and leaves no debugging code or stray files behind. Small \
style points are not a reason to ask for changes.

Finish the review in this one turn. End your final message with exactly one line, either \
VERDICT: APPROVE or VERDICT: CHANGES: <one paragraph of what to fix>.";

/// Sent when a worker's turn ended without a verdict line.
pub(crate) const WORKER_ASK_VERDICT: &str =
    "Please end your reply with a line that says exactly DONE, or STUCK: <why>.";

/// Sent when a reviewer's turn ended without a verdict line.
pub(crate) const REVIEWER_ASK_VERDICT: &str =
    "Please end your reply with exactly one line: VERDICT: APPROVE or VERDICT: CHANGES: <what to fix>.";

/// Sent to a resumed worker whose turn the daemon's restart cut short.
pub(crate) const WORKER_CONTINUE: &str = "cos restarted while you were working. Carry on with \
the task where you left off: check git status and git log, finish the work, commit it, and end \
with DONE or STUCK: <why>.";

/// The worker's first prompt.
pub(crate) fn worker_task(task: &Task) -> String {
    let mut prompt = format!("Task: {}\n\n{}\n\n", task.title, task.instructions.trim());
    prompt.push_str("Files you may change:\n");
    for file in &task.files {
        prompt.push_str(&format!("- {file}\n"));
    }
    match &task.test_command {
        Some(command) => prompt.push_str(&format!(
            "\nTest command: `{command}`. Run it and make it pass before you finish.\n"
        )),
        None => prompt.push_str("\nThis task has no test command.\n"),
    }
    if task.retries > 0
        && let Some(reason) = &task.last_failure
    {
        prompt.push_str(&format!(
            "\nThis is a second attempt. The first one was sent back because:\n{reason}\n"
        ));
    }
    prompt.push_str("\nCommit your work, then end with DONE or STUCK: <why>.");
    prompt
}

/// Typed into the worker when its task comes back for another attempt.
pub(crate) fn worker_retry(task: &Task, reason: &str) -> String {
    let mut prompt = format!(
        "Task \"{}\" needs another pass.\n\n{}\n\nFix this, ",
        task.title,
        reason.trim()
    );
    if let Some(command) = &task.test_command {
        prompt.push_str(&format!("run `{command}`, "));
    }
    prompt.push_str("commit, and end with DONE or STUCK: <why>.");
    prompt
}

/// The reviewer's first prompt.
pub(crate) fn reviewer_task(
    task: &Task,
    base_commit: &str,
    summary: &DiffSummary,
    diff: &str,
    diff_cut: bool,
) -> String {
    let mut prompt = format!(
        "Review task: {}\n\nWhat the worker was asked to do:\n{}\n\nFiles it may change:\n",
        task.title,
        task.instructions.trim()
    );
    for file in &task.files {
        prompt.push_str(&format!("- {file}\n"));
    }
    prompt.push_str(&format!(
        "\nThe changes are the commits since {base_commit}: see them with `git log {base_commit}..HEAD` \
         and `git diff {base_commit}...HEAD`.\n\n{} files changed, {} insertions, {} deletions:\n{}\n",
        summary.files_changed, summary.insertions, summary.deletions, summary.stat
    ));
    prompt.push_str(&format!("\nTests: {}\n", tests_line(task)));
    prompt.push_str("\n```diff\n");
    prompt.push_str(diff.trim_end());
    prompt.push_str("\n```\n");
    if diff_cut {
        prompt.push_str("(The diff is cut short here; read the rest with git diff.)\n");
    }
    prompt.push_str("\nEnd with VERDICT: APPROVE or VERDICT: CHANGES: <what to fix>.");
    prompt
}

fn tests_line(task: &Task) -> String {
    let Some(command) = &task.test_command else {
        return "the task has no test command.".into();
    };
    match task
        .checks
        .as_ref()
        .and_then(|checks: &TaskChecks| checks.tests.as_ref())
    {
        Some(tests) if tests.status == TestsStatus::Passed => format!("`{command}` passed."),
        Some(tests) => format!("`{command}` {}.", tests.status.as_str()),
        None => format!("`{command}` was not run."),
    }
}

/// The last `max` characters of `text`, starting on a line boundary when
/// there is one.
pub(crate) fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.trim_end().to_owned();
    }
    let cut: String = text.chars().skip(count - max).collect();
    let cut = cut.split_once('\n').map_or(cut.as_str(), |(_, rest)| rest);
    format!("…\n{}", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_from_a_line_start() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("line one\nline two\nline three\n", 15), "…\nline three");
    }
}
