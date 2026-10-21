//! CLI commands that talk to the daemon or manage it.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use cos_core::LoggedEvent;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::client::{Client, NotRunning};
use crate::home::CosHome;
use crate::launchd;

#[derive(clap::Args)]
pub(crate) struct EventsArgs {
    /// Start after this sequence number. Defaults to showing the last `--limit` events.
    #[arg(long)]
    after: Option<u64>,
    /// How many events to show.
    #[arg(long, short = 'n', default_value_t = 20)]
    limit: usize,
    /// Keep printing new events as they happen. Survives daemon restarts.
    #[arg(long, short = 'f')]
    follow: bool,
    /// One JSON object per line.
    #[arg(long)]
    json: bool,
}

pub(crate) async fn status(home: &CosHome) -> anyhow::Result<ExitCode> {
    let launchd = if launchd::is_installed() {
        format!("installed ({})", launchd::LABEL)
    } else {
        "not installed".into()
    };

    match Client::new(home.socket()).status().await {
        Ok(status) => {
            let uptime = Timestamp::now().duration_since(status.started_at);
            println!("cos daemon: running");
            println!(
                "  pid      {}, up {}, v{}",
                status.pid,
                human_duration(uptime.as_secs().max(0) as u64),
                status.version
            );
            println!("  home     {}", status.home.display());
            println!("  events   {} (last #{})", status.events, status.last_seq);
            println!("  launchd  {launchd}");
            Ok(ExitCode::SUCCESS)
        }
        Err(error) if error.is::<NotRunning>() => {
            println!("cos daemon: not running");
            println!("  home     {}", home.root().display());
            println!("  launchd  {launchd}");
            println!("  start it with `cos daemon`, or `cos daemon install` to run it at login");
            Ok(ExitCode::FAILURE)
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn events(home: &CosHome, args: EventsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let after = match args.after {
        Some(after) => after,
        None => {
            let last = client.status().await?.last_seq;
            last.saturating_sub(args.limit as u64)
        }
    };

    if !args.follow {
        for event in client.events(after, args.limit).await? {
            print_event(&event, args.json);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut cursor = after;
    let mut waiting = false;
    loop {
        match client.follow(cursor).await {
            Ok(mut stream) => {
                if waiting {
                    eprintln!("(daemon is back)");
                    waiting = false;
                }
                // Ends when the daemon stops or the connection drops.
                while let Ok(Some(event)) = stream.next().await {
                    cursor = event.seq;
                    print_event(&event, args.json);
                }
            }
            Err(error) if error.is::<NotRunning>() => {}
            Err(error) => return Err(error),
        }
        if !waiting {
            eprintln!("(daemon stopped; waiting for it to come back, Ctrl-C to quit)");
            waiting = true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub(crate) async fn install(home: &CosHome) -> anyhow::Result<ExitCode> {
    let plist = launchd::install(home)?;
    println!("Installed {}", plist.display());
    if std::env::current_exe()?
        .components()
        .any(|part| part.as_os_str() == "target")
    {
        println!("  note: it runs this binary from the build directory; rebuilding replaces it");
    }

    // Give launchd a moment to start it.
    let client = Client::new(home.socket());
    for _ in 0..50 {
        if let Ok(status) = client.status().await {
            println!(
                "cos daemon is running (pid {}). It starts again at every login.",
                status.pid
            );
            return Ok(ExitCode::SUCCESS);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!(
        "launchd loaded the agent, but the daemon isn't answering yet. Check {}",
        home.daemon_log().display()
    );
    Ok(ExitCode::FAILURE)
}

pub(crate) fn uninstall() -> anyhow::Result<ExitCode> {
    if launchd::uninstall().context("could not uninstall the launchd agent")? {
        println!("Stopped the daemon and removed the launchd agent.");
    } else {
        println!("The launchd agent wasn't installed.");
    }
    Ok(ExitCode::SUCCESS)
}

fn print_event(event: &LoggedEvent, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(event).expect("events serialize to JSON")
        );
        return;
    }
    let time = event.ts.to_zoned(TimeZone::system()).strftime("%b %d %H:%M:%S");
    let run = event
        .run_id
        .as_deref()
        .map(|id| format!(" [{id}]"))
        .unwrap_or_default();
    println!("#{:<5} {time}  {}{run}  {}", event.seq, event.kind, event.data);
}

pub(crate) fn human_duration(seconds: u64) -> String {
    let (days, hours, minutes, secs) = (
        seconds / 86_400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{secs}s"),
        (0, 0, _) => format!("{minutes}m {secs}s"),
        (0, _, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human_duration(42), "42s");
        assert_eq!(human_duration(125), "2m 5s");
        assert_eq!(human_duration(3 * 3600 + 7 * 60 + 9), "3h 7m");
        assert_eq!(human_duration(2 * 86_400 + 5 * 3600), "2d 5h");
    }
}
