//! Shell builtins and file utilities: what they read, write and delete.

use std::path::Path;

use super::args::{self, Arg, Spec};
use super::{Stdin, Walker, secret_variable, sensitive_variable};
use crate::classify::{shown_rel, write_verdict};
use crate::paths::{Location, secret_kind};
use crate::request::Role;
use crate::shell::is_name;
use crate::verdict::{Needs, Verdict};

/// Print things or change only the shell's own state.
const HARMLESS: &[&str] = &[
    "echo", "printf", "true", "false", ":", "pwd", "sleep", "date", "which", "whereis", "type", "hash",
    "whoami", "id", "uname", "hostname", "uptime", "sw_vers", "nproc", "seq", "expr", "basename", "dirname",
    "exit", "return", "wait", "set", "shopt", "unset", "umask", "ulimit", "alias", "unalias", "tput",
    "clear", "jobs", "local", "declare", "typeset", "readonly", "shift", "break", "continue", "test", "[",
    "[[", "ps", "df", "cal", "yes", "tty", "locale", "arch", "getconf", "groups", "logname", "man", "free",
    "vm_stat", "sysctl",
];

/// Read the files they're given (or stdin), and write nothing.
const READERS: &[&str] = &[
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "nl",
    "tac",
    "rev",
    "strings",
    "xxd",
    "hexdump",
    "od",
    "file",
    "stat",
    "md5",
    "md5sum",
    "shasum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "cksum",
    "b2sum",
    "du",
    "tree",
    "ls",
    "diff",
    "cmp",
    "comm",
    "column",
    "fold",
    "fmt",
    "expand",
    "unexpand",
    "cut",
    "paste",
    "join",
    "tr",
    "uniq",
    "realpath",
    "readlink",
    "bat",
    "exa",
    "eza",
    "colordiff",
    "jsonlint",
    "lsof",
];

/// Readers that list the current folder when given no operands.
const LIST_CWD: &[&str] = &["ls", "du", "tree", "exa", "eza"];

/// Value-taking options of the readers, so values aren't taken for paths.
const READER_SPEC: Spec = Spec {
    short: "nctdfFLIPsXw",
    long: &[
        "lines",
        "bytes",
        "delimiter",
        "fields",
        "characters",
        "max-depth",
        "ignore",
        "width",
    ],
};

