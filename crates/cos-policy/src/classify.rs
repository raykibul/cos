//! Sorts a request into a tier. Claude's file tools are checked here; Bash
//! commands go through [`crate::bash`].

use std::path::Path;

use serde_json::Value;

use crate::paths::{Lexical, Location, PathProbe, PathRules};
use crate::request::{ApprovalRequest, Role};
use crate::shell::{self, Node};
use crate::verdict::{Needs, Verdict};

/// Prefix of the tools cos's own MCP server gives the chief.
const COS_TOOL_PREFIX: &str = "mcp__cos__";

/// Classifies `request`, taking paths at face value (no symlinks followed).
pub fn classify(request: &ApprovalRequest) -> Verdict {
    classify_with(request, &Lexical)
}

/// Classifies `request`, following symlinks through `probe`. Pass
/// [`crate::Filesystem`] when the worktree is on disk.
pub fn classify_with(request: &ApprovalRequest, probe: &dyn PathProbe) -> Verdict {
    if !request.worktree_root.is_absolute() {
        return Verdict::ask("the worktree path isn't absolute, so no path can be checked");
    }
    let context = Context::new(request, probe);
    let input = &request.input;
    match request.tool.as_str() {
        "Bash" => match input.get("command").and_then(Value::as_str) {
            Some(command) => crate::bash::classify_command(&context, command),
            None => Verdict::ask("the Bash request has no command"),
        },
        "Read" | "NotebookRead" => context.file(input, &["file_path", "notebook_path"], false),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            context.file(input, &["file_path", "notebook_path"], true)
        }
        "Grep" | "Glob" | "LS" => context.search(input),
        "WebFetch" => match input.get("url").and_then(Value::as_str) {
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => {
                Verdict::chief(Needs::NETWORK, format!("fetches {url}"))
            }
            _ => Verdict::ask("fetches something that isn't an http(s) URL"),
        },
        "WebSearch" => Verdict::chief(Needs::NETWORK, "searches the web"),
        "Task" | "Agent" | "TodoWrite" | "TodoRead" | "BashOutput" | "KillShell" | "KillBash"
        | "TaskOutput" | "TaskStop" => Verdict::allowed("manages the agent's own work and touches no files"),
        "ExitPlanMode" | "AskUserQuestion" => Verdict::ask("asks for your input"),
        tool if tool.starts_with(COS_TOOL_PREFIX) => {
            if request.role == Role::Chief {
                Verdict::allowed("one of the chief's own cos tools")
            } else {
                Verdict::ask("only the chief uses cos's own tools")
            }
        }
        tool if tool.starts_with("mcp__") => Verdict::ask(format!("{tool} is an MCP tool cos doesn't know")),
        tool => Verdict::ask(format!("cos doesn't know the {tool} tool")),
    }
}

/// What the checks need from the request, worked out once.
pub(crate) struct Context<'a> {
    pub request: &'a ApprovalRequest,
    pub paths: PathRules<'a>,
    /// The task's test command, one word list per simple command in it.
    pub tests: Vec<Vec<String>>,
}

impl<'a> Context<'a> {
    fn new(request: &'a ApprovalRequest, probe: &'a dyn PathProbe) -> Self {
        let paths = PathRules::new(
            &request.worktree_root,
            request.home.as_deref(),
            request.scope.as_deref(),
            probe,
        );
        let tests = request
            .test_command
            .as_deref()
            .and_then(|text| shell::parse(text).ok())
            .map(|script| {
                script
                    .items
                    .iter()
                    .filter_map(|item| match &item.node {
                        Node::Command(command) => command.words.iter().map(|w| w.literal()).collect(),
                        _ => None,
                    })
                    .filter(|words: &Vec<String>| !words.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            request,
            paths,
            tests,
        }
    }

    fn cwd(&self) -> Option<&Path> {
        let cwd = self.request.cwd.as_path();
        cwd.is_absolute().then_some(cwd)
    }

    fn file(&self, input: &Value, keys: &[&str], write: bool) -> Verdict {
        let Some(path) = keys
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
        else {
            return Verdict::ask(format!("the request has no {}", keys[0]));
        };
        let location = self.paths.locate(self.cwd(), path, false);
        if write {
            write_verdict(&location, path, self.request.role)
        } else {
            read_verdict(&location, path)
        }
    }

    fn search(&self, input: &Value) -> Verdict {
        let base = input.get("path").and_then(Value::as_str);
        let mut verdict = match base {
            Some(path) => read_verdict(&self.paths.locate(self.cwd(), path, false), path),
            None => match self.cwd() {
                Some(cwd) => {
                    let shown = cwd.display().to_string();
                    read_verdict(&self.paths.locate(None, &shown, false), &shown)
                }
                None => Verdict::ask("searches a folder cos can't work out"),
            },
        };
        // An absolute Glob pattern names its own folder.
        if let Some(pattern) = input.get("pattern").and_then(Value::as_str)
            && (pattern.starts_with('/') || pattern.starts_with('~') || pattern.contains(".."))
        {
            let base = base.map(Path::new).or(self.cwd());
            verdict = verdict.and(read_verdict(&self.paths.locate(base, pattern, true), pattern));
        }
        verdict
    }
}

/// The tier for reading a path.
pub(crate) fn read_verdict(location: &Location, shown: &str) -> Verdict {
    match location {
        Location::Device | Location::Inside { .. } => Verdict::allowed("reads files in the worktree"),
        Location::Secret { path, kind } => Verdict::never(format!("reads {path}, {kind}")),
        Location::Outside { path, via_symlink } => Verdict::never(format!(
            "reads {path}, outside the worktree{}",
            symlink_note(*via_symlink)
        )),
        Location::Unknown => Verdict::ask(format!("reads {shown}, which cos can't resolve")),
    }
}

/// The tier for creating, changing or deleting a single path.
pub(crate) fn write_verdict(location: &Location, shown: &str, role: Role) -> Verdict {
    match location {
        Location::Device => Verdict::allowed("writes to a device such as /dev/null"),
        Location::Secret { path, kind } => Verdict::never(format!("changes {path}, {kind}")),
        Location::Outside { path, via_symlink } => Verdict::never(format!(
            "changes {path}, outside the worktree{}",
            symlink_note(*via_symlink)
        )),
        Location::Unknown => Verdict::ask(format!("changes {shown}, which cos can't resolve")),
        Location::Inside {
            rel,
            protected: Some(kind),
            ..
        } => Verdict::never(format!("changes {}: {kind}", shown_rel(rel))),
        Location::Inside { .. } if role == Role::Reviewer => Verdict::ask("reviewers don't edit files"),
        Location::Inside {
            rel, in_scope: false, ..
        } => Verdict::chief(
            Needs::WORKTREE,
            format!("changes {}, outside the task's scope", shown_rel(rel)),
        ),
        Location::Inside { .. } => Verdict::allowed("edits files in the worktree, inside the task's scope"),
    }
}

pub(crate) fn shown_rel(rel: &str) -> &str {
    if rel.is_empty() { "the worktree root" } else { rel }
}

fn symlink_note(via_symlink: bool) -> &'static str {
    if via_symlink { " (through a symlink)" } else { "" }
}
