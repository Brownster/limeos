//! Media library previews. No credential, filesystem, approval or effect authority.
use super::discovery::ProjectIdentity;
use crate::{ComposeMount, Error, ErrorCode, Result, StorageContract, StorageMediaIdentity};
use serde::Serialize;
use std::io::Write;

pub const MAX_RECORDS: usize = 16;
pub const MAX_PATHS: usize = 8;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_SUMMARY_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaService {
    Sonarr,
    Radarr,
    Lidarr,
    Jellyfin,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    Movies,
    Tv,
    Music,
    Books,
    Audiobooks,
}
impl LibraryKind {
    pub fn path_component(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Tv => "tv",
            Self::Music => "music",
            Self::Books => "books",
            Self::Audiobooks => "audiobooks",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Movies => "Movies",
            Self::Tv => "TV",
            Self::Music => "Music",
            Self::Books => "Books",
            Self::Audiobooks => "Audiobooks",
        }
    }
}
impl MediaService {
    fn kinds(self) -> &'static [LibraryKind] {
        use LibraryKind::*;
        match self {
            Self::Sonarr => &[Tv],
            Self::Radarr => &[Movies],
            Self::Lidarr => &[Music],
            Self::Jellyfin => &[Movies, Tv, Music, Books, Audiobooks],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceIdentity {
    pub stack: String,
    pub engine_id: String,
    pub project: ProjectIdentity,
    pub service: String,
    pub application: MediaService,
}
pub struct SeedContext<'a> {
    pub identity: &'a ServiceIdentity,
    /// Resolved numeric catalog identity (including managed PUID/PGID); this is
    /// not a claim about the running process user or writable filesystem owner.
    pub declared_media_identity: Option<StorageMediaIdentity>,
    /// Complete resolved declarations, including named volumes and nested binds.
    pub mounts: &'a [ComposeMount],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RootFolder {
    pub id: u32,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Library {
    pub name: String,
    pub collection_type: String,
    pub paths: Vec<String>,
}
#[derive(Clone, Copy)]
pub enum Records<'a> {
    Roots(&'a [RootFolder]),
    Libraries(&'a [Library]),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Partial,
}
pub struct Observation<'a> {
    pub identity: &'a ServiceIdentity,
    pub sampled_at: i64,
    pub completeness: Completeness,
    pub records: Records<'a>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Unavailable,
    Stale,
    Future,
    Incomplete,
    IdentityMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Unknown,
    Blocked,
    ReviewRequired,
    ChangesProposed,
    Unchanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    NumericIdentityMismatch,
    OverlappingStorageRoots,
    MissingBinding,
    AmbiguousBinding,
    ShadowedBinding,
    ReadOnlyBinding,
    DownloadLibraryPath,
    DuplicateRootPath,
    DuplicateLibraryKey,
    ExistingLibraryConflict,
    ExistingDownloadLibrary,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedGate {
    ProtectedSourceAndEndpoint,
    FreshCompleteObservation,
    VerifiedMountsAndNumericOwnership,
    LibraryReferencesAndRemovalReview,
    CurrentAuthorityClaimsAndLocks,
    BackupRecoveryAndVerifiedServiceResult,
}
const GATES: [SeedGate; 6] = [
    SeedGate::ProtectedSourceAndEndpoint,
    SeedGate::FreshCompleteObservation,
    SeedGate::VerifiedMountsAndNumericOwnership,
    SeedGate::LibraryReferencesAndRemovalReview,
    SeedGate::CurrentAuthorityClaimsAndLocks,
    SeedGate::BackupRecoveryAndVerifiedServiceResult,
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LibraryPath {
    pub kind: LibraryKind,
    pub host: String,
    pub container: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SeedPreview {
    pub version: u16,
    pub target: ServiceIdentity,
    pub media_identity: StorageMediaIdentity,
    pub generated_at: i64,
    pub sampled_at: Option<i64>,
    pub source: SourceState,
    pub outcome: Outcome,
    pub expected: Vec<LibraryPath>,
    pub additions: Vec<LibraryPath>,
    /// Suggestions only: never a DELETE plan, receipt or safe-removal proof.
    pub removal_candidates: Vec<RootFolder>,
    pub issues: Vec<Issue>,
    pub unresolved_gates: [SeedGate; 6],
}
fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn limit() -> Error {
    Error(ErrorCode::Unavailable)
}
fn check(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid()) }
}
fn name(s: &str, project: bool) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| {
            (b.is_ascii_alphanumeric() && (!project || !b.is_ascii_uppercase()))
                || b"_-".contains(&b)
                || (!project && b == b'.')
        })
}
fn path(s: &str) -> bool {
    s.len() <= 512
        && s.starts_with('/')
        && s.len() > 1
        && !s.chars().any(char::is_control)
        && !["{{", "}}", "${", "$("].iter().any(|p| s.contains(p))
        && s[1..]
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
}
fn normalized(s: &str) -> &str {
    s.trim_end_matches('/')
}
fn under(s: &str, root: &str) -> bool {
    s == root || s.strip_prefix(root).is_some_and(|p| p.starts_with('/'))
}
fn overlaps(a: &str, b: &str) -> bool {
    under(a, b) || under(b, a)
}
fn mount_target(m: &ComposeMount) -> &str {
    match m {
        ComposeMount::Bind { target, .. } | ComposeMount::Volume { target, .. } => target,
    }
}
fn project_valid(p: &ProjectIdentity) -> bool {
    name(&p.name, true)
        && path(&p.working_directory)
        && p.working_directory[1..].split('/').count() <= 16
        && (1..=4).contains(&p.config_files.len())
        && p.config_files.iter().enumerate().all(|(i, f)| {
            path(f)
                && !f.contains(',')
                && f[1..].split('/').count() <= 16
                && !p.config_files[..i].contains(f)
        })
        && !p.working_directory.contains(',')
}
impl ServiceIdentity {
    pub fn validate(&self) -> Result<()> {
        check(scope_valid(
            &self.stack,
            &self.engine_id,
            &self.project,
            &self.service,
        ))
    }
}
pub(super) fn scope_valid(
    stack: &str,
    engine: &str,
    project: &ProjectIdentity,
    service: &str,
) -> bool {
    name(stack, false)
        && name(service, false)
        && project_valid(project)
        && !engine.is_empty()
        && engine.len() <= 128
        && !engine.chars().any(char::is_control)
}
impl RootFolder {
    pub fn validate(&self) -> Result<()> {
        check(
            (1..=i32::MAX as u32).contains(&self.id)
                && self.path.len() <= 512
                && path(normalized(&self.path)),
        )
    }
}
impl Library {
    pub fn validate(&self) -> Result<()> {
        check(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control)
                && !self.collection_type.is_empty()
                && self.collection_type.len() <= 128
                && !self.collection_type.chars().any(char::is_control)
                && self.paths.len() <= MAX_PATHS,
        )?;
        for (i, p) in self.paths.iter().enumerate() {
            check(
                p.len() <= 512
                    && path(normalized(p))
                    && !self.paths[..i]
                        .iter()
                        .any(|v| normalized(v) == normalized(p)),
            )?;
        }
        Ok(())
    }
}
fn identity_text(id: &ServiceIdentity) -> Vec<&str> {
    let mut values = vec![
        id.stack.as_str(),
        &id.engine_id,
        &id.service,
        &id.project.name,
        &id.project.working_directory,
    ];
    values.extend(id.project.config_files.iter().map(String::as_str));
    values
}
fn admit(
    contract: &StorageContract,
    context: &SeedContext<'_>,
    observation: Option<&Observation<'_>>,
) -> Result<()> {
    contract.validate()?;
    check(path(&contract.locations.media_host) && path(&contract.locations.downloads_host))?;
    context.identity.validate()?;
    check(context.mounts.len() <= 8)?;
    let mut text: usize = 0;
    let mut charge = |s: &str| -> Result<()> {
        text = text.checked_add(s.len()).ok_or_else(limit)?;
        if text > MAX_TEXT_BYTES {
            return Err(limit());
        }
        Ok(())
    };
    for s in [
        &contract.locations.media_host,
        &contract.locations.downloads_host,
        &contract.locations.application_config_host,
        &contract.locations.backup_host,
    ] {
        charge(s)?;
    }
    for d in &contract.devices {
        for s in [&d.id, &d.filesystem_uuid, &d.mountpoint] {
            charge(s)?;
        }
        if let Some(s) = &d.serial {
            charge(s)?;
        }
    }
    for s in identity_text(context.identity) {
        charge(s)?;
    }
    for (i, mount) in context.mounts.iter().enumerate() {
        check(
            path(mount_target(mount))
                && !context.mounts[..i]
                    .iter()
                    .any(|m| mount_target(m) == mount_target(mount)),
        )?;
        charge(mount_target(mount))?;
        match mount {
            ComposeMount::Bind { source, .. } => {
                check(path(source))?;
                charge(source)?;
            }
            ComposeMount::Volume { name: n, .. } => {
                check(name(n, false))?;
                charge(n)?;
            }
        }
    }
    if let Some(observation) = observation {
        observation.identity.validate()?;
        check((0..=MAX_TIME).contains(&observation.sampled_at))?;
        for s in identity_text(observation.identity) {
            charge(s)?;
        }
        match observation.records {
            Records::Roots(rows) => {
                check(
                    context.identity.application != MediaService::Jellyfin
                        && rows.len() <= MAX_RECORDS,
                )?;
                for (i, row) in rows.iter().enumerate() {
                    row.validate()?;
                    check(!rows[..i].iter().any(|r| r.id == row.id))?;
                    charge(&row.path)?;
                }
            }
            Records::Libraries(rows) => {
                check(
                    context.identity.application == MediaService::Jellyfin
                        && rows.len() <= MAX_RECORDS,
                )?;
                for row in rows {
                    row.validate()?;
                    charge(&row.name)?;
                    charge(&row.collection_type)?;
                    for p in &row.paths {
                        charge(p)?;
                    }
                }
            }
        }
    }
    Ok(())
}
fn map_library(
    host: &str,
    context: &SeedContext<'_>,
    downloads: &str,
) -> std::result::Result<String, Issue> {
    let mut candidates = Vec::new();
    for (i, mount) in context.mounts.iter().enumerate() {
        if let ComposeMount::Bind {
            source,
            target,
            read_only,
        } = mount
            && under(host, source)
        {
            if context.identity.application != MediaService::Jellyfin && *read_only {
                return Err(Issue::ReadOnlyBinding);
            }
            let container = format!("{}{}", target, &host[source.len()..]);
            if !path(&container) {
                return Err(Issue::MissingBinding);
            }
            if under(&container, "/downloads")
                || under(&container, "/data/downloads")
                || under(host, downloads)
            {
                return Err(Issue::DownloadLibraryPath);
            }
            // A nested/ancestor mount can obscure the declared library subtree.
            if context
                .mounts
                .iter()
                .enumerate()
                .any(|(j, m)| j != i && overlaps(mount_target(m), &container))
            {
                return Err(Issue::ShadowedBinding);
            }
            candidates.push(container);
        }
    }
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(Issue::MissingBinding),
        _ => Err(Issue::AmbiguousBinding),
    }
}
fn forbidden_paths(contract: &StorageContract, mounts: &[ComposeMount]) -> Vec<String> {
    let downloads = &contract.locations.downloads_host;
    let mut paths = vec!["/downloads".into(), "/data/downloads".into()];
    for mount in mounts {
        if let ComposeMount::Bind { source, target, .. } = mount {
            if under(downloads, source) {
                paths.push(format!("{}{}", target, &downloads[source.len()..]));
            } else if under(source, downloads) {
                paths.push(target.clone());
            }
        }
    }
    paths
}

