//! Build tools and package managers. Builds and tests in the worktree are for
//! the chief; installing into the worktree is "install" (Trusted only);
//! installing globally, publishing and changing registry settings are never
//! delegated.

use super::Walker;
use super::args::Arg;
use crate::verdict::{Needs, Verdict};

/// Build, test and lint tools that run code in the worktree.
const BUILD_TOOLS: &[&str] = &[
    "cmake",
    "ctest",
    "ninja",
    "meson",
    "bazel",
    "bazelisk",
    "buck",
    "just",
    "task",
    "mage",
    "rake",
    "ant",
    "tsc",
    "tsx",
    "ts-node",
    "esbuild",
    "vite",
    "vitest",
    "jest",
    "mocha",
    "ava",
    "playwright",
    "cypress",
    "webpack",
    "rollup",
    "parcel",
    "next",
    "nuxt",
    "turbo",
    "nx",
    "lerna",
    "eslint",
    "prettier",
    "biome",
    "stylelint",
    "black",
    "isort",
    "ruff",
    "flake8",
    "pylint",
    "mypy",
    "pyright",
    "pytest",
    "coverage",
    "rustc",
    "rustfmt",
    "rustdoc",
    "cc",
    "gcc",
    "g++",
    "clang",
    "clang++",
    "ld",
    "swiftc",
    "swift",
    "xcodebuild",
    "xcrun",
    "xcodegen",
    "javac",
    "java",
    "kotlinc",
    "scalac",
    "goimports",
    "golangci-lint",
    "staticcheck",
    "shellcheck",
    "shfmt",
    "hadolint",
    "actionlint",
    "yamllint",
    "markdownlint",
    "clang-format",
    "swiftformat",
    "swiftlint",
    "elixir",
    "ghc",
    "zig",
    "nim",
    "protoc",
    "buf",
    "sqlc",
    "wasm-pack",
    "trunk",
    "cargo-nextest",
    "hyperfine",
    "tokei",
    "cloc",
    "swift-format",
    "lint-staged",
    "storybook",
    "tailwindcss",
    "postcss",
    "babel",
    "svelte-kit",
    "astro",
    "remix",
    "expo",
    "detox",
    "phpunit",
    "rspec",
    "rails",
    "dart",
];

/// Build tools that download dependencies as they go.
const DOWNLOADING_TOOLS: &[&str] = &[
    "mvn", "mvnw", "gradle", "gradlew", "sbt", "dotnet", "flutter", "mix", "tox", "nox", "cabal", "stack",
    "rebar3", "composer", "pod", "carthage", "spm",
];

/// System package managers and language version managers.
const SYSTEM_PACKAGES: &[&str] = &[
    "apt", "apt-get", "yum", "dnf", "pacman", "port", "snap", "flatpak", "nix-env", "mas", "zypper", "apk",
];
const VERSION_MANAGERS: &[&str] = &[
    "nvm", "fnm", "volta", "asdf", "mise", "pyenv", "rbenv", "sdk", "jenv", "goenv", "n",
];

