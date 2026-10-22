//! The Macs the app can talk to: this one, through its socket, and the ones
//! you saved, over Tailscale. Saved as `hosts.json` in the app's config
//! folder, with the one you picked last. See "App: hosts and remote access"
//! in `docs/chief-of-staff.md`.

use std::path::{Path, PathBuf};

use cos_bridge::{BridgeError, Target, parse_address};
use serde::{Deserialize, Serialize};

/// The id of this Mac, which always exists and can't be removed.
pub(crate) const THIS_MAC: &str = "local";
const THIS_MAC_NAME: &str = "This Mac";
const MAX_NAME: usize = 64;

/// One host, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Host {
    pub id: String,
    pub name: String,
    /// `host:port` over Tailscale; null for this Mac.
    pub address: Option<String>,
}

impl Host {
    fn this_mac() -> Self {
        Self {
            id: THIS_MAC.to_owned(),
            name: THIS_MAC_NAME.to_owned(),
            address: None,
        }
    }

    /// Where its daemon is: `None` means this Mac's socket.
    pub(crate) fn target(&self) -> Option<Target> {
        self.address.clone().map(Target::Tcp)
    }
}

/// `hosts_list`'s answer: This Mac first, then the saved hosts in the order
/// they were added, and the id of the one in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct HostList {
    pub hosts: Vec<Host>,
    pub current: String,
}

/// The file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    hosts: Vec<Host>,
    #[serde(default)]
    current: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Hosts {
    /// Where they're saved; `None` keeps them in memory only.
    file: Option<PathBuf>,
    saved: Vec<Host>,
    current: String,
}

impl Hosts {
    /// Reads `file`, leniently: a missing or unreadable file is no saved
    /// hosts, and entries that don't parse are dropped.
    pub(crate) fn load(file: Option<PathBuf>) -> Self {
        let saved: Saved = file
            .as_deref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut hosts: Vec<Host> = Vec::new();
        for host in saved.hosts {
            let Some(address) = host.address.as_deref().and_then(|a| parse_address(a).ok()) else {
                continue;
            };
            let name = host.name.trim();
            if host.id == THIS_MAC || name.is_empty() || hosts.iter().any(|h| h.id == host.id) {
                continue;
            }
            hosts.push(Host {
                id: host.id,
                name: name.to_owned(),
                address: Some(address),
            });
        }
        let current = saved
            .current
            .filter(|id| hosts.iter().any(|h| &h.id == id))
            .unwrap_or_else(|| THIS_MAC.to_owned());
        Self {
            file,
            saved: hosts,
            current,
        }
    }

    pub(crate) fn list(&self) -> HostList {
        HostList {
            hosts: std::iter::once(Host::this_mac())
                .chain(self.saved.iter().cloned())
                .collect(),
            current: self.current.clone(),
        }
    }

    pub(crate) fn current(&self) -> Host {
        self.find(&self.current).unwrap_or_else(Host::this_mac)
    }

    fn find(&self, id: &str) -> Option<Host> {
        if id == THIS_MAC {
            return Some(Host::this_mac());
        }
        self.saved.iter().find(|h| h.id == id).cloned()
    }

