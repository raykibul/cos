//! Classifies a Bash command. The command is parsed into simple commands
//! (see [`crate::shell`]); each is classified on its own, with `cd` followed
//! along the way, and the whole command gets the most restrictive tier of its
//! parts. Command substitutions, `bash -c` bodies, `eval` strings, heredocs
//! fed to a shell, `find -exec` and `xargs` commands are classified the same
//! way, recursively.

mod args;
mod files;
mod git;
mod network;
mod packages;
mod runtimes;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::classify::{Context, read_verdict, write_verdict};
use crate::paths::{Location, lexical, secret_name};
use crate::shell::{self, Command, Node, Part, RedirOp, Script, Word};
use crate::verdict::{Needs, Verdict};

pub(crate) use args::Arg;

/// How deep substitutions and `bash -c` may nest before cos gives up.
const MAX_DEPTH: usize = 6;
/// The most words one word may expand to through loop variables.
const MAX_EXPANSIONS: usize = 64;

/// Classifies a Bash tool command.
pub(crate) fn classify_command(context: &Context<'_>, command: &str) -> Verdict {
    let cwd = context
        .request
        .cwd
        .is_absolute()
        .then(|| lexical(&context.request.cwd));
    Walker::new(context, cwd, HashMap::new(), 0).run(command)
}

/// Where a command's stdin comes from.
#[derive(Debug, Clone, Copy)]
enum Stdin<'s> {
    Terminal,
    /// The previous command in a pipeline.
    Pipe,
    /// A file redirected with `<`.
    File,
    /// A heredoc or here-string, when its text is known.
    Text(Option<&'s str>),
}

/// Variable values cos knows: from `NAME=value` and `for NAME in …`. `None`
/// means the variable is set but its value isn't known.
type Env = HashMap<String, Option<Vec<String>>>;

struct Walker<'c, 'a> {
    context: &'c Context<'a>,
    /// The shell's current directory as `cd` moves it; `None` once unknown.
    cwd: Option<PathBuf>,
    /// `pushd` history.
    dirs: Vec<Option<PathBuf>>,
    env: Env,
    depth: usize,
    verdict: Option<Verdict>,
    /// A folder whose files the previous pipeline stage listed, so `xargs`
    /// knows where its arguments come from.
    incoming: Option<PathBuf>,
    /// What this stage lists, for the next one.
    outgoing: Option<PathBuf>,
}

impl<'c, 'a> Walker<'c, 'a> {
    fn new(context: &'c Context<'a>, cwd: Option<PathBuf>, env: Env, depth: usize) -> Self {
        Self {
            context,
            cwd,
            dirs: Vec::new(),
            env,
            depth,
            verdict: None,
            incoming: None,
            outgoing: None,
        }
    }

    fn run(mut self, text: &str) -> Verdict {
        match shell::parse(text) {
            Ok(script) => self.script(&script),
            Err(error) => self.add(Verdict::ask(format!(
                "cos couldn't parse the command confidently ({error})"
            ))),
        }
        self.verdict
            .unwrap_or_else(|| Verdict::allowed("runs nothing that needs approval"))
    }

    fn add(&mut self, verdict: Verdict) {
        self.verdict = Some(match self.verdict.take() {
            None => verdict,
            Some(old) => old.and(verdict),
        });
    }

    /// Classifies script text that runs as part of this command: a
    /// substitution, a `bash -c` body, an `eval` string.
    fn nested(&mut self, text: &str) {
        if self.depth >= MAX_DEPTH {
            self.add(Verdict::ask("the command nests shells too deeply to check"));
            return;
        }
        let inner = Walker::new(self.context, self.cwd.clone(), self.env.clone(), self.depth + 1);
        let verdict = inner.run(text);
        self.add(verdict);
    }

    fn script(&mut self, script: &Script) {
        for item in &script.items {
            let listed = self.outgoing.take();
            self.incoming = if item.piped_in { listed } else { None };
            match &item.node {
                Node::Command(command) => self.command(command, item.piped_in),
                Node::Subshell(inner) => {
                    let saved = (self.cwd.clone(), self.dirs.clone(), self.env.clone());
                    self.script(inner);
                    (self.cwd, self.dirs, self.env) = saved;
                    self.outgoing = None;
                }
                Node::Bind { name, values } => {
                    let values = values.as_ref().and_then(|words| {
                        words.iter().for_each(|word| self.word_effects(word));
                        let args: Vec<Arg> = words.iter().flat_map(|word| self.expand(word)).collect();
                        args.into_iter().map(|arg| arg.text).collect::<Option<Vec<_>>>()
                    });
                    self.env.insert(name.clone(), values);
                }
            }
        }
    }

