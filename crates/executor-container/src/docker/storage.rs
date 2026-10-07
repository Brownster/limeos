//! Bounded GET-only enumeration; never substitute a partial result for no consumers.
use super::*;
use limeos_domain::{
    CONTAINER_STORAGE_MAX_CONTAINERS, CONTAINER_STORAGE_MAX_MOUNTS, ContainerStorageConsumer,
    ContainerStorageInventory, ContainerStorageMount, ContainerStorageSource,
    container_storage_path,
};
use std::collections::BTreeSet;

fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
impl Docker {
    async fn storage_engine_id(&self, version: &str) -> Result<String> {
        let info = self.json(format!("/{version}/info"), 128 * 1024).await?;
        Ok(info["ID"].as_str().ok_or_else(unavailable)?.into())
    }
    async fn storage_container_ids(&self, version: &str) -> Result<Vec<String>> {
        let value = self
            .json(format!("/{version}/containers/json?all=1"), 512 * 1024)
            .await?;
        parse_storage_container_ids(&value)
    }
    async fn storage_container(&self, version: &str, id: &str) -> Result<ContainerStorageConsumer> {
        let value = self
            .json(format!("/{version}/containers/{id}/json"), 512 * 1024)
            .await?;
        parse_storage_container(&value, id)
    }
    pub async fn storage_inventory(&self) -> Result<ContainerStorageInventory> {
        let observed_at = clock();
        tokio::time::timeout(Duration::from_secs(4), async {
            let version = self.version().await?;
            let engine_id = self.storage_engine_id(&version).await?;
            let ids = self.storage_container_ids(&version).await?;
            let mut containers = vec![];
            let mut mounts = 0usize;
            for id in &ids {
                let container = self.storage_container(&version, id).await?;
                mounts += container.mounts.len();
                if mounts > limeos_domain::CONTAINER_STORAGE_MAX_TOTAL_MOUNTS {
                    return Err(unavailable());
                }
                containers.push(container);
            }
            let inventory = ContainerStorageInventory {
                version: 1,
                engine_id,
                observed_at,
                containers,
            };
            inventory.validate(clock()).map_err(|_| unavailable())?;
            for (id, before) in ids.iter().zip(&inventory.containers) {
                if self.storage_container(&version, id).await? != *before {
                    return Err(Error(ErrorCode::Conflict));
                }
            }
            if self.storage_container_ids(&version).await? != ids
                || self.storage_engine_id(&version).await? != inventory.engine_id
            {
                return Err(Error(ErrorCode::Conflict));
            }
            inventory.validate(clock()).map_err(|_| unavailable())?;
            Ok(inventory)
        })
        .await
        .map_err(|_| unavailable())?
    }
}

pub(super) fn parse_storage_container_ids(value: &Value) -> Result<Vec<String>> {
    let list = value.as_array().ok_or_else(unavailable)?;
    if list.len() > CONTAINER_STORAGE_MAX_CONTAINERS {
        return Err(unavailable());
    }
    let mut ids = BTreeSet::new();
    for item in list {
        let id = item["Id"].as_str().ok_or_else(unavailable)?;
        if !opaque_id(id) || !ids.insert(id.to_owned()) {
            return Err(unavailable());
        }
    }
    Ok(ids.into_iter().collect())
}

pub(super) fn parse_storage_container(value: &Value, id: &str) -> Result<ContainerStorageConsumer> {
    if value["Id"].as_str() != Some(id) {
        return Err(unavailable());
    }
    let container = ContainerSnapshot {
        resource: format!("container:{id}"),
        image: value["Image"].as_str().ok_or_else(unavailable)?.into(),
        started_at: value["State"]["StartedAt"]
            .as_str()
            .ok_or_else(unavailable)?
            .into(),
        running: value["State"]["Running"]
            .as_bool()
            .ok_or_else(unavailable)?,
    };
    let raw_mounts = value["Mounts"].as_array().ok_or_else(unavailable)?;
    if raw_mounts.len() > CONTAINER_STORAGE_MAX_MOUNTS {
        return Err(unavailable());
    }
    let mut mounts = vec![];
    let mut destinations = BTreeSet::new();
    for raw in raw_mounts {
        let destination = raw["Destination"].as_str().ok_or_else(unavailable)?;
        let writable = raw["RW"].as_bool().ok_or_else(unavailable)?;
        if !container_storage_path(destination) || !destinations.insert(destination) {
            return Err(unavailable());
        }
        let source = match raw["Type"].as_str() {
            Some("bind") => ContainerStorageSource::Bind {
                path: raw["Source"].as_str().ok_or_else(unavailable)?.into(),
            },
            Some("volume") => ContainerStorageSource::Volume {
                path: raw["Source"].as_str().ok_or_else(unavailable)?.into(),
                name: raw["Name"].as_str().ok_or_else(unavailable)?.into(),
                driver: raw["Driver"].as_str().ok_or_else(unavailable)?.into(),
            },
            Some("tmpfs") if raw.get("Source").is_none() || raw["Source"].as_str() == Some("") => {
                continue;
            }
            _ => return Err(unavailable()),
        };
        mounts.push(ContainerStorageMount {
            source,
            destination: destination.into(),
            writable,
        });
    }
    mounts.sort_by(|a, b| a.destination.cmp(&b.destination));
    Ok(ContainerStorageConsumer { container, mounts })
}

#[cfg(test)]
mod tests;
