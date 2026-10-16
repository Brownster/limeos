//! Selected registration metadata only. No endpoint, credential or effect authority.
use super::media::MediaService;
pub use super::media_settings::{SourceState, Target};
use serde::Serialize;
use std::io::Write;

pub const MAX_CLIENTS: usize = 16;
pub const MAX_FIELDS: usize = 64;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationError {
    InvalidInput,
    Unsupported,
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
    s.len() <= 128 && !s.chars().any(char::is_control)
}
fn service(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Transmission,
    Sabnzbd,
}
impl Provider {
    pub fn implementation(self) -> &'static str {
        match self {
            Self::Transmission => "Transmission",
            Self::Sabnzbd => "Sabnzbd",
        }
    }
    pub fn config_contract(self) -> &'static str {
        match self {
            Self::Transmission => "TransmissionSettings",
            Self::Sabnzbd => "SabnzbdSettings",
        }
    }
    pub fn matches(self, implementation: &str) -> bool {
        self.implementation().eq_ignore_ascii_case(implementation)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryField {
    TvCategory,
    MovieCategory,
    MusicCategory,
}
impl CategoryField {
    pub fn for_app(app: MediaService) -> Result<Self> {
        match app {
            MediaService::Sonarr => Ok(Self::TvCategory),
            MediaService::Radarr => Ok(Self::MovieCategory),
            MediaService::Lidarr => Ok(Self::MusicCategory),
            _ => Err(RegistrationError::Unsupported),
        }
    }
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::TvCategory => "tvCategory",
            Self::MovieCategory => "movieCategory",
            Self::MusicCategory => "musicCategory",
        }
    }
}
pub fn read_path(app: MediaService) -> Result<&'static str> {
    CategoryField::for_app(app)?;
    Ok(if app == MediaService::Lidarr {
        "/api/v1/downloadclient"
    } else {
        "/api/v3/downloadclient"
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    pub provider: Provider,
    /// Explicit, case-sensitive Compose service; never inferred from implementation.
    pub service: String,
    pub category: String,
    pub remove_completed: bool,
    pub remove_failed: bool,
}
impl Request {
    fn validate(&self) -> Result<()> {
        check(
            service(&self.service)
                && text(&self.category)
                && !self.category.is_empty()
                && self.category.trim() == self.category,
        )?;
        if self.provider == Provider::Transmission {
            let category = self.category.strip_prefix('.').unwrap_or(&self.category);
            check(
                !category.is_empty()
                    && category
                        .bytes()
                        .all(|b| b.is_ascii_alphabetic() || b == b'-'),
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Category {
    Known(String),
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub id: u32,
    pub name: String,
    pub implementation: String,
    pub category_field: CategoryField,
    pub category: Category,
    pub enabled: bool,
    pub remove_completed: bool,
    pub remove_failed: bool,
}
impl Client {
    pub fn validate(&self, app: MediaService) -> Result<()> {
        check(self.category_field == CategoryField::for_app(app)?)?;
        check(
            (1..=i32::MAX as u32).contains(&self.id)
                && text(&self.name)
                && !self.name.trim().is_empty()
                && text(&self.implementation)
                && !self.implementation.trim().is_empty(),
        )?;
        if let Category::Known(value) = &self.category {
            check(text(value))?;
        }
        Ok(())
    }
}
pub struct Observation<'a> {
    pub target: &'a Target,
    /// Original oldest timestamp across current declarations and API response.
    pub sampled_at: i64,
    pub complete: bool,
    pub declared_services: &'a [String],
    pub clients: &'a [Client],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    UnknownCategory,
    DuplicateMatch,
    NameCollision,
    PolicyDiffers,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Decision {
    Unknown,
    NotDeclared,
    Present { id: u32 },
    NeedsRegistration,
    Blocked { reason: Conflict, ids: Vec<u32> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub request: Request,
    pub decision: Decision,
}
/// Incomplete review data, not a service POST body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Addition {
    pub request: Request,
    pub name: &'static str,
    pub config_contract: &'static str,
    pub category_field: CategoryField,
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
    VerifiedPeerNetworkCredentialsAndStorage,
    InstalledSchemaAndFullConfiguration,
    CurrentAuthorityClaimsAndLocks,
    BeforeImageAndRecovery,
    VerifiedLoadedServiceResult,
}
const GATES: [Gate; 6] = [
    Gate::ProtectedSourcesAndCurrentInstance,
    Gate::VerifiedPeerNetworkCredentialsAndStorage,
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
    let id = &t.identity;
    id.stack.len()
        + id.engine_id.len()
        + id.service.len()
        + id.project.name.len()
        + id.project.working_directory.len()
        + id.project
            .config_files
            .iter()
            .map(String::len)
            .sum::<usize>()
        + t.container.resource.len()
        + t.container.image.len()
        + t.container.started_at.len()
}
fn classify(request: &Request, sample: &Observation<'_>) -> Decision {
    if !sample.declared_services.contains(&request.service) {
        return Decision::NotDeclared;
    }
    let provider: Vec<_> = sample
        .clients
        .iter()
        .filter(|c| request.provider.matches(&c.implementation))
        .collect();
    let blocked = |reason, records: Vec<&Client>| {
        let mut ids: Vec<_> = records.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        Decision::Blocked { reason, ids }
    };
    let unknown: Vec<_> = provider
        .iter()
        .copied()
        .filter(|c| c.category == Category::Unknown)
        .collect();
    if !unknown.is_empty() {
        return blocked(Conflict::UnknownCategory, unknown);
    }
    let matching: Vec<_> = provider
        .iter()
        .copied()
        .filter(|c| matches!(&c.category, Category::Known(value) if value == &request.category))
        .collect();
    match matching.as_slice() {
        [client]
            if client.enabled
                && client.remove_completed == request.remove_completed
                && client.remove_failed == request.remove_failed =>
        {
            Decision::Present { id: client.id }
        }
        [_] => blocked(Conflict::PolicyDiffers, matching),
        [] => {
            let names: Vec<_> = sample
                .clients
                .iter()
                .filter(|c| {
                    c.name
                        .eq_ignore_ascii_case(request.provider.implementation())
                })
                .collect();
            if names.is_empty() {
                Decision::NeedsRegistration
            } else {
                blocked(Conflict::NameCollision, names)
            }
        }
        _ => blocked(Conflict::DuplicateMatch, matching),
    }
}
pub fn preview(
    now: i64,
    target: &Target,
    requests: &[Request],
    observation: Option<Observation<'_>>,
) -> Result<Preview> {
    let field = CategoryField::for_app(target.identity.application)?;
    check((0..=MAX_TIME).contains(&now) && target.validate().is_ok())?;
    check((1..=2).contains(&requests.len()))?;
    let mut bytes = target_text(target);
    for (i, request) in requests.iter().enumerate() {
        request.validate()?;
        check(
            !requests[..i]
                .iter()
                .any(|r| r.provider == request.provider || r.service == request.service),
        )?;
        bytes += request.service.len() + request.category.len();
    }
    if let Some(o) = &observation {
        check(o.target.validate().is_ok() && (0..=MAX_TIME).contains(&o.sampled_at))?;
        if o.clients.len() > MAX_CLIENTS
            || o.declared_services.len() > super::discovery::MAX_SERVICES
        {
            return Err(RegistrationError::Limit);
        }
        bytes += target_text(o.target);
        for (i, name) in o.declared_services.iter().enumerate() {
            check(service(name) && !o.declared_services[..i].contains(name))?;
            bytes += name.len();
        }
        for (i, client) in o.clients.iter().enumerate() {
            client.validate(o.target.identity.application)?;
            check(!o.clients[..i].iter().any(|c| c.id == client.id))?;
            bytes += client.name.len()
                + client.implementation.len()
                + match &client.category {
                    Category::Known(value) => value.len(),
                    Category::Unknown => 0,
                };
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
        Some(o) if !o.complete || !o.declared_services.contains(&target.identity.service) => {
            SourceState::Incomplete
        }
        Some(_) => SourceState::Fresh,
    };
    let mut rows: Vec<_> = requests
        .iter()
        .map(|r| Row {
            request: r.clone(),
            decision: if source == SourceState::Fresh {
                classify(r, observation.as_ref().unwrap())
            } else {
                Decision::Unknown
            },
        })
        .collect();
    rows.sort_by_key(|r| r.request.provider);
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
                    name: r.request.provider.implementation(),
                    config_contract: r.request.provider.config_contract(),
                    category_field: field,
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
                    return Err(std::io::Error::other("registration preview bound"));
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