    fn command(&mut self, command: &Command, piped_in: bool) {
        let words = command
            .assigns
            .iter()
            .map(|assign| &assign.value)
            .chain(&command.words)
            .chain(command.redirs.iter().map(|redirect| &redirect.target));
        for word in words {
            self.word_effects(word);
        }
        for heredoc in command.redirs.iter().filter_map(|r| r.heredoc.as_ref()) {
            self.word_effects(&heredoc.expansions);
        }

        for assign in &command.assigns {
            if let Some(why) = sensitive_variable(&assign.name) {
                self.add(Verdict::ask(format!("sets {}, {why}", assign.name)));
            }
        }
        if command.words.is_empty() {
            for assign in &command.assigns {
                let value = self.expand_one(&assign.value);
                self.env.insert(assign.name.clone(), value.map(|v| vec![v]));
            }
        }

        let mut stdin = if piped_in { Stdin::Pipe } else { Stdin::Terminal };
        for redirect in &command.redirs {
            match redirect.op {
                RedirOp::In => {
                    for target in self.expand(&redirect.target) {
                        self.read(&target);
                    }
                    stdin = Stdin::File;
                }
                RedirOp::Out | RedirOp::Append | RedirOp::ReadWrite => {
                    for target in self.expand(&redirect.target) {
                        self.write(&target);
                    }
                }
                RedirOp::Dup => {}
                RedirOp::HereDoc => {
                    stdin = Stdin::Text(redirect.heredoc.as_ref().map(|h| h.body.as_str()));
                }
                RedirOp::HereString => stdin = Stdin::Text(None),
            }
        }
        // A here-string's text, when it's literal, is known like a heredoc's.
        let here_string = command
            .redirs
            .iter()
            .rev()
            .find(|r| r.op == RedirOp::HereString)
            .and_then(|r| r.target.literal());
        if let (Stdin::Text(None), Some(text)) = (stdin, here_string.as_deref()) {
            stdin = Stdin::Text(Some(text));
        }

        if command.words.is_empty() {
            return;
        }
        let args: Vec<Arg> = command.words.iter().flat_map(|word| self.expand(word)).collect();
        if args.is_empty() {
            self.add(Verdict::ask("the command name is only known when it runs"));
            return;
        }
        self.dispatch(&args, stdin);
    }

    /// What a word runs or reads just by being expanded.
    fn word_effects(&mut self, word: &Word) {
        for text in &word.substs {
            self.nested(text);
        }
        for name in &word.vars {
            if secret_variable(name) {
                self.add(Verdict::never(format!(
                    "reads ${name}, which looks like a secret"
                )));
            }
        }
    }

    /// Expands a word with what cos knows about variables. A word that
    /// depends on anything unknown becomes one unknown argument.
    fn expand(&self, word: &Word) -> Vec<Arg> {
        let mut texts = vec![String::new()];
        for part in &word.parts {
            let values = match part {
                Part::Lit(text) => vec![text.clone()],
                Part::Var(name) => match self.variable(name) {
                    Some(values) => values,
                    None => return vec![Arg::unknown()],
                },
                Part::Dynamic => return vec![Arg::unknown()],
            };
            if texts.len() * values.len() > MAX_EXPANSIONS {
                return vec![Arg::unknown()];
            }
            texts = texts
                .iter()
                .flat_map(|text| values.iter().map(move |value| format!("{text}{value}")))
                .collect();
        }
        texts
            .into_iter()
            .map(|text| Arg {
                text: Some(text),
                glob: word.glob,
            })
            .collect()
    }

    fn expand_one(&self, word: &Word) -> Option<String> {
        match self.expand(word).as_slice() {
            [arg] => arg.text.clone(),
            _ => None,
        }
    }

