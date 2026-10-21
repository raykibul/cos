//! `cos try --pty`: run Claude's interactive UI in a pseudo-terminal and
//! follow it through hooks, printing each one. The terminal itself isn't shown
//! (that's for the apps); `--screen` prints it at the end.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use cos_harness::claude::hooks::{Hook, HookEvent, HookServer};
use cos_harness::{
    ClaudeOptions, ClaudeTerminal, HookTarget, PermissionRequest, TerminalEvent, TerminalOptions,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::try_cmd::{Approve, TryArgs, decide, first_line, summarize};

/// If no hook has fired by then, show the screen: Claude may be stuck on a
/// dialog, such as trusting the folder.
const STARTUP_WARNING: Duration = Duration::from_secs(20);
/// How long Claude gets to exit after `/exit`.
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn run(args: &TryArgs, claude: ClaudeOptions) -> anyhow::Result<ExitCode> {
    let socket = std::env::temp_dir().join(format!("cos-try-{}.sock", std::process::id()));
    let (server, mut hooks) =
        HookServer::bind(&socket).with_context(|| format!("could not listen on {}", socket.display()))?;
    let exe = std::env::current_exe().context("could not find the cos binary")?;

    let cwd = claude.cwd.clone();
    let mut options = TerminalOptions::new(claude);
    options.prompt = Some(args.prompt.clone());
    options.user_settings = args.user_settings;
    options.hooks = Some(HookTarget {
        command: format!("{} hook", shell_quote(&exe)),
        socket: server.path().to_owned(),
    });
    let (terminal, mut output) = ClaudeTerminal::spawn(options).context("could not start Claude")?;
    println!(
        "● claude in a terminal · pid {} · {}",
        terminal.pid().map_or("?".into(), |pid| pid.to_string()),
        cwd.display()
    );
    println!("  session {}", terminal.session_id());

    let mut terminal_input = BufReader::new(tokio::io::stdin()).lines();
    let mut heard_from_claude = false;
    let mut warned = false;
    let mut asked_trust = false;
    // Claude clears the screen when it exits, so keep the one from the end of the turn.
    let mut last_screen = None;
    // Permission hooks left waiting while someone answers in the terminal.
    let mut held = Vec::new();
    let mut finished = false;
    let mut exit_deadline = None;
    let mut interrupted = false;
    let warning = tokio::time::sleep(STARTUP_WARNING);
    tokio::pin!(warning);

    let code = loop {
        tokio::select! {
            event = hooks.recv() => {
                let Some(event) = event else { continue };
                heard_from_claude = true;
                match event {
                    HookEvent::Withdrawn { id } => println!("  (permission request {id} withdrawn)"),
                    HookEvent::Fired(hook) => {
                        if args.raw {
                            println!("{}", hook.input);
                        }
                        if hook.event == "Stop" {
                            finished = true;
                        }
                        if handle(hook, args, &terminal, &mut terminal_input, &mut held).await? {
                            last_screen = Some(terminal.screen());
                            // One prompt per run: leave once the turn is done.
                            terminal.submit("/exit").await?;
                            exit_deadline = Some(tokio::time::Instant::now() + EXIT_TIMEOUT);
                        }
                    }
                }
            }
            event = output.recv() => match event {
                Some(TerminalEvent::Output(_)) if !heard_from_claude && !asked_trust && terminal.asks_for_trust() => {
                    asked_trust = true;
                    if args.trust {
                        println!("  Claude asked whether to trust {}; answering yes (--trust)", cwd.display());
                        terminal.trust_folder().await?;
                    } else {
                        println!("\nClaude asks whether to trust this folder before it does anything:\n");
                        print_screen(&terminal);
                        println!("\nRun again with --trust to answer yes, or run `claude` there once yourself.");
                        terminal.kill();
                    }
                }
                Some(TerminalEvent::Output(_)) => {}
                Some(TerminalEvent::Exited { code }) => break code,
                None => break None,
            },
            () = &mut warning, if !heard_from_claude && !warned => {
                warned = true;
                println!("\nNo hook has fired after {STARTUP_WARNING:?}. The screen so far:\n");
                print_screen(&terminal);
            }
            () = sleep_until(exit_deadline) => {
                println!("Claude didn't exit after /exit; killing it");
                exit_deadline = None;
                terminal.kill();
            }
            _ = tokio::signal::ctrl_c() => {
                if interrupted {
                    terminal.kill();
                } else {
                    interrupted = true;
                    println!("\nThe screen so far (Ctrl-C again to kill):\n");
                    print_screen(&terminal);
                }
            }
        }
    };

    println!(
        "\nClaude exited ({})",
        code.map_or("killed".into(), |code| format!("code {code}"))
    );
    if args.screen || !finished {
        println!("\nThe screen at the end of the turn:\n");
        print_lines(&last_screen.unwrap_or_else(|| terminal.screen()));
    }
    drop(server);
    Ok(if finished {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Prints one hook and answers it if it's a permission request. Returns true
/// when the turn is over.
async fn handle(
    hook: Hook,
    args: &TryArgs,
    terminal: &ClaudeTerminal,
    terminal_input: &mut tokio::io::Lines<BufReader<tokio::io::Stdin>>,
    held: &mut Vec<Hook>,
) -> anyhow::Result<bool> {
    let text = |key: &str| {
        hook.input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match hook.event.as_str() {
        "SessionStart" => println!("⚑ session start ({})", text("source")),
        "UserPromptSubmit" => println!("› {}", first_line(&text("prompt"), 160)),
        "PreToolUse" => {
            let tool = hook.tool_name().unwrap_or("?");
            let input = hook.tool_input().cloned().unwrap_or(Value::Null);
            println!("→ {tool} {}", summarize(tool, &input));
        }
        "PermissionRequest" => {
            let request = PermissionRequest {
                request_id: hook.id.to_string(),
                tool_name: hook.tool_name().unwrap_or("?").to_owned(),
                input: hook.tool_input().cloned().unwrap_or(Value::Null),
                tool_use_id: None,
                description: None,
                blocked_path: None,
            };
            if args.screen {
                // Give Claude a moment to draw whatever it shows while it waits.
                tokio::time::sleep(Duration::from_secs(1)).await;
                println!("  the terminal meanwhile:");
                print_screen(terminal);
            }
            if args.approve == Approve::Terminal {
                println!(
                    "? permission: {} — pressing 1 in Claude's dialog",
                    request.tool_name
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                terminal.write(b"1")?;
                held.push(hook);
                return Ok(false);
            }
            let decision = decide(&request, args.approve, terminal_input).await?;
            hook.answer(&decision);
        }
        "PostToolUse" => println!("  ✓ {}", hook.tool_name().unwrap_or("?")),
        "Notification" => println!("  ♪ {}", first_line(&text("message"), 160)),
        "Stop" => {
            println!("■ turn finished");
            return Ok(true);
        }
        "SessionEnd" => println!("⚑ session end ({})", text("reason")),
        other => println!("  hook {other}"),
    }
    Ok(false)
}

fn print_screen(terminal: &ClaudeTerminal) {
    print_lines(&terminal.screen());
}

fn print_lines(screen: &str) {
    let lines: Vec<&str> = screen.lines().collect();
    // Skip the blank rows at the bottom.
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |last| last + 1);
    for line in &lines[..end] {
        println!("  │ {line}");
    }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Quotes a path for `sh`.
pub(crate) fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(shell_quote(Path::new("/a b/cos")), "'/a b/cos'");
        assert_eq!(shell_quote(Path::new("/it's/cos")), r"'/it'\''s/cos'");
    }
}