/// Handles `name` if it's a builtin or a file utility. Returns false if not.
pub(super) fn classify(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg]) -> bool {
    let rest = &args[1..];
    match name {
        "read" => {
            for arg in rest.iter().filter(|a| !a.is_option()) {
                if let Some(var) = arg.str().filter(|v| is_name(v)) {
                    walker.env.insert(var.to_owned(), None);
                }
            }
            walker.add(Verdict::allowed("reads a line into a variable"));
        }
        "command" if rest.first().is_some_and(|a| a.is("-v") || a.is("-V")) => {
            walker.add(Verdict::allowed("looks up a command"));
        }
        _ if HARMLESS.contains(&name) => {
            walker.add(Verdict::allowed("prints text or changes only the shell's state"));
            walker.passes_listing();
        }
        "cd" | "pushd" => cd(walker, name, rest),
        "popd" => {
            walker.cwd = walker.dirs.pop().flatten();
            walker.add(Verdict::allowed("changes folder"));
        }
        "export" => export(walker, rest),
        "printenv" => printenv(walker, rest),
        "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" => grep(walker, name, rest),
        "find" => find(walker, rest),
        "fd" | "fdfind" => fd(walker, rest),
        "xargs" => xargs(walker, rest),
        "sed" | "gsed" => sed(walker, rest),
        "awk" | "gawk" | "mawk" | "nawk" => awk(walker, rest),
        "jq" | "yq" | "gojq" => jq(walker, rest),
        "sort" => {
            let parsed = args::parse(
                rest,
                &Spec {
                    short: "oktTS",
                    long: &[
                        "output",
                        "key",
                        "field-separator",
                        "temporary-directory",
                        "buffer-size",
                    ],
                },
            );
            reads(walker, &parsed.operands, false);
            if let Some(out) = parsed.value(&["-o", "--output"]) {
                walker.write(out);
            }
        }
        _ if READERS.contains(&name) => {
            let parsed = args::parse(rest, &READER_SPEC);
            reads(walker, &parsed.operands, LIST_CWD.contains(&name));
            if name == "ls" {
                let listed = parsed.operands.first().cloned().unwrap_or_else(|| Arg::lit("."));
                walker.lists(&listed);
            }
        }
        "tee" | "touch" | "mkdir" | "truncate" => {
            let spec = Spec {
                short: "tdrms",
                long: &["reference", "date", "mode", "size"],
            };
            let parsed = args::parse(rest, &spec);
            walker.add(Verdict::allowed("creates or changes files in the worktree"));
            for operand in &parsed.operands {
                walker.write(operand);
            }
            if let Some(reference) = parsed.value(&["-r", "--reference"]) {
                walker.read(reference);
            }
        }
        "cp" | "install" => copy(walker, rest, false),
        "mv" => copy(walker, rest, true),
        "ln" => link(walker, rest),
        "rm" | "rmdir" | "unlink" | "trash" => {
            let parsed = args::parse(rest, &Spec::NONE);
            let recursive = name == "rm" && parsed.has_any(&["-r", "-R", "--recursive"]);
            for operand in &parsed.operands {
                delete(walker, operand, recursive);
            }
        }
        "chmod" => chmod(walker, rest),
        "chown" | "chgrp" => {
            let operands: Vec<&Arg> = rest.iter().filter(|a| !a.is_option()).skip(1).collect();
            for operand in operands {
                match walker.locate(operand) {
                    Location::Inside { .. } => walker.add(Verdict::ask("changes file ownership")),
                    location => {
                        let verdict = write_verdict(&location, operand.shown(), walker.context.request.role);
                        walker.add(verdict);
                    }
                }
            }
        }
        "dd" => {
            walker.add(Verdict::allowed("copies data"));
            for arg in rest {
                match arg.str() {
                    Some(text) if text.starts_with("of=") => walker.write(&Arg::lit(&text[3..])),
                    Some(text) if text.starts_with("if=") => walker.read(&Arg::lit(&text[3..])),
                    None => walker.add(Verdict::ask("passes an argument that is only known when it runs")),
                    _ => {}
                }
            }
        }
        "tar" | "zip" | "unzip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz" | "zstd" | "7z"
        | "patch" => {
            walker.tool(
                Needs::WORKTREE,
                format!("packs, unpacks or patches files ({name})"),
                args,
            );
        }
        _ => return false,
    }
    true
}

fn reads(walker: &mut Walker<'_, '_>, operands: &[Arg], list_cwd: bool) {
    walker.add(Verdict::allowed("reads files in the worktree"));
    if operands.is_empty() {
        if list_cwd {
            walker.read(&Arg::lit("."));
        } else {
            walker.passes_listing();
        }
    }
    for operand in operands.iter().filter(|o| !o.is("-")) {
        walker.read(operand);
    }
}

fn cd(walker: &mut Walker<'_, '_>, name: &str, rest: &[Arg]) {
    let target = rest.iter().find(|a| !a.is_option() || a.is("-"));
    if name == "pushd" {
        walker.dirs.push(walker.cwd.clone());
    }
    walker.cwd = match target {
        None => Some(walker.context.paths.home().to_path_buf()),
        Some(arg) if arg.is("-") => None,
        Some(arg) => walker.absolute(arg),
    };
    walker.add(Verdict::allowed("changes folder"));
}