    fn variable(&self, name: &str) -> Option<Vec<String>> {
        match name {
            "HOME" => Some(vec![self.context.paths.home().display().to_string()]),
            "PWD" => self.cwd.as_ref().map(|cwd| vec![cwd.display().to_string()]),
            _ => self.env.get(name).cloned().flatten(),
        }
    }

    fn locate(&self, arg: &Arg) -> Location {
        match &arg.text {
            None => Location::Unknown,
            Some(text) => self.context.paths.locate(self.cwd.as_deref(), text, arg.glob),
        }
    }

    fn absolute(&self, arg: &Arg) -> Option<PathBuf> {
        self.context
            .paths
            .absolute(self.cwd.as_deref(), arg.text.as_deref()?)
    }

    fn read(&mut self, arg: &Arg) {
        let verdict = read_verdict(&self.locate(arg), arg.shown());
        self.add(verdict);
    }

    fn write(&mut self, arg: &Arg) {
        let verdict = write_verdict(&self.locate(arg), arg.shown(), self.context.request.role);
        self.add(verdict);
    }

    /// The command works on its current directory, which must be inside the
    /// worktree.
    fn uses_cwd(&mut self) {
        let cwd = self.cwd.clone();
        self.uses_dir(cwd.as_deref());
    }

    fn uses_dir(&mut self, dir: Option<&Path>) {
        let Some(dir) = dir else {
            self.add(Verdict::ask("runs in a folder cos can't work out"));
            return;
        };
        let shown = dir.display().to_string();
        match self.context.paths.locate(None, &shown, false) {
            Location::Inside { .. } | Location::Device => {}
            Location::Secret { path, kind } => self.add(Verdict::never(format!("runs in {path}, {kind}"))),
            Location::Outside { path, .. } => {
                self.add(Verdict::never(format!("runs outside the worktree, in {path}")));
            }
            Location::Unknown => self.add(Verdict::ask("runs in a folder cos can't work out")),
        }
    }

    /// Checks the arguments of a program cos doesn't model in detail: any
    /// that look like paths must stay inside the worktree and away from
    /// secrets and protected files.
    fn scan(&mut self, args: &[Arg]) {
        for arg in args {
            let Some(text) = &arg.text else {
                self.add(Verdict::ask("passes an argument that is only known when it runs"));
                continue;
            };
            let candidate = if text.starts_with("--") {
                text.split_once('=').map(|(_, value)| value)
            } else if text.starts_with('-') {
                None
            } else {
                Some(text.as_str())
            };
            let Some(candidate) = candidate.filter(|c| looks_like_path(c)) else {
                continue;
            };
            let location = self
                .context
                .paths
                .locate(self.cwd.as_deref(), candidate, arg.glob);
            let verdict = match location {
                Location::Device | Location::Inside { protected: None, .. } => continue,
                Location::Inside {
                    rel,
                    protected: Some(kind),
                    ..
                } => Verdict::ask(format!("passes {rel}: {kind}")),
                Location::Secret { path, kind } => Verdict::never(format!("passes {path}, {kind}")),
                Location::Outside { path, .. } => {
                    Verdict::never(format!("passes {path}, outside the worktree"))
                }
                Location::Unknown => Verdict::ask(format!("passes {candidate}, which cos can't resolve")),
            };
            self.add(verdict);
        }
    }

    /// A tool cos knows runs code in the worktree (a build, a test runner, a
    /// linter): the chief may approve it, with `needs`.
    fn tool(&mut self, needs: Needs, reason: impl Into<String>, args: &[Arg]) {
        self.add(Verdict::chief(needs, reason));
        self.uses_cwd();
        self.scan(args.get(1..).unwrap_or_default());
    }

    /// Records that this command prints paths under `arg`, for a following
    /// `xargs`.
    fn lists(&mut self, arg: &Arg) {
        if matches!(self.locate(arg), Location::Inside { .. }) {
            self.outgoing = self.absolute(arg);
        }
    }

    /// A filter (`grep`, `sort`, `head`) passes the listing through.
    fn passes_listing(&mut self) {
        self.outgoing = self.incoming.clone();
    }

    fn is_test_command(&self, args: &[Arg]) -> bool {
        self.context
            .tests
            .iter()
            .any(|test| args.len() >= test.len() && test.iter().zip(args).all(|(want, arg)| arg.is(want)))
    }

