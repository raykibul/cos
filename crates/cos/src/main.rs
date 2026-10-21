//! `cos`: chief of staff for coding agents.
//!
//! `cos daemon` runs the long-lived process; `cos status`, `cos events`,
//! `cos project` and `cos workspace` talk to it over `~/.cos/cos.sock`, or to
//! another machine's daemon over the tailnet with `--host`. `cos remote` and
//! `cos devices` manage that remote access.
//! `cos try` runs one prompt through Claude Code without the daemon. See `docs/proposal.md` for where this is going.

mod client;
mod commands;
mod daemon;
mod devices_cmd;
mod home;
mod launchd;
mod projects_cmd;
mod tailscale;
mod try_cmd;

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
    /// Run one prompt through Claude Code and answer its permission requests here (no daemon).
    Try(try_cmd::TryArgs),
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
        Command::Try(args) => try_cmd::run(args).await,
        Command::Remote { action } => devices_cmd::remote(&home, action).await,
        Command::Devices { action } => devices_cmd::devices(&home, action).await,
    }
}
