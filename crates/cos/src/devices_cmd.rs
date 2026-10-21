//! `cos remote …` (the tailnet listener) and `cos devices …` (which tailnet
//! devices may use it). Both only work against the local daemon.

use std::process::ExitCode;

use anyhow::bail;
use clap::Subcommand;
use cos_core::{Device, DeviceState};
use jiff::tz::TimeZone;

use crate::client::Client;
use crate::daemon::tailnet::{ListenerState, TailnetStatus};
use crate::home::CosHome;

#[derive(Subcommand)]
pub(crate) enum RemoteCommand {
    /// Show whether remote access is on and where it listens (the default).
    Status,
    /// Let your other devices reach this daemon over Tailscale.
    On {
        /// TCP port on this machine's tailnet addresses.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Stop listening on the tailnet. The Unix socket is unaffected.
    Off,
}

#[derive(Subcommand)]
pub(crate) enum DevicesCommand {
    /// List devices that have tried to connect (the default).
    List,
    /// Let a device in.
    Approve {
        /// Device number, name or Tailscale node ID.
        device: String,
    },
    /// Refuse a device until it is approved.
    Deny {
        /// Device number, name or Tailscale node ID.
        device: String,
    },
    /// Forget a device. If it connects again it has to be approved again.
    Remove {
        /// Device number, name or Tailscale node ID.
        device: String,
    },
}

pub(crate) async fn remote(home: &CosHome, command: Option<RemoteCommand>) -> anyhow::Result<ExitCode> {
    let client = local_client(home, "cos remote")?;
    let status = match command.unwrap_or(RemoteCommand::Status) {
        RemoteCommand::Status => client.tailnet().await?,
        RemoteCommand::On { port } => client.configure_tailnet(true, port).await?,
        RemoteCommand::Off => client.configure_tailnet(false, None).await?,
    };
    for line in describe(&status) {
        println!("{line}");
    }
    let usable = !status.enabled || status.state == ListenerState::Listening;
    Ok(if usable {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub(crate) async fn devices(home: &CosHome, command: Option<DevicesCommand>) -> anyhow::Result<ExitCode> {
    let client = local_client(home, "cos devices")?;
    match command.unwrap_or(DevicesCommand::List) {
        DevicesCommand::List => {
            let devices = client.devices().await?;
            if devices.is_empty() {
                println!(
                    "No devices yet. A device shows up here the first time it connects over the tailnet."
                );
            }
            for device in &devices {
                print_device(device);
            }
        }
        DevicesCommand::Approve { device } => {
            let device = find_device(&client.devices().await?, &device)?.clone();
            let device = client.approve_device(device.id).await?;
            println!("Approved {} (#{}). It can use cos now.", device.name, device.id);
        }
        DevicesCommand::Deny { device } => {
            let device = find_device(&client.devices().await?, &device)?.clone();
            let device = client.deny_device(device.id).await?;
            println!("Denied {} (#{}).", device.name, device.id);
        }
        DevicesCommand::Remove { device } => {
            let device = find_device(&client.devices().await?, &device)?.clone();
            client.remove_device(device.id).await?;
            println!(
                "Removed {} (#{}). If it connects again it will need approving again.",
                device.name, device.id
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The local daemon's client. Device and listener management are refused
/// over the tailnet, so `--host` makes no sense here.
fn local_client(home: &CosHome, command: &str) -> anyhow::Result<Client> {
    let client = Client::new(home.socket());
    if client.is_remote() {
        bail!("{command} only works on the machine the daemon runs on; drop --host / COS_HOST");
    }
    Ok(client)
}

/// Lines describing the tailnet listener, for `cos remote` and `cos status`.
pub(crate) fn describe(status: &TailnetStatus) -> Vec<String> {
    let mut lines = Vec::new();
    match status.state {
        ListenerState::Off => lines.push("remote access: off (turn it on with `cos remote on`)".into()),
        ListenerState::Starting => lines.push(format!("remote access: starting on port {}", status.port)),
        ListenerState::Listening => {
            let addresses: Vec<String> = status.addresses.iter().map(ToString::to_string).collect();
            lines.push(format!("remote access: listening on {}", addresses.join(", ")));
            if let Some(host) = &status.host {
                lines.push(format!(
                    "  from another device: cos --host {host}:{} status",
                    status.port
                ));
            }
        }
        ListenerState::Unavailable => lines.push(format!(
            "remote access: on, but not listening on port {}; trying again every 30 s",
            status.port
        )),
    }
    if let Some(owner) = &status.owner {
        lines.push(format!("  devices must be logged in to Tailscale as {owner}"));
    }
    if let Some(reason) = &status.reason {
        lines.push(format!("  {reason}"));
    }
    lines
}

fn print_device(device: &Device) {
    let state = match device.state {
        DeviceState::Pending => format!("pending: approve with `cos devices approve {}`", device.id),
        DeviceState::Approved | DeviceState::Denied => device.state.as_str().to_owned(),
    };
    let os = if device.os.is_empty() {
        String::new()
    } else {
        format!(", {}", device.os)
    };
    let seen = device
        .last_seen_at
        .to_zoned(TimeZone::system())
        .strftime("%b %d %H:%M");
    println!(
        "#{} {} ({}{os})  {state}",
        device.id, device.name, device.hostname
    );
    println!(
        "    {}  last seen {seen}{}",
        device.login_name,
        device
            .last_ip
            .as_deref()
            .map(|ip| format!(" from {ip}"))
            .unwrap_or_default()
    );
}

/// A device by number, name or Tailscale node ID.
fn find_device<'a>(devices: &'a [Device], key: &str) -> anyhow::Result<&'a Device> {
    let key = key.strip_prefix('#').unwrap_or(key);
    let matches: Vec<&Device> = devices
        .iter()
        .filter(|device| device.id.to_string() == key || device.name == key || device.node_id == key)
        .collect();
    match matches.as_slice() {
        [device] => Ok(device),
        [] => bail!("no device called {key}; see `cos devices`"),
        _ => bail!("more than one device is called {key}; use its number from `cos devices`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: ListenerState) -> TailnetStatus {
        TailnetStatus {
            enabled: state != ListenerState::Off,
            port: 7700,
            state,
            addresses: Vec::new(),
            reason: None,
            host: None,
            owner: None,
        }
    }

    #[test]
    fn describes_each_listener_state() {
        assert!(describe(&status(ListenerState::Off))[0].contains("off"));

        let listening = TailnetStatus {
            addresses: vec!["100.100.1.1:7700".parse().unwrap()],
            host: Some("my-mac".into()),
            owner: Some("me@example.com".into()),
            ..status(ListenerState::Listening)
        };
        let lines = describe(&listening);
        assert_eq!(lines[0], "remote access: listening on 100.100.1.1:7700");
        assert!(lines[1].contains("cos --host my-mac:7700 status"));
        assert!(lines[2].contains("me@example.com"));

        let unavailable = TailnetStatus {
            reason: Some("Tailscale is not logged in".into()),
            ..status(ListenerState::Unavailable)
        };
        let lines = describe(&unavailable);
        assert!(lines[0].contains("not listening"));
        assert_eq!(lines[1], "  Tailscale is not logged in");
    }
}
