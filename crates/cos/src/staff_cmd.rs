//! `cos staff [project]`, `cos staff say [project] "text"`,
//! `cos staff attach [project]`, `cos staff pause|resume [project]`,
//! `cos staff new [project]` and `cos staff memory [project]`: a project's
//! Chief of Staff. The project defaults to the one you're in, or
//! the only one.

use std::process::ExitCode;

use anyhow::bail;
use clap::Subcommand;
use cos_core::{AgentState, Project};

use crate::client::{Client, Folder};
use crate::home::CosHome;
use crate::projects_cmd::find_project;

#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct StaffArgs {
    #[command(subcommand)]
    action: Option<StaffCommand>,
    /// Project id or name. Defaults to the project you're in, or the only one.
    project: Option<String>,
    /// Set the model it runs from its next start (`default` for the daemon's).
    #[arg(long)]
    model: Option<String>,
}

#[derive(Subcommand)]
enum StaffCommand {
    /// Send it a message: `cos staff say [project] "text"`. Starts it if needed.
    Say {
        #[arg(num_args = 1..=2, required = true, value_name = "PROJECT] [TEXT")]
        args: Vec<String>,
        /// Attach a file (an image, a PDF, a log…); repeat for more. The chief
        /// gets it in its worktree.
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<std::path::PathBuf>,
    },
    /// Open its terminal here, starting it if needed. Ctrl-] detaches.
    Attach {
        /// Project id or name.
        project: Option<String>,
    },
    /// Pause its coordination: news from agents waits; your messages still go in.
    Pause {
        /// Project id or name.
        project: Option<String>,
    },
    /// Resume its coordination: what waited goes in once it's idle.
    Resume {
        /// Project id or name.
        project: Option<String>,
    },
    /// Drop its Claude session and start a new one, with its brief and memory
    /// notes. The agents it started stay its own.
    New {
        /// Project id or name.
        project: Option<String>,
    },
    /// Print its memory notes.
    Memory {
        /// Project id or name.
        project: Option<String>,
    },
}

