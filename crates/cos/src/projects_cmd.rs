//! `cos project …` and `cos workspace …`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Subcommand, ValueEnum};
use cos_core::{Base, Project, Workspace};

use crate::client::Client;
use crate::home::CosHome;

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// Register the git repo at PATH (default: the current folder).
    Add { path: Option<PathBuf> },
    /// List projects and their workspaces.
    List,
    /// Unregister a project. The repo itself is not touched.
    Remove {
        /// Project id or name.
        project: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCommand {
    /// Create a workspace: a new worktree and `cos/<city>` branch for a project.
    New {
        /// Project id or name.
        project: String,
        /// What to start from.
        #[arg(long, value_enum, default_value_t = From::Origin)]
        from: From,
        /// A title. Defaults to the workspace's city name.
        #[arg(long)]
        title: Option<String>,
    },
    /// Give a workspace a new title.
    Rename {
        /// Workspace id or name.
        workspace: String,
        title: String,
    },
    /// Delete a workspace's folder. Refuses if it has uncommitted changes; the branch is kept.
    Remove {
        /// Workspace id or name.
        workspace: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum From {
    /// Freshly fetched origin/<base branch>.
    Origin,
    /// Your main checkout's current HEAD, including unpushed commits.
    Head,
}

pub(crate) async fn project(home: &CosHome, command: ProjectCommand) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match command {
        ProjectCommand::Add { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("."));
            let path = path
                .canonicalize()
                .with_context(|| format!("no such folder: {}", path.display()))?;
            let (project, created) = client.add_project(path).await?;
            let verb = if created { "Added" } else { "Already added:" };
            println!(
                "{verb} {} (#{}) at {}, base branch {}",
                project.name,
                project.id,
                project.root.display(),
                project.base_branch
            );
        }
        ProjectCommand::List => {
            let projects = client.projects().await?;
            if projects.is_empty() {
                println!("No projects yet. Add one with `cos project add <path>`.");
            }
            for project in &projects {
                print_project(project);
            }
        }
        ProjectCommand::Remove { project } => {
            let project = find_project(&client.projects().await?, &project)?.clone();
            client.remove_project(project.id).await?;
            println!("Removed project {} (the repo is untouched).", project.name);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn workspace(home: &CosHome, command: WorkspaceCommand) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match command {
        WorkspaceCommand::New { project, from, title } => {
            let project = find_project(&client.projects().await?, &project)?.clone();
            let base = match from {
                From::Origin => Base::Origin,
                From::Head => Base::Head,
            };
            if base == Base::Origin {
                println!("Fetching origin/{}…", project.base_branch);
            }
            let workspace = client.create_workspace(project.id, base, title).await?;
            println!(
                "Created workspace {} (#{}) on {} from {} {}",
                workspace.title,
                workspace.id,
                workspace.branch,
                workspace.base_ref,
                short(&workspace.base_commit)
            );
            println!("  {}", workspace.path.display());
        }
        WorkspaceCommand::Rename { workspace, title } => {
            let workspace = find_workspace(&client.projects().await?, &workspace)?.clone();
            let renamed = client.rename_workspace(workspace.id, title).await?;
            println!("Renamed {} to {}", workspace.name, renamed.title);
        }
        WorkspaceCommand::Remove { workspace } => {
            let workspace = find_workspace(&client.projects().await?, &workspace)?.clone();
            client.remove_workspace(workspace.id).await?;
            println!(
                "Removed workspace {}. Its branch {} is kept.",
                workspace.title, workspace.branch
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_project(project: &Project) {
    let origin = if project.has_origin { "" } else { ", no origin" };
    println!(
        "#{} {}  {}  (base {}{origin})",
        project.id,
        project.name,
        project.root.display(),
        project.base_branch
    );
    if project.workspaces.is_empty() {
        println!("    no workspaces yet: cos workspace new {}", project.name);
    }
    for workspace in &project.workspaces {
        let title = if workspace.title == workspace.name {
            String::new()
        } else {
            format!(" \"{}\"", workspace.title)
        };
        println!(
            "    #{} {}{title}  {}  from {} {}",
            workspace.id,
            workspace.name,
            workspace.branch,
            workspace.base_ref,
            short(&workspace.base_commit)
        );
        println!("        {}", workspace.path.display());
    }
}

/// A project by id or name.
fn find_project<'a>(projects: &'a [Project], key: &str) -> anyhow::Result<&'a Project> {
    let matches: Vec<&Project> = projects
        .iter()
        .filter(|project| project.id.to_string() == key || project.name == key)
        .collect();
    match matches.as_slice() {
        [project] => Ok(project),
        [] => bail!("no project called {key}; see `cos project list`"),
        _ => bail!("more than one project is called {key}; use its id from `cos project list`"),
    }
}

/// A workspace by id, name or title.
fn find_workspace<'a>(projects: &'a [Project], key: &str) -> anyhow::Result<&'a Workspace> {
    let matches: Vec<&Workspace> = projects
        .iter()
        .flat_map(|project| &project.workspaces)
        .filter(|workspace| {
            workspace.id.to_string() == key || workspace.name == key || workspace.title == key
        })
        .collect();
    match matches.as_slice() {
        [workspace] => Ok(workspace),
        [] => bail!("no workspace called {key}; see `cos project list`"),
        _ => bail!("more than one workspace matches {key}; use its id from `cos project list`"),
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(8)]
}
