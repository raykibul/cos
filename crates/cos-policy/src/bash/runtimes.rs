//! Wrappers that run another command (`env`, `timeout`, …), shells and
//! language interpreters.

use super::args::Arg;
use super::{Stdin, Walker, packages, sensitive_variable};
use crate::paths::Location;
use crate::shell::is_name;
use crate::verdict::{Needs, Verdict};

/// Programs that, found inside the worktree (`.venv/bin/python`,
/// `node_modules/.bin/jest`, `./gradlew`), are classified by name. Any other
/// program in the worktree is a script.
pub(super) fn is_local_tool(name: &str) -> bool {
    const LOCAL: &[&str] = &[
        "python",
        "pip",
        "node",
        "ruby",
        "perl",
        "php",
        "pytest",
        "jest",
        "vitest",
        "mocha",
        "tsc",
        "eslint",
        "prettier",
        "biome",
        "ruff",
        "black",
        "mypy",
        "isort",
        "flake8",
        "pylint",
        "gradlew",
        "mvnw",
        "tsx",
        "ts-node",
        "vite",
        "next",
        "playwright",
    ];
    LOCAL.contains(&super::normalize_name(name))
}

/// Handles `name` if it's a wrapper, a shell or an interpreter.
pub(super) fn classify(
    walker: &mut Walker<'_, '_>,
    name: &str,
    args: &[Arg],
    stdin: Stdin<'_>,
    local: bool,
) -> bool {
    match name {
        "env" => env(walker, args, stdin),
        "nice" | "nohup" | "time" | "timeout" | "gtimeout" | "command" | "exec" | "builtin" | "stdbuf"
        | "caffeinate" | "unbuffer" => wrapper(walker, name, args, stdin),
        "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish" => shell(walker, args, stdin),
        "eval" => {
            let words: Option<Vec<&str>> = args[1..].iter().map(Arg::str).collect();
            match words {
                Some(words) => walker.nested(&words.join(" ")),
                None => walker.add(Verdict::ask("evaluates a string that is only known when it runs")),
            }
        }
        "source" | "." => match args.get(1) {
            Some(script) => {
                run_script(walker, script, "a shell script");
                walker.scan(&args[2..]);
            }
            None => walker.add(Verdict::ask("sources nothing")),
        },
        "python" | "pypy" | "pypy3" | "node" | "nodejs" | "ruby" | "perl" | "php" | "Rscript" | "lua" => {
            interpreter(walker, name, args, stdin, local);
        }
        "deno" => deno(walker, args),
        _ => return false,
    }
    true
}

/// Runs a script file: fine for the chief inside the worktree, never outside.
fn run_script(walker: &mut Walker<'_, '_>, script: &Arg, what: &str) {
    let verdict = match walker.locate(script) {
        Location::Inside {
            rel,
            protected: Some(kind),
            ..
        } => Verdict::ask(format!("runs {rel}: {kind}")),
        Location::Inside { rel, .. } => {
            Verdict::chief(Needs::WORKTREE, format!("runs {rel}, {what} in the worktree"))
        }
        Location::Outside { path, .. } => Verdict::never(format!("runs {path}, {what} outside the worktree")),
        Location::Secret { path, kind } => Verdict::never(format!("runs {path}, {kind}")),
        Location::Unknown | Location::Device => {
            Verdict::ask(format!("runs {}, which cos can't resolve", script.shown()))
        }
    };
    walker.add(verdict);
    walker.uses_cwd();
}

/// Code arriving on stdin: a heredoc is visible, a pipe or the terminal isn't.
fn stdin_code(walker: &mut Walker<'_, '_>, stdin: Stdin<'_>, lang: &str) {
    let verdict = match stdin {
        Stdin::Text(Some(_)) => Verdict::chief(Needs::WORKTREE, format!("runs {lang} code from a heredoc")),
        Stdin::File => Verdict::chief(Needs::WORKTREE, format!("runs {lang} code from a file")),
        Stdin::Text(None) | Stdin::Pipe => Verdict::ask(format!("runs {lang} code piped in")),
        Stdin::Terminal => Verdict::ask(format!("starts an interactive {lang} session")),
    };
    walker.add(verdict);
    walker.uses_cwd();
}

