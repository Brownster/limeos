//! Bounded display samples. Deserialization never restores retained authority.
use super::*;
use serde::de::{self, DeserializeSeed, SeqAccess, Visitor};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, TS)]
pub struct CatalogFacts {
    pub engine_id: String,
    #[ts(type = "number")]
    pub sampled_at: i64,
    pub stacks: Vec<Declaration>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, TS)]
pub struct EngineFacts {
    pub engine_id: String,
    #[ts(type = "number")]
    pub sampled_at: i64,
    pub members: Vec<Membership>,
    /// Engine containers beyond `MAX_ENGINE_CONTAINERS`, not in `members`.
    pub truncated: u16,
    /// Inspected containers left out of `members`, with their reason.
    pub omitted: Vec<OmittedContainer>,
    /// Distinct Compose projects that Engine containers outside `members`
    /// (truncated or omitted) declare in their list labels: a stack named by
    /// one has an unread member, and each takes part in the overlap checks.
    pub hidden_projects: Vec<ProjectIdentity>,
    /// Listed containers whose Compose project could not be established: list
    /// labels that do not decode, or beyond the hidden-project bounds.
    pub unattributed: u16,
}
impl EngineFacts {
    /// Left-out containers are distinct, never members, and with the members
    /// within the Engine bound.
    fn check_omitted(&self) -> Result<()> {
        if self.members.len().saturating_add(self.omitted.len()) > MAX_ENGINE_CONTAINERS {
            return Err(limit());
        }
        for (i, omitted) in self.omitted.iter().enumerate() {
            omitted.validate()?;
            if self.omitted[..i]
                .iter()
                .any(|o| o.resource == omitted.resource)
                || self
                    .members
                    .iter()
                    .any(|m| m.container().resource == omitted.resource)
            {
                return Err(invalid());
            }
        }
        if self.hidden_projects.len() > MAX_HIDDEN_PROJECTS {
            return Err(limit());
        }
        let mut text = 0_usize;
        for (i, project) in self.hidden_projects.iter().enumerate() {
            if !name(&project.name, true)
                || !path(&project.working_directory)
                || !files_valid(&project.config_files)
                || self.hidden_projects[..i].contains(project)
            {
                return Err(invalid());
            }
            text = project
                .config_files
                .iter()
                .chain([&project.name, &project.working_directory])
                .fold(text, |total, field| total.saturating_add(field.len()));
        }
        if text > MAX_HIDDEN_TEXT {
            return Err(limit());
        }
        Ok(())
    }
}

