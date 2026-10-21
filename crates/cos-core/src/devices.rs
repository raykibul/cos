//! Devices: other machines on your tailnet that have reached the daemon over
//! its tailnet listener. The first visit records a device as pending; it gets
//! in only once you approve it on the host. See "Remote access" in
//! `docs/daemon.md`.

use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::events::kind;
use crate::{EventLog, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: i64,
    /// Tailscale's stable node ID. Survives renames and IP changes.
    pub node_id: String,
    /// The MagicDNS name, such as `my-iphone`.
    pub name: String,
    /// What the device calls itself, such as `Rakib's iPhone`.
    pub hostname: String,
    /// `iOS`, `macOS`, `linux`, … Empty if Tailscale didn't say.
    pub os: String,
    /// The Tailscale user the device was logged in as when last seen.
    pub login_name: String,
    pub state: DeviceState,
    /// The tailnet address it last connected from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ip: Option<String>,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// When it was last approved or denied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    /// Seen, not yet approved. Its requests are refused.
    Pending,
    Approved,
    /// Refused until approved.
    Denied,
}

impl DeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

/// What Tailscale says about a device that just connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenDevice {
    pub node_id: String,
    pub name: String,
    pub hostname: String,
    pub os: String,
    pub login_name: String,
    pub ip: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for DeviceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error.into())
    }
}

pub struct Devices {
    store: Arc<Store>,
    log: EventLog,
}

impl Devices {
    pub fn new(store: Arc<Store>, log: EventLog) -> Self {
        Self { store, log }
    }

    /// Records a visit. A device seen for the first time is stored as
    /// pending and logged as `device.requested`; a known one gets its name,
    /// OS, login and address refreshed. Its state is never changed here.
    pub fn seen(&self, seen: &SeenDevice) -> Result<Device, DeviceError> {
        let (device, created) = self.store.upsert_device(seen)?;
        if created {
            self.log.append(
                kind::DEVICE_REQUESTED,
                None,
                json!({
                    "device_id": device.id,
                    "node_id": device.node_id,
                    "name": device.name,
                    "hostname": device.hostname,
                    "os": device.os,
                }),
            )?;
        }
        Ok(device)
    }

    /// All devices, oldest first.
    pub fn list(&self) -> Result<Vec<Device>, DeviceError> {
        Ok(self.store.devices()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Device>, DeviceError> {
        Ok(self.store.device(id)?)
    }

    /// Lets the device in. Approving an approved device changes nothing.
    pub fn approve(&self, id: i64) -> Result<Device, DeviceError> {
        self.decide(id, DeviceState::Approved, kind::DEVICE_APPROVED)
    }

    /// Shuts the device out until it is approved.
    pub fn deny(&self, id: i64) -> Result<Device, DeviceError> {
        self.decide(id, DeviceState::Denied, kind::DEVICE_DENIED)
    }

    /// Forgets the device. If it connects again it starts over as pending.
    pub fn remove(&self, id: i64) -> Result<(), DeviceError> {
        let device = self.existing(id)?;
        self.store.delete_device(id)?;
        self.log.append(
            kind::DEVICE_REMOVED,
            None,
            json!({ "device_id": id, "node_id": device.node_id, "name": device.name }),
        )?;
        Ok(())
    }

    fn decide(&self, id: i64, state: DeviceState, event: &str) -> Result<Device, DeviceError> {
        let device = self.existing(id)?;
        if device.state == state {
            return Ok(device);
        }
        self.store.set_device_state(id, state)?;
        self.log.append(
            event,
            None,
            json!({ "device_id": id, "node_id": device.node_id, "name": device.name }),
        )?;
        self.existing(id)
    }

    fn existing(&self, id: i64) -> Result<Device, DeviceError> {
        self.store
            .device(id)?
            .ok_or_else(|| DeviceError::NotFound(format!("there is no device {id}")))
    }
}

// ---- Storage ----

const DEVICE_COLUMNS: &str = "id, node_id, name, hostname, os, login_name, state, last_ip, \
                              first_seen_ms, last_seen_ms, decided_ms";

impl Store {
    /// Inserts a pending device or refreshes a known one, in one locked step
    /// so two connections from a new device can't both insert it. Returns the
    /// device and whether it is new.
    fn upsert_device(&self, seen: &SeenDevice) -> Result<(Device, bool), StoreError> {
        let conn = self.lock();
        let now = now_ms();
        let updated = conn.execute(
            "UPDATE devices SET name = ?2, hostname = ?3, os = ?4, login_name = ?5, last_ip = ?6,
                                last_seen_ms = ?7
             WHERE node_id = ?1",
            params![
                seen.node_id,
                seen.name,
                seen.hostname,
                seen.os,
                seen.login_name,
                seen.ip,
                now
            ],
        )?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO devices (node_id, name, hostname, os, login_name, state, last_ip,
                                      first_seen_ms, last_seen_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?7)",
                params![
                    seen.node_id,
                    seen.name,
                    seen.hostname,
                    seen.os,
                    seen.login_name,
                    seen.ip,
                    now
                ],
            )?;
        }
        let device = conn
            .prepare_cached(&format!(
                "SELECT {DEVICE_COLUMNS} FROM devices WHERE node_id = ?1"
            ))?
            .query_row([&seen.node_id], read_device)?;
        Ok((device, updated == 0))
    }