fn env(walker: &mut Walker<'_, '_>, args: &[Arg], stdin: Stdin<'_>) {
    let mut i = 1;
    let mut chdir = None;
    while let Some(arg) = args.get(i) {
        let Some(text) = arg.str() else { break };
        match text {
            "-i" | "--ignore-environment" | "-0" | "--null" | "-v" | "--debug" | "-" => {}
            "-u" | "--unset" => i += 1,
            "-C" | "--chdir" => {
                chdir = args.get(i + 1).cloned();
                i += 1;
            }
            "-S" | "--split-string" => {
                match args.get(i + 1).and_then(Arg::str) {
                    Some(text) => walker.nested(text),
                    None => walker.add(Verdict::ask("env -S with a string cos can't read")),
                }
                return;
            }
            text if text.starts_with("--chdir=") => chdir = Some(Arg::lit(&text[8..])),
            text if text.split_once('=').is_some_and(|(name, _)| is_name(name)) => {
                let name = text.split_once('=').map(|(name, _)| name).unwrap_or_default();
                if let Some(why) = sensitive_variable(name) {
                    walker.add(Verdict::ask(format!("sets {name}, {why}")));
                }
            }
            _ => break,
        }
        i += 1;
    }
    let inner = &args[i.min(args.len())..];
    if inner.is_empty() {
        walker.add(Verdict::never(
            "prints every environment variable, which can include tokens",
        ));
        return;
    }
    match chdir {
        Some(dir) => {
            let saved = walker.cwd.clone();
            walker.cwd = walker.absolute(&dir);
            walker.dispatch(inner, stdin);
            walker.cwd = saved;
        }
        None => walker.dispatch(inner, stdin),
    }
}

fn wrapper(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg], stdin: Stdin<'_>) {
    let mut i = 1;
    // Options of each wrapper that take a value.
    let valued: &[&str] = match name {
        "nice" => &["-n", "--adjustment"],
        "timeout" | "gtimeout" => &["-s", "--signal", "-k", "--kill-after"],
        "exec" => &["-a"],
        "stdbuf" => &["-i", "-o", "-e"],
        "caffeinate" => &["-t", "-w"],
        "time" => &["-o", "--output", "-f", "--format"],
        _ => &[],
    };
    while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        if name == "time"
            && matches!(text, "-o" | "--output")
            && let Some(file) = args.get(i + 1)
        {
            walker.write(file);
        }
        i += if valued.contains(&text) { 2 } else { 1 };
    }
    if matches!(name, "timeout" | "gtimeout") {
        i += 1; // the duration
    }
    let inner = &args[i.min(args.len())..];
    if inner.is_empty() {
        walker.add(Verdict::allowed(format!("{name} without a command")));
    } else {
        walker.dispatch(inner, stdin);
    }
}