fn export(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    walker.add(Verdict::allowed("sets shell variables"));
    for arg in rest.iter().filter(|a| !a.is_option()) {
        let Some(text) = arg.str() else {
            walker.add(Verdict::ask("exports a variable cos can't read"));
            continue;
        };
        let (name, value) = match text.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (text, None),
        };
        if let Some(why) = sensitive_variable(name) {
            walker.add(Verdict::ask(format!("sets {name}, {why}")));
        }
        if value.is_some() {
            walker.env.insert(name.to_owned(), value.map(|v| vec![v]));
        }
    }
}

fn printenv(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let names: Vec<&Arg> = rest.iter().filter(|a| !a.is_option()).collect();
    if names.is_empty() {
        walker.add(Verdict::never(
            "prints every environment variable, which can include tokens",
        ));
    } else if names.iter().any(|a| a.str().is_none_or(secret_variable)) {
        walker.add(Verdict::never("prints a variable that looks like a secret"));
    } else {
        walker.add(Verdict::allowed("prints environment variables"));
    }
}

fn grep(walker: &mut Walker<'_, '_>, name: &str, rest: &[Arg]) {
    let spec = match name {
        "rg" => Spec {
            short: "efmABCgtTMjrE",
            long: &[
                "regexp",
                "file",
                "max-count",
                "after-context",
                "before-context",
                "context",
                "glob",
                "iglob",
                "type",
                "type-not",
                "type-add",
                "max-columns",
                "threads",
                "replace",
                "max-depth",
                "sort",
                "sortr",
                "encoding",
                "ignore-file",
                "max-filesize",
                "path-separator",
                "pre",
                "pre-glob",
                "colors",
                "color",
                "engine",
                "context-separator",
            ],
        },
        _ => Spec {
            short: "efmABCdDG",
            long: &[
                "regexp",
                "file",
                "max-count",
                "after-context",
                "before-context",
                "context",
                "include",
                "exclude",
                "exclude-dir",
                "exclude-from",
                "label",
                "color",
                "colour",
                "devices",
                "directories",
                "binary-files",
                "ignore-dir",
                "type",
            ],
        },
    };
    let parsed = args::parse(rest, &spec);
    if parsed.has("--pre") {
        walker.add(Verdict::ask("runs a preprocessor command for every file (--pre)"));
    }
    for file in parsed.values(&["-f", "--file", "--ignore-file", "--exclude-from"]) {
        walker.read(file);
    }
    let pattern_given = parsed.has_any(&["-e", "-f", "--regexp", "--file", "--files"]);
    let paths = if pattern_given {
        &parsed.operands[..]
    } else {
        parsed.operands.get(1..).unwrap_or_default()
    };
    let recursive = !matches!(name, "grep" | "egrep" | "fgrep")
        || parsed.has_any(&["-r", "-R", "--recursive", "--dereference-recursive"]);
    walker.add(Verdict::allowed("searches files in the worktree"));
    if paths.is_empty() {
        if recursive {
            walker.read(&Arg::lit("."));
        } else {
            walker.passes_listing();
        }
    }
    for path in paths {
        walker.read(path);
    }
    if parsed.has_any(&[
        "-l",
        "-L",
        "--files-with-matches",
        "--files-without-match",
        "--files",
    ]) {
        let listed = paths.first().cloned().unwrap_or_else(|| Arg::lit("."));
        walker.lists(&listed);
    }
}

/// Whether a `-name` style pattern could match a secret file.
fn secret_pattern(walker: &Walker<'_, '_>, pattern: &Arg) -> bool {
    let Some(text) = pattern.str() else {
        return false;
    };
    let last = text.rsplit('/').next().unwrap_or(text);
    secret_kind(Path::new(last), walker.context.paths.home(), true).is_some()
}

