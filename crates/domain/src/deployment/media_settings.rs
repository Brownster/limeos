//! Ordinary selected settings and pure previews. No endpoint/source or effect authority.
use super::media::{LibraryKind, LibraryPath, MediaService, ServiceIdentity};
use crate::ContainerSnapshot;
use serde::Serialize;
use std::io::Write;

pub const MAX_PROFILES: usize = 16;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsError {
    InvalidInput,
    Unsupported,
    Limit,
}
type Result<T> = std::result::Result<T, SettingsError>;
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(SettingsError::InvalidInput)
    }
}
fn text(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}
fn path(s: &str) -> bool {
    s.len() <= 512
        && s.starts_with('/')
        && s != "/"
        && text(s, 512)
        && !s.contains(['$', '{', '}', '\\'])
        && s[1..]
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
}
fn id(v: u32) -> bool {
    (1..=i32::MAX as u32).contains(&v)
}
fn app_kind(app: MediaService) -> Result<LibraryKind> {
    match app {
        MediaService::Sonarr => Ok(LibraryKind::Tv),
        MediaService::Radarr => Ok(LibraryKind::Movies),
        MediaService::Lidarr => Ok(LibraryKind::Music),
        _ => Err(SettingsError::Unsupported),
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub identity: ServiceIdentity,
    pub container: ContainerSnapshot,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        app_kind(self.identity.application)?;
        check(self.identity.validate().is_ok() && self.container.validate().is_ok())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    Move,
    Hardlink,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NamingPreset {
    Standard,
    KeepOriginal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Intent {
    pub library: LibraryPath,
    pub import_mode: ImportMode,
    pub completed_download_handling: bool,
    pub naming: NamingPreset,
    /// Availability check only: this never assigns a global default.
    pub quality_profile: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    MediaManagement,
    DownloadClientConfig,
    Naming,
    QualityProfiles,
}
impl Resource {
    /// Fixed read paths only; hosts/credentials/acquisition are a separate boundary.
    pub fn read_path(self, app: MediaService) -> Result<&'static str> {
        app_kind(app)?;
        Ok(match (app == MediaService::Lidarr, self) {
            (false, Self::MediaManagement) => "/api/v3/config/mediamanagement",
            (false, Self::DownloadClientConfig) => "/api/v3/config/downloadclient",
            (false, Self::Naming) => "/api/v3/config/naming",
            (false, Self::QualityProfiles) => "/api/v3/qualityprofile",
            (true, Self::MediaManagement) => "/api/v1/config/mediamanagement",
            (true, Self::DownloadClientConfig) => "/api/v1/config/downloadclient",
            (true, Self::Naming) => "/api/v1/config/naming",
            (true, Self::QualityProfiles) => "/api/v1/qualityprofile",
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingField {
    CopyUsingHardlinks,
    RecycleBin,
    EnableCompletedDownloadHandling,
    RenameEpisodes,
    RenameMovies,
    RenameTracks,
    ReplaceIllegalCharacters,
}
impl SettingField {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::CopyUsingHardlinks => "copyUsingHardlinks",
            Self::RecycleBin => "recycleBin",
            Self::EnableCompletedDownloadHandling => "enableCompletedDownloadHandling",
            Self::RenameEpisodes => "renameEpisodes",
            Self::RenameMovies => "renameMovies",
            Self::RenameTracks => "renameTracks",
            Self::ReplaceIllegalCharacters => "replaceIllegalCharacters",
        }
    }
    fn resource(self) -> Resource {
        match self {
            Self::CopyUsingHardlinks | Self::RecycleBin => Resource::MediaManagement,
            Self::EnableCompletedDownloadHandling => Resource::DownloadClientConfig,
            _ => Resource::Naming,
        }
    }
    pub fn rename_field(app: MediaService) -> Result<Self> {
        match app {
            MediaService::Sonarr => Ok(Self::RenameEpisodes),
            MediaService::Radarr => Ok(Self::RenameMovies),
            MediaService::Lidarr => Ok(Self::RenameTracks),
            _ => Err(SettingsError::Unsupported),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Management {
    pub id: u32,
    pub copy_using_hardlinks: bool,
    pub recycle_bin: String,
    pub use_script_import: bool,
}
impl Management {
    pub fn validate(&self) -> Result<()> {
        check(id(self.id) && text(&self.recycle_bin, 512))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadHandling {
    pub id: u32,
    pub enabled: bool,
}
impl DownloadHandling {
    pub fn validate(&self) -> Result<()> {
        check(id(self.id))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Naming {
    pub id: u32,
    pub field: SettingField,
    pub rename: bool,
    pub replace_illegal_characters: bool,
}
impl Naming {
    pub fn validate(&self, app: MediaService) -> Result<()> {
        check(id(self.id) && self.field == SettingField::rename_field(app)?)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityProfile {
    pub id: u32,
    pub name: String,
}
impl QualityProfile {
    pub fn validate(&self) -> Result<()> {
        check(id(self.id) && text(&self.name, 128) && !self.name.trim().is_empty())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentSettings {
    pub management: Management,
    pub download_handling: DownloadHandling,
    pub naming: Naming,
    pub profiles: Vec<QualityProfile>,
}
impl CurrentSettings {
    pub fn validate(&self, app: MediaService) -> Result<()> {
        self.management.validate()?;
        self.download_handling.validate()?;
        self.naming.validate(app)?;
        if self.profiles.len() > MAX_PROFILES {
            return Err(SettingsError::Limit);
        }
        for (i, profile) in self.profiles.iter().enumerate() {
            profile.validate()?;
            check(!self.profiles[..i].iter().any(|p| p.id == profile.id))?;
        }
        Ok(())
    }
}
pub struct Observation<'a> {
    pub target: &'a Target,
    /// Original oldest timestamp across the required resource responses.
    pub sampled_at: i64,
    pub complete: bool,
    pub settings: &'a CurrentSettings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Unavailable,
    Stopped,
    IdentityMismatch,
    Future,
    Stale,
    Incomplete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Unknown,
    Blocked,
    ChangesProposed,
    Unchanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsIssue {
    ScriptImportEnabled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProfileAvailability {
    NotRequested,
    Unknown,
    Present { id: u32, name: String },
    Missing { name: String },
    Ambiguous { name: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SettingValue {
    Boolean(bool),
    Path(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SettingChange {
    pub resource: Resource,
    pub resource_id: u32,
    pub field: SettingField,
    pub before: SettingValue,
    pub after: SettingValue,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsGate {
    ProtectedSourceAndEndpoint,
    VerifiedLibraryMountsOwnershipAndHardlinks,
    CurrentConfigurationAndFieldPreservation,
    CurrentAuthorityClaimsAndLocks,
    BeforeImageAndRecovery,
    VerifiedLoadedServiceResult,
}
const GATES: [SettingsGate; 6] = [
    SettingsGate::ProtectedSourceAndEndpoint,
    SettingsGate::VerifiedLibraryMountsOwnershipAndHardlinks,
    SettingsGate::CurrentConfigurationAndFieldPreservation,
    SettingsGate::CurrentAuthorityClaimsAndLocks,
    SettingsGate::BeforeImageAndRecovery,
    SettingsGate::VerifiedLoadedServiceResult,
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SettingsPreview {
    pub version: u16,
    pub target: Target,
    pub generated_at: i64,
    pub sampled_at: Option<i64>,
    pub source: SourceState,
    pub max_age_seconds: i64,
    pub outcome: Outcome,
    pub desired: Intent,
    pub recycle_bin: String,
    pub quality_profile: ProfileAvailability,
    pub changes: Vec<SettingChange>,
    pub issues: Vec<SettingsIssue>,
    pub unresolved: [SettingsGate; 6],
}
fn target_text(target: &Target) -> usize {
    let id = &target.identity;
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
        + target.container.resource.len()
        + target.container.image.len()
        + target.container.started_at.len()
}
pub fn preview(
    now: i64,
    target: &Target,
    intent: &Intent,
    observation: Option<Observation<'_>>,
) -> Result<SettingsPreview> {
    check((0..=MAX_TIME).contains(&now))?;
    target.validate()?;
    check(
        intent.library.kind == app_kind(target.identity.application)?
            && path(&intent.library.host)
            && path(&intent.library.container),
    )?;
    for root in ["/downloads", "/data/downloads"] {
        check(
            intent.library.container != root
                && !intent
                    .library
                    .container
                    .strip_prefix(root)
                    .is_some_and(|p| p.starts_with('/')),
        )?;
    }
    let recycle_bin = format!("{}/.recycle", intent.library.container);
    check(path(&recycle_bin))?;
    if let Some(name) = &intent.quality_profile {
        check(text(name, 128) && !name.is_empty() && name.trim() == name)?;
    }
    let mut bytes = target_text(target)
        + intent.library.host.len()
        + intent.library.container.len()
        + intent.quality_profile.as_ref().map_or(0, String::len);
    if let Some(o) = &observation {
        o.target.validate()?;
        check((0..=MAX_TIME).contains(&o.sampled_at))?;
        o.settings.validate(o.target.identity.application)?;
        bytes += target_text(o.target)
            + o.settings.management.recycle_bin.len()
            + o.settings
                .profiles
                .iter()
                .map(|p| p.name.len())
                .sum::<usize>();
    }
    if bytes > MAX_TEXT_BYTES {
        return Err(SettingsError::Limit);
    }
    let source = match &observation {
        None => SourceState::Unavailable,
        Some(_) if !target.container.running => SourceState::Stopped,
        Some(o) if o.target != target => SourceState::IdentityMismatch,
        Some(o) if o.sampled_at > now => SourceState::Future,
        Some(o) if now - o.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(o) if !o.complete => SourceState::Incomplete,
        Some(_) => SourceState::Fresh,
    };
    let mut result = SettingsPreview {
        version: 1,
        target: target.clone(),
        generated_at: now,
        sampled_at: observation.as_ref().map(|o| o.sampled_at),
        source,
        max_age_seconds: MAX_AGE_SECONDS,
        outcome: Outcome::Unknown,
        desired: intent.clone(),
        recycle_bin,
        quality_profile: if intent.quality_profile.is_some() {
            ProfileAvailability::Unknown
        } else {
            ProfileAvailability::NotRequested
        },
        changes: vec![],
        issues: vec![],
        unresolved: GATES,
    };
    if source != SourceState::Fresh {
        return Ok(result);
    }
    let current = observation.unwrap().settings;
    if current.management.use_script_import {
        result.issues.push(SettingsIssue::ScriptImportEnabled);
    }
    if let Some(name) = &intent.quality_profile {
        let matches: Vec<_> = current
            .profiles
            .iter()
            .filter(|p| p.name == *name)
            .collect();
        result.quality_profile = match matches.as_slice() {
            [] => ProfileAvailability::Missing { name: name.clone() },
            [profile] => ProfileAvailability::Present {
                id: profile.id,
                name: name.clone(),
            },
            _ => ProfileAvailability::Ambiguous { name: name.clone() },
        };
        if matches.len() != 1 {
            result.outcome = Outcome::Blocked;
            return Ok(result);
        }
    }
    if !result.issues.is_empty() {
        result.outcome = Outcome::Blocked;
        return Ok(result);
    }
    let mut change =
        |field: SettingField, resource_id, before: SettingValue, after: SettingValue| {
            if before != after {
                result.changes.push(SettingChange {
                    resource: field.resource(),
                    resource_id,
                    field,
                    before,
                    after,
                });
            }
        };
    change(
        SettingField::CopyUsingHardlinks,
        current.management.id,
        SettingValue::Boolean(current.management.copy_using_hardlinks),
        SettingValue::Boolean(intent.import_mode == ImportMode::Hardlink),
    );
    change(
        SettingField::RecycleBin,
        current.management.id,
        SettingValue::Path(current.management.recycle_bin.clone()),
        SettingValue::Path(result.recycle_bin.clone()),
    );
    change(
        SettingField::EnableCompletedDownloadHandling,
        current.download_handling.id,
        SettingValue::Boolean(current.download_handling.enabled),
        SettingValue::Boolean(intent.completed_download_handling),
    );
    change(
        SettingField::rename_field(target.identity.application)?,
        current.naming.id,
        SettingValue::Boolean(current.naming.rename),
        SettingValue::Boolean(intent.naming == NamingPreset::Standard),
    );
    change(
        SettingField::ReplaceIllegalCharacters,
        current.naming.id,
        SettingValue::Boolean(current.naming.replace_illegal_characters),
        SettingValue::Boolean(true),
    );
    result.outcome = if result.changes.is_empty() {
        Outcome::Unchanged
    } else {
        Outcome::ChangesProposed
    };
    result.check_size()?;
    Ok(result)
}
impl SettingsPreview {
    pub fn check_size(&self) -> Result<()> {
        struct Bound(usize);
        impl Write for Bound {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.0 {
                    return Err(std::io::Error::other("settings preview bound"));
                }
                self.0 -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Bound(MAX_REPORT_BYTES), self).map_err(|_| SettingsError::Limit)
    }
}
#[cfg(test)]
mod tests;
