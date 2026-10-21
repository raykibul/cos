//! `git`. Reads, `add` and `commit` in the worktree are always allowed; other
//! local changes are for the chief; anything that reaches the shared
//! repository's branches, settings or remotes is never delegated.

use super::Walker;
use super::args::Arg;
use crate::paths::{Location, secret_name};
use crate::verdict::{Needs, Verdict};

/// Subcommands that only read.
const READS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "blame",
    "annotate",
    "shortlog",
    "describe",
    "rev-parse",
    "rev-list",
    "ls-files",
    "ls-tree",
    "cat-file",
    "merge-base",
    "name-rev",
    "check-ignore",
    "check-attr",
    "grep",
    "whatchanged",
    "cherry",
    "range-diff",
    "show-ref",
    "for-each-ref",
    "count-objects",
    "fsck",
    "verify-commit",
    "verify-tag",
    "version",
    "help",
    "var",
    "diff-tree",
    "diff-files",
    "diff-index",
    "show-branch",
    "difftool-check",
];

/// Subcommands that change the worktree, its index or local history only.
const LOCAL_CHANGES: &[&str] = &[
    "checkout",
    "switch",
    "restore",
    "reset",
    "clean",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "rm",
    "mv",
    "apply",
    "am",
    "notes",
    "bisect",
    "format-patch",
    "sparse-checkout",
    "read-tree",
    "checkout-index",
    "update-index",
    "commit-tree",
    "write-tree",
    "mktree",
    "hash-object",
    "archive",
    "stripspace",
];

/// Subcommands that rewrite data shared with your main checkout.
const SHARED_REWRITES: &[&str] = &[
    "gc",
    "prune",
    "repack",
    "update-ref",
    "symbolic-ref",
    "filter-branch",
    "filter-repo",
    "replace",
    "pack-refs",
    "maintenance",
    "fast-import",
    "update-server-info",
];

/// `-c` settings that only change output or identity.
const SAFE_CONFIG_PREFIXES: &[&str] = &[
    "color.",
    "user.name",
    "user.email",
    "advice.",
    "init.defaultbranch",
    "log.",
];

/// Options whose values are text, not paths.
const TEXT_VALUES: &[&str] = &[
    "-m",
    "--message",
    "--author",
    "--format",
    "--pretty",
    "-S",
    "-G",
    "--grep",
    "--date",
    "--trailer",
    "--since",
    "--until",
    "--after",
    "--before",
];

pub(super) fn classify(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    let mut dir = walker.cwd.clone();
    let mut i = 1;
    while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        i += 1;
        match text {
            "-C" => {
                let target = args.get(i).cloned().unwrap_or_else(Arg::unknown);
                dir = match (&dir, target.str()) {
                    (Some(base), Some(path)) => walker.context.paths.absolute(Some(base), path),
                    _ => None,
                };
                i += 1;
            }
            "-c" => {
                let setting = args
                    .get(i)
                    .and_then(Arg::str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !SAFE_CONFIG_PREFIXES
                    .iter()
                    .any(|prefix| setting.starts_with(prefix))
                {
                    walker.add(Verdict::ask(format!(
                        "overrides git config for this command ({setting})"
                    )));
                }
                i += 1;
            }
            "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" => {
                if let Some(path) = args.get(i) {
                    git_dir(walker, path);
                }
                i += 1;
            }
            text if text.starts_with("--git-dir=") || text.starts_with("--work-tree=") => {
                let path = Arg::lit(text.split_once('=').map(|(_, v)| v).unwrap_or_default());
                git_dir(walker, &path);
            }
            text if text.starts_with("--exec-path") || text.starts_with("--config-env") => {
                walker.add(Verdict::ask(format!("changes how git runs ({text})")));
            }
            "--version" | "--help" | "-h" => {
                walker.add(Verdict::allowed("prints git's version or help"));
                return;
            }
            _ => {}
        }
    }
    let Some(sub) = args.get(i).map(|a| a.str().map(str::to_owned)) else {
        walker.add(Verdict::allowed("prints git's help"));
        return;
    };
    let Some(sub) = sub else {
        walker.add(Verdict::ask(
            "runs a git subcommand that is only known when it runs",
        ));
        return;
    };
    walker.uses_dir(dir.as_deref());
    let rest = &args[i + 1..];
    let saved = std::mem::replace(&mut walker.cwd, dir.clone());
    subcommand(walker, &sub, rest);
    walker.cwd = saved;
}

