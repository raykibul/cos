//! `cos judge`: jev in shadow mode (see `docs/judge.md`).
//!
//! - `cos judge status [--reload]`: where the key was found, the
//!   per-project switches and how many decisions were made
//! - `cos judge stats`: per decision point, jev against what you did
//! - `cos judge on|off --project P`
//! - `cos judge test`: one real call with a synthetic `ls` request, made
//!   here rather than in the daemon, printing the typed answers

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use cos_core::PointStats;
use cos_judge::key::{
    KEY_ENV, KeySource, PROBE_TIMEOUT, Probe, probe_shell, read_env_file, resolve_without_shell,
};
use cos_judge::permission::{Locality, PermissionInput, RawRequest, would_have};
use cos_judge::{ApiKey, JudgeClient};
use jiff::Timestamp;

use crate::client::Client;
use crate::daemon::judge::{JudgeOverview, login_shell};
use crate::home::CosHome;
use crate::projects_cmd::find_project;

#[derive(clap::Args)]
pub(crate) struct JudgeArgs {
    #[command(subcommand)]
    command: JudgeCommand,
}

#[derive(Subcommand)]
enum JudgeCommand {
    /// Where the key was found, which projects are judged, and counts.
    Status {
        /// Look for the key again, your login shell included.
        #[arg(long)]
        reload: bool,
    },
    /// Per decision point: decisions, agreement with you, would-have
    /// auto-approvals and disagreements.
    Stats,
    /// Let jev judge a project's permission requests and inbox items. They
    /// go to TypeSafe, trimmed; nothing changes what happens.
    On {
        /// The project (id or name).
        #[arg(long)]
        project: String,
    },
    /// Stop judging a project.
    Off {
        /// The project (id or name).
        #[arg(long)]
        project: String,
    },
    /// One real call to jev with a synthetic `ls` permission request, made
    /// from here, not the daemon. Sends nothing from your repos.
    Test,
}

pub(crate) async fn judge(home: &CosHome, args: JudgeArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match args.command {
        JudgeCommand::Status { reload } => {
            let overview = if reload {
                client.reload_judge().await?
            } else {
                client.judge().await?
            };
            print_lines(render_status(&overview));
        }
        JudgeCommand::Stats => print_lines(render_stats(&client.judge().await?.points)),
        JudgeCommand::On { project } => switch(&client, &project, true).await?,
        JudgeCommand::Off { project } => switch(&client, &project, false).await?,
        JudgeCommand::Test => return test(home).await,
    }
    Ok(ExitCode::SUCCESS)
}

fn print_lines(lines: Vec<String>) {
    for line in lines {
        println!("{line}");
    }
}

async fn switch(client: &Client, key: &str, enabled: bool) -> anyhow::Result<()> {
    let project = find_project(&client.projects().await?, key)?.clone();
    let switch = client.set_judge(project.id, enabled).await?;
    if switch.enabled {
        println!(
            "jev now judges {} in shadow mode: its permission requests and chief inbox items go to TypeSafe, \
             trimmed, and what jev would do is logged. Nothing else changes.",
            switch.name
        );
        let overview = client.judge().await?;
        if !overview.active {
            println!(
                "  note: jev is off until it has a key: {}",
                overview.off_reason.unwrap_or_default()
            );
        }
    } else {
        println!("jev no longer judges {}.", switch.name);
    }
    Ok(())
}

fn render_status(overview: &JudgeOverview) -> Vec<String> {
    let mut lines = Vec::new();
    if overview.active {
        lines.push(format!(
            "jev: on, in shadow mode (key found in {})",
            overview.key.source.describe()
        ));
    } else {
        lines.push(format!(
            "jev: off ({})",
            overview.off_reason.as_deref().unwrap_or("no key")
        ));
    }
    if let Some(detail) = &overview.key.detail {
        lines.push(format!("  note: {detail}"));
    }
    lines.push(format!("  {} ({})", overview.endpoint, overview.model));
    if overview.projects.is_empty() {
        lines.push("No projects yet.".into());
    } else {
        lines.push("Projects:".into());
        let width = overview
            .projects
            .iter()
            .map(|project| project.name.chars().count())
            .max()
            .unwrap_or(0);
        for project in &overview.projects {
            let state = if project.enabled { "on" } else { "off" };
            lines.push(format!(
                "  {:>3}  {:<width$}  {state}",
                project.project_id, project.name
            ));
        }
    }
    let counts: Vec<String> = overview
        .points
        .iter()
        .map(|point| format!("{} {}", point_name(&point.point), point.decisions))
        .collect();
    lines.push(format!("Decisions: {}", counts.join(", ")));
    lines
}