fn find(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut i = 0;
    let mut follows_links = false;
    while let Some(text) = rest.get(i).and_then(Arg::str) {
        match text {
            "-H" | "-P" => {}
            "-L" | "-follow" => follows_links = true,
            "-D" => i += 1,
            text if text.starts_with("-O") => {}
            _ => break,
        }
        i += 1;
    }
    let mut roots = Vec::new();
    while let Some(arg) = rest.get(i) {
        if arg
            .str()
            .is_some_and(|t| t.starts_with('-') || matches!(t, "(" | "!" | ")" | ","))
        {
            break;
        }
        roots.push(arg.clone());
        i += 1;
    }
    if roots.is_empty() {
        roots.push(Arg::lit("."));
    }
    walker.add(Verdict::allowed("lists files in the worktree"));
    for root in &roots {
        walker.read(root);
    }
    let mut changes = false;
    while let Some(arg) = rest.get(i) {
        i += 1;
        let Some(text) = arg.str() else { continue };
        match text {
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                let mut inner = Vec::new();
                while let Some(arg) = rest.get(i) {
                    i += 1;
                    if arg.is(";") || arg.is("+") {
                        break;
                    }
                    inner.push(match (arg.str(), roots[0].str()) {
                        (Some(text), Some(root)) if text.contains("{}") => Arg::lit(text.replace("{}", root)),
                        (Some(text), None) if text.contains("{}") => Arg::unknown(),
                        _ => arg.clone(),
                    });
                }
                if inner.is_empty() {
                    walker.add(Verdict::ask("find -exec without a command"));
                } else {
                    walker.dispatch(&inner, Stdin::Terminal);
                }
                changes = true;
            }
            "-delete" => {
                for root in roots.clone() {
                    delete_under(walker, &root);
                }
                changes = true;
            }
            "-fprint" | "-fprint0" | "-fls" | "-fprintf" => {
                if let Some(file) = rest.get(i) {
                    walker.write(file);
                }
                i += if text == "-fprintf" { 2 } else { 1 };
            }
            "-name" | "-iname" | "-path" | "-ipath" | "-wholename" | "-iwholename" | "-lname" | "-ilname" => {
                if let Some(pattern) = rest.get(i)
                    && secret_pattern(walker, pattern)
                {
                    walker.add(Verdict::never(format!(
                        "looks for secret files ({})",
                        pattern.shown()
                    )));
                }
                i += 1;
            }
            "-type" | "-xtype" | "-maxdepth" | "-mindepth" | "-size" | "-mtime" | "-mmin" | "-atime"
            | "-amin" | "-ctime" | "-cmin" | "-perm" | "-user" | "-group" | "-uid" | "-gid" | "-links"
            | "-inum" | "-printf" | "-fstype" | "-samefile" | "-used" | "-regex" | "-iregex"
            | "-regextype" | "-context" => i += 1,
            text if text.starts_with("-newer") || text == "-anewer" || text == "-cnewer" => i += 1,
            _ => {}
        }
    }
    if follows_links && changes {
        walker.add(Verdict::ask("follows symlinks while changing files"));
    }
    if !changes {
        walker.lists(&roots[0]);
    }
}

fn fd(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let parsed = args::parse(
        rest,
        &Spec {
            short: "edtExXjSco",
            long: &[
                "extension",
                "max-depth",
                "min-depth",
                "type",
                "exclude",
                "exec",
                "exec-batch",
                "threads",
                "size",
                "changed-within",
                "changed-before",
                "owner",
                "color",
                "base-directory",
                "search-path",
                "ignore-file",
            ],
        },
    );
    if parsed.has_any(&["-x", "-X", "--exec", "--exec-batch"]) {
        walker.add(Verdict::ask("fd runs a command for every match"));
    }
    walker.add(Verdict::allowed("lists files in the worktree"));
    let mut roots: Vec<Arg> = parsed.operands.iter().skip(1).cloned().collect();
    roots.extend(
        parsed
            .values(&["--base-directory", "--search-path"])
            .into_iter()
            .cloned(),
    );
    if roots.is_empty() {
        roots.push(Arg::lit("."));
    }
    if let Some(pattern) = parsed.operands.first()
        && secret_pattern(walker, pattern)
    {
        walker.add(Verdict::never(format!(
            "looks for secret files ({})",
            pattern.shown()
        )));
    }
    for root in &roots {
        walker.read(root);
    }
    walker.lists(&roots[0]);
}

