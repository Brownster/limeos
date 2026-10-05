//! Pure storage configuration and readiness planning. This is not live disk authority.
use crate::{Error, ErrorCode, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const STORAGE_MAX_DEVICES: usize = 32;
pub const STORAGE_MAX_WAIT_SECONDS: u16 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum StorageSchemaVersion {
    #[serde(rename = "1")]
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StorageProfile {
    SingleDisk,
    SeparateDownloads,
    ProtectedPool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StorageRole {
    Data,
    Downloads,
    Parity,
    ConfigBackup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StorageFilesystem {
    Btrfs,
    Exfat,
    Ext2,
    Ext3,
    Ext4,
    Ntfs,
    Vfat,
    Xfs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMediaIdentity {
    #[schemars(range(min = 1, max = 2147483647))]
    pub uid: u32,
    #[schemars(range(min = 1, max = 2147483647))]
    pub gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageLocations {
    #[schemars(length(max = 512))]
    pub media_host: String,
    #[schemars(length(max = 512))]
    pub downloads_host: String,
    #[schemars(length(max = 512))]
    pub application_config_host: String,
    #[schemars(length(max = 512))]
    pub backup_host: String,
    #[schemars(regex(pattern = "^/data/media$"))]
    #[ts(type = "\"/data/media\"")]
    pub media_container: String,
    #[schemars(regex(pattern = "^/data/downloads$"))]
    #[ts(type = "\"/data/downloads\"")]
    pub downloads_container: String,
    #[schemars(regex(pattern = "^/config$"))]
    #[ts(type = "\"/config\"")]
    pub config_container: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageDevice {
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[a-z0-9][a-z0-9_-]*$"))]
    pub id: String,
    pub role: StorageRole,
    #[schemars(
        length(min = 1, max = 128),
        regex(pattern = "^[A-Za-z0-9][A-Za-z0-9._-]*$")
    )]
    pub filesystem_uuid: String,
    pub filesystem: StorageFilesystem,
    #[schemars(length(max = 512))]
    pub mountpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    #[ts(optional = nullable)]
    pub serial: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageContract {
    pub schema_version: StorageSchemaVersion,
    pub profile: StorageProfile,
    pub media_identity: StorageMediaIdentity,
    pub locations: StorageLocations,
    #[schemars(length(min = 1, max = 32))]
    pub devices: Vec<StorageDevice>,
}

/// A requested wait on a filesystem assignment, never on its media subdirectories.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountWaitPlan {
    #[schemars(range(min = 1, max = 120))]
    pub timeout_seconds: u16,
    #[schemars(length(min = 1, max = 32))]
    pub devices: Vec<StorageDevice>,
}

fn valid(condition: bool) -> Result<()> {
    condition
        .then_some(())
        .ok_or(Error(ErrorCode::InvalidInput))
}

fn canonical_path(path: &str, under_mnt: bool) -> bool {
    path.starts_with('/')
        && path.len() <= 512
        && path.len() > 1
        && !path.chars().any(char::is_control)
        && path[1..]
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
        && (!under_mnt || path.starts_with("/mnt/"))
}

// Component relationships for configuration only. Live paths require descriptor
// traversal and actual mount identity; this comparison cannot authorize effects.
fn descendant(path: &str, root: &str, allow_root: bool) -> bool {
    (allow_root && path == root)
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

impl StorageContract {
    pub fn validate(&self) -> Result<()> {
        valid((1..=STORAGE_MAX_DEVICES).contains(&self.devices.len()))?;
        valid(
            (1..=i32::MAX as u32).contains(&self.media_identity.uid)
                && (1..=i32::MAX as u32).contains(&self.media_identity.gid),
        )?;
        let locations = &self.locations;
        valid(
            canonical_path(&locations.media_host, true)
                && canonical_path(&locations.downloads_host, true)
                && canonical_path(&locations.application_config_host, false)
                && canonical_path(&locations.backup_host, true)
                && locations.media_container == "/data/media"
                && locations.downloads_container == "/data/downloads"
                && locations.config_container == "/config",
        )?;
        let (mut ids, mut uuids, mut mountpoints) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for device in &self.devices {
            valid(
                !device.id.is_empty()
                    && device.id.len() <= 64
                    && (device.id.as_bytes()[0].is_ascii_lowercase()
                        || device.id.as_bytes()[0].is_ascii_digit())
                    && device.id.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c)
                    })
                    && ids.insert(&device.id),
            )?;
            valid(
                !device.filesystem_uuid.is_empty()
                    && device.filesystem_uuid.len() <= 128
                    && device.filesystem_uuid.as_bytes()[0].is_ascii_alphanumeric()
                    && device
                        .filesystem_uuid
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                    && uuids.insert(&device.filesystem_uuid),
            )?;
            valid(
                canonical_path(&device.mountpoint, true) && mountpoints.insert(&device.mountpoint),
            )?;
            valid(device.serial.as_ref().is_none_or(|serial| {
                !serial.is_empty()
                    && serial.len() <= 128
                    && serial.trim() == serial
                    && !serial.chars().any(char::is_control)
            }))?;
        }
        let role = |role| {
            self.devices
                .iter()
                .filter(move |device| device.role == role)
        };
        let data: Vec<_> = role(StorageRole::Data).collect();
        let downloads: Vec<_> = role(StorageRole::Downloads).collect();
        let parity: Vec<_> = role(StorageRole::Parity).collect();
        let backup: Vec<_> = role(StorageRole::ConfigBackup).collect();
        valid(backup.len() <= 1)?;
        match self.profile {
            StorageProfile::SingleDisk => {
                valid(data.len() == 1 && downloads.is_empty() && parity.is_empty())?;
                valid(
                    descendant(&locations.media_host, &data[0].mountpoint, false)
                        && descendant(&locations.downloads_host, &data[0].mountpoint, false),
                )?;
            }
            StorageProfile::SeparateDownloads => {
                valid(data.len() == 1 && downloads.len() == 1 && parity.is_empty())?;
                valid(
                    descendant(&locations.media_host, &data[0].mountpoint, true)
                        && descendant(&locations.downloads_host, &downloads[0].mountpoint, true),
                )?;
            }
            StorageProfile::ProtectedPool => {
                valid(!data.is_empty() && !parity.is_empty() && downloads.len() <= 1)?;
                valid(descendant(&locations.media_host, "/mnt/storage", false))?;
                valid(if let Some(downloads) = downloads.first() {
                    descendant(&locations.downloads_host, &downloads.mountpoint, true)
                } else {
                    descendant(&locations.downloads_host, "/mnt/storage", false)
                })?;
            }
        }
        if let Some(backup) = backup.first() {
            valid(descendant(
                &locations.backup_host,
                &backup.mountpoint,
                false,
            ))?;
        }
        Ok(())
    }

    pub fn mount_wait_plan(&self, timeout_seconds: u16) -> Result<StorageMountWaitPlan> {
        self.validate()?;
        valid((1..=STORAGE_MAX_WAIT_SECONDS).contains(&timeout_seconds))?;
        if self.profile == StorageProfile::ProtectedPool {
            valid(
                self.devices
                    .iter()
                    .all(|device| !descendant(&device.mountpoint, "/mnt/storage", true)),
            )?;
        }
        let mut devices = self.devices.clone();
        devices.sort_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
        let plan = StorageMountWaitPlan {
            timeout_seconds,
            devices,
        };
        plan.validate()?;
        Ok(plan)
    }
}

impl StorageMountWaitPlan {
    /// Executor input is validated independently, including when no contract
    /// generator produced it. Profile role counts do not apply to a wait subset.
    pub fn validate(&self) -> Result<()> {
        valid((1..=STORAGE_MAX_WAIT_SECONDS).contains(&self.timeout_seconds))?;
        valid((1..=STORAGE_MAX_DEVICES).contains(&self.devices.len()))?;
        let (mut ids, mut uuids, mut mounts) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for d in &self.devices {
            valid(
                !d.id.is_empty()
                    && d.id.len() <= 64
                    && d.id.as_bytes()[0].is_ascii_alphanumeric()
                    && d.id.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c)
                    })
                    && ids.insert(&d.id),
            )?;
            valid(
                !d.filesystem_uuid.is_empty()
                    && d.filesystem_uuid.len() <= 128
                    && d.filesystem_uuid.as_bytes()[0].is_ascii_alphanumeric()
                    && d.filesystem_uuid
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                    && uuids.insert(&d.filesystem_uuid),
            )?;
            valid(canonical_path(&d.mountpoint, true) && mounts.insert(&d.mountpoint))?;
            valid(d.serial.as_ref().is_none_or(|s| {
                !s.is_empty() && s.len() <= 128 && s.trim() == s && !s.chars().any(char::is_control)
            }))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
