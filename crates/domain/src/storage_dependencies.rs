//! Fresh Docker declarations. Physical path resolution is still required before effects.
use crate::{ContainerSnapshot, Error, ErrorCode, Result, StorageContract};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const CONTAINER_STORAGE_MAX_CONTAINERS: usize = 64;
pub const CONTAINER_STORAGE_MAX_MOUNTS: usize = 64;
pub const CONTAINER_STORAGE_MAX_TOTAL_MOUNTS: usize = 256;
pub const CONTAINER_STORAGE_MAX_AGE_SECONDS: i64 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContainerStorageSource {
    Bind {
        path: String,
    },
    Volume {
        path: String,
        name: String,
        driver: String,
    },
}
impl ContainerStorageSource {
    pub fn path(&self) -> &str {
        match self {
            Self::Bind { path } | Self::Volume { path, .. } => path,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerStorageMount {
    pub source: ContainerStorageSource,
    pub destination: String,
    pub writable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerStorageConsumer {
    pub container: ContainerSnapshot,
    pub mounts: Vec<ContainerStorageMount>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerStorageInventory {
    pub version: u16,
    pub engine_id: String,
    // Start of the collection, so the first inspection's age cannot be hidden.
    #[ts(type = "number")]
    pub observed_at: i64,
    pub containers: Vec<ContainerStorageConsumer>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerStorageInventoryView {
    pub inventory: ContainerStorageInventory,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeclaredContainerStorageDependency {
    pub device_id: String,
    pub filesystem_uuid: String,
    pub mountpoint: String,
    pub containers: Vec<String>,
}
pub fn container_storage_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 512
        && !path.chars().any(char::is_control)
        && (path == "/" || path[1..].split('/').all(|c| !matches!(c, "" | "." | "..")))
}
fn bounded_name(name: &str, maximum: usize) -> bool {
    !name.is_empty()
        && name.len() <= maximum
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}
impl ContainerStorageInventory {
    pub fn validate(&self, at: i64) -> Result<()> {
        if self.version != 1
            || self.engine_id.is_empty()
            || self.engine_id.len() > 128
            || !self
                .engine_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b":_-".contains(&c))
            || self.observed_at < 0
            || self.containers.len() > CONTAINER_STORAGE_MAX_CONTAINERS
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if at < self.observed_at
            || at.saturating_sub(self.observed_at) > CONTAINER_STORAGE_MAX_AGE_SECONDS
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let mut identities = BTreeSet::new();
        let mut total = 0usize;
        for consumer in &self.containers {
            consumer.container.validate()?;
            total += consumer.mounts.len();
            if !identities.insert(&consumer.container.resource)
                || consumer.mounts.len() > CONTAINER_STORAGE_MAX_MOUNTS
                || total > CONTAINER_STORAGE_MAX_TOTAL_MOUNTS
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
            let mut destinations = BTreeSet::new();
            for mount in &consumer.mounts {
                if !container_storage_path(mount.source.path())
                    || !container_storage_path(&mount.destination)
                    || !destinations.insert(&mount.destination)
                    || matches!(&mount.source, ContainerStorageSource::Volume { name, driver, .. }
                        if !bounded_name(name, 255) || !bounded_name(driver, 64))
                {
                    return Err(Error(ErrorCode::InvalidInput));
                }
            }
        }
        Ok(())
    }
}
fn beneath(path: &str, root: &str) -> bool {
    root == "/" || path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
/// Declared overlap only. This cannot establish absence of physical consumers:
/// aliases, symlinks, volumes, pools and nested mounts need protected resolution.
pub fn declared_container_storage_dependencies(
    contract: &StorageContract,
    inventory: &ContainerStorageInventory,
    at: i64,
) -> Result<Vec<DeclaredContainerStorageDependency>> {
    contract.validate()?;
    inventory.validate(at)?;
    let mut result = vec![];
    for device in &contract.devices {
        let containers: BTreeSet<_> = inventory
            .containers
            .iter()
            .filter(|consumer| {
                consumer.mounts.iter().any(|mount| {
                    beneath(mount.source.path(), &device.mountpoint)
                        || beneath(&device.mountpoint, mount.source.path())
                })
            })
            .map(|c| c.container.resource.clone())
            .collect();
        result.push(DeclaredContainerStorageDependency {
            device_id: device.id.clone(),
            filesystem_uuid: device.filesystem_uuid.clone(),
            mountpoint: device.mountpoint.clone(),
            containers: containers.into_iter().collect(),
        });
    }
    result.sort_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
    Ok(result)
}

#[cfg(test)]
mod tests;