fn xargs(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut i = 0;
    let mut replace: Option<String> = None;
    while let Some(arg) = rest.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        i += 1;
        match text {
            "-I" | "--replace" => {
                replace = rest.get(i).and_then(Arg::str).map(str::to_owned);
                i += 1;
            }
            "-i" => replace = Some("{}".to_owned()),
            "-n" | "-L" | "-P" | "-d" | "-E" | "-s" | "--max-args" | "--max-lines" | "--max-procs"
            | "--delimiter" | "--eof" | "--max-chars" => i += 1,
            "-a" | "--arg-file" => {
                if let Some(file) = rest.get(i) {
                    walker.read(file);
                }
                i += 1;
            }
            text if text.starts_with("-I") => replace = Some(text[2..].to_owned()),
            text if text.starts_with("--replace=") => replace = Some(text[10..].to_owned()),
            _ => {}
        }
    }
    let mut inner: Vec<Arg> = rest[i.min(rest.len())..].to_vec();
    if inner.is_empty() {
        walker.add(Verdict::allowed("prints its input"));
        return;
    }
    // The arguments xargs adds come from stdin: paths under the folder the
    // previous stage listed, or anything at all.
    let filler = match walker.incoming.clone() {
        Some(dir) => Arg::lit(dir.display().to_string()),
        None => Arg::unknown(),
    };
    match replace {
        Some(token) => {
            for arg in &mut inner {
                if let Some(text) = arg.str().filter(|t| t.contains(token.as_str())) {
                    *arg = match filler.str() {
                        Some(fill) => Arg::lit(text.replace(token.as_str(), fill)),
                        None => Arg::unknown(),
                    };
                }
            }
        }
        None => inner.push(filler),
    }
    walker.dispatch(&inner, Stdin::Terminal);
}

fn sed(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut in_place = false;
    let mut scripts = Vec::new();
    let mut script_file = false;
    let mut operands = Vec::new();
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        i += 1;
        let Some(text) = arg.str().filter(|_| arg.is_option()) else {
            operands.push(arg.clone());
            continue;
        };
        match text {
            "--" => {
                operands.extend(rest[i..].iter().cloned());
                break;
            }
            "-i" | "--in-place" => {
                in_place = true;
                // BSD sed: `-i ''` or `-i .bak`.
                if rest
                    .get(i)
                    .and_then(Arg::str)
                    .is_some_and(|t| t.is_empty() || t.starts_with('.'))
                {
                    i += 1;
                }
            }
            "-e" | "--expression" => {
                scripts.extend(rest.get(i).cloned());
                i += 1;
            }
            "-f" | "--file" => {
                if let Some(file) = rest.get(i) {
                    walker.read(file);
                }
                script_file = true;
                i += 1;
            }
            "-l" | "--line-length" => i += 1,
            text if text.starts_with("--in-place") => in_place = true,
            text if text.starts_with("--expression=") => scripts.push(Arg::lit(&text[13..])),
            text if text.starts_with("--") => {}
            text => {
                // In a cluster, whatever follows `i` is the backup suffix.
                let flags = &text[1..];
                let flags = match flags.find('i') {
                    Some(at) => {
                        in_place = true;
                        &flags[..at]
                    }
                    None => flags,
                };
                if flags.ends_with('e') {
                    scripts.extend(rest.get(i).cloned());
                    i += 1;
                } else if flags.ends_with('f') {
                    if let Some(file) = rest.get(i) {
                        walker.read(file);
                    }
                    script_file = true;
                    i += 1;
                }
            }
        }
    }
    if scripts.is_empty() && !script_file && !operands.is_empty() {
        scripts.push(operands.remove(0));
    }
    for script in &scripts {
        match script.str().map(sed_effect) {
            None => walker.add(Verdict::ask("the sed script is only known when it runs")),
            Some(Some(effect)) => walker.add(Verdict::ask(format!("the sed script {effect}"))),
            Some(None) => {}
        }
    }
    if script_file {
        walker.add(Verdict::chief(Needs::WORKTREE, "runs a sed script from a file"));
    }
    if in_place {
        walker.add(Verdict::allowed(
            "edits files in the worktree, inside the task's scope",
        ));
        for operand in &operands {
            walker.write(operand);
        }
    } else {
        reads(walker, &operands, false);
    }
}

