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
/// Why one container's facts were not admitted. `Unsupported` is a shape this
/// reader does not model (a mount type, a label set); a display projection
/// leaves that one container out with a reason. `Fatal` (budget, transport or
/// Engine inconsistency) refuses the whole collection. Evidence for an effect
/// treats both as a refusal.
#[derive(Debug)]
pub(super) enum Refusal {
    Unsupported,
    Fatal(Error),
}
impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Self::Fatal(error)
    }
}
impl Refusal {
    pub(super) fn strict(self) -> Error {
        match self {
            Self::Unsupported => unavailable(),
            Self::Fatal(error) => error,
        }
    }
}
fn unsupported() -> Refusal {
    Refusal::Unsupported
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
        parse_storage_container(&value, id).map_err(Refusal::strict)
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
    parse_storage_container_ids_with_admission(value, None)
}
pub(super) fn parse_storage_container_ids_with_admission(
    value: &Value,
    mut admission: Option<&mut super::read_admission::Scope>,
) -> Result<Vec<String>> {
    let list = value.as_array().ok_or_else(unavailable)?;
    let limit = if let Some(scope) = &admission {
        scope.limits().consumers
    } else {
        CONTAINER_STORAGE_MAX_CONTAINERS
    };
    if list.len() > limit {
        return Err(unavailable());
    }
    let mut ids = BTreeSet::new();
    for item in list {
        let id = item["Id"].as_str().ok_or_else(unavailable)?;
        if let Some(scope) = &mut admission {
            scope.charge(48)?;
        }
        if !opaque_id(id) || !ids.insert(id) {
            return Err(unavailable());
        }
    }
    let mut result = Vec::new();
    if let Some(scope) = &mut admission {
        scope.reserve(&mut result, ids.len())?;
    }
    for id in ids {
        result.push(owned(&mut admission, id)?);
    }
    Ok(result)
}

/// One row of the selective Engine listing: only what orders truncation.
#[derive(Debug, serde::Deserialize)]
pub(super) struct Listed {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Names", default)]
    names: Vec<String>,
}
/// Decode at most `rows` list rows, skipping every other field unallocated.
pub(super) fn parse_listing(bytes: &[u8], rows: usize) -> Result<Vec<Listed>> {
    struct Rows(usize);
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = Vec<Listed>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {} listed containers", self.0)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out: Vec<Listed> = Vec::new();
            while let Some(row) = seq.next_element::<Listed>()? {
                if out.len() >= self.0 {
                    return Err(serde::de::Error::custom("listing bound"));
                }
                out.try_reserve(1)
                    .map_err(|_| serde::de::Error::custom("listing allocation"))?;
                out.push(row);
            }
            Ok(out)
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let rows = serde::Deserializer::deserialize_seq(&mut decoder, Rows(rows))
        .map_err(|_| unavailable())?;
    decoder.end().map_err(|_| unavailable())?;
    Ok(rows)
}
/// Keep `keep` listed containers, running first, then by name and ID, and
/// return their IDs in ID order with the number left out. A malformed or
/// repeated ID is an Engine fault and refuses the listing.
pub(super) fn select_listed(
    rows: Vec<Listed>,
    keep: usize,
    scope: &mut super::read_admission::Scope,
) -> Result<(Vec<String>, usize)> {
    let mut seen = BTreeSet::new();
    for row in &rows {
        if !opaque_id(&row.id) || !seen.insert(row.id.as_str()) {
            return Err(unavailable());
        }
    }
    let mut order: Vec<&Listed> = rows.iter().collect();
    order.sort_by(|a, b| {
        (a.state != "running")
            .cmp(&(b.state != "running"))
            .then_with(|| a.names.first().cmp(&b.names.first()))
            .then_with(|| a.id.cmp(&b.id))
    });
    let truncated = order.len().saturating_sub(keep);
    let kept: BTreeSet<&str> = order
        .into_iter()
        .take(keep)
        .map(|r| r.id.as_str())
        .collect();
    let mut result = Vec::new();
    scope.reserve(&mut result, kept.len())?;
    for id in kept {
        scope.charge(48)?;
        result.push(scope.string(id)?);
    }
    Ok((result, truncated))
}

pub(super) fn parse_storage_container(
    value: &Value,
    id: &str,
) -> std::result::Result<ContainerStorageConsumer, Refusal> {
    parse_storage_container_with_admission(value, id, usize::MAX, None)
}
fn owned(admission: &mut Option<&mut super::read_admission::Scope>, text: &str) -> Result<String> {
    match admission {
        Some(scope) => scope.string(text),
        None => Ok(text.to_owned()),
    }
}
pub(super) fn parse_storage_container_with_admission(
    value: &Value,
    id: &str,
    selected_limit: usize,
    mut admission: Option<&mut super::read_admission::Scope>,
) -> std::result::Result<ContainerStorageConsumer, Refusal> {
    if value["Id"].as_str() != Some(id) {
        return Err(unavailable().into());
    }
    if let Some(scope) = &mut admission {
        scope.charge(id.len() + 10)?;
    }
    let container = ContainerSnapshot {
        resource: format!("container:{id}"),
        image: owned(
            &mut admission,
            value["Image"].as_str().ok_or_else(unsupported)?,
        )?,
        started_at: owned(
            &mut admission,
            value["State"]["StartedAt"]
                .as_str()
                .ok_or_else(unsupported)?,
        )?,
        running: value["State"]["Running"]
            .as_bool()
            .ok_or_else(unsupported)?,
    };
    let raw_mounts = value["Mounts"].as_array().ok_or_else(unsupported)?;
    if raw_mounts.len() > CONTAINER_STORAGE_MAX_MOUNTS {
        return Err(unsupported());
    }
    let mut mounts = vec![];
    let mut destinations = BTreeSet::new();
    for raw in raw_mounts {
        let destination = raw["Destination"].as_str().ok_or_else(unsupported)?;
        let writable = raw["RW"].as_bool().ok_or_else(unsupported)?;
        if let Some(scope) = &mut admission {
            scope.charge(48)?;
        }
        if !container_storage_path(destination) || !destinations.insert(destination) {
            return Err(unsupported());
        }
        if admission.is_some()
            && raw["Type"].as_str() != Some("tmpfs")
            && mounts.len() >= selected_limit
        {
            return Err(unavailable().into());
        }
        let source = match raw["Type"].as_str() {
            Some("bind") => ContainerStorageSource::Bind {
                path: owned(
                    &mut admission,
                    raw["Source"].as_str().ok_or_else(unsupported)?,
                )?,
            },
            Some("volume") => ContainerStorageSource::Volume {
                path: owned(
                    &mut admission,
                    raw["Source"].as_str().ok_or_else(unsupported)?,
                )?,
                name: owned(
                    &mut admission,
                    raw["Name"].as_str().ok_or_else(unsupported)?,
                )?,
                driver: owned(
                    &mut admission,
                    raw["Driver"].as_str().ok_or_else(unsupported)?,
                )?,
            },
            Some("tmpfs") if raw.get("Source").is_none() || raw["Source"].as_str() == Some("") => {
                continue;
            }
            _ => return Err(unsupported()),
        };
        if let Some(scope) = &mut admission {
            scope.reserve(&mut mounts, 1)?;
        }
        mounts.push(ContainerStorageMount {
            source,
            destination: owned(&mut admission, destination)?,
            writable,
        });
    }
    mounts.sort_by(|a, b| a.destination.cmp(&b.destination));
    Ok(ContainerStorageConsumer { container, mounts })
}

#[cfg(test)]
mod tests;