/// Handles `name` if it's a build tool or a package manager.
pub(super) fn classify(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg], local: bool) -> bool {
    match name {
        "cargo" => cargo(walker, args),
        "npm" | "pnpm" | "yarn" | "bun" => node_packages(walker, name, args),
        "npx" | "bunx" | "pnpx" => walker.tool(
            Needs::INSTALL,
            format!("may download and run a package ({name})"),
            args,
        ),
        "pip" => pip(walker, &args[0..], local),
        "uv" => uv(walker, args),
        "uvx" | "pipx" => match args.get(1).and_then(Arg::str) {
            Some("install" | "uninstall" | "upgrade" | "inject" | "ensurepath") => {
                walker.add(Verdict::never(format!("installs tools globally ({name})")));
            }
            _ => walker.tool(
                Needs::INSTALL,
                format!("downloads and runs a Python tool ({name})"),
                args,
            ),
        },
        "poetry" | "pipenv" | "pdm" | "hatch" | "rye" => python_project(walker, name, args),
        "conda" | "mamba" | "micromamba" => {
            walker.add(Verdict::ask("conda environments live outside the worktree"));
        }
        "go" => go(walker, args),
        "gofmt" => {
            let writes = args.iter().any(|a| a.is("-w"));
            walker.add(Verdict::allowed("formats Go code"));
            for file in args[1..].iter().filter(|a| !a.is_option()) {
                if writes {
                    walker.write(file)
                } else {
                    walker.read(file)
                }
            }
        }
        "brew" => brew(walker, args),
        "gem" => match sub(args) {
            Some("list" | "search" | "info" | "environment" | "contents" | "which" | "--version") => {
                walker.add(Verdict::allowed("reads installed gems"));
            }
            _ => walker.add(Verdict::never("installs or publishes gems globally")),
        },
        "bundle" | "bundler" => match sub(args) {
            Some("install" | "update" | "add" | "remove" | "lock" | "package" | "cache") | None => {
                walker.tool(Needs::INSTALL, "installs gems into the project", args);
            }
            Some("exec") => walker.dispatch(&args[2..], super::Stdin::Terminal),
            Some("config") => walker.add(Verdict::never("changes Bundler settings")),
            Some("check" | "list" | "show" | "outdated" | "info" | "version" | "--version") => {
                walker.add(Verdict::allowed("reads the bundle"));
            }
            Some(_) => walker.add(Verdict::ask("cos doesn't know this bundle command")),
        },
        "rustup" => match sub(args) {
            Some("show" | "which" | "--version" | "doc") => {
                walker.add(Verdict::allowed("reads the Rust toolchain"))
            }
            Some("toolchain" | "target" | "component") if args.get(2).is_some_and(|a| a.is("list")) => {
                walker.add(Verdict::allowed("lists Rust toolchains"));
            }
            Some("run") if args.len() > 3 => walker.dispatch(&args[3..], super::Stdin::Terminal),
            _ => walker.add(Verdict::never("changes the Rust toolchains installed for you")),
        },
        "pre-commit" => match sub(args) {
            Some("run") | None => {
                walker.tool(Needs::INSTALL, "runs pre-commit hooks (may download them)", args)
            }
            Some("autoupdate") => walker.tool(Needs::INSTALL, "updates pre-commit hooks", args),
            Some(_) => walker.add(Verdict::never("installs or removes git hooks in your repository")),
        },
        "husky" => walker.add(Verdict::never("installs git hooks in your repository")),
        "make" | "gmake" => make(walker, args),
        _ if SYSTEM_PACKAGES.contains(&name) => {
            walker.add(Verdict::never(format!("changes system packages ({name})")))
        }
        _ if VERSION_MANAGERS.contains(&name) => match sub(args) {
            Some("ls" | "list" | "current" | "which" | "version" | "--version" | "ls-remote") => {
                walker.add(Verdict::allowed("lists installed versions"));
            }
            _ => walker.add(Verdict::never(format!(
                "changes installed language versions ({name})"
            ))),
        },
        _ if DOWNLOADING_TOOLS.contains(&name) => {
            walker.tool(
                Needs::INSTALL,
                format!("builds with {name}, which may download dependencies"),
                args,
            );
        }
        _ if BUILD_TOOLS.contains(&name) => {
            walker.tool(Needs::WORKTREE, format!("runs {name} in the worktree"), args)
        }
        _ => return false,
    }
    true
}

/// The first argument that isn't an option.
fn sub(args: &[Arg]) -> Option<&str> {
    args[1..].iter().find(|a| !a.is_option()).and_then(Arg::str)
}