/// What a sed script does besides editing the stream: `None` if nothing.
fn sed_effect(script: &str) -> Option<&'static str> {
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    let skip_line = |i: &mut usize| {
        while *i < chars.len() && chars[*i] != '\n' {
            *i += 1;
        }
    };
    // Reads up to an unescaped `delimiter`; returns false if it never comes.
    let until = |i: &mut usize, delimiter: char| -> bool {
        while *i < chars.len() {
            match chars[*i] {
                '\\' => *i += 2,
                c if c == delimiter => {
                    *i += 1;
                    return true;
                }
                _ => *i += 1,
            }
        }
        false
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace()
            || matches!(c, ';' | '{' | '}' | ',' | '!' | '$' | '~' | '+')
            || c.is_ascii_digit()
        {
            i += 1;
            continue;
        }
        match c {
            '/' => {
                i += 1;
                if !until(&mut i, '/') {
                    return Some("can't be read confidently");
                }
                while i < chars.len() && matches!(chars[i], 'I' | 'M') {
                    i += 1;
                }
            }
            '\\' => {
                let Some(&delimiter) = chars.get(i + 1) else {
                    return Some("can't be read confidently");
                };
                i += 2;
                if !until(&mut i, delimiter) {
                    return Some("can't be read confidently");
                }
            }
            's' | 'y' => {
                let Some(&delimiter) = chars.get(i + 1) else {
                    return Some("can't be read confidently");
                };
                i += 2;
                if !until(&mut i, delimiter) || !until(&mut i, delimiter) {
                    return Some("can't be read confidently");
                }
                if c == 's' {
                    while i < chars.len() && !matches!(chars[i], ';' | '\n' | '}') {
                        match chars[i] {
                            'w' | 'W' => return Some("writes a file"),
                            'e' => return Some("runs a command"),
                            _ => i += 1,
                        }
                    }
                }
            }
            'a' | 'i' | 'c' | '#' => skip_line(&mut i),
            'b' | 't' | 'T' | ':' => {
                while i < chars.len() && !matches!(chars[i], ';' | '\n') {
                    i += 1;
                }
            }
            'w' | 'W' => return Some("writes a file"),
            'r' | 'R' => return Some("reads another file"),
            'e' => return Some("runs a command"),
            'p' | 'P' | 'd' | 'D' | 'n' | 'N' | 'g' | 'G' | 'h' | 'H' | 'x' | '=' | 'l' | 'q' | 'Q' | 'z'
            | 'F' => i += 1,
            _ => return Some("uses commands cos doesn't know"),
        }
    }
    None
}

