//! Guided assignment previews. No filesystem formatting or executable intent.
use super::*;
use crate::{PLAN_TTL_SECONDS, identifier, opaque_id};

pub const STORAGE_PLAN_VERSION: u16 = 1;
pub const STORAGE_FSTAB_BEGIN: &str = "# BEGIN LIMEOS STORAGE V1";
pub const STORAGE_FSTAB_END: &str = "# END LIMEOS STORAGE V1";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(deny_unknown_fields)]
pub struct StorageBlockDevice {
    pub major: u32,
    pub minor: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageObservedMount {
    pub mountpoint: String,
    #[ts(type = "number")]
    pub mount_id: u64,
    pub filesystem_root: String,
    pub filesystem: String,
    pub writable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageObservedDevice {
    pub device: StorageBlockDevice,
    pub filesystem_uuid: String,
    pub filesystem: String,
    pub serial: Option<String>,
    pub boot_backing: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub in_use_as_swap: bool,
    pub mounts: Vec<StorageObservedMount>,
}
/// Only identity/target fields leave the protected reader. Fstab options may
/// contain credentials and are never copied into RPC, plans or HTTP responses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageFstabEntry {
    pub filesystem_uuid: Option<String>,
    pub device: Option<StorageBlockDevice>,
    pub mountpoint: String,
    pub managed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageInventory {
    #[ts(type = "number")]
    pub host_root_mount_id: u64,
    pub topology_digest: String,
    pub mounts_digest: String,
    pub fstab_digest: String,
    pub fstab_entries: Vec<StorageFstabEntry>,
    pub devices: Vec<StorageObservedDevice>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageInventoryView {
    pub inventory: StorageInventory,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageSetupInput {
    pub contract: StorageContract,
    pub inventory_digest: String,
    pub timeout_seconds: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageSetupPlan {
    pub id: String,
    pub version: u16,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub expected: StorageInventory,
    pub desired: StorageContract,
    pub readiness: StorageMountWaitPlan,
    // Generated owned section only. Unmanaged fstab bytes are never rewritten.
    pub managed_fstab: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedStorageSetup {
    pub plan: StorageSetupPlan,
    pub digest: String,
}

impl StorageInventory {
    pub fn validate(&self) -> Result<()> {
        valid(self.host_root_mount_id > 0 && self.host_root_mount_id < (1u64 << 53))?;
        valid(
            [
                &self.topology_digest,
                &self.mounts_digest,
                &self.fstab_digest,
            ]
            .iter()
            .all(|s| opaque_id(s)),
        )?;
        valid(self.devices.len() <= 128 && self.fstab_entries.len() <= 128)?;
        let mut numbers = BTreeSet::new();
        for d in &self.devices {
            valid(numbers.insert(d.device) && d.device.major > 0)?;
            valid(
                !d.filesystem_uuid.is_empty()
                    && d.filesystem_uuid.len() <= 128
                    && d.filesystem_uuid
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            )?;
            valid(
                !d.filesystem.is_empty()
                    && d.filesystem.len() <= 64
                    && d.filesystem
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
            )?;
            valid(d.serial.as_ref().is_none_or(|s| {
                !s.is_empty() && s.len() <= 128 && s.trim() == s && !s.chars().any(char::is_control)
            }))?;
            valid(d.mounts.len() <= 32)?;
            for m in &d.mounts {
                valid(
                    !m.filesystem.is_empty()
                        && m.filesystem.len() <= 64
                        && m.filesystem
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
                )?;
                valid(
                    m.mount_id > 0
                        && canonical_path_or_root(&m.mountpoint)
                        && canonical_path_or_root(&m.filesystem_root),
                )?;
            }
        }
        for entry in &self.fstab_entries {
            valid(canonical_path_or_root(&entry.mountpoint))?;
            valid(entry.filesystem_uuid.as_ref().is_none_or(|u| {
                !u.is_empty()
                    && u.len() <= 128
                    && u.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            }))?;
        }
        Ok(())
    }
}
fn canonical_path_or_root(path: &str) -> bool {
    path == "/" || canonical_path(path, false)
}
fn overlaps(a: &str, b: &str) -> bool {
    descendant(a, b, true) || descendant(b, a, true)
}

impl StorageSetupInput {
    pub fn validate(&self) -> Result<()> {
        valid(opaque_id(&self.inventory_digest))?;
        validate_assignments(&self.contract, self.timeout_seconds)?;
        Ok(())
    }
}
fn validate_assignments(contract: &StorageContract, timeout: u16) -> Result<StorageMountWaitPlan> {
    let readiness = contract.mount_wait_plan(timeout)?;
    // Nested physical mounts make readiness and dependency ownership ambiguous.
    for (i, a) in contract.devices.iter().enumerate() {
        valid(
            contract
                .devices
                .iter()
                .skip(i + 1)
                .all(|b| !overlaps(&a.mountpoint, &b.mountpoint)),
        )?;
    }
    Ok(readiness)
}
/// Deterministic managed fstab rendering. Callers cannot supply options or units.
/// Btrfs and removable ownership need additional adapter qualification first.
pub fn storage_managed_fstab(
    contract: &StorageContract,
    timeout: u16,
    inventory: &StorageInventory,
) -> Result<String> {
    validate_assignments(contract, timeout)?;
    inventory.validate()?;
    let mut devices = contract.devices.clone();
    devices.sort_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
    let mut text = format!("{STORAGE_FSTAB_BEGIN}\n");
    for d in &devices {
        let matches: Vec<_> = inventory
            .devices
            .iter()
            .filter(|a| a.filesystem_uuid == d.filesystem_uuid)
            .collect();
        if matches.len() != 1 {
            return Err(Error(ErrorCode::Conflict));
        }
        let actual = matches[0];
        let (kind, pass) = match d.filesystem {
            StorageFilesystem::Ext2 => ("ext2", 2),
            StorageFilesystem::Ext3 => ("ext3", 2),
            StorageFilesystem::Ext4 => ("ext4", 2),
            StorageFilesystem::Xfs => ("xfs", 0),
            _ => return Err(Error(ErrorCode::Unavailable)),
        };
        if actual.boot_backing
            || actual.in_use_as_swap
            || actual.filesystem != kind
            || d.serial
                .as_ref()
                .is_some_and(|s| actual.serial.as_ref() != Some(s))
            || actual.mounts.len() > 1
            || actual.mounts.iter().any(|m| {
                m.mountpoint != d.mountpoint
                    || m.filesystem_root != "/"
                    || m.filesystem != kind
                    || !m.writable
            })
        {
            return Err(Error(ErrorCode::Conflict));
        }
        if inventory
            .fstab_entries
            .iter()
            .filter(|e| !e.managed)
            .any(|e| {
                e.filesystem_uuid.as_ref() == Some(&d.filesystem_uuid)
                    || e.device == Some(actual.device)
                    || overlaps(&d.mountpoint, &e.mountpoint)
                        && (e.mountpoint == "/mnt" || e.mountpoint.starts_with("/mnt/"))
            })
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let target = d.mountpoint.replace('\\', "\\134").replace(' ', "\\040");
        text.push_str(&format!("UUID={} {} {} defaults,nofail,nodev,nosuid,x-systemd.device-timeout={}s,x-systemd.mount-timeout={}s 0 {}\n", d.filesystem_uuid, target, kind, timeout, timeout, pass));
    }
    text.push_str(&format!("{STORAGE_FSTAB_END}\n"));
    Ok(text)
}
impl StorageSetupPlan {
    pub fn validate(&self, now: i64) -> Result<()> {
        valid(
            self.version == STORAGE_PLAN_VERSION
                && opaque_id(&self.id)
                && identifier(&self.principal)
                && self.grant_revision > 0
                && self.created_at >= 0
                && self.expires_at > self.created_at
                && self.expires_at <= self.created_at.saturating_add(PLAN_TTL_SECONDS),
        )?;
        valid(
            self.readiness == validate_assignments(&self.desired, self.readiness.timeout_seconds)?,
        )?;
        valid(
            self.managed_fstab
                == storage_managed_fstab(
                    &self.desired,
                    self.readiness.timeout_seconds,
                    &self.expected,
                )?,
        )?;
        if now < self.created_at || now >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
    pub fn check_current(&self, actual: &StorageInventory, now: i64) -> Result<()> {
        self.validate(now)?;
        actual.validate()?;
        if &self.expected != actual {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
