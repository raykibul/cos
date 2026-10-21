//! The file-scope check: after a worker finishes, every file its branch
//! changes must be on the task's allowed list, and none may be a protected
//! path.

use serde::{Deserialize, Serialize};

use super::{TaskGitError, TaskWorktree, run};
use crate::git;

/// Folder names no task may touch, at any depth, compared case-insensitively
/// because macOS file systems are: git internals, agent settings and cos's own
/// files. Changing them could grant an agent more permissions.
const PROTECTED_DIRS: &[&str] = &[".git", ".claude", ".codex", ".cos"];
/// File names no task may touch, at any depth: MCP servers Claude starts.
const PROTECTED_FILES: &[&str] = &[".mcp.json"];
/// Any file or folder whose name starts with this is protected: secrets.
const PROTECTED_PREFIX: &str = ".env";

/// A task's allowed files, as globs relative to the repo root.
///
/// - `*` matches within one path segment, `?` one character of it, and a
///   segment that is exactly `**` matches any number of segments, including
///   none. Everything else, `[` included, is literal, so `app/[id].tsx` means
///   that file.
/// - Patterns are anchored at the root: `*.md` is `README.md` but not
///   `docs/a.md`; write `**/*.md` for that. A leading `./` or `/` is ignored.
/// - A pattern that matches a folder covers everything in it: `src/auth`,
///   `src/auth/` and `src/auth/**` are the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    patterns: Vec<Vec<Segment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    AnyDepth,
    Glob(Vec<char>),
}

impl Scope {
    /// Refuses empty patterns and ones that climb out with `..`.
    pub fn new<I, S>(patterns: I) -> Result<Self, TaskGitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let patterns = patterns
            .into_iter()
            .map(|pattern| parse(pattern.as_ref()))
            .collect::<Result<_, _>>()?;
        Ok(Self { patterns })
    }

    /// Whether `path` (repo-relative, `/`-separated) is on the list. Protected
    /// paths are a separate check; see [`is_protected`].
    pub fn allows(&self, path: &str) -> bool {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.patterns
            .iter()
            .any(|pattern| (1..=segments.len()).any(|depth| matches(pattern, &segments[..depth])))
    }
}

fn parse(pattern: &str) -> Result<Vec<Segment>, TaskGitError> {
    let trimmed = pattern.trim();
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    let segments: Vec<Segment> = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(|segment| match segment {
            "**" => Segment::AnyDepth,
            other => Segment::Glob(other.chars().collect()),
        })
        .collect();
    if segments.is_empty() {
        return Err(TaskGitError::Invalid(format!(
            "file pattern {pattern:?} is empty"
        )));
    }
    if segments.contains(&Segment::Glob(vec!['.', '.'])) {
        return Err(TaskGitError::Invalid(format!(
            "file pattern {pattern:?} points outside the repo"
        )));
    }
    Ok(segments)
}

/// Whether the whole of `path` matches the whole of `pattern`.
fn matches(pattern: &[Segment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::AnyDepth, rest)) => (0..=path.len()).any(|skip| matches(rest, &path[skip..])),
        Some((Segment::Glob(glob), rest)) => path
            .split_first()
            .is_some_and(|(first, others)| matches_segment(glob, first) && matches(rest, others)),
    }
}

/// `*` and `?` within one segment, by the usual backtracking to the last `*`.
fn matches_segment(glob: &[char], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let (mut g, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match glob.get(g) {
            Some('*') => {
                star = Some((g, n));
                g += 1;
            }
            Some('?') => {
                g += 1;
                n += 1;
            }
            Some(&c) if c == name[n] => {
                g += 1;
                n += 1;
            }
            _ => match star {
                Some((star_g, star_n)) => {
                    g = star_g + 1;
                    n = star_n + 1;
                    star = Some((star_g, star_n + 1));
                }
                None => return false,
            },
        }
    }
    glob[g..].iter().all(|&c| c == '*')
}

/// Whether no task may change `path`, whatever its scope says: anything in
/// `.git/`, `.claude/`, `.codex/` or `.cos/`, any `.env*` file or folder, and
/// `.mcp.json`, at any depth and in any letter case.
pub fn is_protected(path: &str) -> bool {
    path.split('/').any(|segment| {
        let segment = segment.to_lowercase();
        PROTECTED_DIRS.contains(&segment.as_str())
            || PROTECTED_FILES.contains(&segment.as_str())
            || segment.starts_with(PROTECTED_PREFIX)
    })
}

