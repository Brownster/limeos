//! Declared device consumers composed from checked configuration and Docker facts.
//! A graph row establishes no physical identity, retained evidence or effect authority.

use crate::{
    ContainerStorageInventory, PoolsConfig, SnapraidConfig, StorageContract, StorageMediaIdentity,
    StoragePlanningFindingCode, StoragePlanningResult,
    configured_dependencies::{ConfiguredDeviceDependency, configured_storage_dependencies},
    pools::finding,
    shares::{SambaDependencyRoute, SambaSharesConfig},
};
use serde::Serialize;

pub const MAX_CONFIGURED_CONSUMER_MATCHES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredShareConsumer {
    /// Exact unique name in `share_configuration`; flags and users are stored once.
    pub share_name: String,
    pub via: SambaDependencyRoute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredStorageGraphDevice {
    pub storage: ConfiguredDeviceDependency,
    pub shares: Vec<ConfiguredShareConsumer>,
}

/// Independent serialize-only format; existing standalone previews stay unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredStorageGraphPreview {
    pub version: u16,
    pub media_identity: StorageMediaIdentity,
    pub engine_id: String,
    /// Original Docker collection time, preserved without renewal.
    pub containers_observed_at: i64,
    pub share_configuration: SambaSharesConfig,
    pub unmapped_shares: Vec<String>,
    pub devices: Vec<ConfiguredStorageGraphDevice>,
}

/// Whole configured composition or refusal. All share/container expansion counts
/// against one ceiling, even when each standalone preview would pass its own cap.
/// Disabled, stopped and read-only consumers remain possible dependents. Empty
/// rows or caller-declared empty shares never establish complete live absence.
pub fn configured_storage_dependency_graph(
    contract: &StorageContract,
    pools: &PoolsConfig,
    protection: &SnapraidConfig,
    containers: &ContainerStorageInventory,
    shares: &SambaSharesConfig,
    at: i64,
) -> StoragePlanningResult<ConfiguredStorageGraphPreview> {
    let share_plan = shares.plan(contract, pools)?;
    let storage = configured_storage_dependencies(contract, pools, protection, containers, at)?;
    let mut devices: Vec<_> = storage
        .devices
        .into_iter()
        .map(|storage| ConfiguredStorageGraphDevice {
            storage,
            shares: Vec::new(),
        })
        .collect();
    let mut count: usize = devices.iter().map(|d| d.storage.containers.len()).sum();
    for dependency in share_plan.dependencies {
        for requirement in dependency.requirements {
            if count == MAX_CONFIGURED_CONSUMER_MATCHES {
                return Err(vec![finding(
                    "dependencies.consumers",
                    StoragePlanningFindingCode::ThresholdExceeded,
                )]);
            }
            let device = devices
                .iter_mut()
                .find(|device| device.storage.expected == requirement.expected)
                .ok_or_else(|| {
                    vec![finding(
                        "dependencies.shares",
                        StoragePlanningFindingCode::UnknownIdentity,
                    )]
                })?;
            count += 1;
            device.shares.push(ConfiguredShareConsumer {
                share_name: dependency.share_name.clone(),
                via: requirement.via,
            });
        }
    }
    for device in &mut devices {
        device
            .shares
            .sort_by(|a, b| (&a.share_name, &a.via).cmp(&(&b.share_name, &b.via)));
    }
    Ok(ConfiguredStorageGraphPreview {
        version: 1,
        media_identity: storage.media_identity,
        engine_id: storage.engine_id,
        containers_observed_at: storage.containers_observed_at,
        share_configuration: share_plan.desired,
        unmapped_shares: share_plan.unmapped_shares,
        devices,
    })
}

#[cfg(test)]
mod tests;