fn cargo(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    let mut i = 1;
    if args.get(i).and_then(Arg::str).is_some_and(|t| t.starts_with('+')) {
        i += 1;
    }
    let mut dir = walker.cwd.clone();
    while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        i += 1;
        match text {
            "--manifest-path" | "--target-dir" => {
                walker.scan(&args[i..(i + 1).min(args.len())]);
                i += 1;
            }
            "-C" => {
                dir = match (args.get(i).and_then(Arg::str), &dir) {
                    (Some(path), Some(base)) => walker.context.paths.absolute(Some(base), path),
                    _ => None,
                };
                i += 1;
            }
            "--config" => {
                walker.add(Verdict::ask("overrides cargo configuration (--config)"));
                i += 1;
            }
            "-Z" | "--color" | "-j" | "--jobs" => i += 1,
            "--version" | "-V" | "--help" | "-h" | "--list" => {
                walker.add(Verdict::allowed("prints cargo's help"));
                return;
            }
            _ => {}
        }
    }
    let Some(sub) = args.get(i).and_then(Arg::str).map(str::to_owned) else {
        walker.add(Verdict::allowed("prints cargo's help"));
        return;
    };
    let saved = std::mem::replace(&mut walker.cwd, dir);
    let rest = &args[i..];
    let has = |name: &str| rest.iter().any(|a| a.is(name));
    match sub.as_str() {
        "fmt" if has("--check") => {
            walker.add(Verdict::allowed("checks formatting"));
            walker.uses_cwd();
        }
        "build" | "b" | "check" | "c" | "test" | "t" | "run" | "r" | "clippy" | "doc" | "d" | "bench"
        | "nextest" | "expand" | "llvm-cov" | "miri" | "tarpaulin" | "fix" | "clean" | "fmt" | "rustc"
        | "rustdoc" | "package" | "insta" | "hack" | "machete" | "metadata" | "tree" | "bloat" | "asm" => {
            walker.tool(
                Needs::WORKTREE,
                format!("builds or runs code with cargo {sub}"),
                rest,
            );
        }
        "add" | "remove" | "rm" | "update" | "up" | "fetch" | "generate-lockfile" | "vendor" | "deny"
        | "audit" | "outdated" | "udeps" | "semver-checks" => {
            walker.tool(
                Needs::INSTALL,
                format!("changes or downloads dependencies (cargo {sub})"),
                rest,
            );
        }
        "search" | "info" => walker.tool(Needs::NETWORK, format!("searches crates.io (cargo {sub})"), rest),
        "install" | "uninstall" | "binstall" => {
            walker.add(Verdict::never("installs a program globally (~/.cargo/bin)"));
        }
        "publish" | "login" | "logout" | "owner" | "yank" => {
            walker.add(Verdict::never(format!(
                "publishes or logs in to a registry (cargo {sub})"
            )));
        }
        "new" | "init" => {
            walker.add(Verdict::chief(Needs::WORKTREE, "creates a new crate"));
            let target = rest[1..]
                .iter()
                .find(|a| !a.is_option())
                .cloned()
                .unwrap_or_else(|| Arg::lit("."));
            walker.write(&target);
        }
        "version" | "help" | "locate-project" | "pkgid" | "verify-project" | "read-manifest" | "config" => {
            walker.add(Verdict::allowed("reads cargo metadata"));
        }
        _ => walker.add(Verdict::ask(format!(
            "cos doesn't know `cargo {sub}` (it may be a plugin)"
        ))),
    }
    walker.cwd = saved;
}