/// Selected past facts, never file/Engine ownership or execution permission.
#[derive(Clone, Serialize, JsonSchema, TS)]
pub struct DiscoveryCohort {
    version: u16,
    catalog: Option<CatalogFacts>,
    engine: Option<EngineFacts>,
    catalog_current: bool,
    engine_current: bool,
    catalog_valid_for_millis: u32,
    engine_valid_for_millis: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u16,
    catalog: Option<CatalogWire>,
    engine: Option<EngineWire>,
    catalog_current: bool,
    engine_current: bool,
    catalog_valid_for_millis: u32,
    engine_valid_for_millis: u32,
}
impl<'de> Deserialize<'de> for DiscoveryCohort {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        let wire = Wire::deserialize(decoder)?;
        if wire.version != 1 {
            return Err(de::Error::custom("unsupported stack cohort version"));
        }
        Self::with_current(
            wire.catalog.map(|c| CatalogFacts {
                engine_id: c.engine_id.0,
                sampled_at: c.sampled_at,
                stacks: c.stacks.into_iter().map(Into::into).collect(),
            }),
            wire.engine.map(|e| EngineFacts {
                engine_id: e.engine_id.0,
                sampled_at: e.sampled_at,
                members: e.members.into_iter().map(Into::into).collect(),
                truncated: e.truncated,
                omitted: e.omitted.into_iter().map(Into::into).collect(),
                hidden_projects: e.hidden_projects.into_iter().map(Into::into).collect(),
                unattributed: e.unattributed,
            }),
            wire.catalog_current,
            wire.engine_current,
        )
        .and_then(|c| c.with_remaining(wire.catalog_valid_for_millis, wire.engine_valid_for_millis))
        .map_err(|_| de::Error::custom("invalid or oversized stack cohort"))
    }
}
impl fmt::Debug for DiscoveryCohort {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("DiscoveryCohort")
            .field("stacks", &self.catalog.as_ref().map(|c| c.stacks.len()))
            .field("members", &self.engine.as_ref().map(|e| e.members.len()))
            .finish()
    }
}
impl DiscoveryCohort {
    pub fn new(catalog: Option<CatalogFacts>, engine: Option<EngineFacts>) -> Result<Self> {
        let current = (catalog.is_some(), engine.is_some());
        Self::with_current(catalog, engine, current.0, current.1)
    }
    /// Availability describes this past observation, never execution authority.
    pub fn with_current(
        catalog: Option<CatalogFacts>,
        engine: Option<EngineFacts>,
        catalog_current: bool,
        engine_current: bool,
    ) -> Result<Self> {
        if (catalog_current && catalog.is_none()) || (engine_current && engine.is_none()) {
            return Err(invalid());
        }
        let value = Self {
            version: 1,
            catalog,
            engine,
            catalog_current,
            engine_current,
            catalog_valid_for_millis: if catalog_current { 90_000 } else { 0 },
            engine_valid_for_millis: if engine_current { 90_000 } else { 0 },
        };
        let (catalog, engine) = value.inputs();
        admit(catalog.as_ref(), engine.as_ref())?;
        if let Some(facts) = &value.engine {
            facts.check_omitted()?;
        }
        source(catalog.as_ref().map(|c| c.sampled_at), MAX_TIME)?;
        source(engine.as_ref().map(|e| e.sampled_at), MAX_TIME)?;
        value.check_size()?;
        Ok(value)
    }
    /// The frame is checked before any record/string allocation by serde.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(limit());
        }
        serde_json::from_slice(bytes).map_err(|_| invalid())
    }
    pub fn catalog(&self) -> Option<&CatalogFacts> {
        self.catalog.as_ref()
    }
    pub fn engine(&self) -> Option<&EngineFacts> {
        self.engine.as_ref()
    }
    pub fn current_sources(&self) -> (bool, bool) {
        (self.catalog_current, self.engine_current)
    }
    pub fn remaining_millis(&self) -> (u32, u32) {
        (self.catalog_valid_for_millis, self.engine_valid_for_millis)
    }
    /// Transported freshness can only shorten the closed observation lifetime.
    /// Cache layers preserve this remaining monotonic duration across relays.
    pub fn with_remaining(mut self, catalog: u32, engine: u32) -> Result<Self> {
        if catalog > 90_000
            || engine > 90_000
            || (!self.catalog_current && catalog != 0)
            || (!self.engine_current && engine != 0)
        {
            return Err(invalid());
        }
        self.catalog_valid_for_millis = catalog;
        self.engine_valid_for_millis = engine;
        Ok(self)
    }
    pub fn project_sources(
        &self,
        now: i64,
        catalog_current: bool,
        engine_current: bool,
    ) -> Result<StackInventory> {
        let (catalog, engine) = self.inputs();
        let mut view = discover_current(
            now,
            catalog,
            engine,
            self.engine
                .as_ref()
                .map_or(&[][..], |e| e.hidden_projects.as_slice()),
            catalog_current && self.catalog_current,
            engine_current && self.engine_current,
        )?;
        view.catalog.valid_for_millis = view
            .catalog
            .valid_for_millis
            .min(self.catalog_valid_for_millis);
        view.engine.valid_for_millis = view
            .engine
            .valid_for_millis
            .min(self.engine_valid_for_millis);
        if let (Some(count), Some(engine)) = (view.truncated_containers.as_mut(), &self.engine) {
            *count = engine.truncated;
            view.omitted_containers = engine.omitted.clone();
            view.unattributed_containers = Some(engine.unattributed);
            for stack in &mut view.stacks {
                let unread = engine
                    .omitted
                    .iter()
                    .filter_map(|o| o.project.as_deref())
                    .chain(engine.hidden_projects.iter().map(|p| p.name.as_str()))
                    .any(|project| project == stack.project);
                if unread && !stack.issues.contains(&StackIssue::OmittedMembers) {
                    stack.issues.push(StackIssue::OmittedMembers);
                }
                if engine.unattributed > 0 {
                    stack.issues.push(StackIssue::IncompleteInventory);
                }
            }
            view.check_size()?;
        }
        Ok(view)
    }
    pub fn project(&self, now: i64) -> Result<StackInventory> {
        self.project_sources(now, true, true)
    }
    fn inputs(&self) -> (Option<CatalogInput<'_>>, Option<EngineInput<'_>>) {
        (
            self.catalog.as_ref().map(|c| CatalogInput {
                engine_id: &c.engine_id,
                sampled_at: c.sampled_at,
                stacks: &c.stacks,
            }),
            self.engine.as_ref().map(|e| EngineInput {
                engine_id: &e.engine_id,
                sampled_at: e.sampled_at,
                members: &e.members,
            }),
        )
    }
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_REPORT_BYTES - self.0 {
                    return Err(std::io::Error::other("stack cohort bound"));
                }
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Count(0), self).map_err(|_| limit())
    }
}

