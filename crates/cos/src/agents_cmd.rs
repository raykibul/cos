//! `cos agent …`, `cos agents`, `cos approvals`, `cos allow` and `cos deny`:
//! agents the daemon runs in terminals, and their permission requests.

use std::process::ExitCode;

use anyhow::bail;
use cos_core::{Agent, Approval, ApprovalState};
use jiff::Timestamp;

use crate::client::Client;
use crate::commands::human_duration;
use crate::daemon::agents::StartAgent;
use crate::home::CosHome;
use crate::projects_cmd::find_workspace;
use crate::try_cmd::first_line;

#[derive(clap::Subcommand)]
pub(crate) enum AgentCommand {
    /// Start Claude in a workspace, in a terminal the daemon owns.
    New {
        /// Workspace id, name or title.
        workspace: String,
        /// The first message. Without it Claude waits for input.
        prompt: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// A model alias such as `sonnet`, or a full model name.
        #[arg(long)]
        model: Option<String>,
        /// default, acceptEdits, plan or dontAsk.
        #[arg(long)]
        permission_mode: Option<String>,
        /// Only the repo's settings, not yours: your allow rules don't
        /// apply, so more requests come to `cos approvals`.
        #[arg(long)]
        project_settings: bool,
    },
    /// Ask an agent to exit; it is killed if it hasn't after 10 s.
    Stop { id: i64 },
    /// Start an exited or interrupted agent again, in the same session.
    Resume { id: i64 },
    /// Forget an agent that has exited.
    Rm { id: i64 },
    /// Type a message into an agent and press Return.
    Say { id: i64, text: String },
    /// Print an agent's screen.
    Screen { id: i64 },
    /// Answer an agent's "trust this folder?" dialog with yes.
    Trust { id: i64 },
    /// Make your terminal the agent's terminal. Ctrl-] detaches.
    Attach { id: i64 },
}

#[derive(clap::Args)]
pub(crate) struct AgentsArgs {
    /// Only agents in this workspace (id, name or title).
    #[arg(long)]
    workspace: Option<String>,
}

#[derive(clap::Args)]
pub(crate) struct ApprovalsArgs {
    /// Every approval, not only pending ones.
    #[arg(long)]
    all: bool,
}

pub(crate) async fn agent(home: &CosHome, command: AgentCommand) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match command {
        AgentCommand::New {
            workspace,
            prompt,
            title,
            model,
            permission_mode,
            project_settings,
        } => {
            let workspace = find_workspace(&client.projects().await?, &workspace)?.clone();
            let request = StartAgent {
                prompt,
                title,
                model,
                permission_mode,
                user_settings: project_settings.then_some(false),
                ..StartAgent::default()
            };
            let agent = client.start_agent(workspace.id, &request).await?;
            println!(
                "Started agent #{} \"{}\" in {} (session {})",
                agent.id, agent.title, workspace.title, agent.session_id
            );
            println!("  watch it with `cos agent attach {}`", agent.id);
        }
        AgentCommand::Stop { id } => {
            client.stop_agent(id).await?;
            println!("Asked agent #{id} to exit.");
        }
        AgentCommand::Resume { id } => {
            let agent = client.resume_agent(id).await?;
            println!("Resumed agent #{} (session {})", agent.id, agent.session_id);
        }
        AgentCommand::Rm { id } => {
            client.remove_agent(id).await?;
            println!("Forgot agent #{id}.");
        }
        AgentCommand::Say { id, text } => client.say(id, text).await?,
        AgentCommand::Screen { id } => {
            let screen = client.screen(id).await?;
            let lines: Vec<&str> = screen.text.lines().collect();
            let end = lines
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map_or(0, |last| last + 1);
            for line in &lines[..end] {
                println!("{line}");
            }
        }
        AgentCommand::Trust { id } => {
            client.trust_agent(id).await?;
            println!("Told agent #{id}'s Claude to trust its folder.");
        }
        AgentCommand::Attach { id } => return crate::attach::run(&client, id).await,
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn agents(home: &CosHome, args: AgentsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let workspace_id = match &args.workspace {
        Some(key) => Some(find_workspace(&client.projects().await?, key)?.id),
        None => None,
    };
    let agents = client.agents(workspace_id).await?;
    if agents.is_empty() {
        println!("No agents. Start one with `cos agent new <workspace> \"prompt\"`.");
    }
    for agent in &agents {
        print_agent(agent);
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn approvals(home: &CosHome, args: ApprovalsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let approvals = client.approvals((!args.all).then_some("pending")).await?;
    if approvals.is_empty() {
        println!("Nothing is waiting for an answer.");
    }
    for approval in &approvals {
        print_approval(approval);
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn answer(
    home: &CosHome,
    id: i64,
    allow: bool,
    message: Option<String>,
) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let approval = client.answer_approval(id, allow, message).await?;
    let verb = match approval.state {
        ApprovalState::Allowed => "Allowed",
        ApprovalState::Denied => "Denied",
        _ => bail!("approval #{id} ended up {:?}", approval.state),
    };
    println!("{verb} #{}: {}", approval.id, approval.summary);
    Ok(ExitCode::SUCCESS)
}

fn print_agent(agent: &Agent) {
    let state = name(&agent.state);
    println!(
        "#{:<4} {state:<16} {}  (workspace #{}, {})",
        agent.id,
        agent.title,
        agent.workspace_id,
        name(&agent.role)
    );
    let mut details = vec![format!("session {}", agent.session_id)];
    if let Some(pid) = agent.pid.filter(|_| agent.state.is_running()) {
        details.push(format!("pid {pid}"));
    }
    details.push(format!("active {} ago", ago(agent.last_activity_ms)));
    if let Some(id) = agent.pending_approval_id {
        details.push(format!("waiting on approval #{id}"));
    }
    println!("      {}", details.join(" · "));
    if let Some(message) = &agent.last_message {
        println!("      ⏺ {}", first_line(message, 100));
    }
}

fn print_approval(approval: &Approval) {
    let state = match approval.state {
        ApprovalState::Pending => String::new(),
        other => format!(" [{}]", name(&other)),
    };
    println!(
        "#{:<4} agent #{}  {}{state}  ({} ago)",
        approval.id,
        approval.agent_id,
        first_line(&approval.summary, 120),
        ago(approval.created_ms)
    );
    if let Some(route) = approval.route {
        let who = match route {
            cos_core::ApprovalRoute::User => "for you",
            cos_core::ApprovalRoute::Chief => "for the chief",
            cos_core::ApprovalRoute::Auto => "by cos's rules",
        };
        let reason = approval
            .reason
            .as_deref()
            .map(|reason| format!(": {}", first_line(reason, 110)))
            .unwrap_or_default();
        println!("      {who}{reason}");
    }
}

/// The serde name of a unit enum variant, such as `waiting_approval`.
fn name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn ago(ms: i64) -> String {
    let now = Timestamp::now().as_millisecond();
    human_duration(u64::try_from((now - ms) / 1000).unwrap_or(0))
}
