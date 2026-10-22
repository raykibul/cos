//! `cos env [--project P] [--show]`, `cos env set NAME=value [--project P]`
//! and `cos env unset NAME [--project P]`: the environment variables agents
//! get, globally or for one project. Values are masked unless `--show`.

use std::io::Read;
use std::process::ExitCode;

use anyhow::Context;
use clap::Subcommand;
use cos_core::{Project, Variables};

use crate::client::Client;
use crate::home::CosHome;
use crate::projects_cmd::find_project;

/// Values longer than this show their first [`SHOWN_CHARS`] characters.
const SHORT_VALUE: usize = 8;
const SHOWN_CHARS: usize = 3;
const APPLIES: &str = "New agents and resumed ones get it; running agents keep what they started with.";

#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct EnvArgs {
    #[command(subcommand)]
    action: Option<EnvCommand>,
    /// A project's variables (id or name), with the global ones under them.
    #[arg(long)]
    project: Option<String>,
    /// Print the values instead of masking them.
    #[arg(long)]
    show: bool,
}

#[derive(Subcommand)]
enum EnvCommand {
    /// Set a variable: `cos env set NAME=value`. With only `NAME`, the value
    /// is read from stdin, which keeps it out of your shell history.
    Set {
        #[arg(value_name = "NAME=VALUE")]
        assignment: String,
        /// Set it for this project (id or name) instead of globally.
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove a variable.
    Unset {
        name: String,
        /// Remove it from this project (id or name) instead of the global list.
        #[arg(long)]
        project: Option<String>,
    },
}

pub(crate) async fn env(home: &CosHome, args: EnvArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    match args.action {
        None => {
            let lines = match args.project {
                Some(key) => {
                    let project = project(&client, &key).await?;
                    let env = client.project_env(project.id).await?;
                    render(Some((&project.name, &env.variables)), &env.global, args.show)
                }
                None => render(None, &client.global_env().await?.variables, args.show),
            };
            for line in lines {
                println!("{line}");
            }
        }
        Some(EnvCommand::Set {
            assignment,
            project: key,
        }) => {
            let (name, value) = match assignment.split_once('=') {
                Some((name, value)) => (name.to_owned(), value.to_owned()),
                None => (assignment, read_stdin()?),
            };
            match key {
                Some(key) => {
                    let project = project(&client, &key).await?;
                    client.set_project_env(project.id, &name, value).await?;
                    println!("Set {name} for {}. {APPLIES}", project.name);
                }
                None => {
                    client.set_global_env(&name, value).await?;
                    println!("Set {name} for every project. {APPLIES}");
                }
            }
        }
        Some(EnvCommand::Unset { name, project: key }) => match key {
            Some(key) => {
                let project = project(&client, &key).await?;
                let had = client
                    .project_env(project.id)
                    .await?
                    .variables
                    .contains_key(&name);
                client.unset_project_env(project.id, &name).await?;
                if had {
                    println!("Removed {name} from {}.", project.name);
                } else {
                    println!("{name} wasn't set for {}.", project.name);
                }
            }
            None => {
                let had = client.global_env().await?.variables.contains_key(&name);
                client.unset_global_env(&name).await?;
                if had {
                    println!("Removed {name} from the global variables.");
                } else {
                    println!("{name} wasn't a global variable.");
                }
            }
        },
    }
    Ok(ExitCode::SUCCESS)
}

async fn project(client: &Client, key: &str) -> anyhow::Result<Project> {
    Ok(find_project(&client.projects().await?, key)?.clone())
}

/// A value from stdin, without the newline that ends it.
fn read_stdin() -> anyhow::Result<String> {
    let mut value = String::new();
    std::io::stdin()
        .read_to_string(&mut value)
        .context("could not read the value from stdin")?;
    let trimmed = value.strip_suffix('\n').unwrap_or(&value);
    Ok(trimmed.strip_suffix('\r').unwrap_or(trimmed).to_owned())
}

/// The listing: a project's own variables if given, then the global ones,
/// marking those the project overrides.
fn render(project: Option<(&str, &Variables)>, global: &Variables, show: bool) -> Vec<String> {
    let width = project
        .iter()
        .flat_map(|(_, variables)| variables.keys())
        .chain(global.keys())
        .map(|name| name.chars().count())
        .max()
        .unwrap_or(0);
    let row = |name: &str, value: &str| {
        let value = if show { value.to_owned() } else { mask(value) };
        format!("  {name:<width$}  {value}")
    };
    let mut lines = Vec::new();
    if let Some((name, variables)) = project {
        lines.push(format!("{name}:"));
        if variables.is_empty() {
            lines.push("  (none)".into());
        }
        lines.extend(variables.iter().map(|(name, value)| row(name, value)));
        lines.push("Global:".into());
        if global.is_empty() {
            lines.push("  (none)".into());
        }
        for (key, value) in global {
            let mut line = row(key, value);
            if variables.contains_key(key) {
                line.push_str(&format!("  · overridden by {name}"));
            }
            lines.push(line);
        }
        if variables.is_empty() {
            lines.push(format!("Set one with `cos env set NAME=value --project {name}`."));
        }
    } else if global.is_empty() {
        lines.push("No global variables. Set one with `cos env set NAME=value`.".into());
    } else {
        lines.push("Global (every project):".into());
        lines.extend(global.iter().map(|(name, value)| row(name, value)));
    }
    lines
}

/// `abc…  (42 chars)`: the first few characters of a longer value, and its
/// length. A short value shows none of itself.
fn mask(value: &str) -> String {
    let length = value.chars().count();
    if length == 0 {
        return "(empty)".into();
    }
    let shown: String = if length > SHORT_VALUE {
        value.chars().take(SHOWN_CHARS).collect()
    } else {
        String::new()
    };
    format!("{shown}…  ({length} chars)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Variables {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn values_are_masked_to_a_prefix_and_a_length() {
        assert_eq!(mask("sk-ant-api03-secret"), "sk-…  (19 chars)");
        assert_eq!(mask("short"), "…  (5 chars)");
        assert_eq!(mask(""), "(empty)");
        assert_eq!(mask("ééééééééé"), "ééé…  (9 chars)");
    }

    #[test]
    fn lists_the_global_variables() {
        let global = vars(&[("ANTHROPIC_API_KEY", "sk-ant-api03-secret"), ("DEBUG", "1")]);
        assert_eq!(
            render(None, &global, false),
            [
                "Global (every project):",
                "  ANTHROPIC_API_KEY  sk-…  (19 chars)",
                "  DEBUG              …  (1 chars)",
            ]
        );
        assert_eq!(
            render(None, &global, true),
            [
                "Global (every project):",
                "  ANTHROPIC_API_KEY  sk-ant-api03-secret",
                "  DEBUG              1",
            ]
        );
        assert_eq!(
            render(None, &Variables::new(), false),
            ["No global variables. Set one with `cos env set NAME=value`."]
        );
    }

    #[test]
    fn lists_a_project_over_the_global_variables() {
        let own = vars(&[("CLAUDE_CONFIG_DIR", "~/.claude-personal")]);
        let global = vars(&[("CLAUDE_CONFIG_DIR", "~/.claude"), ("DEBUG", "1")]);
        assert_eq!(
            render(Some(("topflexi", &own)), &global, false),
            [
                "topflexi:",
                "  CLAUDE_CONFIG_DIR  ~/.…  (18 chars)",
                "Global:",
                "  CLAUDE_CONFIG_DIR  ~/.…  (9 chars)  · overridden by topflexi",
                "  DEBUG              …  (1 chars)",
            ]
        );
        assert_eq!(
            render(Some(("app", &Variables::new())), &Variables::new(), true),
            [
                "app:",
                "  (none)",
                "Global:",
                "  (none)",
                "Set one with `cos env set NAME=value --project app`.",
            ]
        );
    }
}