pub(crate) async fn staff(home: &CosHome, args: StaffArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match args.action {
        None => {
            if let Some(model) = args.model {
                let project = project(&client, args.project.as_deref()).await?;
                let model = Some(model).filter(|model| model != "default");
                client.set_staff_model(project.id, model).await?;
            }
            show(&client, args.project.as_deref()).await
        }
        Some(StaffCommand::Say { args, files }) => {
            let (key, text) = match args.as_slice() {
                [text] => (None, text.clone()),
                [key, text] => (Some(key.as_str()), text.clone()),
                _ => bail!("use `cos staff say [project] \"text\"`"),
            };
            let project = project(&client, key).await?;
            let attachments = client.upload_files(Folder::Staff(project.id), &files).await?;
            client.message_staff_with(project.id, text, attachments).await?;
            println!(
                "Sent to the Chief of Staff of {}. Follow it with `cos staff attach {}`.",
                project.name, project.name
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(StaffCommand::Attach { project: key }) => {
            let project = project(&client, key.as_deref()).await?;
            let staff = client.start_staff(project.id).await?;
            let Some(agent_id) = staff.agent_id else {
                bail!("the Chief of Staff of {} didn't start", project.name);
            };
            crate::attach::run(&client, agent_id).await
        }
        Some(StaffCommand::Pause { project: key }) => pause(&client, key.as_deref(), true).await,
        Some(StaffCommand::Resume { project: key }) => pause(&client, key.as_deref(), false).await,
        Some(StaffCommand::New { project: key }) => {
            let project = project(&client, key.as_deref()).await?;
            client.new_staff_session(project.id).await?;
            println!(
                "Started the Chief of Staff of {} in a new session. Follow it with `cos staff attach {}`.",
                project.name, project.name
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(StaffCommand::Memory { project: key }) => {
            let project = project(&client, key.as_deref()).await?;
            let notes = client.staff_memory(project.id).await?;
            if notes.is_empty() {
                println!("The Chief of Staff of {} has no memory notes yet.", project.name);
            }
            for (index, note) in notes.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                println!("## {}: {}", note.name, note.description);
                println!("{}", note.body);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// `cos staff pause|resume [project]`.
async fn pause(client: &Client, key: Option<&str>, paused: bool) -> anyhow::Result<ExitCode> {
    let project = project(client, key).await?;
    let staff = client.set_staff_paused(project.id, paused).await?;
    let waiting = match staff.updates_waiting {
        1 => "1 update waits".to_owned(),
        n => format!("{n} updates wait"),
    };
    if paused {
        println!(
            "Paused the Chief of Staff of {}: news from agents waits ({waiting}); your messages still go in.",
            project.name
        );
    } else {
        println!("Resumed the Chief of Staff of {} ({waiting}).", project.name);
    }
    Ok(ExitCode::SUCCESS)
}

async fn show(client: &Client, key: Option<&str>) -> anyhow::Result<ExitCode> {
    let project = project(client, key).await?;
    let staff = client.staff(project.id).await?;
    println!("Chief of Staff · {} (project {})", project.name, project.id);
    match (staff.agent_id, staff.state) {
        (Some(agent_id), Some(state)) => println!("  state:    {} (agent {agent_id})", state_name(state)),
        _ => println!(
            "  state:    not started; `cos staff say {} \"…\"` starts it",
            project.name
        ),
    }
    if staff.paused {
        println!(
            "  paused:   {} updates waiting; `cos staff resume {}` lets them in",
            staff.updates_waiting, project.name
        );
    }
    println!(
        "  model:    {}",
        staff.model.as_deref().unwrap_or("Claude's default")
    );
    println!(
        "  worktree: {} (branch {})",
        staff.worktree.display(),
        staff.branch
    );
    if let Some(agent_id) = staff.agent_id {
        let spawned: Vec<_> = client
            .agents(None)
            .await?
            .into_iter()
            .filter(|agent| agent.spawned_by == Some(agent_id))
            .collect();
        if spawned.is_empty() {
            println!("  agents:   none yet");
        } else {
            println!(
                "  agents:   {} working, {} waiting on you",
                staff.working_agents, staff.waiting_on_you
            );
            for agent in &spawned {
                println!("    #{} {} · {}", agent.id, state_name(agent.state), agent.title);
            }
        }
    }
    let brief = staff.brief.trim();
    if brief.is_empty() {
        println!("  brief:    (empty)");
    } else {
        println!("  brief:");
        for line in brief.lines() {
            println!("    {line}");
        }
    }
    let news = client.staff_inbox(project.id).await?;
    if !news.is_empty() {
        println!("  latest news:");
        for item in news.iter().take(5) {
            let heading = item.text.lines().next().unwrap_or_default();
            let waiting = if item.delivered_ms.is_none() {
                " (waiting)"
            } else {
                ""
            };
            println!("    {heading}{waiting}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The project named, else the one the current folder is in, else the
/// only one.
async fn project(client: &Client, key: Option<&str>) -> anyhow::Result<Project> {
    let projects = client.projects().await?;
    if let Some(key) = key {
        return Ok(find_project(&projects, key)?.clone());
    }
    let here = std::env::current_dir().ok();
    let inside = projects.iter().find(|project| {
        here.as_ref().is_some_and(|here| {
            here.starts_with(&project.root)
                || project
                    .workspaces
                    .iter()
                    .any(|workspace| here.starts_with(&workspace.path))
        })
    });
    match (inside, projects.as_slice()) {
        (Some(project), _) | (None, [project]) => Ok(project.clone()),
        (None, []) => bail!("no projects yet; add one with `cos project add <path>`"),
        (None, _) => bail!("which project? Name one: `cos staff <project>` (see `cos project list`)"),
    }
}

fn state_name(state: AgentState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|state| state.as_str().map(str::to_owned))
        .unwrap_or_default()
}