fn node_packages(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg]) {
    let mut i = 1;
    let mut dir = walker.cwd.clone();
    let mut global = false;
    while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
        let text = arg.str().unwrap_or_default();
        i += 1;
        match text {
            "--prefix" | "-C" | "--dir" | "--cwd" => {
                dir = match (args.get(i).and_then(Arg::str), &dir) {
                    (Some(path), Some(base)) => walker.context.paths.absolute(Some(base), path),
                    _ => None,
                };
                i += 1;
            }
            "--filter" | "-F" | "--workspace" | "-w" | "--loglevel" => i += 1,
            "-g" | "--global" | "--location=global" => global = true,
            "-v" | "--version" | "-h" | "--help" if args.len() == 2 => {
                walker.add(Verdict::allowed(format!("prints {name}'s version")));
                return;
            }
            _ => {}
        }
    }
    let rest = &args[i.min(args.len())..];
    global |= rest
        .iter()
        .any(|a| a.is("-g") || a.is("--global") || a.is("--location=global"));
    let sub = rest.first().and_then(Arg::str).unwrap_or_default().to_owned();
    let second = rest
        .iter()
        .skip(1)
        .find(|a| !a.is_option())
        .and_then(Arg::str)
        .unwrap_or_default()
        .to_owned();
    if global || (name == "yarn" && sub == "global") {
        walker.add(Verdict::never(format!("installs packages globally ({name})")));
        return;
    }
    if rest.first().is_some_and(|a| a.str().is_none()) {
        walker.add(Verdict::ask(
            "runs a package command that is only known when it runs",
        ));
        return;
    }
    let saved = std::mem::replace(&mut walker.cwd, dir);
    let all = &args[i.saturating_sub(1)..];
    let installs = format!("changes or downloads dependencies ({name} {sub})");
    match sub.as_str() {
        "" if name == "yarn" || name == "bun" => {
            walker.tool(Needs::INSTALL, format!("installs dependencies ({name})"), all)
        }
        "test" | "t" | "tst" | "run" | "run-script" | "rum" | "urn" | "start" | "stop" | "restart" => {
            walker.tool(
                Needs::WORKTREE,
                format!("runs a package script ({name} {sub})"),
                all,
            );
        }
        "install" | "i" | "in" | "ins" | "isntall" | "add" | "a" | "ci" | "clean-install" | "ic"
        | "install-clean" | "update" | "up" | "upgrade" | "uninstall" | "remove" | "rm" | "r" | "un"
        | "dedupe" | "ddp" | "prune" | "rebuild" | "rb" | "install-test" | "it" | "import" | "fetch"
        | "patch" | "patch-commit" => {
            walker.tool(Needs::INSTALL, installs, all);
        }
        "audit" if second == "fix" || rest.iter().any(|a| a.is("--fix")) => {
            walker.tool(Needs::INSTALL, installs, all)
        }
        "exec" | "x" | "dlx" | "create" | "init" | "innit" => {
            walker.tool(
                Needs::INSTALL,
                format!("may download and run a package ({name} {sub})"),
                all,
            );
        }
        "audit" | "outdated" | "view" | "v" | "info" | "show" | "search" | "s" | "se" | "find" | "ping"
        | "doctor" | "fund" => walker.tool(
            Needs::NETWORK,
            format!("reads from the package registry ({name} {sub})"),
            all,
        ),
        "ls" | "list" | "la" | "ll" | "why" | "explain" | "root" | "bin" | "prefix" | "help" | "query"
        | "sbom" | "licenses" | "workspaces" => {
            walker.add(Verdict::allowed(format!(
                "reads installed packages ({name} {sub})"
            )));
            walker.uses_cwd();
        }
        "version" if second.is_empty() => walker.add(Verdict::allowed("prints package versions")),
        "version" | "pack" => walker.tool(
            Needs::WORKTREE,
            format!("changes package files ({name} {sub})"),
            all,
        ),
        "pkg" if second == "get" => walker.add(Verdict::allowed("reads package.json")),
        "pkg" => walker.tool(Needs::WORKTREE, "changes package.json", all),
        "config" | "c" | "set" | "get"
            if matches!(second.as_str(), "get" | "list" | "ls") || sub == "get" =>
        {
            walker.add(Verdict::allowed(format!("reads {name} settings")));
        }
        "config" | "c" | "set" => walker.add(Verdict::never(format!("changes {name} settings"))),
        "publish" | "unpublish" | "deprecate" | "dist-tag" | "owner" | "access" | "adduser" | "add-user"
        | "login" | "logout" | "token" | "team" | "org" | "profile" | "star" | "unstar" | "link"
        | "unlink" | "npm" => {
            walker.add(Verdict::never(format!(
                "publishes, links globally or changes your registry account ({name} {sub})"
            )));
        }
        "cache" | "store" | "env" | "setup" | "self-update" | "upgrade-self" | "edit" | "explore"
        | "docs" | "home" | "bugs" | "repo" => {
            walker.add(Verdict::ask(format!("{name} {sub} acts outside the worktree")))
        }
        _ if name == "npm" => walker.add(Verdict::ask(format!("cos doesn't know `npm {sub}`"))),
        // yarn, pnpm and bun run a package script for an unknown command,
        // or a file for bun.
        _ => walker.tool(
            Needs::WORKTREE,
            format!("runs a package script or file ({name} {sub})"),
            all,
        ),
    }
    walker.cwd = saved;
}

/// `pip` and `python -m pip`. `args[0]` is the program or module name.
/// `local` says the interpreter lives in the worktree, so installs land in
/// its virtualenv.
pub(super) fn pip(walker: &mut Walker<'_, '_>, args: &[Arg], local: bool) {
    let rest = &args[1..];
    let sub = rest
        .iter()
        .find(|a| !a.is_option())
        .and_then(Arg::str)
        .unwrap_or_default();
    let has = |names: &[&str]| {
        rest.iter().any(|a| {
            a.str()
                .is_some_and(|t| names.iter().any(|n| t == *n || t.starts_with(&format!("{n}="))))
        })
    };
    match sub {
        "install" | "download" | "uninstall" | "wheel" => {
            if has(&["--user", "--break-system-packages"]) {
                walker.add(Verdict::never("installs into your user or system Python"));
            } else if has(&["-t", "--target", "--prefix", "--root"]) || local {
                walker.tool(
                    Needs::INSTALL,
                    format!("installs Python packages into the worktree (pip {sub})"),
                    args,
                );
            } else if sub == "download" || sub == "wheel" {
                walker.tool(
                    Needs::INSTALL,
                    format!("downloads Python packages (pip {sub})"),
                    args,
                );
            } else {
                walker.add(Verdict::ask(
                    "can't tell whether pip installs into a project virtualenv or globally",
                ));
            }
            let mut files = rest.iter();
            while let Some(arg) = files.next() {
                let names_file = ["-r", "--requirement", "-c", "--constraint"]
                    .iter()
                    .any(|f| arg.is(f));
                if names_file && let Some(file) = files.next() {
                    walker.read(file);
                }
            }
        }
        "list" | "show" | "freeze" | "check" | "inspect" | "debug" | "help" | "" => {
            walker.add(Verdict::allowed("reads installed Python packages"));
        }
        "index" | "search" => walker.add(Verdict::chief(Needs::NETWORK, "reads from the package index")),
        "config" | "cache" => walker.add(Verdict::never(format!(
            "changes pip settings or its cache (pip {sub})"
        ))),
        _ => walker.add(Verdict::ask(format!("cos doesn't know `pip {sub}`"))),
    }
}

