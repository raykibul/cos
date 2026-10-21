//! `cos try`: run one prompt through Claude Code directly (no daemon) and
//! answer its permission requests from the terminal.
//!
//! By default Claude runs headless (stream-json). With `--pty` it runs its own
//! interactive UI in a pseudo-terminal and cos follows it through hooks.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::ValueEnum;
use cos_harness::{ClaudeOptions, ClaudeSession, Decision, Event, PermissionMode, PermissionRequest};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};

#[derive(clap::Args)]
pub(crate) struct TryArgs {
    /// What to ask Claude.
    pub(crate) prompt: String,
    /// Directory Claude works in.
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    /// Model alias (haiku, sonnet, opus) or full name. Defaults to Claude's default.
    #[arg(long)]
    model: Option<String>,
    /// How to answer permission requests.
    #[arg(long, value_enum, default_value_t = Approve::Ask)]
    pub(crate) approve: Approve,
    /// Claude's permission mode: manual, acceptEdits, plan or dontAsk.
    #[arg(long, default_value = "manual")]
    mode: PermissionMode,
    /// The claude binary to run.
    #[arg(long, env = "COS_CLAUDE_BIN", default_value = "claude")]
    claude_bin: PathBuf,
    /// Print every event in full, including ones cos doesn't interpret.
    #[arg(long)]
    pub(crate) raw: bool,
    /// Run Claude's interactive UI in a pseudo-terminal and follow it through hooks.
    #[arg(long)]
    pub(crate) pty: bool,
    /// With --pty: load your own Claude settings (allow rules, hooks, MCP servers).
    #[arg(long, requires = "pty")]
    pub(crate) user_settings: bool,
    /// With --pty: print the terminal screen when Claude exits.
    #[arg(long, requires = "pty")]
    pub(crate) screen: bool,
    /// With --pty: if Claude asks whether to trust the folder, say yes. Claude remembers it.
    #[arg(long, requires = "pty")]
    pub(crate) trust: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Approve {
    /// Ask in this terminal.
    Ask,
    /// Allow everything.
    Allow,
    /// Deny everything.
    Deny,
    /// With --pty: leave the hook waiting and press 1 in Claude's own dialog,
    /// as someone at the terminal would.
    Terminal,
}

pub(crate) async fn run(args: TryArgs) -> anyhow::Result<ExitCode> {
    let cwd = args
        .cwd
        .canonicalize()
        .with_context(|| format!("no such directory: {}", args.cwd.display()))?;

    let mut options = ClaudeOptions::new(&cwd);
    options.binary = args.claude_bin.clone();
    options.model = args.model.clone();
    options.permission_mode = args.mode;
    if args.pty {
        return crate::try_pty::run(&args, options).await;
    }
    anyhow::ensure!(
        args.approve != Approve::Terminal,
        "--approve terminal needs --pty"
    );

    let (session, mut events) = ClaudeSession::spawn(options)
        .await
        .context("could not start Claude")?;
    session.send(&args.prompt).await?;

    let mut terminal = BufReader::new(tokio::io::stdin()).lines();
    let mut stderr = Vec::new();
    let mut succeeded = false;
    let mut interrupted = false;

    loop {
        let event = tokio::select! {
            event = events.recv() => match event {
                Some(event) => event,
                None => break,
            },
            _ = tokio::signal::ctrl_c() => {
                if interrupted {
                    eprintln!("\nkilling Claude");
                    session.kill();
                } else {
                    eprintln!("\ninterrupting (Ctrl-C again to kill)");
                    interrupted = true;
                    if let Err(error) = session.interrupt().await {
                        eprintln!("interrupt failed: {error}");
                    }
                }
                continue;
            }
        };

        if args.raw {
            println!("{event:?}");
        }
        match event {
            Event::Started {
                session_id,
                model,
                permission_mode,
            } => {
                println!("● {model} · {permission_mode} mode · {}", cwd.display());
                println!("  session {session_id}");
            }
            Event::Text(text) => println!("\n{text}"),
            Event::ToolUse { name, input, .. } => println!("→ {name} {}", summarize(&name, &input)),
            Event::ToolResult {
                is_error, content, ..
            } => {
                let mark = if is_error { "✗" } else { "✓" };
                println!("  {mark} {}", first_line(&content, 160));
            }
            Event::PermissionRequest(request) => {
                let decision = decide(&request, args.approve, &mut terminal).await?;
                session.answer(&request.request_id, decision).await?;
            }
            Event::PermissionCancelled { .. } => println!("  (permission request withdrawn)"),
            Event::TurnFinished(result) => {
                succeeded = !result.is_error;
                let mut summary = vec![result.subtype.clone()];
                if let Some(cost) = result.cost_usd {
                    summary.push(format!("${cost:.4}"));
                }
                if let Some(ms) = result.duration_ms {
                    summary.push(format!("{:.1}s", ms as f64 / 1000.0));
                }
                if !result.denied_tools.is_empty() {
                    summary.push(format!("denied: {}", result.denied_tools.join(", ")));
                }
                println!("\n■ {}", summary.join(" · "));
                // One prompt per run: let Claude exit once the turn is done.
                session.close_input().await;
            }
            Event::Stderr(line) => stderr.push(line),
            Event::Other(_) => {}
            Event::Exited { code } => {
                if !succeeded {
                    eprintln!(
                        "Claude exited ({})",
                        code.map_or("killed".into(), |c| format!("code {c}"))
                    );
                    for line in stderr.iter().rev().take(20).rev() {
                        eprintln!("  {line}");
                    }
                }
                break;
            }
        }
    }

    Ok(if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub(crate) async fn decide(
    request: &PermissionRequest,
    approve: Approve,
    terminal: &mut Lines<BufReader<Stdin>>,
) -> anyhow::Result<Decision> {
    let action = format!(
        "{} {}",
        request.tool_name,
        summarize(&request.tool_name, &request.input)
    );
    println!("? permission: {action}");
    if let Some(path) = &request.blocked_path {
        println!("  path: {path}");
    }

    let allow = match approve {
        Approve::Allow => true,
        Approve::Deny | Approve::Terminal => false,
        Approve::Ask => {
            use std::io::Write;
            print!("  allow? [y/N] ");
            std::io::stdout().flush()?;
            tokio::select! {
                line = terminal.next_line() => {
                    matches!(line?.as_deref().map(str::trim), Some("y" | "Y" | "yes"))
                }
                // Ctrl-C at the prompt counts as no.
                _ = tokio::signal::ctrl_c() => {
                    println!();
                    false
                }
            }
        }
    };

    println!("  {}", if allow { "allowed" } else { "denied" });
    Ok(if allow {
        Decision::Allow
    } else {
        Decision::Deny {
            message: "The user denied this in cos.".into(),
        }
    })
}

/// A one-line view of a tool call's input.
pub(crate) fn summarize(tool: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let text = match tool {
        "Bash" => field("command"),
        _ => field("file_path")
            .or_else(|| field("pattern"))
            .or_else(|| field("url")),
    };
    match text {
        Some(text) => first_line(text, 160),
        None => first_line(&input.to_string(), 160),
    }
}

pub(crate) fn first_line(text: &str, max_chars: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut short: String = line.chars().take(max_chars).collect();
    if short.len() < line.len() || text.lines().nth(1).is_some() {
        short.push('…');
    }
    short
}