/// Deterministic preview only. An unchanged preview still has all unresolved gates.
pub fn preview(
    now: i64,
    contract: &StorageContract,
    context: SeedContext<'_>,
    observation: Option<Observation<'_>>,
) -> Result<SeedPreview> {
    check((0..=MAX_TIME).contains(&now))?;
    admit(contract, &context, observation.as_ref())?;
    let source = match observation.as_ref() {
        None => SourceState::Unavailable,
        Some(o) if o.identity != context.identity => SourceState::IdentityMismatch,
        Some(o) if o.sampled_at > now => SourceState::Future,
        Some(o) if now - o.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(o) if o.completeness != Completeness::Complete => SourceState::Incomplete,
        Some(_) => SourceState::Fresh,
    };
    let mut result = SeedPreview {
        version: 1,
        target: context.identity.clone(),
        media_identity: contract.media_identity.clone(),
        generated_at: now,
        sampled_at: observation.as_ref().map(|o| o.sampled_at),
        source,
        outcome: Outcome::Unknown,
        expected: vec![],
        additions: vec![],
        removal_candidates: vec![],
        issues: vec![],
        unresolved_gates: GATES,
    };
    if context.declared_media_identity.as_ref() != Some(&contract.media_identity) {
        result.issues.push(Issue::NumericIdentityMismatch);
    }
    if overlaps(
        &contract.locations.media_host,
        &contract.locations.downloads_host,
    ) {
        result.issues.push(Issue::OverlappingStorageRoots);
    }
    for &kind in context.identity.application.kinds() {
        let host = format!(
            "{}/{}",
            contract.locations.media_host,
            kind.path_component()
        );
        check(path(&host))?;
        match map_library(&host, &context, &contract.locations.downloads_host) {
            Ok(container) => result.expected.push(LibraryPath {
                kind,
                host,
                container,
            }),
            Err(issue) => {
                if !result.issues.contains(&issue) {
                    result.issues.push(issue);
                }
            }
        }
    }
    if !result.issues.is_empty() {
        result.outcome = Outcome::Blocked;
    } else if source == SourceState::Fresh {
        let forbidden = forbidden_paths(contract, context.mounts);
        match observation.unwrap().records {
            Records::Roots(rows) => {
                if rows.iter().enumerate().any(|(i, row)| {
                    rows[..i]
                        .iter()
                        .any(|r| normalized(&r.path) == normalized(&row.path))
                }) {
                    result.issues.push(Issue::DuplicateRootPath);
                }
                for row in rows {
                    if forbidden.iter().any(|p| under(normalized(&row.path), p)) {
                        result.removal_candidates.push(row.clone());
                    }
                }
                for expected in &result.expected {
                    if !rows
                        .iter()
                        .any(|r| normalized(&r.path) == expected.container)
                    {
                        result.additions.push(expected.clone());
                    }
                }
            }
            Records::Libraries(rows) => {
                if rows.iter().enumerate().any(|(i, row)| {
                    rows[..i].iter().any(|r| {
                        r.name.eq_ignore_ascii_case(&row.name)
                            && r.collection_type.eq_ignore_ascii_case(&row.collection_type)
                    })
                }) {
                    result.issues.push(Issue::DuplicateLibraryKey);
                }
                for expected in &result.expected {
                    let matches: Vec<_> = rows
                        .iter()
                        .filter(|r| r.name.eq_ignore_ascii_case(expected.kind.name()))
                        .collect();
                    match matches.as_slice() {
                        [] => result.additions.push(expected.clone()),
                        [row]
                            if row
                                .collection_type
                                .eq_ignore_ascii_case(expected.kind.path_component()) =>
                        {
                            if row
                                .paths
                                .iter()
                                .any(|p| forbidden.iter().any(|f| under(normalized(p), f)))
                            {
                                result.issues.push(Issue::ExistingDownloadLibrary);
                            } else if !row
                                .paths
                                .iter()
                                .any(|p| normalized(p) == expected.container)
                            {
                                result.issues.push(Issue::ExistingLibraryConflict);
                            }
                        }
                        _ => result.issues.push(Issue::ExistingLibraryConflict),
                    }
                }
            }
        }
        result.removal_candidates.sort_by_key(|r| r.id);
        if !result.issues.is_empty() {
            result.outcome = Outcome::Blocked;
            result.additions.clear();
            result.removal_candidates.clear();
        } else if !result.removal_candidates.is_empty() {
            result.outcome = Outcome::ReviewRequired;
        } else if !result.additions.is_empty() {
            result.outcome = Outcome::ChangesProposed;
        } else {
            result.outcome = Outcome::Unchanged;
        }
    }
    result.check_size()?;
    Ok(result)
}
impl SeedPreview {
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_SUMMARY_BYTES - self.0 {
                    return Err(std::io::Error::other("media preview bound"));
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
#[cfg(test)]
mod tests;