    /// Classifies one simple command: `args[0]` is the program.
    fn dispatch(&mut self, args: &[Arg], stdin: Stdin<'_>) {
        let Some(program) = args[0].text.clone() else {
            self.add(Verdict::ask("the command name is only known when it runs"));
            return;
        };
        if self.is_test_command(args) {
            self.add(Verdict::allowed("runs the task's test command"));
            self.uses_cwd();
            self.scan(&args[1..]);
            return;
        }
        if !program.contains('/') {
            self.program(&program, args, stdin, false);
            return;
        }
        let name = program.rsplit('/').next().unwrap_or_default().to_owned();
        match self.locate(&args[0]) {
            Location::Inside {
                protected: Some(kind),
                rel,
                ..
            } => self.add(Verdict::ask(format!("runs {rel}: {kind}"))),
            Location::Inside { .. } if runtimes::is_local_tool(&name) => {
                self.program(&name, args, stdin, true)
            }
            Location::Inside { rel, .. } => {
                self.tool(
                    Needs::WORKTREE,
                    format!("runs {rel}, a script in the worktree"),
                    args,
                );
            }
            Location::Outside { path, .. } if is_system_program(&path) => {
                self.program(&name, args, stdin, false);
            }
            Location::Outside { path, .. } => {
                self.add(Verdict::never(format!(
                    "runs {path}, a program outside the worktree"
                )));
            }
            Location::Secret { path, kind } => self.add(Verdict::never(format!("runs {path}, {kind}"))),
            Location::Unknown | Location::Device => {
                self.add(Verdict::ask(format!("runs {program}, which cos can't resolve")));
            }
        }
    }

    /// Classifies a program by name. `local` says it lives in the worktree,
    /// like `.venv/bin/pip`.
    fn program(&mut self, name: &str, args: &[Arg], stdin: Stdin<'_>, local: bool) {
        let name = normalize_name(name);
        if files::classify(self, name, args) {
            return;
        }
        if runtimes::classify(self, name, args, stdin, local) {
            return;
        }
        if name == "git" {
            git::classify(self, args);
            return;
        }
        if network::classify(self, name, args) {
            return;
        }
        if packages::classify(self, name, args, local) {
            return;
        }
        if let Some(reason) = system_change(name, args) {
            self.add(Verdict::never(reason));
            return;
        }
        if INTERACTIVE.contains(&name) {
            self.add(Verdict::ask(format!(
                "`{name}` is interactive or acts outside the shell"
            )));
            return;
        }
        self.add(Verdict::ask(format!("cos doesn't know what `{name}` does")));
    }
}

/// `python3.12` → `python`, `pip3` → `pip`, `g++-14` stays.
fn normalize_name(name: &str) -> &str {
    for base in ["python", "pip", "pydoc"] {
        if let Some(rest) = name.strip_prefix(base)
            && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return base;
        }
    }
    name
}

/// Programs started by full path from these folders are classified by name.
fn is_system_program(path: &str) -> bool {
    const BIN_DIRS: &[&str] = &[
        "/bin",
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/usr/sbin",
        "/sbin",
    ];
    Path::new(path)
        .parent()
        .is_some_and(|dir| BIN_DIRS.iter().any(|bin| dir == Path::new(bin)))
}

/// Opens a UI, waits for input, or acts on the desktop.
const INTERACTIVE: &[&str] = &[
    "open",
    "code",
    "cursor",
    "vim",
    "vi",
    "nvim",
    "nano",
    "emacs",
    "subl",
    "pbcopy",
    "pbpaste",
    "tmux",
    "screen",
    "watch",
    "history",
    "trap",
    "kill",
    "fg",
    "bg",
    "disown",
    "osascript",
    "say",
    "top",
    "htop",
];