fn uv(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    let sub = sub(args).unwrap_or_default();
    let rest_from = |name: &str| args.iter().position(|a| a.is(name)).map_or(args.len(), |p| p + 1);
    match sub {
        "run" => {
            // Options of `uv run` before the command.
            let mut i = rest_from("run");
            while let Some(arg) = args.get(i).filter(|a| a.is_option()) {
                let text = arg.str().unwrap_or_default();
                i += 1;
                match text {
                    "--with" | "--with-requirements" | "--with-editable" => {
                        walker.add(Verdict::chief(
                            Needs::INSTALL,
                            "installs extra packages for the run (uv run --with)",
                        ));
                        i += 1;
                    }
                    "--env-file" => {
                        if let Some(file) = args.get(i) {
                            walker.read(file);
                        }
                        i += 1;
                    }
                    "--python" | "-p" | "--directory" | "--project" | "--package" | "--extra" | "--group"
                    | "--index" => {
                        walker.scan(&args[i..(i + 1).min(args.len())]);
                        i += 1;
                    }
                    _ => {}
                }
            }
            let inner = &args[i.min(args.len())..];
            if inner.is_empty() {
                walker.add(Verdict::ask("uv run without a command"));
            } else {
                walker.dispatch(inner, super::Stdin::Terminal);
            }
        }
        "add" | "remove" | "sync" | "lock" | "venv" | "export" | "build" | "init" | "tree" => {
            walker.tool(
                Needs::INSTALL,
                format!("changes or downloads Python dependencies (uv {sub})"),
                args,
            );
        }
        "pip" => {
            let pip_sub = args
                .iter()
                .skip(rest_from("pip"))
                .find(|a| !a.is_option())
                .and_then(Arg::str);
            match pip_sub {
                Some("list" | "show" | "freeze" | "check" | "tree") => {
                    walker.add(Verdict::allowed("reads installed packages"))
                }
                _ => walker.tool(
                    Needs::INSTALL,
                    "installs Python packages into the project's virtualenv (uv pip)",
                    args,
                ),
            }
        }
        "tool" => match args
            .iter()
            .skip(rest_from("tool"))
            .find(|a| !a.is_option())
            .and_then(Arg::str)
        {
            Some("run") => walker.tool(
                Needs::INSTALL,
                "downloads and runs a Python tool (uv tool run)",
                args,
            ),
            Some("list" | "dir") => walker.add(Verdict::allowed("lists installed tools")),
            _ => walker.add(Verdict::never("installs tools globally (uv tool)")),
        },
        "python" | "self" | "cache" => {
            walker.add(Verdict::never(format!("changes uv's global state (uv {sub})")))
        }
        "publish" => walker.add(Verdict::never("publishes a package")),
        "version" | "--version" | "help" | "" => walker.add(Verdict::allowed("prints uv's version")),
        _ => walker.add(Verdict::ask(format!("cos doesn't know `uv {sub}`"))),
    }
}