fn point_name(point: &str) -> &str {
    match point {
        "permission" => "permission requests",
        "wake" => "chief wakeups",
        other => other,
    }
}

fn render_stats(points: &[PointStats]) -> Vec<String> {
    let mut lines = Vec::new();
    for point in points {
        let mut head = format!(
            "{}: {} judged",
            capitalized(point_name(&point.point)),
            point.decisions
        );
        if point.no_opinion > 0 || point.errors > 0 {
            head.push_str(&format!(
                " ({} no opinion, {} errors)",
                point.no_opinion, point.errors
            ));
        }
        lines.push(head);
        match point.point.as_str() {
            "permission" => {
                if point.answered == 0 {
                    lines.push("  You haven't answered any of them through cos yet.".into());
                } else {
                    let mut line = format!(
                        "  Would have auto-approved {} of {}, disagreed on {}",
                        point.would_approve, point.answered, point.disagreed
                    );
                    if point.missed > 0 {
                        line.push_str(&format!("; asked you about {} you then allowed", point.missed));
                    }
                    lines.push(format!("{line}. Agreed on {}.", point.agreed));
                }
                if !point.disagreements.is_empty() {
                    lines.push("  Disagreements (jev would approve, you denied):".into());
                    for example in &point.disagreements {
                        let at = Timestamp::from_millisecond(example.created_ms)
                            .map(|at| at.strftime("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        lines.push(format!(
                            "    #{:<4} {}  ({}) {at}",
                            example.id,
                            example.summary,
                            example.why.as_deref().unwrap_or("")
                        ));
                    }
                }
            }
            _ => {
                let would: Vec<String> = point
                    .would
                    .iter()
                    .map(|(would, count)| format!("{would} {count}"))
                    .collect();
                if !would.is_empty() {
                    lines.push(format!("  Would have: {}", would.join(", ")));
                }
            }
        }
    }
    lines
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `cos judge test`: finds the key as the daemon would (this shell's
/// environment, `daemon.env`, the login shell), then makes one call.
async fn test(home: &CosHome) -> anyhow::Result<ExitCode> {
    let Some((key, source)) = find_key(home).await else {
        println!(
            "No {KEY_ENV} in this environment, {} or your login shell; nothing sent.",
            crate::daemon_env::path(home).display()
        );
        return Ok(ExitCode::FAILURE);
    };
    println!("Key found in {}.", source.describe());

    // Synthetic: no repo, no real paths.
    let root = PathBuf::from("/tmp/cos-judge-test");
    let input = serde_json::json!({ "command": "ls", "description": "List files in the current directory" });
    let request = PermissionInput::from_request(&RawRequest {
        tool: "Bash",
        input: &input,
        cwd: Some(&root),
        worktree_root: &root,
        home: None,
        agent_role: "worker",
        task_title: Some("List the files in the project"),
        task_instructions: Some("Run ls to see which files are there."),
        policy_tier: "ask you",
        policy_reason: "a synthetic request from `cos judge test`",
    });
    println!("Asking jev about: Bash ls (synthetic)");
    let client = JudgeClient::new(key);
    let result = client.permission(&request).await;
    match &result {
        Ok(judged) => {
            let answers = &judged.answers;
            println!(
                "  model: {}, {} ms",
                judged.model.as_deref().unwrap_or("?"),
                judged.latency_ms
            );
            println!(
                "  action_kind: {} (confidence {:.2})",
                answers.action_kind.picked, answers.action_kind.confidence
            );
            for (option, probability) in &answers.action_kind.probabilities {
                println!("    {option:<18} {probability:.3}");
            }
            println!("  serves_task: {:.3}", answers.serves_task.probability);
            println!("  leaves_worktree: {:.3}", answers.leaves_worktree.probability);
        }
        Err(error) => println!("  jev failed: {error}"),
    }
    let verdict = would_have(Locality::Worktree, result.as_ref().map(|judged| &judged.answers));
    println!("  would have: {} ({})", verdict.action.as_str(), verdict.why);
    Ok(if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn find_key(home: &CosHome) -> Option<(ApiKey, KeySource)> {
    let file = read_env_file(&crate::daemon_env::path(home));
    if let Err(error @ cos_judge::key::EnvFileError::Unsafe { .. }) = &file {
        eprintln!("cos: {error}");
    }
    if let Some(found) = resolve_without_shell(std::env::var(KEY_ENV).ok().as_deref(), &file) {
        return Some(found);
    }
    let shell = login_shell();
    match probe_shell(Path::new(&shell), KEY_ENV, PROBE_TIMEOUT).await {
        Probe::Found(key) => Some((key, KeySource::Shell)),
        Probe::NotSet => None,
        Probe::Failed(why) => {
            eprintln!("cos: could not ask {} for {KEY_ENV}: {why}", shell.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cos_core::ProjectSwitch;
    use cos_core::judge::Example;
    use cos_judge::key::KeySource;

    use super::*;
    use crate::daemon::judge::KeyStatus;

    fn stats() -> Vec<PointStats> {
        vec![
            PointStats {
                point: "permission".into(),
                decisions: 45,
                no_opinion: 2,
                answered: 45,
                would_approve: 41,
                agreed: 39,
                disagreed: 4,
                missed: 1,
                disagreements: vec![Example {
                    id: 12,
                    point: "permission".into(),
                    project_id: Some(1),
                    subject: "approval:30".into(),
                    summary: "Bash rm -rf build".into(),
                    would: "approve".into(),
                    why: Some("edit_in_worktree (0.93), serves_task 0.85".into()),
                    actual: Some("denied".into()),
                    answers: None,
                    created_ms: 0,
                }],
                ..PointStats::default()
            },
            PointStats {
                point: "wake".into(),
                decisions: 3,
                would: BTreeMap::from([("now".into(), 1), ("batch".into(), 2)]),
                ..PointStats::default()
            },
        ]
    }

    #[test]
    fn stats_read_like_the_app() {
        let lines = render_stats(&stats()).join("\n");

        assert!(lines.contains("Permission requests: 45 judged (2 no opinion, 0 errors)"));
        assert!(lines.contains("Would have auto-approved 41 of 45, disagreed on 4"));
        assert!(lines.contains("asked you about 1 you then allowed"));
        assert!(lines.contains("#12   Bash rm -rf build"));
        assert!(lines.contains("Chief wakeups: 3 judged"));
        assert!(lines.contains("Would have: batch 2, now 1"));
    }

    #[test]
    fn status_says_where_the_key_is_and_never_what() {
        let mut overview = JudgeOverview {
            key: KeyStatus {
                source: KeySource::Shell,
                detail: None,
                probing: false,
            },
            active: true,
            off_reason: None,
            endpoint: cos_judge::ENDPOINT.into(),
            model: cos_judge::MODEL.into(),
            projects: vec![ProjectSwitch {
                project_id: 1,
                name: "cos".into(),
                enabled: true,
            }],
            points: stats(),
        };

        let on = render_status(&overview).join("\n");
        overview.active = false;
        overview.key.source = KeySource::Missing;
        overview.off_reason = Some("no TYPESAFEAI_API_KEY anywhere".into());
        let off = render_status(&overview).join("\n");

        assert!(on.contains("jev: on, in shadow mode (key found in your login shell)"));
        assert!(on.contains("  1  cos  on"));
        assert!(on.contains("Decisions: permission requests 45, chief wakeups 3"));
        assert!(off.contains("jev: off (no TYPESAFEAI_API_KEY anywhere)"));
    }
}
