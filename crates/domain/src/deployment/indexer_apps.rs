//! Prowlarr selected metadata and pure previews; no source or effect authority.
pub use super::media_settings::SourceState;
use super::{discovery::ProjectIdentity, media::scope_valid};
use crate::ContainerSnapshot;
use serde::Serialize;
use std::io::Write;

pub const READ_PATH: &str = "/api/v1/applications";
pub const MAX_APPLICATIONS: usize = 16;
pub const MAX_FIELDS: usize = 64;
pub const MAX_IDS: usize = 16;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationError {
    InvalidInput,
    CategoryScope,
    Limit,
}
type Result<T> = std::result::Result<T, RegistrationError>;
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(RegistrationError::InvalidInput)
    }
}
fn text(s: &str) -> bool {
    s.len() <= 128 && !s.trim().is_empty() && !s.chars().any(char::is_control)
}
fn service(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
pub fn validate_ids(ids: &[u32]) -> Result<()> {
    if ids.len() > MAX_IDS {
        return Err(RegistrationError::Limit);
    }
    check(
        ids.iter()
            .enumerate()
            .all(|(i, id)| (1..=i32::MAX as u32).contains(id) && !ids[..i].contains(id)),
    )
}
fn set_eq(a: &[u32], b: &[u32]) -> bool {
    a.len() == b.len() && a.iter().all(|id| b.contains(id))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    Sonarr,
    Radarr,
    Lidarr,
}
impl App {
    pub fn implementation(self) -> &'static str {
        match self {
            Self::Sonarr => "Sonarr",
            Self::Radarr => "Radarr",
            Self::Lidarr => "Lidarr",
        }
    }
    pub fn config_contract(self) -> &'static str {
        match self {
            Self::Sonarr => "SonarrSettings",
            Self::Radarr => "RadarrSettings",
            Self::Lidarr => "LidarrSettings",
        }
    }
    pub fn from_implementation(s: &str) -> Option<Self> {
        [Self::Sonarr, Self::Radarr, Self::Lidarr]
            .into_iter()
            .find(|app| app.implementation().eq_ignore_ascii_case(s))
    }
    fn root(self) -> u32 {
        match self {
            Self::Sonarr => 5000,
            Self::Radarr => 2000,
            Self::Lidarr => 3000,
        }
    }
}
/// Explicit Prowlarr target, never an execution capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub stack: String,
    pub engine_id: String,
    pub project: ProjectIdentity,
    pub service: String,
    pub container: ContainerSnapshot,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        check(
            scope_valid(&self.stack, &self.engine_id, &self.project, &self.service)
                && self.container.validate().is_ok(),
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncLevel {
    Disabled,
    AddOnly,
    FullSync,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub sync_level: SyncLevel,
    pub categories: Vec<u32>,
    pub anime_categories: Option<Vec<u32>>,
    pub tags: Vec<u32>,
}
impl Settings {
    fn validate(&self, app: App) -> Result<()> {
        validate_ids(&self.categories)?;
        validate_ids(&self.tags)?;
        check(self.anime_categories.is_some() == (app == App::Sonarr))?;
        if let Some(ids) = &self.anime_categories {
            validate_ids(ids)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: u32,
    pub name: String,
    pub implementation: String,
    /// None means incomplete/unknown selected metadata, never default settings.
    pub settings: Option<Settings>,
}
impl Application {
    pub fn validate(&self) -> Result<()> {
        check(
            (1..=i32::MAX as u32).contains(&self.id)
                && text(&self.name)
                && text(&self.implementation),
        )?;
        if let Some(settings) = &self.settings {
            settings.validate(
                App::from_implementation(&self.implementation)
                    .ok_or(RegistrationError::InvalidInput)?,
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    pub application: App,
    pub service: String,
    pub categories: Vec<u32>,
    pub anime_categories: Option<Vec<u32>>,
    pub tags: Vec<u32>,
}
impl Request {
    fn validate(&self) -> Result<()> {
        check(service(&self.service) && !self.categories.is_empty())?;
        validate_ids(&self.categories)?;
        validate_ids(&self.tags)?;
        check(self.anime_categories.is_some() == (self.application == App::Sonarr))?;
        if let Some(ids) = &self.anime_categories {
            validate_ids(ids)?;
        }
        let root = self.application.root();
        if self
            .categories
            .iter()
            .chain(self.anime_categories.iter().flatten())
            .any(|id| !(root..root + 1000).contains(id))
        {
            return Err(RegistrationError::CategoryScope);
        }
        Ok(())
    }
    fn normalized(&self) -> Self {
        let mut result = self.clone();
        result.categories.sort_unstable();
        result.tags.sort_unstable();
        if let Some(ids) = &mut result.anime_categories {
            ids.sort_unstable();
        }
        result
    }
}
pub struct Observation<'a> {
    pub target: &'a Target,
    /// Original oldest sample across declarations and application responses.
    pub sampled_at: i64,
    pub complete: bool,
    pub declared_services: &'a [String],
    pub applications: &'a [Application],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    AmbiguousImplementation,
    IncompleteMetadata,
    PolicyDiffers,
    NameCollision,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Decision {
    Unknown,
    NotDeclared,
    MatchingMetadata { id: u32 },
    NeedsRegistration,
    Blocked { reason: Conflict, ids: Vec<u32> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub request: Request,
    pub decision: Decision,
}
/// Incomplete review data, never an API body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Addition {
    pub request: Request,
    pub name: &'static str,
    pub config_contract: &'static str,
    pub sync_level: SyncLevel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Unknown,
    Blocked,
    AdditionsProposed,
    NoAdditions,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    ProtectedSourcesAndCurrentInstance,
    VerifiedPeerBindingsNetworkAndCredentials,
    InstalledSchemaAndFullConfiguration,
    CurrentAuthorityClaimsAndLocks,
    BeforeImageAndRecovery,
    VerifiedLoadedServiceResult,
}
const GATES: [Gate; 6] = [
    Gate::ProtectedSourcesAndCurrentInstance,
    Gate::VerifiedPeerBindingsNetworkAndCredentials,
    Gate::InstalledSchemaAndFullConfiguration,
    Gate::CurrentAuthorityClaimsAndLocks,
    Gate::BeforeImageAndRecovery,
    Gate::VerifiedLoadedServiceResult,
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub version: u16,
    pub target: Target,
    pub generated_at: i64,
    pub sampled_at: Option<i64>,
    pub max_age_seconds: i64,
    pub source: SourceState,
    pub outcome: Outcome,
    pub rows: Vec<Row>,
    pub additions: Vec<Addition>,
    pub unresolved: [Gate; 6],
}
fn target_text(t: &Target) -> usize {
    t.stack.len()
        + t.engine_id.len()
        + t.service.len()
        + t.project.name.len()
        + t.project.working_directory.len()
        + t.project
            .config_files
            .iter()
            .map(String::len)
            .sum::<usize>()
        + t.container.resource.len()
        + t.container.image.len()
        + t.container.started_at.len()
}
fn classify(request: &Request, observation: &Observation<'_>) -> Decision {
    if !observation.declared_services.contains(&request.service) {
        return Decision::NotDeclared;
    }
    let blocked = |reason, applications: Vec<&Application>| {
        let mut ids: Vec<_> = applications.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        Decision::Blocked { reason, ids }
    };
    let matches: Vec<_> = observation
        .applications
        .iter()
        .filter(|a| App::from_implementation(&a.implementation) == Some(request.application))
        .collect();
    match matches.as_slice() {
        [] => {
            let names: Vec<_> = observation
                .applications
                .iter()
                .filter(|a| {
                    a.name
                        .eq_ignore_ascii_case(request.application.implementation())
                })
                .collect();
            if names.is_empty() {
                Decision::NeedsRegistration
            } else {
                blocked(Conflict::NameCollision, names)
            }
        }
        [application] => match &application.settings {
            None => blocked(Conflict::IncompleteMetadata, matches),
            Some(s)
                if s.sync_level == SyncLevel::FullSync
                    && set_eq(&s.categories, &request.categories)
                    && set_eq(&s.tags, &request.tags)
                    && match (&s.anime_categories, &request.anime_categories) {
                        (None, None) => true,
                        (Some(a), Some(b)) => set_eq(a, b),
                        _ => false,
                    } =>
            {
                Decision::MatchingMetadata { id: application.id }
            }
            Some(_) => blocked(Conflict::PolicyDiffers, matches),
        },
        _ => blocked(Conflict::AmbiguousImplementation, matches),
    }
}
pub fn preview(
    now: i64,
    target: &Target,
    requests: &[Request],
    observation: Option<Observation<'_>>,
) -> Result<Preview> {
    check((0..=MAX_TIME).contains(&now))?;
    target.validate()?;
    check((1..=3).contains(&requests.len()))?;
    let mut bytes = target_text(target);
    for (i, request) in requests.iter().enumerate() {
        request.validate()?;
        check(
            request.service != target.service
                && !requests[..i]
                    .iter()
                    .any(|r| r.application == request.application || r.service == request.service),
        )?;
        bytes += request.service.len();
    }
    if let Some(o) = &observation {
        o.target.validate()?;
        check((0..=MAX_TIME).contains(&o.sampled_at))?;
        if o.applications.len() > MAX_APPLICATIONS
            || o.declared_services.len() > super::discovery::MAX_SERVICES
        {
            return Err(RegistrationError::Limit);
        }
        bytes += target_text(o.target);
        for (i, name) in o.declared_services.iter().enumerate() {
            check(service(name) && !o.declared_services[..i].contains(name))?;
            bytes += name.len();
        }
        for (i, app) in o.applications.iter().enumerate() {
            app.validate()?;
            check(!o.applications[..i].iter().any(|a| a.id == app.id))?;
            bytes += app.name.len() + app.implementation.len();
        }
    }
    if bytes > MAX_TEXT_BYTES {
        return Err(RegistrationError::Limit);
    }
    let source = match &observation {
        None => SourceState::Unavailable,
        Some(_) if !target.container.running => SourceState::Stopped,
        Some(o) if o.target != target => SourceState::IdentityMismatch,
        Some(o) if o.sampled_at > now => SourceState::Future,
        Some(o) if now - o.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(o) if !o.complete || !o.declared_services.contains(&target.service) => {
            SourceState::Incomplete
        }
        Some(_) => SourceState::Fresh,
    };
    let mut rows: Vec<_> = requests
        .iter()
        .map(|r| Row {
            request: r.normalized(),
            decision: if source == SourceState::Fresh {
                classify(r, observation.as_ref().unwrap())
            } else {
                Decision::Unknown
            },
        })
        .collect();
    rows.sort_by_key(|r| r.request.application);
    let mut result = Preview {
        version: 1,
        target: target.clone(),
        generated_at: now,
        sampled_at: observation.as_ref().map(|o| o.sampled_at),
        max_age_seconds: MAX_AGE_SECONDS,
        source,
        outcome: Outcome::Unknown,
        rows,
        additions: vec![],
        unresolved: GATES,
    };
    if source == SourceState::Fresh {
        if result
            .rows
            .iter()
            .any(|r| matches!(r.decision, Decision::Blocked { .. }))
        {
            result.outcome = Outcome::Blocked;
        } else {
            result.additions = result
                .rows
                .iter()
                .filter(|r| r.decision == Decision::NeedsRegistration)
                .map(|r| Addition {
                    request: r.request.clone(),
                    name: r.request.application.implementation(),
                    config_contract: r.request.application.config_contract(),
                    sync_level: SyncLevel::FullSync,
                })
                .collect();
            result.outcome = if result.additions.is_empty() {
                Outcome::NoAdditions
            } else {
                Outcome::AdditionsProposed
            };
        }
    }
    result.check_size()?;
    Ok(result)
}
impl Preview {
    pub fn check_size(&self) -> Result<()> {
        struct Bound(usize);
        impl Write for Bound {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.0 {
                    return Err(std::io::Error::other("indexer preview bound"));
                }
                self.0 -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Bound(MAX_REPORT_BYTES), self).map_err(|_| RegistrationError::Limit)
    }
}
#[cfg(test)]
mod tests;
