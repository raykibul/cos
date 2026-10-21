//! HTTP clients, `gh`, remote shells and containers. Requests that only fetch
//! are network for the chief (Trusted); anything that sends data is never
//! delegated.

use super::Walker;
use super::args::{self, Arg, Spec};
use crate::verdict::{Needs, Verdict};

/// Handles `name` if it talks to the network or to other machines.
pub(super) fn classify(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg]) -> bool {
    match name {
        "curl" => curl(walker, &args[1..]),
        "wget" => wget(walker, &args[1..]),
        "http" | "https" | "xh" | "xhs" => httpie(walker, &args[1..]),
        "gh" => gh(walker, &args[1..]),
        "ssh" | "scp" | "sftp" | "nc" | "ncat" | "netcat" | "telnet" | "ftp" | "socat" | "mosh"
        | "ssh-copy-id" | "ssh-add" | "ssh-keygen" => {
            walker.add(Verdict::never(format!(
                "connects to another machine or uses SSH keys ({name})"
            )));
        }
        "rsync" => rsync(walker, &args[1..]),
        "docker" | "podman" | "colima" | "orbctl" | "docker-compose" => docker(walker, &args[1..]),
        "ping" | "dig" | "nslookup" | "host" | "traceroute" => {
            walker.add(Verdict::chief(
                Needs::NETWORK,
                format!("looks up or probes a host ({name})"),
            ));
        }
        _ => return false,
    }
    true
}

/// Rejects URLs that read local files instead of fetching.
fn check_urls(walker: &mut Walker<'_, '_>, urls: &[Arg]) {
    for url in urls {
        match url.str() {
            None => walker.add(Verdict::ask("fetches a URL that is only known when it runs")),
            Some(text) if text.to_ascii_lowercase().starts_with("file:") => {
                walker.add(Verdict::never("reads a local file through a file:// URL"));
            }
            Some(_) => {}
        }
    }
}

fn curl(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let spec = Spec {
        short: "XdoHuFTAbcexwmKEeyYzrCtQUDP",
        long: &[
            "request",
            "data",
            "data-raw",
            "data-binary",
            "data-urlencode",
            "data-ascii",
            "json",
            "form",
            "form-string",
            "upload-file",
            "output",
            "output-dir",
            "header",
            "user-agent",
            "referer",
            "max-time",
            "connect-timeout",
            "write-out",
            "range",
            "cookie",
            "cookie-jar",
            "user",
            "proxy",
            "cert",
            "key",
            "cacert",
            "capath",
            "config",
            "dump-header",
            "unix-socket",
            "abstract-unix-socket",
            "retry",
            "retry-delay",
            "retry-max-time",
            "limit-rate",
            "resolve",
            "connect-to",
            "url",
            "oauth2-bearer",
            "proxy-user",
            "max-filesize",
            "netrc-file",
            "trace",
            "trace-ascii",
            "stderr",
            "interface",
        ],
    };
    let parsed = args::parse(rest, &spec);
    let sends = parsed.has_any(&[
        "-d",
        "--data",
        "--data-raw",
        "--data-binary",
        "--data-urlencode",
        "--data-ascii",
        "--json",
        "-F",
        "--form",
        "--form-string",
        "-T",
        "--upload-file",
    ]);
    let method = parsed
        .value(&["-X", "--request"])
        .map(|m| m.str().unwrap_or("?").to_ascii_uppercase());
    let as_get = parsed.has_any(&["-G", "--get"]);
    match method.as_deref() {
        Some("GET" | "HEAD") | None if !sends || as_get => {
            walker.add(Verdict::chief(Needs::NETWORK, "fetches a URL (GET)"));
        }
        Some(method) if method != "GET" && method != "HEAD" => {
            walker.add(Verdict::never(format!("sends a {method} request")));
        }
        _ => walker.add(Verdict::never("sends data to a server")),
    }
    if parsed.has_any(&["-n", "--netrc", "--netrc-optional", "--netrc-file"]) {
        walker.add(Verdict::never("uses your .netrc credentials"));
    }
    if parsed.has_any(&["-K", "--config"]) {
        walker.add(Verdict::ask("reads curl options from a file"));
    }
    for path in parsed.values(&[
        "--unix-socket",
        "--abstract-unix-socket",
        "-E",
        "--cert",
        "--key",
        "--cacert",
        "-b",
        "--cookie",
    ]) {
        if path.str().is_some_and(|p| p.contains('/') || p.contains('.')) {
            walker.read(path);
        }
    }
    for path in parsed.values(&[
        "-o",
        "--output",
        "-c",
        "--cookie-jar",
        "-D",
        "--dump-header",
        "--output-dir",
        "--trace",
        "--trace-ascii",
        "--stderr",
    ]) {
        if !path.is("-") {
            walker.write(path);
        }
    }
    if parsed.has_any(&["-O", "--remote-name", "--remote-name-all"]) && !parsed.has("--output-dir") {
        walker.write(&Arg::lit("."));
    }
    let mut urls = parsed.operands.clone();
    urls.extend(parsed.values(&["--url"]).into_iter().cloned());
    check_urls(walker, &urls);
}

