//! `cos`: chief of staff for coding agents.
//!
//! `cos daemon` runs the long-lived process; `cos status`, `cos events`,
//! `cos project`, `cos workspace` and the run commands (`cos run`, `cos runs`,
//! `cos board`, `cos approve-plan`, `cos cancel`) talk to it over `~/.cos/cos.sock`, or to
//! another machine's daemon over the tailnet with `--host`. `cos remote` and
//! `cos devices` manage that remote access.
//! `cos try` runs one prompt through Claude Code without the daemon, and
//! `cos sessions` lists the Claude sessions running on this Mac. See
//! `docs/proposal.md` for where this is going.

mod client;
mod commands;
mod daemon;
mod devices_cmd;
mod home;
mod launchd;
mod projects_cmd;
mod runs_cmd;
mod sessions_cmd;
mod tailscale;
mod try_cmd;
mod try_pty;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::home::CosHome;

#[derive(Parser)]
#[command(name = "cos", version, about = "Chief of staff for coding agents")]
struct Cli {
    /// Talk to the daemon on another machine over its tailnet listener
    /// (`100.x.y.z`, `my-mac:7700`), instead of the local socket.
    #[arg(long, global = true, env = "COS_HOST")]
    host: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon in the foreground, or install it to run at login.
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
    /// Show whether the daemon is running.
    Status,
    /// Print the daemon's event log.
    Events(commands::EventsArgs),
    /// Register git repos for cos to work in.
    #[command(subcommand)]
    Project(projects_cmd::ProjectCommand),
    /// Create and manage workspaces: worktrees of a project where agents run.
    #[command(subcommand)]
    Workspace(projects_cmd::WorkspaceCommand),
    /// Start a run: one goal in a workspace, planned by the chief.
    Run(runs_cmd::RunArgs),
    /// List runs, newest first.
    Runs,
    /// Show a run's board: its tasks and where each one is.
    Board(runs_cmd::BoardArgs),
    /// Approve the chief's plan so workers can start.
    ApprovePlan(runs_cmd::RunRef),
    /// Cancel a run and its unfinished tasks.
    Cancel(runs_cmd::CancelArgs),
    /// Run one prompt through Claude Code and answer its permission requests here (no daemon).
    Try(try_cmd::TryArgs),
    /// List the Claude sessions running on this Mac, including ones cos didn't start.
    Sessions(sessions_cmd::SessionsArgs),
    /// Run by Claude Code's hooks in sessions cos starts: forwards the hook to cos.
    #[command(hide = true)]
    Hook,
    /// Let your other devices reach this daemon over Tailscale: `on`, `off` or show the state.
    Remote {
        #[command(subcommand)]
        action: Option<devices_cmd::RemoteCommand>,
    },
    /// Approve, deny or remove the tailnet devices that may use this daemon.
    Devices {
        #[command(subcommand)]
        action: Option<devices_cmd::DevicesCommand>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Run the daemon as a launchd agent: start now, at every login, and after a crash.
    Install,
    /// Stop the launchd agent and remove it.
    Uninstall,
}

#[tokio::main]
async fn main() -> ExitCode {
    run(Cli::parse()).await.unwrap_or_else(|error| {
        eprintln!("cos: {error:#}");
        ExitCode::FAILURE
    })
}

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    if let Command::Hook = cli.command {
        let socket = std::env::var_os(cos_harness::claude::hooks::SOCKET_ENV).map(std::path::PathBuf::from);
        cos_harness::claude::hooks::relay(socket.as_deref(), &mut std::io::stdin(), &mut std::io::stdout());
        return Ok(ExitCode::SUCCESS);
    }
    let home = CosHome::from_env()?;
    if let Some(host) = cli
        .host
        .as_deref()
        .filter(|_| !matches!(cli.command, Command::Daemon { .. }))
    {
        client::use_remote_host(host);
    }
    match cli.command {
        Command::Daemon { action: None } => {
            // Under launchd stderr is a log file, so no colours there.
            tracing_subscriber::fmt()
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal())
                .init();
            daemon::run(&home).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Daemon {
            action: Some(DaemonAction::Install),
        } => commands::install(&home).await,
        Command::Daemon {
            action: Some(DaemonAction::Uninstall),
        } => commands::uninstall(),
        Command::Status => commands::status(&home).await,
        Command::Events(args) => commands::events(&home, args).await,
        Command::Project(command) => projects_cmd::project(&home, command).await,
        Command::Workspace(command) => projects_cmd::workspace(&home, command).await,
        Command::Run(args) => runs_cmd::start(&home, args).await,
        Command::Runs => runs_cmd::list(&home).await,
        Command::Board(args) => runs_cmd::board(&home, args).await,
        Command::ApprovePlan(args) => runs_cmd::approve_plan(&home, args).await,
        Command::Cancel(args) => runs_cmd::cancel(&home, args).await,
        Command::Try(args) => try_cmd::run(args).await,
        Command::Sessions(args) => sessions_cmd::run(&home, args).await,
        Command::Hook => unreachable!("handled above"),
        Command::Remote { action } => devices_cmd::remote(&home, action).await,
        Command::Devices { action } => devices_cmd::devices(&home, action).await,
    }
}
