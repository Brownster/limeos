//! Declared impact and resource candidates. No live claim or safe-unmount authority.
use crate::{
    ContainerSnapshot, ContainerStorageInventory, PoolsConfig, SnapraidConfig, StorageContract,
    StorageDevice, StoragePlanningFindingCode, StoragePlanningResult,
    pools::finding,
    shares::SambaSharesConfig,
    storage_dependency_graph::{
        ConfiguredStorageGraphPreview, configured_storage_dependency_graph,
    },
};
use serde::Serialize;
use std::{collections::BTreeSet, io};

pub const MAX_STORAGE_IMPACT_RESOURCES: usize = 128;
pub const MAX_STORAGE_IMPACT_BYTES: usize = 256 * 1024;

/// Independent typed subjects, not Store claim keys or acquired locks.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfiguredStorageImpactResource {
    Device { device_id: String },
    Mount { mountpoint: String },
    Pool { pool_id: String },
    Snapraid,
    Share { share_name: String },
    Container { resource: String },
}

/// Every preview has these unresolved boundaries, including an empty dependency row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageImpactEvidenceNeeded {
    RetainedIdentityAndFreshness,
    CompletePhysicalConsumers,
    ActualPoolState,
    ActualProtectionState,
    ActualShareState,
    StackOwnership,
    CompleteClaimsAndApproval,
    EffectTimeValidation,
}

/// Pure serialize-only review format; selected and related devices stay distinct.
/// ```compile_fail
/// use limeos_domain::storage_impact::ConfiguredStorageImpactPreview;
/// let imported: ConfiguredStorageImpactPreview = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfiguredStorageImpactPreview {
    pub version: u16,
    /// The complete checked configured graph, with its original Docker timestamp.
    pub graph: ConfiguredStorageGraphPreview,
    pub selected_devices: Vec<String>,
    /// Includes selected devices and related pool/protection members, not new targets.
    pub related_devices: Vec<String>,
    pub resource_candidates: Vec<ConfiguredStorageImpactResource>,
    /// Full running identities for review; no Compose/stack ownership is inferred.
    pub running_containers: Vec<ContainerSnapshot>,
    pub evidence_needed: Vec<StorageImpactEvidenceNeeded>,
}

fn refuse<T>(field: &str, code: StoragePlanningFindingCode) -> StoragePlanningResult<T> {
    Err(vec![finding(field, code)])
}
fn add(
    resources: &mut BTreeSet<ConfiguredStorageImpactResource>,
    resource: ConfiguredStorageImpactResource,
) -> StoragePlanningResult<()> {
    if !resources.contains(&resource) && resources.len() == MAX_STORAGE_IMPACT_RESOURCES {
        return refuse(
            "impact.resources",
            StoragePlanningFindingCode::ThresholdExceeded,
        );
    }
    resources.insert(resource);
    Ok(())
}

/// Revalidate all original inputs before selecting rows. No imported graph is trusted.
/// Complete expected-device matching prevents an ID from concealing changed metadata.
/// Pool/global protection membership expands resource candidates, not stop targets.
pub fn configured_storage_impact(
    contract: &StorageContract,
    pools: &PoolsConfig,
    protection: &SnapraidConfig,
    containers: &ContainerStorageInventory,
    shares: &SambaSharesConfig,
    targets: &[StorageDevice],
    at: i64,
) -> StoragePlanningResult<ConfiguredStorageImpactPreview> {
    use ConfiguredStorageImpactResource as Resource;
    use StoragePlanningFindingCode as Code;
    if targets.is_empty() || targets.len() > crate::STORAGE_MAX_DEVICES {
        return refuse("impact.selection", Code::InvalidConfiguration);
    }
    let graph =
        configured_storage_dependency_graph(contract, pools, protection, containers, shares, at)?;
    let mut selected = BTreeSet::new();
    for target in targets {
        let Some(row) = graph
            .devices
            .iter()
            .find(|r| r.storage.expected.id == target.id)
        else {
            return refuse("impact.selection", Code::UnknownIdentity);
        };
        if row.storage.expected != *target {
            return refuse("impact.selection", Code::StaleEvidence);
        }
        if !selected.insert(row.storage.expected.id.as_str()) {
            return refuse("impact.selection", Code::Duplicate);
        }
    }
    let mut resources = BTreeSet::new();
    let mut affected_pools = BTreeSet::new();
    let mut affected_containers = BTreeSet::new();
    let mut affected_protection = false;
    for row in graph
        .devices
        .iter()
        .filter(|r| selected.contains(r.storage.expected.id.as_str()))
    {
        for pool in &row.storage.pools {
            affected_pools.insert(pool.pool_id.as_str());
            add(
                &mut resources,
                Resource::Pool {
                    pool_id: pool.pool_id.clone(),
                },
            )?;
            add(
                &mut resources,
                Resource::Mount {
                    mountpoint: pool.mountpoint.clone(),
                },
            )?;
        }
        affected_protection |= !row.storage.protection.is_empty();
        for consumer in &row.storage.containers {
            affected_containers.insert(consumer.container.resource.as_str());
            add(
                &mut resources,
                Resource::Container {
                    resource: consumer.container.resource.clone(),
                },
            )?;
        }
        for share in &row.shares {
            add(
                &mut resources,
                Resource::Share {
                    share_name: share.share_name.clone(),
                },
            )?;
        }
    }
    if affected_protection {
        add(&mut resources, Resource::Snapraid)?;
    }
    let mut related = BTreeSet::new();
    for row in &graph.devices {
        if selected.contains(row.storage.expected.id.as_str())
            || row
                .storage
                .pools
                .iter()
                .any(|p| affected_pools.contains(p.pool_id.as_str()))
            || (affected_protection && !row.storage.protection.is_empty())
        {
            related.insert(row.storage.expected.id.as_str());
            add(
                &mut resources,
                Resource::Device {
                    device_id: row.storage.expected.id.clone(),
                },
            )?;
            add(
                &mut resources,
                Resource::Mount {
                    mountpoint: row.storage.expected.mountpoint.clone(),
                },
            )?;
        }
    }
    // Select from the complete original inventory once, rather than every expanded route.
    let mut running_containers: Vec<_> = containers
        .containers
        .iter()
        .filter(|c| {
            c.container.running && affected_containers.contains(c.container.resource.as_str())
        })
        .map(|c| c.container.clone())
        .collect();
    running_containers.sort_by(|a, b| a.resource.cmp(&b.resource));
    use StorageImpactEvidenceNeeded::*;
    let preview = ConfiguredStorageImpactPreview {
        version: 1,
        selected_devices: selected.into_iter().map(str::to_owned).collect(),
        related_devices: related.into_iter().map(str::to_owned).collect(),
        resource_candidates: resources.into_iter().collect(),
        running_containers,
        evidence_needed: vec![
            RetainedIdentityAndFreshness,
            CompletePhysicalConsumers,
            ActualPoolState,
            ActualProtectionState,
            ActualShareState,
            StackOwnership,
            CompleteClaimsAndApproval,
            EffectTimeValidation,
        ],
        graph,
    };
    // Counts serialized bytes without building a second copy of the graph/report.
    serde_json::to_writer(PreviewBytes(0), &preview)
        .map_err(|_| vec![finding("impact.preview", Code::ThresholdExceeded)])?;
    Ok(preview)
}

struct PreviewBytes(usize);
impl io::Write for PreviewBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_STORAGE_IMPACT_BYTES - self.0 {
            return Err(io::Error::other("preview limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