/// `--git-dir` and `--work-tree` must point inside the worktree.
fn git_dir(walker: &mut Walker<'_, '_>, path: &Arg) {
    match walker.locate(path) {
        Location::Inside { .. } => {}
        Location::Unknown => walker.add(Verdict::ask("points git at a folder cos can't resolve")),
        _ => walker.add(Verdict::never(format!(
            "runs git on {}, outside the worktree",
            path.shown()
        ))),
    }
}

fn subcommand(walker: &mut Walker<'_, '_>, sub: &str, rest: &[Arg]) {
    let first = rest.iter().find(|a| !a.is_option()).and_then(Arg::str);
    let has = |names: &[&str]| rest.iter().any(|a| a.str().is_some_and(|t| names.contains(&t)));
    match sub {
        "push" | "send-email" => walker.add(Verdict::never("pushes to a remote")),
        "reflog" if matches!(first, Some("expire" | "delete")) => {
            walker.add(Verdict::never(
                "rewrites the reflog shared with your main checkout",
            ));
        }
        "reflog" => walker.add(Verdict::allowed("reads the repository")),
        _ if READS.contains(&sub) => {
            walker.add(Verdict::allowed("reads the repository"));
            for arg in rest {
                if let Some(output) = arg.str().and_then(|t| t.strip_prefix("--output=")) {
                    walker.write(&Arg::lit(output));
                }
            }
            paths(walker, rest);
            let lists = match sub {
                "ls-files" => true,
                "diff" => has(&["--name-only", "--name-status"]),
                "grep" => has(&["-l", "--files-with-matches", "--name-only"]),
                _ => false,
            };
            if lists {
                let root = walker.context.paths.root().display().to_string();
                walker.lists(&Arg::lit(root));
            }
        }
        "add" => {
            walker.add(Verdict::allowed("stages files in the worktree"));
            paths(walker, rest);
        }
        "commit" => {
            walker.add(Verdict::allowed("commits in the worktree"));
            paths(walker, rest);
        }
        "branch" => branch(walker, rest),
        "tag" => {
            if has(&["-d", "--delete", "-f", "--force"]) {
                walker.add(Verdict::never(
                    "deletes or moves a tag shared with your main checkout",
                ));
            } else if first.is_none() || has(&["-l", "--list", "-n", "--contains", "--points-at", "--merged"])
            {
                walker.add(Verdict::allowed("lists tags"));
            } else {
                walker.add(Verdict::chief(Needs::WORKTREE, "creates a tag"));
            }
        }
        "stash" => match first {
            Some("list" | "show") => walker.add(Verdict::allowed("reads the stash")),
            Some("drop" | "clear") => {
                walker.add(Verdict::never("deletes stashes shared with your main checkout"))
            }
            _ => walker.add(Verdict::ask(
                "the stash is shared with your main checkout and other worktrees",
            )),
        },
        "config" => {
            let reads = has(&[
                "--get",
                "--get-all",
                "--get-regexp",
                "--list",
                "-l",
                "--get-urlmatch",
                "get",
                "list",
            ]) || rest.iter().filter(|a| !a.is_option()).count() == 1
                && first.is_some_and(|f| f.contains('.'));
            if reads {
                walker.add(Verdict::allowed("reads git settings"));
            } else {
                walker.add(Verdict::never("changes git settings"));
            }
        }
        "remote" => match first {
            None | Some("get-url") => walker.add(Verdict::allowed("lists remotes")),
            Some("show" | "update" | "prune") => {
                walker.add(Verdict::chief(Needs::NETWORK, "talks to a git remote"))
            }
            Some(_) => walker.add(Verdict::never("changes git remotes (repository settings)")),
        },
        "worktree" => match first {
            Some("list") => walker.add(Verdict::allowed("lists worktrees")),
            _ => walker.add(Verdict::never(
                "adds, moves or removes worktrees of your repository",
            )),
        },
        "submodule" => match first {
            None | Some("status" | "summary") => walker.add(Verdict::allowed("reads submodules")),
            _ => walker.add(Verdict::ask("changes submodules or runs commands in them")),
        },
        "fetch" | "pull" | "ls-remote" => {
            walker.add(Verdict::chief(
                Needs::NETWORK,
                format!("fetches from a remote (git {sub})"),
            ));
        }
        "clone" => {
            walker.add(Verdict::chief(Needs::NETWORK, "clones a repository"));
            if has(&["-c", "--config", "--template", "-u", "--upload-pack"]) {
                walker.add(Verdict::ask("clones with custom config or commands"));
            }
            let operands: Vec<&Arg> = rest.iter().filter(|a| !a.is_option()).collect();
            match operands.get(1) {
                Some(target) => walker.write(target),
                None => walker.write(&Arg::lit(".")),
            }
        }
        "rebase" if has(&["-i", "--interactive", "-x", "--exec"]) => {
            walker.add(Verdict::ask("runs an interactive rebase or commands during it"));
        }
        _ if LOCAL_CHANGES.contains(&sub) => {
            walker.add(Verdict::chief(
                Needs::WORKTREE,
                format!("changes the worktree (git {sub})"),
            ));
            paths(walker, rest);
        }
        _ if SHARED_REWRITES.contains(&sub) => {
            walker.add(Verdict::never(format!(
                "rewrites repository data shared with your main checkout (git {sub})"
            )));
        }
        _ if sub.starts_with("credential") => walker.add(Verdict::never("reads or stores git credentials")),
        _ => walker.add(Verdict::ask(format!(
            "cos doesn't know `git {sub}` (it may be an alias)"
        ))),
    }
}