fn wget(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let spec = Spec {
        short: "OPioaeUTtwlAR",
        long: &[
            "output-document",
            "directory-prefix",
            "input-file",
            "output-file",
            "append-output",
            "execute",
            "user-agent",
            "timeout",
            "tries",
            "wait",
            "level",
            "accept",
            "reject",
            "post-data",
            "post-file",
            "body-data",
            "body-file",
            "method",
            "header",
            "user",
            "password",
            "config",
        ],
    };
    let parsed = args::parse(rest, &spec);
    let method = parsed
        .value(&["--method"])
        .map(|m| m.str().unwrap_or("?").to_ascii_uppercase());
    if parsed.has_any(&["--post-data", "--post-file", "--body-data", "--body-file"])
        || method.as_deref().is_some_and(|m| m != "GET" && m != "HEAD")
    {
        walker.add(Verdict::never("sends data to a server"));
    } else {
        walker.add(Verdict::chief(Needs::NETWORK, "downloads a URL (GET)"));
    }
    if parsed.has_any(&["-e", "--execute", "--config"]) {
        walker.add(Verdict::ask("passes wget extra configuration"));
    }
    for file in parsed.values(&["-i", "--input-file"]) {
        walker.read(file);
    }
    let output = parsed.value(&["-O", "--output-document"]).cloned();
    match output {
        Some(file) if file.is("-") => {}
        Some(file) => walker.write(&file),
        None => match parsed.value(&["-P", "--directory-prefix"]) {
            Some(dir) => walker.write(&dir.clone()),
            None if !parsed.has("--spider") => walker.write(&Arg::lit(".")),
            None => {}
        },
    }
    for log in parsed.values(&["-o", "--output-file", "-a", "--append-output"]) {
        walker.write(&log.clone());
    }
    check_urls(walker, &parsed.operands);
}

/// HTTPie and xh: `http [METHOD] URL [items]`. Any request item that is a
/// field (`a=b`, `a:=1`) or a file (`@f`) sends data.
fn httpie(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let parsed = args::parse(
        rest,
        &Spec {
            short: "aoA",
            long: &[
                "auth",
                "output",
                "auth-type",
                "session",
                "verify",
                "cert",
                "cert-key",
                "timeout",
            ],
        },
    );
    let mut operands = parsed.operands.iter();
    let mut first = operands.next();
    let method = first
        .and_then(Arg::str)
        .filter(|m| m.chars().all(|c| c.is_ascii_uppercase()))
        .map(str::to_owned);
    if method.is_some() {
        first = operands.next();
    }
    let sends_items = operands.any(|item| {
        item.str()
            .is_none_or(|t| t.contains('=') || t.contains(":=") || t.starts_with('@') || t.contains("=@"))
    });
    match method.as_deref() {
        Some(m) if m != "GET" && m != "HEAD" => walker.add(Verdict::never(format!("sends a {m} request"))),
        _ if sends_items => walker.add(Verdict::never("sends data to a server")),
        _ => walker.add(Verdict::chief(Needs::NETWORK, "fetches a URL (GET)")),
    }
    if parsed.has_any(&["--session"]) {
        walker.add(Verdict::ask("uses a stored HTTPie session"));
    }
    for file in parsed.values(&["-o", "--output"]) {
        walker.write(&file.clone());
    }
    if parsed.has_any(&["-d", "--download"]) {
        walker.write(&Arg::lit("."));
    }
    check_urls(walker, first.cloned().as_slice());
}