fn shell(walker: &mut Walker<'_, '_>, args: &[Arg], stdin: Stdin<'_>) {
    let mut i = 1;
    while let Some(arg) = args
        .get(i)
        .filter(|a| a.is_option() || a.str().is_some_and(|t| t.starts_with('+')))
    {
        let text = arg.str().unwrap_or_default();
        let short = !text.starts_with("--") && text.starts_with('-');
        if short && text.contains('c') {
            match args.get(i + 1).map(Arg::str) {
                Some(Some(code)) => walker.nested(code),
                _ => walker.add(Verdict::ask(
                    "runs a shell command that is only known when it runs",
                )),
            }
            return;
        }
        match text {
            "-o" | "+o" | "-O" | "+O" => i += 1,
            "--rcfile" | "--init-file" => {
                if let Some(file) = args.get(i + 1) {
                    run_script(walker, file, "a shell startup file");
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    match args.get(i).filter(|a| !a.is("-")) {
        Some(script) => {
            run_script(walker, script, "a shell script");
            walker.scan(&args[i + 1..]);
        }
        None => match stdin {
            Stdin::Text(Some(body)) => walker.nested(body),
            other => stdin_code(walker, other, "shell"),
        },
    }
}

fn interpreter(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg], stdin: Stdin<'_>, local: bool) {
    let lang = match name {
        "python" | "pypy" | "pypy3" => "Python",
        "node" | "nodejs" => "JavaScript",
        "ruby" => "Ruby",
        "perl" => "Perl",
        "php" => "PHP",
        "Rscript" => "R",
        _ => "Lua",
    };
    let mut i = 1;
    let mut inline = false;
    let mut in_place = false;
    while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        i += 1;
        match (lang, text) {
            (_, "-V" | "--version" | "-v" | "-h" | "--help") if args.len() == 2 => {
                walker.add(Verdict::allowed(format!("prints the {lang} version")));
                return;
            }
            ("Python", "-m") => {
                let module = args.get(i).cloned().unwrap_or_else(Arg::unknown);
                python_module(walker, &module, &args[i..], local);
                return;
            }
            ("Python", "-W" | "-X" | "-Q") => i += 1,
            ("Python", t) if !t.starts_with("--") && t.ends_with('c') => inline = true,
            ("JavaScript", "-e" | "--eval" | "-p" | "--print") => inline = true,
            ("JavaScript", "-r" | "--require" | "--import" | "--loader" | "--experimental-loader") => {
                walker.scan(&args[i..i + 1]);
                i += 1;
            }
            ("JavaScript", "--test") => {
                walker.tool(Needs::WORKTREE, "runs the Node test runner", args);
                return;
            }
            ("Ruby", "-r" | "-I") => i += 1,
            ("Ruby", t) if t.starts_with("-i") => in_place = true,
            ("Perl", t) if !t.starts_with("--") && !t.starts_with("-I") && !t.starts_with("-M") => {
                let flags = &t[1..];
                let flags = match flags.find('i') {
                    Some(at) => {
                        in_place = true;
                        &flags[..at]
                    }
                    None => flags,
                };
                inline |= flags.contains(['e', 'E']);
            }
            ("PHP", "-f") => {}
            ("PHP" | "R" | "Lua" | "Ruby", "-r" | "-e") => inline = true,
            _ => {}
        }
        if inline {
            break;
        }
    }
    if inline {
        walker.add(Verdict::chief(
            Needs::WORKTREE,
            format!("runs inline {lang} code"),
        ));
        walker.uses_cwd();
        // The code itself is the next argument; the rest are files or args.
        let rest = args.get(i + 1..).unwrap_or_default();
        if in_place {
            rest.iter().for_each(|file| walker.write(file));
        } else {
            walker.scan(rest);
        }
        return;
    }
    match args.get(i).filter(|a| !a.is("-")) {
        Some(script) => {
            run_script(walker, script, &format!("a {lang} script"));
            let rest = &args[i + 1..];
            if in_place {
                rest.iter().for_each(|file| walker.write(file));
            } else {
                walker.scan(rest);
            }
        }
        None => stdin_code(walker, stdin, lang),
    }
}

fn python_module(walker: &mut Walker<'_, '_>, module: &Arg, from_module: &[Arg], local: bool) {
    let name = module.str().unwrap_or_default();
    match name {
        "pip" => packages::pip(walker, from_module, local),
        "pytest" | "unittest" | "doctest" | "nose2" => {
            walker.tool(Needs::WORKTREE, "runs Python tests", from_module)
        }
        "venv" | "virtualenv" => {
            walker.add(Verdict::chief(Needs::WORKTREE, "creates a virtualenv"));
            for target in from_module[1..].iter().filter(|a| !a.is_option()) {
                walker.write(target);
            }
        }
        "http.server" | "SimpleHTTPServer" | "smtpd" => walker.add(Verdict::ask("starts a server")),
        "json.tool" => {
            walker.add(Verdict::allowed("formats JSON"));
            for file in from_module[1..].iter().filter(|a| !a.is_option()) {
                walker.read(file);
            }
        }
        "ensurepip" => walker.add(Verdict::ask("installs pip itself")),
        "" => walker.add(Verdict::ask(
            "runs a Python module that is only known when it runs",
        )),
        _ => walker.tool(
            Needs::WORKTREE,
            format!("runs the Python module {name}"),
            from_module,
        ),
    }
}

fn deno(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    match args.get(1).and_then(Arg::str) {
        Some("-V" | "--version" | "--help") => walker.add(Verdict::allowed("prints Deno's version")),
        Some("add" | "install" | "cache" | "outdated" | "remove")
            if !args.iter().any(|a| a.is("-g") || a.is("--global")) =>
        {
            walker.tool(Needs::INSTALL, "changes or downloads Deno dependencies", args);
        }
        Some("install" | "upgrade" | "uninstall") => {
            walker.add(Verdict::never("installs or upgrades Deno tools globally"))
        }
        Some("repl") | None => walker.add(Verdict::ask("starts an interactive Deno session")),
        Some(_) => walker.tool(Needs::WORKTREE, "runs Deno in the worktree", args),
    }
}
