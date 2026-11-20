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

/// One row of the selective Engine listing: what orders truncation, and the
/// Compose project its labels declare.
#[derive(Debug, serde::Deserialize)]
pub(super) struct Listed {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Names", default)]
    names: Vec<String>,
    #[serde(rename = "Labels", default = "labels_missing")]
    compose: ListedCompose,
}
/// What one listed row's labels declare about Compose, read as strictly as an
/// inspection reads them, so a truncated or omitted container is still
/// attributed to its project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ListedCompose {
    /// No `com.docker.compose.*` label, or no labels.
    None,
    Project(super::compose_evidence::ComposeProjectIdentity),
    /// Labels this reader cannot decode: its project is unknown.
    Undecodable,
}
fn labels_missing() -> ListedCompose {
    ListedCompose::Undecodable
}
impl<'de> serde::Deserialize<'de> for ListedCompose {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        use super::compose_evidence::{DIRECTORY, FILES, FILES_LABEL, PREFIX, PROJECT};
        use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
        /// Which label a key is; nothing else of a key is kept.
        enum Key {
            Project,
            Directory,
            Files,
            Compose,
            Other,
        }
        impl<'de> serde::Deserialize<'de> for Key {
            fn deserialize<D: serde::Deserializer<'de>>(
                decoder: D,
            ) -> std::result::Result<Self, D::Error> {
                struct Keys;
                impl Visitor<'_> for Keys {
                    type Value = Key;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str("a label name")
                    }
                    fn visit_str<E: serde::de::Error>(
                        self,
                        key: &str,
                    ) -> std::result::Result<Key, E> {
                        Ok(match key {
                            PROJECT => Key::Project,
                            DIRECTORY => Key::Directory,
                            FILES => Key::Files,
                            _ if key.starts_with(PREFIX) => Key::Compose,
                            _ => Key::Other,
                        })
                    }
                }
                decoder.deserialize_str(Keys)
            }
        }
        /// A label value: text kept when short enough, or not text at all.
        enum Value {
            Kept(String),
            Text,
            NotText,
        }
        struct Values(usize);
        impl<'de> serde::de::DeserializeSeed<'de> for Values {
            type Value = Value;
            fn deserialize<D: serde::Deserializer<'de>>(
                self,
                decoder: D,
            ) -> std::result::Result<Value, D::Error> {
                decoder.deserialize_any(self)
            }
        }
        impl<'de> Visitor<'de> for Values {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a label value")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
                if v.len() > self.0 {
                    return Ok(Value::Text);
                }
                let mut kept = String::new();
                kept.try_reserve_exact(v.len())
                    .map_err(|_| E::custom("label allocation"))?;
                kept.push_str(v);
                Ok(Value::Kept(kept))
            }
            fn visit_bool<E>(self, _: bool) -> std::result::Result<Value, E> {
                Ok(Value::NotText)
            }
            fn visit_i64<E>(self, _: i64) -> std::result::Result<Value, E> {
                Ok(Value::NotText)
            }
            fn visit_u64<E>(self, _: u64) -> std::result::Result<Value, E> {
                Ok(Value::NotText)
            }
            fn visit_f64<E>(self, _: f64) -> std::result::Result<Value, E> {
                Ok(Value::NotText)
            }
            fn visit_unit<E>(self) -> std::result::Result<Value, E> {
                Ok(Value::NotText)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Value::NotText)
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Value::NotText)
            }
        }
        struct Labels;
        impl<'de> Visitor<'de> for Labels {
            type Value = ListedCompose;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("container labels")
            }
            fn visit_unit<E>(self) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::None)
            }
            fn visit_none<E>(self) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::None)
            }
            fn visit_bool<E>(self, _: bool) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::Undecodable)
            }
            fn visit_i64<E>(self, _: i64) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::Undecodable)
            }
            fn visit_u64<E>(self, _: u64) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::Undecodable)
            }
            fn visit_f64<E>(self, _: f64) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::Undecodable)
            }
            fn visit_str<E>(self, _: &str) -> std::result::Result<ListedCompose, E> {
                Ok(ListedCompose::Undecodable)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<ListedCompose, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ListedCompose::Undecodable)
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<ListedCompose, A::Error> {
                let (mut compose, mut odd) = (false, false);
                let (mut project, mut directory, mut files) = (None, None, None);
                while let Some(key) = map.next_key::<Key>()? {
                    let (slot, bound) = match key {
                        Key::Project => (Some(&mut project), 64),
                        Key::Directory => (Some(&mut directory), 512),
                        Key::Files => (Some(&mut files), FILES_LABEL),
                        Key::Compose | Key::Other => (None, 0),
                    };
                    compose |= !matches!(key, Key::Other);
                    match (map.next_value_seed(Values(bound))?, slot) {
                        (Value::NotText, _) => odd = true,
                        // A repeated label, or one too long to be valid.
                        (Value::Kept(text), Some(slot)) if slot.is_none() => *slot = Some(text),
                        (_, Some(_)) => odd = true,
                        (_, None) => {}
                    }
                }
                Ok(match (odd, compose, project, directory, files) {
                    (true, ..) => ListedCompose::Undecodable,
                    (false, false, ..) => ListedCompose::None,
                    (false, true, Some(project), Some(directory), Some(files)) => {
                        super::compose_evidence::listed_identity(&project, &directory, &files)
                            .map_or(ListedCompose::Undecodable, ListedCompose::Project)
                    }
                    _ => ListedCompose::Undecodable,
                })
            }
        }
        decoder.deserialize_any(Labels)
    }
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
/// The selective listing of one collection pass.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Selected {
    /// Kept container IDs, in ID order.
    pub(super) ids: Vec<String>,
    /// Listed containers left out.
    pub(super) truncated: usize,
    /// Every listed container ID, kept or not, in ID order; empty where
    /// nothing is truncated (`ids` is then the whole listing).
    pub(super) listed: Vec<String>,
    /// Every listed row whose labels name Compose, kept or not, by ID.
    pub(super) compose: Vec<(String, ListedCompose)>,
}
impl Selected {
    /// Every listed container ID, in ID order.
    pub(super) fn all(&self) -> &[String] {
        if self.listed.is_empty() {
            &self.ids
        } else {
            &self.listed
        }
    }
}
/// Keep `keep` listed containers, running first, then by name and ID, and
/// return their IDs in ID order with the number left out. A malformed or
/// repeated ID is an Engine fault and refuses the listing.
pub(super) fn select_listed(
    rows: Vec<Listed>,
    keep: usize,
    scope: &mut super::read_admission::Scope,
) -> Result<Selected> {
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
    let mut listed = Vec::new();
    scope.reserve(&mut listed, seen.len())?;
    for id in &seen {
        scope.charge(48)?;
        listed.push(scope.string(id)?);
    }
    let mut compose = Vec::new();
    for row in &rows {
        let value = match &row.compose {
            ListedCompose::None => continue,
            ListedCompose::Undecodable => ListedCompose::Undecodable,
            ListedCompose::Project(identity) => {
                let mut files = Vec::new();
                scope.reserve(&mut files, identity.config_files.len())?;
                for file in &identity.config_files {
                    files.push(scope.string(file)?);
                }
                ListedCompose::Project(super::compose_evidence::ComposeProjectIdentity {
                    name: scope.string(&identity.name)?,
                    working_directory: scope.string(&identity.working_directory)?,
                    config_files: files,
                })
            }
        };
        scope.reserve(&mut compose, 1)?;
        compose.push((scope.string(&row.id)?, value));
    }
    compose.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Selected {
        ids: result,
        truncated,
        listed,
        compose,
    })
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