fn gh(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let group = rest.first().and_then(Arg::str).unwrap_or_default();
    let action = rest
        .iter()
        .skip(1)
        .find(|a| !a.is_option())
        .and_then(Arg::str)
        .unwrap_or_default();
    match group {
        "--version" | "version" | "help" | "--help" | "" => walker.add(Verdict::allowed("prints gh's help")),
        "auth" | "secret" | "ssh-key" | "gpg-key" | "config" | "extension" | "alias" | "variable" => {
            walker.add(Verdict::never(format!(
                "changes GitHub credentials or gh settings (gh {group})"
            )));
        }
        "api" => {
            let parsed = args::parse(
                &rest[1..],
                &Spec {
                    short: "XfFHqtp",
                    long: &[
                        "method",
                        "field",
                        "raw-field",
                        "header",
                        "jq",
                        "template",
                        "input",
                        "preview",
                        "hostname",
                        "cache",
                    ],
                },
            );
            let method = parsed
                .value(&["-X", "--method"])
                .map(|m| m.str().unwrap_or("?").to_ascii_uppercase());
            let sends = parsed.has_any(&["-f", "-F", "--field", "--raw-field", "--input"]);
            if sends || method.as_deref().is_some_and(|m| m != "GET") {
                walker.add(Verdict::never("sends a request that changes things on GitHub"));
            } else {
                walker.add(Verdict::chief(Needs::NETWORK, "reads from the GitHub API"));
            }
        }
        "browse" | "codespace" => walker.add(Verdict::ask(format!("gh {group} acts outside the shell"))),
        "search" | "status" => walker.add(Verdict::chief(Needs::NETWORK, "reads from GitHub")),
        _ => match action {
            "view" | "list" | "status" | "diff" | "checks" | "watch" | "ls" => {
                walker.add(Verdict::chief(
                    Needs::NETWORK,
                    format!("reads from GitHub (gh {group} {action})"),
                ));
            }
            "checkout" | "clone" | "download" => {
                walker.add(Verdict::chief(
                    Needs::NETWORK,
                    format!("downloads from GitHub (gh {group} {action})"),
                ));
                walker.uses_cwd();
            }
            _ => walker.add(Verdict::never(format!(
                "changes things on GitHub (gh {group} {action})"
            ))),
        },
    }
}

fn rsync(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    let parsed = args::parse(
        rest,
        &Spec {
            short: "eTBf",
            long: &[
                "rsh",
                "rsync-path",
                "exclude",
                "include",
                "exclude-from",
                "include-from",
                "filter",
                "temp-dir",
                "files-from",
                "password-file",
                "backup-dir",
                "chmod",
                "log-file",
            ],
        },
    );
    let remote = parsed.operands.iter().any(|a| {
        a.str().is_none_or(|t| {
            t.starts_with("rsync://") || t.split_once(':').is_some_and(|(host, _)| !host.contains('/'))
        })
    });
    if remote || parsed.has_any(&["-e", "--rsh", "--rsync-path"]) {
        walker.add(Verdict::never("copies files to or from another machine"));
        return;
    }
    walker.add(Verdict::allowed("copies files in the worktree"));
    let mut operands = parsed.operands.clone();
    if let Some(destination) = operands.pop() {
        for source in &operands {
            walker.read(source);
        }
        if parsed.has_any(&[
            "--delete",
            "--delete-after",
            "--delete-before",
            "--delete-during",
            "--remove-source-files",
        ]) {
            operands.iter().for_each(|source| walker.write(source));
        }
        walker.write(&destination);
    }
}

fn docker(walker: &mut Walker<'_, '_>, rest: &[Arg]) {
    const READS: &[&str] = &[
        "ps",
        "images",
        "version",
        "info",
        "inspect",
        "logs",
        "stats",
        "top",
        "port",
        "--version",
    ];
    let sub = rest
        .iter()
        .find(|a| !a.is_option())
        .and_then(Arg::str)
        .unwrap_or_default();
    let compose_read = sub == "compose"
        && rest
            .iter()
            .filter(|a| !a.is_option())
            .nth(1)
            .and_then(Arg::str)
            .is_some_and(|s| matches!(s, "ps" | "logs" | "config" | "ls" | "images" | "version"));
    if READS.contains(&sub) || compose_read {
        walker.add(Verdict::allowed("reads container state"));
    } else {
        walker.add(Verdict::ask(
            "runs or changes containers, which can mount your files and use the network",
        ));
    }
}