    fn devices(&self) -> Result<Vec<Device>, StoreError> {
        Ok(self
            .lock()
            .prepare_cached(&format!("SELECT {DEVICE_COLUMNS} FROM devices ORDER BY id"))?
            .query_map([], read_device)?
            .collect::<Result<_, _>>()?)
    }

    fn device(&self, id: i64) -> Result<Option<Device>, StoreError> {
        Ok(self
            .lock()
            .prepare_cached(&format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?1"))?
            .query_row([id], read_device)
            .optional()?)
    }

    fn set_device_state(&self, id: i64, state: DeviceState) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE devices SET state = ?2, decided_ms = ?3 WHERE id = ?1",
            params![id, state.as_str(), now_ms()],
        )?;
        Ok(())
    }

    fn delete_device(&self, id: i64) -> Result<(), StoreError> {
        self.lock().execute("DELETE FROM devices WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn read_device(row: &Row<'_>) -> rusqlite::Result<Device> {
    let state = match row.get_ref(6)?.as_str()? {
        "pending" => DeviceState::Pending,
        "approved" => DeviceState::Approved,
        "denied" => DeviceState::Denied,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                6,
                Type::Text,
                format!("unknown device state {other:?}").into(),
            ));
        }
    };
    Ok(Device {
        id: row.get(0)?,
        node_id: row.get(1)?,
        name: row.get(2)?,
        hostname: row.get(3)?,
        os: row.get(4)?,
        login_name: row.get(5)?,
        state,
        last_ip: row.get(7)?,
        first_seen_at: timestamp(row.get(8)?)?,
        last_seen_at: timestamp(row.get(9)?)?,
        decided_at: row.get::<_, Option<i64>>(10)?.map(timestamp).transpose()?,
    })
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

fn timestamp(ms: i64) -> rusqlite::Result<Timestamp> {
    Timestamp::from_millisecond(ms)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> (Devices, EventLog) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        (Devices::new(store, log.clone()), log)
    }

    fn phone() -> SeenDevice {
        SeenDevice {
            node_id: "nPHONE1CNTRL".into(),
            name: "phone".into(),
            hostname: "My Phone".into(),
            os: "iOS".into(),
            login_name: "me@example.com".into(),
            ip: "100.100.2.1".into(),
        }
    }

    fn kinds(log: &EventLog) -> Vec<String> {
        log.after(0, 100)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }

    #[test]
    fn a_new_device_starts_pending_and_is_logged_once() {
        let (devices, log) = devices();

        let first = devices.seen(&phone()).unwrap();
        let again = devices
            .seen(&SeenDevice {
                hostname: "Renamed Phone".into(),
                ip: "100.100.2.9".into(),
                ..phone()
            })
            .unwrap();

        assert_eq!(first.state, DeviceState::Pending);
        assert_eq!(again.id, first.id);
        assert_eq!(again.state, DeviceState::Pending);
        assert_eq!(again.hostname, "Renamed Phone");
        assert_eq!(again.last_ip.as_deref(), Some("100.100.2.9"));
        assert_eq!(again.first_seen_at, first.first_seen_at);
        assert_eq!(kinds(&log), [kind::DEVICE_REQUESTED]);
        let requested = &log.after(0, 1).unwrap()[0];
        assert_eq!(requested.data["node_id"], "nPHONE1CNTRL");
        assert_eq!(requested.data["os"], "iOS");
    }

    #[test]
    fn approve_deny_and_remove() {
        let (devices, log) = devices();
        let id = devices.seen(&phone()).unwrap().id;

        let approved = devices.approve(id).unwrap();
        assert_eq!(approved.state, DeviceState::Approved);
        assert!(approved.decided_at.is_some());
        // A visit doesn't change the decision.
        assert_eq!(devices.seen(&phone()).unwrap().state, DeviceState::Approved);
        // Approving twice logs once.
        devices.approve(id).unwrap();

        assert_eq!(devices.deny(id).unwrap().state, DeviceState::Denied);
        assert_eq!(devices.approve(id).unwrap().state, DeviceState::Approved);

        devices.remove(id).unwrap();
        assert!(devices.get(id).unwrap().is_none());
        assert!(devices.list().unwrap().is_empty());

        // It comes back as a new, pending device.
        let back = devices.seen(&phone()).unwrap();
        assert_ne!(back.id, id);
        assert_eq!(back.state, DeviceState::Pending);

        assert_eq!(
            kinds(&log),
            [
                kind::DEVICE_REQUESTED,
                kind::DEVICE_APPROVED,
                kind::DEVICE_DENIED,
                kind::DEVICE_APPROVED,
                kind::DEVICE_REMOVED,
                kind::DEVICE_REQUESTED,
            ]
        );
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let (devices, _) = devices();

        for result in [devices.approve(7), devices.deny(7)] {
            assert!(matches!(result, Err(DeviceError::NotFound(_))));
        }
        assert!(matches!(devices.remove(7), Err(DeviceError::NotFound(_))));
    }

    #[test]
    fn device_json_shape() {
        let (devices, _) = devices();
        let device = devices.seen(&phone()).unwrap();

        let value = serde_json::to_value(&device).unwrap();

        assert_eq!(value["state"], "pending");
        assert_eq!(value["node_id"], "nPHONE1CNTRL");
        assert!(value.get("decided_at").is_none());
        assert_eq!(serde_json::from_value::<Device>(value).unwrap(), device);
    }
}