// Wire-only nested decoders enforce limits before accepting an excess value.
// The pure model additionally validates identities, uniqueness and total text.
struct Text<const N: usize>(String);
impl<'de, const N: usize> Deserialize<'de> for Text<N> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        struct Bounded<const N: usize>;
        impl<const N: usize> Visitor<'_> for Bounded<N> {
            type Value = Text<N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} text bytes")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                if value.len() > N {
                    return Err(E::custom("stack text limit"));
                }
                let mut owned = String::new();
                owned
                    .try_reserve_exact(value.len())
                    .map_err(|_| E::custom("stack allocation"))?;
                owned.push_str(value);
                Ok(Text(owned))
            }
        }
        decoder.deserialize_str(Bounded::<N>)
    }
}
fn bounded<'de, D, T, const N: usize>(decoder: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: de::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Rows<T, const N: usize>(std::marker::PhantomData<T>);
    struct Entry<T> {
        allowed: bool,
        marker: std::marker::PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Entry<T> {
        type Value = T;
        fn deserialize<D: de::Deserializer<'de>>(
            self,
            decoder: D,
        ) -> std::result::Result<T, D::Error> {
            if !self.allowed {
                return Err(de::Error::custom("stack array limit"));
            }
            T::deserialize(decoder)
        }
    }
    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Rows<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {N} stack entries")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut rows: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(row) = rows.next_element_seed(Entry {
                allowed: result.len() < N,
                marker: std::marker::PhantomData,
            })? {
                result
                    .try_reserve_exact(1)
                    .map_err(|_| de::Error::custom("stack allocation"))?;
                result.push(row);
            }
            Ok(result)
        }
    }
    decoder.deserialize_seq(Rows::<T, N>(std::marker::PhantomData))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogWire {
    engine_id: Text<128>,
    sampled_at: i64,
    #[serde(deserialize_with = "bounded::<_, DeclarationWire, MAX_STACKS>")]
    stacks: Vec<DeclarationWire>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineWire {
    engine_id: Text<128>,
    sampled_at: i64,
    #[serde(deserialize_with = "bounded::<_, MembershipWire, MAX_ENGINE_CONTAINERS>")]
    members: Vec<MembershipWire>,
    #[serde(default)]
    truncated: u16,
    #[serde(
        default,
        deserialize_with = "bounded::<_, OmittedWire, MAX_ENGINE_CONTAINERS>"
    )]
    omitted: Vec<OmittedWire>,
    #[serde(
        default,
        deserialize_with = "bounded::<_, ProjectWire, MAX_HIDDEN_PROJECTS>"
    )]
    hidden_projects: Vec<ProjectWire>,
    #[serde(default)]
    unattributed: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OmittedWire {
    resource: Text<74>,
    project: Option<Text<64>>,
    reason: OmissionReason,
}
impl From<OmittedWire> for OmittedContainer {
    fn from(w: OmittedWire) -> Self {
        Self {
            resource: w.resource.0,
            project: w.project.map(|p| p.0),
            reason: w.reason,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationWire {
    stack: Text<64>,
    project: Text<64>,
    working_directory: Text<512>,
    #[serde(deserialize_with = "bounded::<_, Text<512>, 4>")]
    config_files: Vec<Text<512>>,
    #[serde(deserialize_with = "bounded::<_, Text<64>, MAX_SERVICES>")]
    services: Vec<Text<64>>,
    files: FileWire,
}
impl From<DeclarationWire> for Declaration {
    fn from(w: DeclarationWire) -> Self {
        Self {
            stack: w.stack.0,
            project: w.project.0,
            working_directory: w.working_directory.0,
            config_files: w.config_files.into_iter().map(|v| v.0).collect(),
            services: w.services.into_iter().map(|v| v.0).collect(),
            files: w.files.into(),
        }
    }
}
// Avoid serde's internally tagged-enum Content buffer: decode bounded fields
// directly in arbitrary JSON field order, then enforce each variant's shape.
#[derive(Deserialize)]
#[serde(try_from = "FileFields")]
struct FileWire(FileStatus);
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileState {
    NotInspected,
    Conflict,
    Unsupported,
    Unavailable,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileFields {
    state: FileState,
    #[serde(default, deserialize_with = "optional_candidates")]
    candidates: Option<Vec<ComposeFileName>>,
    #[serde(default, deserialize_with = "present")]
    reason: Option<UnsupportedSource>,
}
fn present<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn optional_candidates<'de, D: de::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Vec<ComposeFileName>>, D::Error> {
    bounded::<D, ComposeFileName, 4>(d).map(Some)
}
impl TryFrom<FileFields> for FileWire {
    type Error = &'static str;
    fn try_from(w: FileFields) -> std::result::Result<Self, Self::Error> {
        Ok(Self(match (w.state, w.candidates, w.reason) {
            (FileState::NotInspected, None, None) => FileStatus::NotInspected,
            (FileState::Conflict, Some(candidates), None) => FileStatus::Conflict { candidates },
            (FileState::Unsupported, None, Some(reason)) => FileStatus::Unsupported { reason },
            (FileState::Unavailable, None, None) => FileStatus::Unavailable,
            _ => return Err("invalid stack file diagnostic"),
        }))
    }
}
impl From<FileWire> for FileStatus {
    fn from(w: FileWire) -> Self {
        w.0
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectWire {
    name: Text<64>,
    working_directory: Text<512>,
    #[serde(deserialize_with = "bounded::<_, Text<512>, 4>")]
    config_files: Vec<Text<512>>,
}
impl From<ProjectWire> for ProjectIdentity {
    fn from(w: ProjectWire) -> Self {
        Self {
            name: w.name.0,
            working_directory: w.working_directory.0,
            config_files: w.config_files.into_iter().map(|v| v.0).collect(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerWire {
    resource: Text<74>,
    image: Text<71>,
    started_at: Text<64>,
    running: bool,
}
impl From<ContainerWire> for ContainerSnapshot {
    fn from(w: ContainerWire) -> Self {
        Self {
            resource: w.resource.0,
            image: w.image.0,
            started_at: w.started_at.0,
            running: w.running,
        }
    }
}
#[derive(Deserialize)]
#[serde(try_from = "MemberFields")]
struct MembershipWire(Membership);
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MemberKind {
    Standalone,
    Compose,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberFields {
    kind: MemberKind,
    container: ContainerWire,
    #[serde(default, deserialize_with = "present")]
    project: Option<ProjectWire>,
    #[serde(default, deserialize_with = "present")]
    service: Option<Text<64>>,
    #[serde(default, deserialize_with = "present")]
    one_off: Option<bool>,
}
impl TryFrom<MemberFields> for MembershipWire {
    type Error = &'static str;
    fn try_from(w: MemberFields) -> std::result::Result<Self, Self::Error> {
        Ok(Self(match (w.kind, w.project, w.service, w.one_off) {
            (MemberKind::Standalone, None, None, None) => Membership::Standalone {
                container: w.container.into(),
            },
            (MemberKind::Compose, Some(project), Some(service), Some(one_off)) => {
                Membership::Compose {
                    container: w.container.into(),
                    service: service.0,
                    one_off,
                    project: project.into(),
                }
            }
            _ => return Err("invalid stack member shape"),
        }))
    }
}
impl From<MembershipWire> for Membership {
    fn from(w: MembershipWire) -> Self {
        w.0
    }
}

#[cfg(test)]
mod tests;