fn awk(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut program: Option<Arg> = None;
    let mut program_file = false;
    let mut in_place = false;
    let mut operands = Vec::new();
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        i += 1;
        let Some(text) = arg.str().filter(|_| arg.is_option()) else {
            operands.push(arg.clone());
            continue;
        };
        match text {
            "-f" | "--file" => {
                if let Some(file) = rest.get(i) {
                    walker.read(file);
                }
                program_file = true;
                i += 1;
            }
            "-v" | "-F" | "--assign" | "--field-separator" => i += 1,
            "-i" | "--include" => {
                in_place |= rest.get(i).is_some_and(|a| a.is("inplace"));
                i += 1;
            }
            "-e" | "--source" => {
                program = rest.get(i).cloned();
                i += 1;
            }
            "--" => {
                operands.extend(rest[i..].iter().cloned());
                break;
            }
            _ => {}
        }
    }
    if program.is_none() && !program_file && !operands.is_empty() {
        program = Some(operands.remove(0));
    }
    match program.as_ref().map(Arg::str) {
        Some(None) => walker.add(Verdict::ask("the awk program is only known when it runs")),
        Some(Some(text)) if ["system", "|", ">", "getline"].iter().any(|s| text.contains(s)) => {
            walker.add(Verdict::chief(
                Needs::WORKTREE,
                "the awk program can run commands or write files",
            ));
        }
        _ => {}
    }
    if program_file {
        walker.add(Verdict::chief(Needs::WORKTREE, "runs an awk program from a file"));
    }
    // `name=value` operands set variables, not files.
    let files: Vec<Arg> = operands
        .into_iter()
        .filter(|a| {
            !a.str()
                .and_then(|t| t.split_once('='))
                .is_some_and(|(n, _)| is_name(n))
        })
        .collect();
    if in_place {
        walker.add(Verdict::allowed(
            "edits files in the worktree, inside the task's scope",
        ));
        for file in &files {
            walker.write(file);
        }
    } else {
        reads(walker, &files, false);
    }
}

fn jq(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut filter_given = false;
    let mut in_place = false;
    let mut operands = Vec::new();
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        i += 1;
        let Some(text) = arg.str().filter(|_| arg.is_option()) else {
            operands.push(arg.clone());
            continue;
        };
        match text {
            "--arg" | "--argjson" => i += 2,
            "--slurpfile" | "--rawfile" => {
                if let Some(file) = rest.get(i + 1) {
                    walker.read(file);
                }
                i += 2;
            }
            "-f" | "--from-file" => {
                if let Some(file) = rest.get(i) {
                    walker.read(file);
                }
                filter_given = true;
                i += 1;
            }
            "-L" | "--indent" | "-o" | "--output-format" | "-p" | "--input-format" => i += 1,
            "-i" | "--inplace" => in_place = true,
            "--args" | "--jsonargs" => break,
            _ => {}
        }
    }
    if !filter_given && !operands.is_empty() {
        operands.remove(0);
    }
    if in_place {
        walker.add(Verdict::allowed(
            "edits files in the worktree, inside the task's scope",
        ));
        for operand in &operands {
            walker.write(operand);
        }
    } else {
        reads(walker, &operands, false);
    }
}

/// `cp` and `mv`: sources are read (or, for `mv`, removed), the destination
/// written.
fn copy(walker: &mut Walker<'_, '_>, rest: &[Arg], moves: bool) {
    let parsed = args::parse(
        rest,
        &Spec {
            short: "tS",
            long: &["target-directory", "suffix"],
        },
    );
    walker.add(Verdict::allowed("copies or moves files in the worktree"));
    let mut operands = parsed.operands.clone();
    let destination = match parsed.value(&["-t", "--target-directory"]) {
        Some(target) => Some(target.clone()),
        None if operands.len() >= 2 => operands.pop(),
        None => None,
    };
    for source in &operands {
        if moves {
            walker.write(source);
        } else {
            walker.read(source);
        }
    }
    if let Some(destination) = destination {
        walker.write(&destination);
    }
}