/// One file a task branch changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub status: ChangeStatus,
    /// The old path of a rename or the source of a copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    /// A file became a symlink or the other way round.
    TypeChanged,
}

/// The result of [`TaskWorktree::check_scope`]. Each violating path is listed
/// once: under `protected` if it is protected, else under `out_of_scope`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeReport {
    /// Everything the task branch changes since its base commit.
    pub changes: Vec<Change>,
    /// Changed, renamed-away or deleted paths not on the allowed list.
    pub out_of_scope: Vec<String>,
    /// Changed paths no task may touch.
    pub protected: Vec<String>,
    /// Modified or untracked files the worker didn't commit. Ignored files
    /// don't count.
    pub uncommitted: Vec<String>,
    /// The branch the worktree is on, if the worker switched away from the
    /// task branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrong_branch: Option<String>,
}

impl ScopeReport {
    pub fn is_ok(&self) -> bool {
        self.out_of_scope.is_empty()
            && self.protected.is_empty()
            && self.uncommitted.is_empty()
            && self.wrong_branch.is_none()
    }
}

impl TaskWorktree {
    /// Checks what the worker did. The task's changes are the committed
    /// difference `<base commit>...<task branch>`, which is what a merge would
    /// bring in. A rename touches both its old and new path, so both must be
    /// allowed. Uncommitted work is never merged, so leaving any is a
    /// violation of its own: the worker must commit.
    pub async fn check_scope(&self, scope: &Scope) -> Result<ScopeReport, TaskGitError> {
        let range = format!("{}...{}", self.base_commit, self.branch);
        let diff = run(
            &self.workspace,
            &["diff", "--name-status", "-z", "-M", "--no-color", &range],
        )
        .await?;
        let changes = parse_name_status(&diff)?;

        let mut out_of_scope = Vec::new();
        let mut protected = Vec::new();
        for change in &changes {
            // A copy leaves its source alone, so only its new path counts.
            let source = change
                .from
                .as_ref()
                .filter(|_| change.status == ChangeStatus::Renamed);
            for path in source.into_iter().chain([&change.path]) {
                if is_protected(path) {
                    push_once(&mut protected, path);
                } else if !scope.allows(path) {
                    push_once(&mut out_of_scope, path);
                }
            }
        }

        let status = run(
            &self.path,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .await?;
        let uncommitted = parse_porcelain(&status);

        let on = git::current_branch(&self.path).await?;
        Ok(ScopeReport {
            changes,
            out_of_scope,
            protected,
            uncommitted,
            wrong_branch: (on != self.branch).then_some(on),
        })
    }
}

fn push_once(list: &mut Vec<String>, path: &str) {
    if !list.iter().any(|p| p == path) {
        list.push(path.to_owned());
    }
}

/// `git diff --name-status -z`: `<status>\0<path>\0`, or
/// `<R|C><score>\0<from>\0<to>\0`.
fn parse_name_status(out: &str) -> Result<Vec<Change>, TaskGitError> {
    let mut fields = out.split('\0').filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(code) = fields.next() {
        let mut next_path = || {
            fields
                .next()
                .map(str::to_owned)
                .ok_or_else(|| TaskGitError::Invalid(format!("git diff ended after status {code:?}")))
        };
        let (status, from) = match code.chars().next() {
            Some('A') => (ChangeStatus::Added, None),
            Some('M') | Some('U') => (ChangeStatus::Modified, None),
            Some('D') => (ChangeStatus::Deleted, None),
            Some('T') => (ChangeStatus::TypeChanged, None),
            Some('R') => (ChangeStatus::Renamed, Some(next_path()?)),
            Some('C') => (ChangeStatus::Copied, Some(next_path()?)),
            _ => {
                return Err(TaskGitError::Invalid(format!(
                    "git diff printed an unknown status {code:?}"
                )));
            }
        };
        changes.push(Change {
            path: next_path()?,
            status,
            from,
        });
    }
    Ok(changes)
}

/// `git status --porcelain=v1 -z`: `XY <path>\0`, with a rename's or copy's
/// old path as an extra field after it.
fn parse_porcelain(out: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut fields = out.split('\0').filter(|field| !field.is_empty());
    while let Some(entry) = fields.next() {
        let Some(path) = entry.get(3..) else { continue };
        paths.push(path.to_owned());
        if matches!(entry.as_bytes()[0], b'R' | b'C')
            && let Some(from) = fields.next()
        {
            paths.push(from.to_owned());
        }
    }
    paths
}
