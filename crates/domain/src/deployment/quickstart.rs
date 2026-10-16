//! Pure bundle preparation; directories, workflow and profile are proposals only.
pub use super::media_settings::SourceState;
use super::{
    catalog::{InstallTarget, InstallationKey},
    discovery::ProjectIdentity,
    media::scope_valid,
};
use crate::{StorageContract, StorageMediaIdentity};
use serde::Serialize;
use std::io::Write;

pub const MAX_RECORDS: usize = 16;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationError {
    InvalidInput,
    UnsupportedWithoutVpn,
    OverlappingRoots,
    Limit,
}
type Result<T> = std::result::Result<T, PreparationError>;
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(PreparationError::InvalidInput)
    }
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn path(s: &str) -> bool {
    s.starts_with('/')
        && s != "/"
        && s.len() <= 512
        && !s.chars().any(char::is_control)
        && !s.contains(['$', '{', '}', '\\'])
        && s[1..]
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
}
fn overlaps(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|p| p.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|p| p.starts_with('/'))
}
fn timezone(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
        })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub stack: String,
    pub engine_id: String,
    pub project: ProjectIdentity,
}
impl Target {
    fn validate(&self) -> Result<()> {
        check(scope_valid(
            &self.stack,
            &self.engine_id,
            &self.project,
            "vpn",
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub use_vpn: bool,
    pub timezone: String,
    pub requested_identity: Option<StorageMediaIdentity>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Member {
    Vpn,
    Transmission,
    Sabnzbd,
    Prowlarr,
    Sonarr,
    Radarr,
    Lidarr,
    Jellyfin,
}
pub const MEMBERS: [Member; 8] = [
    Member::Vpn,
    Member::Transmission,
    Member::Sabnzbd,
    Member::Prowlarr,
    Member::Sonarr,
    Member::Radarr,
    Member::Lidarr,
    Member::Jellyfin,
];
impl Member {
    pub fn id(self) -> &'static str {
        match self {
            Self::Vpn => "vpn",
            Self::Transmission => "transmission",
            Self::Sabnzbd => "sabnzbd",
            Self::Prowlarr => "prowlarr",
            Self::Sonarr => "sonarr",
            Self::Radarr => "radarr",
            Self::Lidarr => "lidarr",
            Self::Jellyfin => "jellyfin",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Managed,
    Operator,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installation {
    pub key: InstallationKey,
    pub service: String,
    pub ownership: Ownership,
}
pub struct Observation<'a> {
    pub target: &'a Target,
    pub sampled_at: i64,
    pub complete: bool,
    /// Registered declaration/catalog existence, never physical target absence.
    pub exists: bool,
    pub declared_services: &'a [String],
    pub installations: &'a [Installation],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Layout {
    pub storage_root: String,
    pub downloads_root: String,
    pub config_root: String,
    pub backup_root: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryKind {
    Library,
    IncompleteDownload,
    CompleteDownload,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Directory {
    pub kind: DirectoryKind,
    pub name: &'static str,
    pub host: String,
}
const LIBRARIES: [&str; 6] = ["movies", "tv", "music", "books", "audiobooks", "podcasts"];
const DOWNLOADS: [&str; 9] = [
    "sonarr",
    "radarr",
    "lidarr",
    "readarr",
    "sabnzbd",
    "transmission",
    "rdtclient",
    "jackett",
    "get_iplayer",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    AmbiguousInstallation,
    UnownedInstallation,
    NoncanonicalService,
    NameCollision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MemberDecision {
    Unknown,
    NeedsInstall,
    AlreadyDeclared,
    Blocked { reason: Conflict },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub member: Member,
    pub decision: MemberDecision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    ReviewCompleteSourcesAndStorage,
    PrepareDirectories,
    PrepareCatalogFiles,
    ConfigureQuiescentDownloadClients,
    StartStack,
    VerifyStartup,
    SeedServiceConfiguration,
    VerifyLoadedConfiguration,
    SaveProfile,
    VerifyProfile,
}
const STAGES: [Stage; 10] = [
    Stage::ReviewCompleteSourcesAndStorage,
    Stage::PrepareDirectories,
    Stage::PrepareCatalogFiles,
    Stage::ConfigureQuiescentDownloadClients,
    Stage::StartStack,
    Stage::VerifyStartup,
    Stage::SeedServiceConfiguration,
    Stage::VerifyLoadedConfiguration,
    Stage::SaveProfile,
    Stage::VerifyProfile,
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Step {
    pub stage: Stage,
    pub requires: Option<Stage>,
}
/// Proposed format only. No execution completion flag or arbitrary values bag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileDraft {
    pub version: u16,
    pub bundle_id: &'static str,
    pub stack_name: String,
    pub use_vpn: bool,
    pub timezone: String,
    pub media_identity: StorageMediaIdentity,
    pub layout: Layout,
    pub members: [Member; 8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    ManagedIdentityOverridesRequest,
    ServiceCapacityExceeded,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Unknown,
    Blocked,
    PreparationProposed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    ProtectedSourcesAndPreparedStackTarget,
    VerifiedStorageMountsAndNumericOwnership,
    CompleteCatalogTemplatesAndVpn,
    CurrentAuthorityClaimsAndLocks,
    BeforeImageQuiescenceAndRecovery,
    IndependentlyVerifiedWorkflowResults,
}
const GATES: [Gate; 6] = [
    Gate::ProtectedSourcesAndPreparedStackTarget,
    Gate::VerifiedStorageMountsAndNumericOwnership,
    Gate::CompleteCatalogTemplatesAndVpn,
    Gate::CurrentAuthorityClaimsAndLocks,
    Gate::BeforeImageQuiescenceAndRecovery,
    Gate::IndependentlyVerifiedWorkflowResults,
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
    pub catalog_target: Option<InstallTarget>,
    pub media_identity: StorageMediaIdentity,
    pub directories: Vec<Directory>,
    pub members: Vec<Row>,
    pub install_candidates: Vec<InstallationKey>,
    pub workflow: Vec<Step>,
    pub profile: Option<ProfileDraft>,
    pub issues: Vec<Issue>,
    pub unresolved: [Gate; 6],
}
fn target_text(t: &Target) -> usize {
    t.stack.len()
        + t.engine_id.len()
        + t.project.name.len()
        + t.project.working_directory.len()
        + t.project
            .config_files
            .iter()
            .map(String::len)
            .sum::<usize>()
}
fn classify(member: Member, o: &Observation<'_>) -> MemberDecision {
    let matching: Vec<_> = o
        .installations
        .iter()
        .filter(|i| i.key.app == member.id())
        .collect();
    match matching.as_slice() {
        [] if o.declared_services.iter().any(|s| s == member.id()) => MemberDecision::Blocked {
            reason: Conflict::NameCollision,
        },
        [] => MemberDecision::NeedsInstall,
        [i] if i.ownership != Ownership::Managed => MemberDecision::Blocked {
            reason: Conflict::UnownedInstallation,
        },
        [i] if i.service != member.id() => MemberDecision::Blocked {
            reason: Conflict::NoncanonicalService,
        },
        [_] => MemberDecision::AlreadyDeclared,
        _ => MemberDecision::Blocked {
            reason: Conflict::AmbiguousInstallation,
        },
    }
}
pub fn prepare(
    now: i64,
    target: &Target,
    contract: &StorageContract,
    options: &Options,
    observation: Option<Observation<'_>>,
) -> Result<Preview> {
    check((0..=MAX_TIME).contains(&now))?;
    target.validate()?;
    if !options.use_vpn {
        return Err(PreparationError::UnsupportedWithoutVpn);
    }
    check(timezone(&options.timezone) && contract.validate().is_ok())?;
    if let Some(id) = &options.requested_identity {
        check((1..=i32::MAX as u32).contains(&id.uid) && (1..=i32::MAX as u32).contains(&id.gid))?;
    }
    let locations = &contract.locations;
    let roots = [
        locations.media_host.as_str(),
        locations.downloads_host.as_str(),
        locations.application_config_host.as_str(),
        locations.backup_host.as_str(),
    ];
    for (i, root) in roots.iter().enumerate() {
        check(path(root))?;
        if roots[..i].iter().any(|other| overlaps(root, other)) {
            return Err(PreparationError::OverlappingRoots);
        }
    }
    let mut text =
        target_text(target) + roots.iter().map(|r| r.len()).sum::<usize>() + options.timezone.len();
    if let Some(o) = &observation {
        o.target.validate()?;
        check((0..=MAX_TIME).contains(&o.sampled_at))?;
        if o.declared_services.len() > MAX_RECORDS || o.installations.len() > MAX_RECORDS {
            return Err(PreparationError::Limit);
        }
        check(o.exists || (o.declared_services.is_empty() && o.installations.is_empty()))?;
        text += target_text(o.target);
        for (i, s) in o.declared_services.iter().enumerate() {
            check(name(s) && !o.declared_services[..i].contains(s))?;
            text += s.len();
        }
        for (i, entry) in o.installations.iter().enumerate() {
            check(
                name(&entry.key.app)
                    && entry.key.stack == o.target.stack
                    && name(&entry.service)
                    && (!o.complete || o.declared_services.contains(&entry.service))
                    && !o.installations[..i]
                        .iter()
                        .any(|other| other.service == entry.service),
            )?;
            text += entry.key.app.len() + entry.key.stack.len() + entry.service.len();
        }
    }
    if text > MAX_TEXT_BYTES {
        return Err(PreparationError::Limit);
    }
    let layout = Layout {
        storage_root: locations.media_host.clone(),
        downloads_root: locations.downloads_host.clone(),
        config_root: locations.application_config_host.clone(),
        backup_root: locations.backup_host.clone(),
    };
    let mut directories = vec![];
    for name in LIBRARIES {
        directories.push(Directory {
            kind: DirectoryKind::Library,
            name,
            host: format!("{}/{name}", layout.storage_root),
        });
    }
    directories.push(Directory {
        kind: DirectoryKind::IncompleteDownload,
        name: "incomplete",
        host: format!("{}/incomplete", layout.downloads_root),
    });
    for name in DOWNLOADS {
        directories.push(Directory {
            kind: DirectoryKind::CompleteDownload,
            name,
            host: format!("{}/complete/{name}", layout.downloads_root),
        });
    }
    for dir in &directories {
        check(path(&dir.host))?;
    }
    let source = match &observation {
        None => SourceState::Unavailable,
        Some(o) if o.target != target => SourceState::IdentityMismatch,
        Some(o) if o.sampled_at > now => SourceState::Future,
        Some(o) if now - o.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(o) if !o.complete => SourceState::Incomplete,
        Some(_) => SourceState::Fresh,
    };
    let members: Vec<_> = MEMBERS
        .into_iter()
        .map(|member| Row {
            member,
            decision: if source == SourceState::Fresh {
                classify(member, observation.as_ref().unwrap())
            } else {
                MemberDecision::Unknown
            },
        })
        .collect();
    let mut result = Preview {
        version: 1,
        target: target.clone(),
        generated_at: now,
        sampled_at: observation.as_ref().map(|o| o.sampled_at),
        max_age_seconds: MAX_AGE_SECONDS,
        source,
        outcome: Outcome::Unknown,
        catalog_target: None,
        media_identity: contract.media_identity.clone(),
        directories,
        members,
        install_candidates: vec![],
        workflow: STAGES
            .into_iter()
            .enumerate()
            .map(|(i, stage)| Step {
                stage,
                requires: i.checked_sub(1).map(|prior| STAGES[prior]),
            })
            .collect(),
        profile: None,
        issues: if options
            .requested_identity
            .as_ref()
            .is_some_and(|id| id != &contract.media_identity)
        {
            vec![Issue::ManagedIdentityOverridesRequest]
        } else {
            vec![]
        },
        unresolved: GATES,
    };
    if source == SourceState::Fresh {
        let desired_services = observation.as_ref().unwrap().declared_services.len()
            + result
                .members
                .iter()
                .filter(|m| m.decision == MemberDecision::NeedsInstall)
                .count();
        let exceeds_capacity = desired_services > super::catalog::MAX_SERVICES;
        if exceeds_capacity {
            result.issues.push(Issue::ServiceCapacityExceeded);
        }
        if exceeds_capacity
            || result
                .members
                .iter()
                .any(|m| matches!(m.decision, MemberDecision::Blocked { .. }))
        {
            result.outcome = Outcome::Blocked;
        } else {
            result.outcome = Outcome::PreparationProposed;
            result.catalog_target = Some(if observation.as_ref().unwrap().exists {
                InstallTarget::Existing {
                    stack: target.stack.clone(),
                }
            } else {
                InstallTarget::New {
                    stack: target.stack.clone(),
                }
            });
            result.install_candidates = result
                .members
                .iter()
                .filter(|m| m.decision == MemberDecision::NeedsInstall)
                .map(|m| InstallationKey {
                    app: m.member.id().into(),
                    stack: target.stack.clone(),
                })
                .collect();
            result.profile = Some(ProfileDraft {
                version: 2,
                bundle_id: "media-server",
                stack_name: target.stack.clone(),
                use_vpn: true,
                timezone: options.timezone.clone(),
                media_identity: contract.media_identity.clone(),
                layout,
                members: MEMBERS,
            });
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
                    return Err(std::io::Error::other("quickstart preview bound"));
                }
                self.0 -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Bound(MAX_REPORT_BYTES), self).map_err(|_| PreparationError::Limit)
    }
}
#[cfg(test)]
mod tests;