fn branch(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    const CHANGES_EXISTING: &[&str] = &[
        "-d",
        "-D",
        "--delete",
        "-m",
        "-M",
        "--move",
        "-c",
        "-C",
        "--copy",
        "-f",
        "--force",
        "-u",
        "--set-upstream-to",
        "--unset-upstream",
        "--edit-description",
    ];
    const LISTS: &[&str] = &[
        "-a",
        "--all",
        "-r",
        "--remotes",
        "-v",
        "-vv",
        "--verbose",
        "--list",
        "-l",
        "--show-current",
        "--contains",
        "--no-contains",
        "--merged",
        "--no-merged",
        "--points-at",
        "--column",
        "--no-column",
        "--color",
        "--no-color",
    ];
    let flag = |a: &Arg| a.str().map(|t| t.split('=').next().unwrap_or(t).to_owned());
    if rest
        .iter()
        .filter_map(flag)
        .any(|f| CHANGES_EXISTING.contains(&f.as_str()))
    {
        walker.add(Verdict::never("changes branches shared with your main checkout"));
    } else if rest.is_empty()
        || rest
            .iter()
            .filter_map(flag)
            .any(|f| LISTS.contains(&f.as_str()) || f.starts_with("--sort") || f.starts_with("--format"))
    {
        walker.add(Verdict::allowed("lists branches"));
    } else {
        walker.add(Verdict::chief(Needs::WORKTREE, "creates a branch"));
    }
}

/// Paths given to git must be inside the worktree.
fn paths(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut skip_next = false;
    let mut after_dashes = false;
    for arg in rest {
        if skip_next {
            skip_next = false;
            continue;
        }
        let Some(text) = arg.str() else {
            if after_dashes {
                walker.add(Verdict::ask("passes git a path that is only known when it runs"));
            }
            continue;
        };
        if text == "--" {
            after_dashes = true;
            continue;
        }
        if !after_dashes && TEXT_VALUES.contains(&text) {
            skip_next = true;
            continue;
        }
        if !after_dashes && text.starts_with('-') {
            if let Some(file) = text
                .strip_prefix("--file=")
                .or_else(|| text.strip_prefix("--pathspec-from-file="))
            {
                walker.read(&Arg::lit(file));
            }
            continue;
        }
        let looks_like_path = after_dashes
            || text.starts_with('/')
            || text.starts_with('~')
            || text.starts_with("..")
            || text.contains("/..")
            || secret_name(text.rsplit('/').next().unwrap_or(text));
        if looks_like_path {
            walker.read(arg);
        }
    }
}