fn link(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let parsed = args::parse(
        rest,
        &Spec {
            short: "tS",
            long: &["target-directory", "suffix"],
        },
    );
    let symbolic = parsed.has_any(&["-s", "--symbolic"]);
    let mut operands = parsed.operands.clone();
    let link = match parsed.value(&["-t", "--target-directory"]) {
        Some(dir) => dir.clone(),
        None if operands.len() >= 2 => operands.pop().unwrap_or_else(Arg::unknown),
        None => Arg::lit("."),
    };
    walker.add(Verdict::allowed("creates links inside the worktree"));
    walker.write(&link);
    // A symlink's target is relative to the folder the link is in.
    let base = if symbolic {
        walker.absolute(&link).and_then(|path| {
            let is_dir = operands.len() > 1 || link.is(".");
            if is_dir {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            }
        })
    } else {
        walker.cwd.clone()
    };
    for target in &operands {
        let location = match target.str() {
            Some(text) => walker.context.paths.locate(base.as_deref(), text, false),
            None => Location::Unknown,
        };
        match location {
            Location::Inside { .. } => {}
            Location::Unknown => walker.add(Verdict::ask("creates a link to a path cos can't resolve")),
            _ => walker.add(Verdict::never(format!(
                "creates a link to {}, outside the worktree",
                target.shown()
            ))),
        }
    }
}

fn chmod(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let mut mode_seen = false;
    walker.add(Verdict::allowed("changes file modes in the worktree"));
    for arg in rest {
        let text = arg.str().unwrap_or_default();
        let is_flag = matches!(
            text,
            "-R" | "-f" | "-v" | "-h" | "-c" | "--recursive" | "--verbose"
        ) || text.starts_with("--reference=");
        if is_flag {
            continue;
        }
        if !mode_seen {
            mode_seen = true;
            continue;
        }
        walker.write(arg);
    }
}

/// Folders that only hold generated files, safe to delete recursively.
const GENERATED_ANYWHERE: &[&str] = &[
    "node_modules",
    "target",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".svelte-kit",
    ".gradle",
    ".tox",
    "htmlcov",
    ".coverage",
    "DerivedData",
];
/// Names that are generated output at the top of the worktree, but could be
/// source further down.
const GENERATED_AT_ROOT: &[&str] = &[
    "dist", "build", "out", "coverage", ".cache", "tmp", ".venv", "venv",
];

fn is_generated(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.iter().any(|part| GENERATED_ANYWHERE.contains(part))
        || parts
            .first()
            .is_some_and(|first| GENERATED_AT_ROOT.contains(first))
}

fn delete(walker: &mut Walker<'_, '_>, arg: &Arg, recursive: bool) {
    let role = walker.context.request.role;
    let verdict = match walker.locate(arg) {
        Location::Unknown if recursive => {
            Verdict::never("deletes recursively a path that is only known when it runs")
        }
        Location::Inside { rel, protected, .. } if protected.is_none() && role != Role::Reviewer => {
            let root_glob = rel
                .split('/')
                .next()
                .is_some_and(|first| first.contains(['*', '?', '[']));
            if rel.is_empty() || (recursive && root_glob) {
                Verdict::never("deletes the whole worktree")
            } else if recursive && is_generated(&rel) {
                Verdict::allowed(format!("deletes generated files ({rel})"))
            } else if recursive {
                Verdict::chief(Needs::WORKTREE, format!("deletes {rel} recursively"))
            } else {
                let location = walker.locate(arg);
                write_verdict(&location, arg.shown(), role)
            }
        }
        location => write_verdict(&location, arg.shown(), role),
    };
    walker.add(verdict);
}

/// `find … -delete` removes matching files anywhere under `root`.
fn delete_under(walker: &mut Walker<'_, '_>, root: &Arg) {
    let role = walker.context.request.role;
    let verdict = match walker.locate(root) {
        Location::Inside { rel, protected, .. } if protected.is_none() && role != Role::Reviewer => {
            if is_generated(&rel) {
                Verdict::allowed(format!("deletes generated files ({rel})"))
            } else {
                Verdict::chief(
                    Needs::WORKTREE,
                    format!("deletes files found under {}", shown_rel(&rel)),
                )
            }
        }
        location => write_verdict(&location, root.shown(), role),
    };
    walker.add(verdict);
}
