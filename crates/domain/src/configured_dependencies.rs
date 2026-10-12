//! Pure configured-path propagation for previews, never physical or effect authority.
//!
//! Pool declarations expand to every configured member. They cannot resolve
//! aliases, symlinks, running namespaces, nested mounts or block backing. Shares
//! are an additional missing input; an empty row never proves safe unmounting.

use crate::{
    ContainerSnapshot, ContainerStorageInventory, ContainerStorageSource, ErrorCode, PoolsConfig,
    ProtectionPathRequirement, SnapraidConfig, StorageContract, StorageDevice,
    StorageMediaIdentity, StoragePlanningFindingCode, StoragePlanningResult, pools::finding,
};
use serde::Serialize;

pub const MAX_CONFIGURED_CONTAINER_MATCHES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredPoolConsumer {
    pub pool_id: String,
    pub mountpoint: String,
    pub enabled: bool,
    /// Preserve configured branch order, even when multiple branches use one disk.
    pub branches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredProtectionConsumer {
    pub drive_id: String,
    pub enabled: bool,
    /// The existing planner's ordered source/data/content/parity requirements.
    pub requirements: Vec<ProtectionPathRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfiguredContainerRoute {
    Direct,
    Pool { pool_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredContainerConsumer {
    pub container: ContainerSnapshot,
    pub source: ContainerStorageSource,
    pub destination: String,
    pub writable: bool,
    pub via: ConfiguredContainerRoute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredDeviceDependency {
    pub expected: StorageDevice,
    pub pools: Vec<ConfiguredPoolConsumer>,
    pub protection: Vec<ConfiguredProtectionConsumer>,
    pub containers: Vec<ConfiguredContainerConsumer>,
}

/// A declared-only view. No retained descriptor, lock, claim or approval exists here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredStorageDependencyPreview {
    pub version: u16,
    pub media_identity: StorageMediaIdentity,
    pub engine_id: String,
    /// Original Docker collection time; the planner does not refresh it.
    pub containers_observed_at: i64,
    pub devices: Vec<ConfiguredDeviceDependency>,
}

fn beneath(path: &str, root: &str) -> bool {
    root == "/" || path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}

fn overlaps(a: &str, b: &str) -> bool {
    beneath(a, b) || beneath(b, a)
}

/// Validate all inputs and return all configured relations, or refuse the request.
///
/// Disabled configuration and stopped containers remain possible consumers.
/// A declared source above a pool root conservatively expands to every member.
/// Every source/destination and route remains distinct; results are never clipped.
/// `at` validates the original Docker age only, not a live collection lifetime.
pub fn configured_storage_dependencies(
    contract: &StorageContract,
    pools: &PoolsConfig,
    protection: &SnapraidConfig,
    containers: &ContainerStorageInventory,
    at: i64,
) -> StoragePlanningResult<ConfiguredStorageDependencyPreview> {
    containers.validate(at).map_err(|e| {
        vec![finding(
            "containers",
            if e.0 == ErrorCode::Conflict {
                StoragePlanningFindingCode::StaleEvidence
            } else {
                StoragePlanningFindingCode::InvalidConfiguration
            },
        )]
    })?;
    let pool_plan = pools.plan(contract)?;
    let protection_plan = protection.plan(pools, contract)?;
    let mut count = 0usize;
    let mut devices = Vec::with_capacity(contract.devices.len());
    for device in &contract.devices {
        let mut configured_pools = Vec::new();
        for pool in &pools.pools {
            let dependency = pool_plan
                .dependencies
                .iter()
                .find(|d| d.pool_id == pool.id)
                .ok_or_else(|| {
                    vec![finding(
                        "pools",
                        StoragePlanningFindingCode::InvalidConfiguration,
                    )]
                })?;
            if dependency.devices.contains(device) {
                configured_pools.push(ConfiguredPoolConsumer {
                    pool_id: pool.id.clone(),
                    mountpoint: pool.mount_point.clone(),
                    enabled: pool.enabled,
                    branches: pool
                        .branches
                        .iter()
                        .filter(|b| beneath(b, &device.mountpoint))
                        .cloned()
                        .collect(),
                });
            }
        }
        configured_pools.sort_by(|a, b| a.pool_id.cmp(&b.pool_id));
        let mut configured_protection = Vec::new();
        for drive in &protection.drives {
            if drive.uuid == device.filesystem_uuid {
                configured_protection.push(ConfiguredProtectionConsumer {
                    drive_id: drive.id.clone(),
                    enabled: protection.enabled,
                    requirements: protection_plan
                        .requirements
                        .iter()
                        .filter(|r| r.expected == *device)
                        .cloned()
                        .collect(),
                });
            }
        }
        configured_protection.sort_by(|a, b| a.drive_id.cmp(&b.drive_id));
        let mut matches = Vec::new();
        for consumer in &containers.containers {
            for mount in &consumer.mounts {
                let direct = overlaps(mount.source.path(), &device.mountpoint);
                let routes = direct
                    .then_some(ConfiguredContainerRoute::Direct)
                    .into_iter()
                    .chain(
                        configured_pools
                            .iter()
                            .filter(|pool| overlaps(mount.source.path(), &pool.mountpoint))
                            .map(|pool| ConfiguredContainerRoute::Pool {
                                pool_id: pool.pool_id.clone(),
                            }),
                    );
                for via in routes {
                    if count == MAX_CONFIGURED_CONTAINER_MATCHES {
                        return Err(vec![finding(
                            "dependencies.containers",
                            StoragePlanningFindingCode::ThresholdExceeded,
                        )]);
                    }
                    count += 1;
                    matches.push(ConfiguredContainerConsumer {
                        container: consumer.container.clone(),
                        source: mount.source.clone(),
                        destination: mount.destination.clone(),
                        writable: mount.writable,
                        via,
                    });
                }
            }
        }
        matches.sort_by(|a, b| {
            (&a.container.resource, &a.destination, &a.via).cmp(&(
                &b.container.resource,
                &b.destination,
                &b.via,
            ))
        });
        devices.push(ConfiguredDeviceDependency {
            expected: device.clone(),
            pools: configured_pools,
            protection: configured_protection,
            containers: matches,
        });
    }
    devices.sort_by(|a, b| a.expected.id.cmp(&b.expected.id));
    Ok(ConfiguredStorageDependencyPreview {
        version: 1,
        media_identity: contract.media_identity.clone(),
        engine_id: containers.engine_id.clone(),
        containers_observed_at: containers.observed_at,
        devices,
    })
}

#[cfg(test)]
mod tests;
