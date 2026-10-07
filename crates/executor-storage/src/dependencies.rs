//! Read-only UUID/backing binding. Running namespaces and effects remain gated.
use crate::{
    ContainerSourceEvidence, ContainerSourceSnapshot, DeviceNumber, Failure, Result,
    StorageInventoryEvidence, filesystem_matches, inspect_container_sources,
    inspect_storage_inventory, inventory::digest, mounts::Mount, topology::Topology,
};
use limeos_domain::{ContainerStorageInventory, StorageContract, StorageInventory};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

const MAX_MATCHES: usize = 512;
const MAX_REPORT_BYTES: usize = 48 * 1024;
const WORK_SECONDS: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerFilesystemRelation {
    SameFilesystem,
    SharedBacking,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerFilesystemMatch {
    pub mount_id: u64,
    pub filesystem_uuid: String,
    pub relation: ContainerFilesystemRelation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerFilesystemConsumer {
    pub container: String,
    pub destination: String,
    pub matches: Vec<ContainerFilesystemMatch>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PhysicalContainerStorageDependency {
    pub device_id: String,
    pub filesystem_uuid: String,
    pub mountpoint: String,
    pub consumers: Vec<ContainerFilesystemConsumer>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerDependencySnapshot {
    pub version: u16,
    pub contract_digest: String,
    pub container_inventory_digest: String,
    pub storage_inventory_digest: String,
    pub source_snapshot_digest: String,
    pub host_root_mount_id: u64,
    /// Full raw mount table; the wire storage inventory uses a selected-row digest.
    pub mounts_digest: String,
    pub dependencies: Vec<PhysicalContainerStorageDependency>,
}

/// Private ownership, rather than a serialized report, retains probe and path facts.
pub struct ContainerDependencyEvidence {
    snapshot: ContainerDependencySnapshot,
    sources: ContainerSourceEvidence,
    storage: StorageInventoryEvidence,
}
impl ContainerDependencyEvidence {
    pub fn snapshot(&self) -> &ContainerDependencySnapshot {
        &self.snapshot
    }
    /// Rechecks retained host evidence. Does not reacquire Engine declarations or
    /// inspect running namespaces, and never authorizes mount/unmount effects.
    pub async fn revalidate(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(WORK_SECONDS);
        tokio::time::timeout(Duration::from_secs(WORK_SECONDS), async {
            self.sources.revalidate()?;
            self.storage.revalidate().await?;
            self.sources.revalidate()?;
            check_deadline(deadline)
        })
        .await
        .map_err(|_| Failure::TimedOut)?
    }
}

/// Requires a protected host reader. Later service integration must enforce
/// single-flight process admission and independently bound synchronous work.
pub async fn inspect_container_dependencies(
    contract: &StorageContract,
    containers: &ContainerStorageInventory,
) -> Result<ContainerDependencyEvidence> {
    contract.validate().map_err(|_| Failure::InvalidPlan)?;
    containers.validate(crate::sources::now()?).map_err(|e| {
        if e.0 == limeos_domain::ErrorCode::Conflict {
            Failure::Conflict
        } else {
            Failure::InvalidPlan
        }
    })?;
    let deadline = Instant::now() + Duration::from_secs(WORK_SECONDS);
    tokio::time::timeout(Duration::from_secs(WORK_SECONDS), async {
        let storage = inspect_storage_inventory().await?;
        check_deadline(deadline)?;
        let sources = inspect_container_sources(containers)?;
        let snapshot = bind(
            contract,
            storage.snapshot(),
            &storage.topology,
            &storage.table,
            &storage.raw_mounts_digest,
            sources.snapshot(),
        )?;
        let evidence = ContainerDependencyEvidence {
            snapshot,
            sources,
            storage,
        };
        evidence.revalidate().await?;
        check_deadline(deadline)?;
        Ok(evidence)
    })
    .await
    .map_err(|_| Failure::TimedOut)?
}

fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Failure::TimedOut)
    } else {
        Ok(())
    }
}

fn number(d: limeos_domain::StorageBlockDevice) -> DeviceNumber {
    DeviceNumber {
        major: d.major,
        minor: d.minor,
    }
}
fn mounted_filesystem_matches(probed: &str, mounted: &str) -> bool {
    matches!(mounted, "ext2" | "ext3" | "ext4" | "xfs" | "vfat" | "exfat") && mounted == probed
        || mounted == "ntfs3" && matches!(probed, "ntfs" | "ntfs3")
}

// This helper is private: only the retained collectors supply its observations.
fn bind(
    contract: &StorageContract,
    storage: &StorageInventory,
    topology: &Topology,
    table: &[Mount],
    raw_mounts_digest: &str,
    sources: &ContainerSourceSnapshot,
) -> Result<ContainerDependencySnapshot> {
    contract.validate().map_err(|_| Failure::InvalidPlan)?;
    storage.validate().map_err(|_| Failure::Unavailable)?;
    if sources.host_root_mount_id != storage.host_root_mount_id
        || sources.mounts_digest != raw_mounts_digest
        || digest(&table)? != storage.mounts_digest
    {
        return Err(Failure::IdentityMismatch);
    }
    let mut by_number = BTreeMap::new();
    let mut by_uuid = BTreeMap::new();
    for d in &storage.devices {
        let n = number(d.device);
        if !topology.0.contains_key(&n) || by_number.insert(n, d).is_some() {
            return Err(Failure::Unavailable);
        }
        if by_uuid.insert(&d.filesystem_uuid, d).is_some() {
            return Err(Failure::AmbiguousIdentity);
        }
    }
    let mut source_filesystems = BTreeMap::new();
    for mount in &sources.mounts {
        let actual = table
            .iter()
            .find(|m| m.id == mount.mount_id)
            .ok_or(Failure::IdentityMismatch)?;
        if actual.device != mount.device
            || actual.root != mount.filesystem_root
            || actual.path != mount.mountpoint
            || actual.filesystem != mount.filesystem
            || source_filesystems.contains_key(&mount.mount_id)
        {
            return Err(Failure::IdentityMismatch);
        }
        let fs = if mount.filesystem == "tmpfs" && mount.device.major == 0 {
            None
        } else {
            let d = by_number.get(&mount.device).ok_or(Failure::Unavailable)?;
            if !mounted_filesystem_matches(&d.filesystem, &mount.filesystem) {
                return Err(Failure::Unavailable);
            }
            Some(*d)
        };
        source_filesystems.insert(mount.mount_id, fs);
    }

    let mut dependencies = Vec::new();
    let mut total = 0usize;
    for assignment in &contract.devices {
        let actual = by_uuid
            .get(&assignment.filesystem_uuid)
            .ok_or(Failure::NotReady)?;
        if actual.boot_backing {
            return Err(Failure::BootDevice);
        }
        if actual.in_use_as_swap {
            return Err(Failure::ActiveSwap);
        }
        if !filesystem_matches(assignment.filesystem, &actual.filesystem)
            || assignment
                .serial
                .as_ref()
                .is_some_and(|s| actual.serial.as_ref() != Some(s))
        {
            return Err(Failure::IdentityMismatch);
        }
        let assigned = number(actual.device);
        let connected = topology.connected(&BTreeSet::from([assigned]))?;
        let mut consumers = Vec::new();
        for source in &sources.sources {
            if !source
                .potential_mount_ids
                .contains(&source.identity.mount_id)
            {
                return Err(Failure::IdentityMismatch);
            }
            let identity_mount = sources
                .mounts
                .iter()
                .find(|m| m.mount_id == source.identity.mount_id)
                .ok_or(Failure::IdentityMismatch)?;
            if identity_mount.device != source.identity.device {
                return Err(Failure::IdentityMismatch);
            }
            let mut seen = BTreeSet::new();
            let mut matches = Vec::new();
            for id in &source.potential_mount_ids {
                if !seen.insert(id) {
                    return Err(Failure::IdentityMismatch);
                }
                let Some(fs) = source_filesystems.get(id).ok_or(Failure::Unavailable)? else {
                    continue;
                };
                let n = number(fs.device);
                if connected.contains(&n) {
                    total += 1;
                    if total > MAX_MATCHES {
                        return Err(Failure::Unavailable);
                    }
                    matches.push(ContainerFilesystemMatch {
                        mount_id: *id,
                        filesystem_uuid: fs.filesystem_uuid.clone(),
                        relation: if n == assigned {
                            ContainerFilesystemRelation::SameFilesystem
                        } else {
                            ContainerFilesystemRelation::SharedBacking
                        },
                    });
                }
            }
            matches.sort_by_key(|m| m.mount_id);
            if !matches.is_empty() {
                consumers.push(ContainerFilesystemConsumer {
                    container: source.container.clone(),
                    destination: source.destination.clone(),
                    matches,
                });
            }
        }
        consumers
            .sort_by(|a, b| (&a.container, &a.destination).cmp(&(&b.container, &b.destination)));
        dependencies.push(PhysicalContainerStorageDependency {
            device_id: assignment.id.clone(),
            filesystem_uuid: assignment.filesystem_uuid.clone(),
            mountpoint: assignment.mountpoint.clone(),
            consumers,
        });
    }
    dependencies.sort_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
    let result = ContainerDependencySnapshot {
        version: 1,
        contract_digest: digest(contract)?,
        container_inventory_digest: sources.inventory_digest.clone(),
        storage_inventory_digest: digest(storage)?,
        source_snapshot_digest: digest(sources)?,
        host_root_mount_id: storage.host_root_mount_id,
        mounts_digest: raw_mounts_digest.into(),
        dependencies,
    };
    if serde_json::to_vec(&result)
        .map_err(|_| Failure::Unavailable)?
        .len()
        > MAX_REPORT_BYTES
    {
        return Err(Failure::Unavailable);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
