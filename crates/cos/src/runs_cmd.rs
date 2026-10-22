//! `cos run`, `cos runs`, `cos board`, `cos approve-plan` and `cos cancel`.
//!
//! Commands that take an optional run pick one when it's left out: the
//! unfinished run in the workspace you're in, or else the only unfinished run.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::ValueEnum;
use cos_core::{ApprovalPreset, NewRun, Project, RunState, RunSummary, Workspace, render_board};

use crate::client::Client;
use crate::home::CosHome;
use crate::projects_cmd::find_workspace;

const GOAL_PREVIEW_CHARS: usize = 60;

#[derive(clap::Args)]
pub(crate) struct RunArgs {
    /// What you want done.
    goal: String,
    /// Workspace id, name or title. Defaults to the workspace you're in.
    #[arg(long, short = 'w')]
    workspace: Option<String>,
    /// How much the chief may approve for you.
    #[arg(long, value_enum, default_value_t = Preset::Normal)]
    preset: Preset,
    /// The most approvals the chief may give in this run. Default 20.
    #[arg(long)]
    approval_cap: Option<u32>,
}

#[derive(clap::Args)]
pub(crate) struct BoardArgs {
    /// Run id. Defaults to the current run.
    run: Option<i64>,
    /// Print the board as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub(crate) struct RunRef {
    /// Run id. Defaults to the current run.
    run: Option<i64>,
}

#[derive(clap::Args)]
pub(crate) struct CancelArgs {
    /// Run id. Defaults to the current run.
    run: Option<i64>,
    /// Why, for the log and the report.
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Preset {
    /// Only you approve.
    Strict,
    /// The chief approves routine requests inside the task's worktree.
    Normal,
    /// The chief also approves network access and installs.
    Trusted,
}

impl From<Preset> for ApprovalPreset {
    fn from(preset: Preset) -> Self {
        match preset {
            Preset::Strict => Self::Strict,
            Preset::Normal => Self::Normal,
            Preset::Trusted => Self::Trusted,
        }
    }
}

pub(crate) async fn start(home: &CosHome, args: RunArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let projects = client.projects().await?;
    let workspace = match &args.workspace {
        Some(key) => find_workspace(&projects, key)?.clone(),
        None => workspace_here(&projects, &current_dir()?)?.clone(),
    };
    let run = client
        .create_run(&NewRun {
            workspace_id: workspace.id,
            goal: args.goal,
            preset: args.preset.into(),
            approval_cap: args.approval_cap,
        })
        .await?;
    println!(
        "Started run {} in workspace {} ({}): {}",
        run.id,
        workspace.title,
        workspace.branch,
        preview(&run.goal)
    );
    if let Some(chief) = run.chief_agent_id {
        println!(
            "  The chief (agent {chief}) is exploring and will propose a plan; `cos agent attach {chief}` to watch it."
        );
    }
    println!(
        "  Watch the board with `cos board {}`, then `cos approve-plan {}`. Questions: `cos questions`; stop it with `cos cancel {}`.",
        run.id, run.id, run.id
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn list(home: &CosHome) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let runs = client.runs().await?;
    if runs.is_empty() {
        println!("No runs yet. Start one with `cos run \"<goal>\"` inside a workspace.");
        return Ok(ExitCode::SUCCESS);
    }
    let projects = client.projects().await?;
    for summary in &runs {
        let run = &summary.run;
        let workspace = all_workspaces(&projects)
            .find(|workspace| workspace.id == run.workspace_id)
            .map_or_else(|| format!("#{} (removed)", run.workspace_id), |w| w.title.clone());
        println!(
            "#{:<4} {:<22} {:<12} {}",
            run.id,
            run.state.as_str(),
            workspace,
            preview(&run.goal)
        );
        if summary.tasks.total > 0 {
            println!("      {}", summary.tasks.summary());
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn board(home: &CosHome, args: BoardArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let id = match args.run {
        Some(id) => id,
        None => current_run(&client, Which::Any).await?,
    };
    let board = client.board(id).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&board)?);
    } else {
        print!("{}", render_board(&board));
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn approve_plan(home: &CosHome, args: RunRef) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let id = match args.run {
        Some(id) => id,
        None => current_run(&client, Which::AwaitingApproval).await?,
    };
    let board = client.approve_plan(id).await?;
    println!("Approved the plan for run {}: {}.", id, board.counts.summary());
    println!("  Workers start on the ready tasks now. Watch with `cos board {id}`.");
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn cancel(home: &CosHome, args: CancelArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let id = match args.run {
        Some(id) => id,
        None => current_run(&client, Which::Unfinished).await?,
    };
    let board = client.cancel_run(id, args.reason).await?;
    println!("Cancelled run {id}: {}.", board.counts.summary());
    Ok(ExitCode::SUCCESS)
}

/// Which runs a command can default to.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Which {
    /// Any run; falls back to the newest one if none is unfinished.
    Any,
    Unfinished,
    AwaitingApproval,
}

/// The run a command means when none is named.
pub(crate) async fn current_run(client: &Client, which: Which) -> anyhow::Result<i64> {
    let runs = client.runs().await?;
    let projects = client.projects().await?;
    let here = current_dir()
        .ok()
        .and_then(|dir| workspace_here(&projects, &dir).ok().map(|workspace| workspace.id));
    pick_run(&runs, here, which)
}

fn pick_run(runs: &[RunSummary], here: Option<i64>, which: Which) -> anyhow::Result<i64> {
    let fits = |summary: &&RunSummary| match which {
        Which::AwaitingApproval => summary.run.state == RunState::AwaitingPlanApproval,
        Which::Any | Which::Unfinished => !summary.run.state.is_finished(),
    };
    let candidates: Vec<&RunSummary> = runs.iter().filter(fits).collect();
    let in_here: Vec<&RunSummary> = candidates
        .iter()
        .copied()
        .filter(|summary| Some(summary.run.workspace_id) == here)
        .collect();
    let choice = if in_here.is_empty() { candidates } else { in_here };
    match choice.as_slice() {
        [only] => Ok(only.run.id),
        [] if which == Which::Any => match runs.first() {
            Some(newest) => Ok(newest.run.id),
            None => bail!("there are no runs yet; start one with `cos run \"<goal>\"`"),
        },
        [] if which == Which::AwaitingApproval => bail!("no run has a plan waiting for approval"),
        [] => bail!("there is no unfinished run"),
        several => {
            let ids: Vec<String> = several.iter().map(|summary| summary.run.id.to_string()).collect();
            bail!(
                "more than one run could be meant ({}); name one, e.g. `{}`",
                ids.join(", "),
                ids[0]
            )
        }
    }
}

/// The workspace `dir` is in, or the only workspace of the project it's in.
fn workspace_here<'a>(projects: &'a [Project], dir: &Path) -> anyhow::Result<&'a Workspace> {
    if let Some(workspace) = all_workspaces(projects).find(|workspace| dir.starts_with(&workspace.path)) {
        return Ok(workspace);
    }
    let Some(project) = projects.iter().find(|project| dir.starts_with(&project.root)) else {
        bail!(
            "this folder isn't in a cos workspace; cd into one or pass --workspace (see `cos project list`)"
        );
    };
    match project.workspaces.as_slice() {
        [workspace] => Ok(workspace),
        [] => bail!(
            "{} has no workspace yet; create one with `cos workspace new {}`",
            project.name,
            project.name
        ),
        _ => bail!(
            "{} has several workspaces; cd into one or pass --workspace",
            project.name
        ),
    }
}

fn all_workspaces(projects: &[Project]) -> impl Iterator<Item = &Workspace> {
    projects.iter().flat_map(|project| &project.workspaces)
}

fn current_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = std::env::current_dir().context("could not read the current folder")?;
    Ok(dir.canonicalize().unwrap_or(dir))
}