fn python_project(walker: &mut Walker<'_, '_>, name: &str, args: &[Arg]) {
    match sub(args).unwrap_or_default() {
        "install" | "add" | "remove" | "update" | "lock" | "sync" | "uninstall" => {
            walker.tool(
                Needs::INSTALL,
                format!("changes or downloads Python dependencies ({name})"),
                args,
            );
        }
        "run" => {
            let at = args
                .iter()
                .position(|a| a.is("run"))
                .map_or(args.len(), |p| p + 1);
            let inner = &args[at.min(args.len())..];
            if inner.is_empty() {
                walker.add(Verdict::ask(format!("{name} run without a command")));
            } else {
                walker.dispatch(inner, super::Stdin::Terminal);
            }
        }
        "build" | "check" | "test" => walker.tool(
            Needs::WORKTREE,
            format!("builds or checks the project ({name})"),
            args,
        ),
        "show" | "version" | "--version" | "env" | "graph" | "list" => {
            walker.add(Verdict::allowed("reads project metadata"))
        }
        "publish" | "config" | "self" | "source" => {
            walker.add(Verdict::never(format!("publishes or changes {name} settings")))
        }
        _ => walker.add(Verdict::ask(format!("cos doesn't know this {name} command"))),
    }
}

fn go(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    let sub = sub(args).unwrap_or_default();
    match sub {
        "build" | "test" | "vet" | "run" | "generate" | "fmt" | "list" | "doc" | "tool" | "fix" | "work" => {
            walker.tool(
                Needs::WORKTREE,
                format!("builds or runs Go code (go {sub})"),
                args,
            );
        }
        "clean"
            if args.iter().any(|a| {
                a.str().is_some_and(|t| {
                    t.starts_with("-cache") || t.starts_with("-modcache") || t.starts_with("-testcache")
                })
            }) =>
        {
            walker.add(Verdict::never("clears Go caches outside the worktree"));
        }
        "clean" => walker.tool(Needs::WORKTREE, "removes Go build output", args),
        "get" | "mod" => walker.tool(
            Needs::INSTALL,
            format!("changes or downloads Go modules (go {sub})"),
            args,
        ),
        "install" => walker.add(Verdict::never("installs a program globally (GOBIN)")),
        "env" if args.iter().any(|a| a.is("-w") || a.is("-u")) => {
            walker.add(Verdict::never("changes Go settings"))
        }
        "env" | "version" | "help" | "" => walker.add(Verdict::allowed("prints Go settings")),
        _ => walker.add(Verdict::ask(format!("cos doesn't know `go {sub}`"))),
    }
}

fn brew(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    match sub(args).unwrap_or_default() {
        "list" | "ls" | "info" | "desc" | "deps" | "uses" | "leaves" | "config" | "--version"
        | "--prefix" | "--cellar" | "--repository" | "doctor" | "cat" | "" => {
            walker.add(Verdict::allowed("reads Homebrew state"))
        }
        "search" | "outdated" => walker.add(Verdict::chief(Needs::NETWORK, "searches Homebrew")),
        "home" => walker.add(Verdict::ask("opens a web page")),
        _ => walker.add(Verdict::never("changes software installed on your Mac (brew)")),
    }
}

fn make(walker: &mut Walker<'_, '_>, args: &[Arg]) {
    let mut dir = walker.cwd.clone();
    let mut targets = Vec::new();
    let mut i = 1;
    while let Some(arg) = args.get(i) {
        i += 1;
        let text = arg.str().unwrap_or_default();
        match text {
            "-C" | "--directory" => {
                dir = match (args.get(i).and_then(Arg::str), &dir) {
                    (Some(path), Some(base)) => walker.context.paths.absolute(Some(base), path),
                    _ => None,
                };
                i += 1;
            }
            "-f" | "--file" | "--makefile" => {
                if let Some(file) = args.get(i) {
                    walker.read(file);
                }
                i += 1;
            }
            "-j" | "-l" | "-o" | "-W" | "--jobs" => i += 1,
            text if text.starts_with('-') => {}
            text if text.contains('=') => walker.scan(std::slice::from_ref(arg)),
            text => targets.push(text.to_owned()),
        }
        if arg.str().is_none() {
            walker.add(Verdict::ask("runs a make target that is only known when it runs"));
        }
    }
    const OUTSIDE: &[&str] = &[
        "install",
        "uninstall",
        "deploy",
        "release",
        "publish",
        "push",
        "upload",
    ];
    if let Some(target) = targets.iter().find(|t| OUTSIDE.contains(&t.as_str())) {
        walker.add(Verdict::ask(format!(
            "`make {target}` usually acts outside the worktree"
        )));
    }
    walker.add(Verdict::chief(
        Needs::WORKTREE,
        "runs make targets in the worktree",
    ));
    walker.uses_dir(dir.as_deref());
}