    /// Saves a host. `address` is read as the CLI's `--host` reads it, so
    /// `my-mac` becomes `my-mac:7700`.
    pub(crate) fn add(&mut self, name: &str, address: &str) -> Result<Host, BridgeError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(BridgeError::bad_request("give the Mac a name"));
        }
        if name.chars().count() > MAX_NAME {
            return Err(BridgeError::bad_request(format!(
                "a name can be at most {MAX_NAME} characters"
            )));
        }
        let address = parse_address(address)?;
        if let Some(existing) = self.saved.iter().find(|h| h.address.as_deref() == Some(&address)) {
            return Err(BridgeError::bad_request(format!(
                "{address} is already saved as {}",
                existing.name
            )));
        }
        let next = self
            .saved
            .iter()
            .filter_map(|h| h.id.strip_prefix('h')?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let host = Host {
            id: format!("h{next}"),
            name: name.to_owned(),
            address: Some(address),
        };
        self.change(|hosts| hosts.saved.push(host.clone()))?;
        Ok(host)
    }

    /// Forgets a saved host. Removing the one in use switches to this Mac;
    /// the answer says whether that happened.
    pub(crate) fn remove(&mut self, id: &str) -> Result<bool, BridgeError> {
        if id == THIS_MAC {
            return Err(BridgeError::bad_request("This Mac can't be removed"));
        }
        if self.find(id).is_none() {
            return Err(BridgeError::bad_request(format!("no saved host {id}")));
        }
        let was_current = self.current == id;
        self.change(|hosts| {
            hosts.saved.retain(|h| h.id != id);
            if was_current {
                THIS_MAC.clone_into(&mut hosts.current);
            }
        })?;
        Ok(was_current)
    }

    /// Makes `id` the host in use, and remembers it for the next launch.
    pub(crate) fn select(&mut self, id: &str) -> Result<Host, BridgeError> {
        let host = self
            .find(id)
            .ok_or_else(|| BridgeError::bad_request(format!("no saved host {id}")))?;
        self.change(|hosts| hosts.current = host.id.clone())?;
        Ok(host)
    }

    /// Applies `edit` and saves; if saving fails, nothing changes.
    fn change(&mut self, edit: impl FnOnce(&mut Self)) -> Result<(), BridgeError> {
        let mut next = self.clone();
        edit(&mut next);
        if let Some(file) = &next.file {
            next.write(file)?;
        }
        *self = next;
        Ok(())
    }

    fn write(&self, file: &Path) -> Result<(), BridgeError> {
        let failed = |error: std::io::Error| {
            BridgeError::io(format!("could not save the hosts to {}: {error}", file.display()))
        };
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(failed)?;
        }
        let saved = Saved {
            hosts: self.saved.clone(),
            current: Some(self.current.clone()),
        };
        let json = serde_json::to_vec_pretty(&saved).map_err(|error| BridgeError::io(error.to_string()))?;
        // Write then rename, so a crash never leaves half a file.
        let temporary = file.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(failed)?;
        std::fs::rename(&temporary, file).map_err(failed)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use cos_bridge::ErrorKind;

    use super::*;

    /// A fresh folder under the system's temp dir, removed on drop.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("cos-desktop-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }

        pub(crate) fn file(&self) -> PathBuf {
            self.0.join("config/hosts.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn this_mac_is_always_there_and_first() {
        let hosts = Hosts::load(None);
        assert_eq!(
            hosts.list(),
            HostList {
                hosts: vec![Host::this_mac()],
                current: THIS_MAC.into()
            }
        );
        assert_eq!(hosts.current().target(), None);
        assert_eq!(
            serde_json::to_value(hosts.list()).unwrap(),
            serde_json::json!({"hosts": [{"id": "local", "name": "This Mac", "address": null}], "current": "local"})
        );
    }

    #[test]
    fn adds_selects_and_removes_and_saves_each_change() {
        let scratch = Scratch::new();
        let mut hosts = Hosts::load(Some(scratch.file()));
        let studio = hosts.add(" Studio ", "studio").unwrap();
        assert_eq!(
            studio,
            Host {
                id: "h1".into(),
                name: "Studio".into(),
                address: Some("studio:7700".into())
            }
        );
        let mini = hosts.add("Mini", "fd7a:115c:a1e0::5").unwrap();
        assert_eq!(mini.id, "h2");
        assert_eq!(mini.address.as_deref(), Some("[fd7a:115c:a1e0::5]:7700"));
        hosts.select("h2").unwrap();
        assert_eq!(
            hosts.current().target(),
            Some(Target::Tcp("[fd7a:115c:a1e0::5]:7700".into()))
        );

        // A new launch reads the same list and choice.
        let again = Hosts::load(Some(scratch.file()));
        assert_eq!(again.list(), hosts.list());
        assert_eq!(again.list().current, "h2");

        // Removing the host in use goes back to this Mac.
        assert!(hosts.remove("h2").unwrap());
        assert_eq!(hosts.list().current, THIS_MAC);
        assert!(!hosts.remove("h1").unwrap());
        assert_eq!(
            Hosts::load(Some(scratch.file())).list().hosts,
            vec![Host::this_mac()]
        );
        // Ids aren't reused while a later one exists, and start again after.
        assert_eq!(hosts.add("Studio", "studio").unwrap().id, "h1");
    }

    #[test]
    fn refuses_bad_names_addresses_and_ids() {
        let mut hosts = Hosts::load(None);
        for (name, address) in [
            ("", "studio"),
            ("   ", "studio"),
            ("Studio", ""),
            ("Studio", "http://studio:7700"),
            ("Studio", "studio:0"),
            (&"x".repeat(65), "studio"),
        ] {
            let error = hosts.add(name, address).unwrap_err();
            assert_eq!(error.kind, ErrorKind::BadRequest, "{name:?} {address:?}");
        }
        hosts.add("Studio", "studio:7700").unwrap();
        let error = hosts.add("Again", "studio").unwrap_err();
        assert!(
            error.message.contains("already saved as Studio"),
            "{}",
            error.message
        );
        assert_eq!(hosts.remove(THIS_MAC).unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(hosts.remove("h9").unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(hosts.select("h9").unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(hosts.select(THIS_MAC).unwrap(), Host::this_mac());
    }

    #[test]
    fn reads_the_file_leniently() {
        let scratch = Scratch::new();
        std::fs::create_dir_all(scratch.file().parent().unwrap()).unwrap();
        std::fs::write(scratch.file(), "not json").unwrap();
        assert_eq!(Hosts::load(Some(scratch.file())).list().hosts.len(), 1);

        std::fs::write(
            scratch.file(),
            serde_json::json!({
                "hosts": [
                    {"id": "h1", "name": "Studio", "address": "studio"},
                    {"id": "local", "name": "Fake", "address": "fake:7700"},
                    {"id": "h2", "name": "", "address": "blank:7700"},
                    {"id": "h3", "name": "Broken", "address": "http://x"},
                    {"id": "h4", "name": "No address", "address": null},
                    {"id": "h1", "name": "Duplicate", "address": "dup:7700"}
                ],
                "current": "h3"
            })
            .to_string(),
        )
        .unwrap();
        let hosts = Hosts::load(Some(scratch.file()));
        let list = hosts.list();
        assert_eq!(list.current, THIS_MAC, "a dropped host can't be current");
        assert_eq!(
            list.hosts.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            ["local", "h1"]
        );
        assert_eq!(list.hosts[1].address.as_deref(), Some("studio:7700"));
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let scratch = Scratch::new();
        // A file where the config folder should be.
        std::fs::create_dir_all(&scratch.0).unwrap();
        std::fs::write(scratch.0.join("config"), "").unwrap();
        let mut hosts = Hosts::load(Some(scratch.file()));
        let error = hosts.add("Studio", "studio").unwrap_err();
        assert_eq!(error.kind, ErrorKind::Io);
        assert_eq!(hosts.list().hosts.len(), 1);
    }
}