/// The goal's first line, cut short.
fn preview(goal: &str) -> String {
    let line = goal.lines().next().unwrap_or_default();
    if line.chars().count() > GOAL_PREVIEW_CHARS || goal.lines().nth(1).is_some() {
        let cut: String = line.chars().take(GOAL_PREVIEW_CHARS).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use cos_core::{Run, TaskCounts};
    use jiff::Timestamp;

    use super::*;

    fn summary(id: i64, workspace_id: i64, state: RunState) -> RunSummary {
        RunSummary {
            run: Run {
                id,
                workspace_id,
                goal: "x".into(),
                state,
                preset: ApprovalPreset::Normal,
                approval_cap: 20,
                report_path: None,
                reason: None,
                chief_agent_id: None,
                created_at: Timestamp::UNIX_EPOCH,
                updated_at: Timestamp::UNIX_EPOCH,
            },
            tasks: TaskCounts::default(),
        }
    }

    #[test]
    fn the_run_in_this_workspace_wins() {
        // Newest first, as the daemon lists them.
        let runs = [
            summary(3, 2, RunState::Running),
            summary(2, 1, RunState::AwaitingPlanApproval),
            summary(1, 1, RunState::Done),
        ];

        assert_eq!(pick_run(&runs, Some(1), Which::Unfinished).unwrap(), 2);
        assert_eq!(pick_run(&runs, Some(2), Which::Any).unwrap(), 3);
        assert_eq!(pick_run(&runs, None, Which::AwaitingApproval).unwrap(), 2);
        let error = pick_run(&runs, None, Which::Unfinished).unwrap_err();
        assert!(error.to_string().contains("more than one run"), "{error}");
    }

    #[test]
    fn with_nothing_unfinished_only_board_falls_back_to_the_newest() {
        let runs = [summary(5, 1, RunState::Done), summary(4, 1, RunState::Cancelled)];

        assert_eq!(pick_run(&runs, None, Which::Any).unwrap(), 5);
        assert!(pick_run(&runs, None, Which::Unfinished).is_err());
        assert!(pick_run(&[], None, Which::Any).is_err());
    }

    #[test]
    fn long_goals_are_cut_short() {
        assert_eq!(preview("add rate limiting"), "add rate limiting");
        assert_eq!(preview("first line\nsecond"), "first line…");
        assert_eq!(preview(&"a".repeat(80)).chars().count(), GOAL_PREVIEW_CHARS + 1);
    }
}
