//! `cos`: chief of staff for coding agents.
//!
//! `cos daemon` runs the long-lived process; `cos status`, `cos events`,
//! `cos project` and `cos workspace` talk to it over `~/.cos/cos.sock`, or to
//! another machine's daemon over the tailnet with `--host`. `cos remote` and
//! `cos devices` manage that remote access. `cos agent`, `cos agents`,
//! `cos approvals`, `cos allow` and `cos deny` drive the agents the daemon
//! runs in terminals, and `cos staff` talks to a project's Chief of Staff.
//! `cos env` sets the environment variables agents get, globally or per project.
//! `cos judge` shows and switches jev, which judges in shadow mode.
//! `cos try` runs one prompt through Claude Code without the daemon, and
//! `cos sessions` lists the Claude sessions running on this machine.
//! `cos upgrade` replaces this binary with a newer release. See
//! `docs/proposal.md` for where this is going.

mod agents_cmd;
mod attach;
mod client;
mod commands;
mod daemon;
mod daemon_env;
mod devices_cmd;
mod env_cmd;
mod home;
mod judge_cmd;
mod mcp;
mod projects_cmd;
mod service;
mod sessions_cmd;
mod staff_cmd;
mod tailscale;
mod try_cmd;
mod try_pty;
mod upgrade;
mod which;

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
    /// Run the daemon in the foreground, or install it as a service.
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
    /// List the Claude sessions running on this machine, including ones cos didn't start.
    Sessions(sessions_cmd::SessionsArgs),
    /// Run by Claude Code's hooks in sessions cos starts: forwards the hook to cos.
    #[command(hide = true)]
    Hook {
        /// The agent this session belongs to, added to every hook so the
        /// daemon can follow it into a new session (`/clear`).
        #[arg(long)]
        agent: Option<i64>,
    },
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
    /// Start, watch and drive Claude agents in workspaces.
    #[command(subcommand)]
    Agent(agents_cmd::AgentCommand),
    /// List agents.
    Agents(agents_cmd::AgentsArgs),
    /// List permission requests waiting for an answer.
    Approvals(agents_cmd::ApprovalsArgs),
    /// Allow a pending permission request.
    Allow { id: i64 },
    /// Deny a pending permission request. The message is shown to Claude.
    Deny { id: i64, message: Option<String> },
    /// A project's Chief of Staff: show it, `say` something to it, or `attach` to its terminal.
    Staff(staff_cmd::StaffArgs),
    /// The environment variables agents get, globally or per project: list, `set` or `unset` them.
    Env(env_cmd::EnvArgs),
    /// jev, the fast judgment layer, in shadow mode: status, stats, a
    /// per-project switch and a test call.
    Judge(judge_cmd::JudgeArgs),
    /// Upgrade cos to the latest release, with the Mac app, and restart the daemon on it.
    ///
    /// Replaces this binary in place: it downloads the release's install.sh and runs it,
    /// which checks the download against the release's SHA256SUMS. On macOS, a cos.app in
    /// /Applications or ~/Applications is upgraded too, and reopened if it was running. If
    /// the daemon's service runs this binary, it is restarted on the new version; with
    /// agents at work, cos asks first.
    Upgrade(upgrade::UpgradeArgs),
    /// Run by a Chief of Staff's Claude session: the `cos` MCP server, forwarding its tools to the daemon.
    #[command(hide = true)]
    Mcp(mcp::McpArgs),
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Run the daemon as a service: start now, at login and after a crash
    /// (a launchd agent on macOS, a systemd user unit on Linux).
    Install,
    /// Stop the service and remove it.
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
    if let Command::Hook { agent } = cli.command {
        let socket = std::env::var_os(cos_harness::claude::hooks::SOCKET_ENV).map(std::path::PathBuf::from);
        cos_harness::claude::hooks::relay(
            socket.as_deref(),
            agent,
            &mut std::io::stdin(),
            &mut std::io::stdout(),
        );
        return Ok(ExitCode::SUCCESS);
    }
    let home = CosHome::from_env()?;
    if cli.host.is_some() && matches!(cli.command, Command::Upgrade(_)) {
        anyhow::bail!(
            "`cos upgrade` upgrades this machine only, so it doesn't take --host (or COS_HOST): \
             run `cos upgrade` on that machine"
        );
    }
    if let Some(host) = cli
        .host
        .as_deref()
        .filter(|_| !matches!(cli.command, Command::Daemon { .. } | Command::Mcp(_)))
    {
        client::use_remote_host(host);
    }
    match cli.command {
        Command::Daemon { action: None } => {
            // Under launchd or systemd stderr is a log file, so no colours there.
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
        Command::Sessions(args) => sessions_cmd::run(&home, args).await,
        Command::Hook { .. } => unreachable!("handled above"),
        Command::Remote { action } => devices_cmd::remote(&home, action).await,
        Command::Devices { action } => devices_cmd::devices(&home, action).await,
        Command::Agent(command) => agents_cmd::agent(&home, command).await,
        Command::Agents(args) => agents_cmd::agents(&home, args).await,
        Command::Approvals(args) => agents_cmd::approvals(&home, args).await,
        Command::Allow { id } => agents_cmd::answer(&home, id, true, None).await,
        Command::Deny { id, message } => agents_cmd::answer(&home, id, false, message).await,
        Command::Staff(args) => staff_cmd::staff(&home, args).await,
        Command::Env(args) => env_cmd::env(&home, args).await,
        Command::Judge(args) => judge_cmd::judge(&home, args).await,
        Command::Upgrade(args) => upgrade::upgrade(&home, args).await,
        Command::Mcp(args) => mcp::run(&home, args).await,
    }
}
