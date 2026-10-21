//! `cos limits [run] [--preset …] [--cap N]`: shows or changes how much the
//! chief may approve for you in a run. See `docs/approvals.md`.

use std::process::ExitCode;

use cos_core::{LimitsChange, RunLimits};

use crate::client::Client;
use crate::home::CosHome;
use crate::runs_cmd::{Preset, Which, current_run};

#[derive(clap::Args)]
pub(crate) struct LimitsArgs {
    /// Run id. Defaults to the current run.
    run: Option<i64>,
    /// How much the chief may approve for you from now on.
    #[arg(long, value_enum)]
    preset: Option<Preset>,
    /// The most approvals the chief may give in this run.
    #[arg(long)]
    cap: Option<u32>,
}

pub(crate) async fn limits(home: &CosHome, args: LimitsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let changing = args.preset.is_some() || args.cap.is_some();
    let id = match args.run {
        Some(id) => id,
        None if changing => current_run(&client, Which::Unfinished).await?,
        None => current_run(&client, Which::Any).await?,
    };
    let limits = if changing {
        let change = LimitsChange {
            preset: args.preset.map(Into::into),
            approval_cap: args.cap,
        };
        client.set_run_limits(id, &change).await?
    } else {
        client.run_limits(id).await?
    };
    print!("{}", describe(id, &limits));
    if changing {
        println!("  Requests already waiting are checked against these limits when the chief answers them.");
    }
    Ok(ExitCode::SUCCESS)
}

fn describe(id: i64, limits: &RunLimits) -> String {
    let preset = match limits.preset {
        cos_core::ApprovalPreset::Strict => "Strict: only you approve",
        cos_core::ApprovalPreset::Normal => {
            "Normal: the chief approves routine work inside the task's worktree"
        }
        cos_core::ApprovalPreset::Trusted => "Trusted: the chief also approves network access and installs",
    };
    format!(
        "Run {id}\n  Preset: {preset}\n  Chief approvals: {} of {}\n",
        limits.chief_approvals, limits.approval_cap
    )
}

#[cfg(test)]
mod tests {
    use cos_core::ApprovalPreset;

    use super::*;

    #[test]
    fn describes_the_limits() {
        let limits = RunLimits {
            preset: ApprovalPreset::Trusted,
            approval_cap: 20,
            chief_approvals: 3,
        };
        assert_eq!(
            describe(4, &limits),
            "Run 4\n  Preset: Trusted: the chief also approves network access and installs\n  \
             Chief approvals: 3 of 20\n"
        );
    }
}