/// Commands that change the machine, other processes or remote systems.
fn system_change(name: &str, args: &[Arg]) -> Option<String> {
    const SYSTEM: &[&str] = &[
        "sudo",
        "doas",
        "su",
        "pkexec",
        "launchctl",
        "crontab",
        "systemctl",
        "security",
        "scutil",
        "networksetup",
        "pmset",
        "csrutil",
        "spctl",
        "diskutil",
        "mount",
        "umount",
        "shutdown",
        "reboot",
        "halt",
        "mkfs",
        "fdisk",
        "visudo",
        "useradd",
        "usermod",
        "userdel",
        "passwd",
        "chsh",
        "dscl",
        "killall",
        "pkill",
        "softwareupdate",
        "xcode-select",
        "nvram",
        "tccutil",
        "kextload",
        "dscacheutil",
        "chflags",
        "xattr",
        "launchd",
        "at",
        "batch",
    ];
    const CLOUD: &[&str] = &[
        "aws",
        "gcloud",
        "gsutil",
        "bq",
        "az",
        "kubectl",
        "helm",
        "terraform",
        "tofu",
        "pulumi",
        "heroku",
        "fly",
        "flyctl",
        "vercel",
        "netlify",
        "firebase",
        "wrangler",
        "doctl",
        "eksctl",
        "railway",
        "supabase",
        "op",
        "vault",
    ];
    if name == "defaults" {
        let sub = args.get(1).and_then(|a| a.text.as_deref());
        return match sub {
            Some("read" | "read-type" | "domains" | "find") => None,
            _ => Some("changes macOS preferences (defaults)".to_owned()),
        };
    }
    if SYSTEM.contains(&name) {
        return Some(match name {
            "sudo" | "doas" | "su" | "pkexec" => format!("runs a command as another user ({name})"),
            "killall" | "pkill" => format!("stops processes outside the worktree ({name})"),
            _ => format!("changes your system ({name})"),
        });
    }
    if CLOUD.contains(&name) {
        return Some(format!(
            "talks to cloud infrastructure or a vault with your credentials ({name})"
        ));
    }
    None
}

/// Whether a variable name looks like it holds a credential.
fn secret_variable(name: &str) -> bool {
    const MARKERS: &[&str] = &[
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
        "SESSION_KEY",
    ];
    let upper = name.to_ascii_uppercase();
    MARKERS.iter().any(|marker| upper.contains(marker))
}

/// Variables that change where a command runs, which program runs, or what
/// settings it loads. Setting one is for you to approve.
fn sensitive_variable(name: &str) -> Option<&'static str> {
    const EXACT: &[&str] = &[
        "PATH",
        "HOME",
        "BASH_ENV",
        "ENV",
        "PROMPT_COMMAND",
        "SHELLOPTS",
        "IFS",
        "NODE_OPTIONS",
        "PYTHONSTARTUP",
        "PYTHONPATH",
        "PERL5OPT",
        "RUBYOPT",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "XDG_CONFIG_HOME",
        "EDITOR",
        "VISUAL",
        "PAGER",
        "COS_HOME",
        "SSH_AUTH_SOCK",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_EXEC_PATH",
        "GIT_ASKPASS",
        "GIT_EDITOR",
        "GIT_SEQUENCE_EDITOR",
        "GIT_PAGER",
        "GIT_EXTERNAL_DIFF",
        "GIT_PROXY_COMMAND",
    ];
    const PREFIXES: &[&str] = &[
        "LD_",
        "DYLD_",
        "GIT_CONFIG",
        "NPM_CONFIG_",
        "PIP_",
        "CLAUDE_",
        "ANTHROPIC_",
        "UV_",
        "CARGO_REGISTRIES_",
    ];
    let upper = name.to_ascii_uppercase();
    (EXACT.contains(&name) || PREFIXES.iter().any(|prefix| upper.starts_with(prefix)))
        .then_some("which changes where or how commands run")
}

/// Whether an argument of an unknown program is worth a path check.
fn looks_like_path(text: &str) -> bool {
    if text.is_empty() || text.contains("://") || text.starts_with('@') {
        return false;
    }
    // `host:path`, `scheme:rest`, `key:value`.
    if let Some((before, _)) = text.split_once(':')
        && !before.is_empty()
        && !before.contains('/')
    {
        return false;
    }
    text.starts_with('/')
        || text.starts_with('~')
        || text == "."
        || text == ".."
        || text.contains('/')
        || secret_name(text)
        || text.eq_ignore_ascii_case(".mcp.json")
        || text.eq_ignore_ascii_case("CLAUDE.md")
}
