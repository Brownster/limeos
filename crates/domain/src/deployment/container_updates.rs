//! Diagnostic update drafts and post-update comparisons; no effects or receipts.
use super::discovery::{
    self, CatalogInput, EngineInput, FileStatus, ProjectIdentity, SourceState, SourceStatus,
    StackInventory, StackView,
};
use crate::{Error, ErrorCode, Result, compose_name, opaque_id};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, io::Write};

pub const MAX_STACKS: usize = 8;
pub const MAX_SERVICES: usize = 16;
pub const MAX_IMAGES: usize = 16;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulePreset {
    Disabled,
    #[serde(rename = "daily_4am")]
    Daily4am,
    #[serde(rename = "weekly_sunday_4am")]
    WeeklySunday4am,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    pub version: u16,
    pub enabled: bool,
    pub schedule_preset: SchedulePreset,
    pub excluded_stacks: Vec<String>,
    pub notify_on_update: bool,
}
impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            schedule_preset: SchedulePreset::Disabled,
            excluded_stacks: vec![],
            notify_on_update: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Manual,
    Scheduled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    LinuxAmd64,
    LinuxArm64V8,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageSource {
    Tag {
        repository: String,
        tag: String,
    },
    Digest {
        repository: String,
        manifest_digest: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceTrack {
    pub service: String,
    pub source: ImageSource,
    pub platform: Platform,
    pub replicas: u16,
}
/// Proposed complete supported declarations, not trusted source/registry owners.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackUpdateSpec {
    pub stack: String,
    pub project: ProjectIdentity,
    pub services: Vec<ServiceTrack>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageResolution {
    pub engine_id: String,
    pub stack: String,
    pub project: ProjectIdentity,
    pub service: String,
    pub source: ImageSource,
    pub platform: Platform,
    pub manifest_digest: String,
    pub config_digest: String,
    pub sampled_at: i64,
}
pub struct UpdateInput<'a> {
    pub config: Option<&'a UpdateConfig>,
    pub mode: RunMode,
    pub selected: &'a [String],
    pub platform: Platform,
    pub specs: &'a [StackUpdateSpec],
    pub catalog: Option<CatalogInput<'a>>,
    pub engine: Option<EngineInput<'a>>,
    pub images: &'a [ImageResolution],
    pub now: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Excluded,
    ScheduleDisabled,
    Unknown,
    Blocked,
    Current,
    UpdateProposed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    ConfigUnknown,
    CatalogUnavailable,
    MembershipUnknown,
    StackMissing,
    RegistrationDrift,
    SourceConflict,
    UnsupportedSource,
    RuntimeNotRunning,
    ActiveOneOff,
    ReplicaDrift,
    ResolutionUnavailable,
    ResolutionStale,
    ResolutionMismatch,
    PlatformMismatch,
    PinnedSourceMoved,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceTarget {
    pub service: String,
    pub source: ImageSource,
    pub platform: Platform,
    pub replicas: u16,
    pub manifest_digest: String,
    pub config_digest: String,
    pub resolution_sampled_at: i64,
    pub recreate: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StackUpdate {
    pub stack: String,
    pub project: Option<ProjectIdentity>,
    pub decision: Decision,
    pub issue: Option<Issue>,
    pub services: Vec<ServiceTarget>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateGate {
    CurrentSourceAndResolutionOwners,
    SupportedSourceAndCompleteClaims,
    VerifiedImageOriginAndPlatform,
    FreshAuthorityAndRecurringPolicy,
    CommonLocksAndStorageReadiness,
    DurableJobAndAtomicManagedFileRecovery,
    IndependentRuntimeAndHealthVerification,
    TimezoneAndScheduleMigration,
}
const GATES: &[UpdateGate] = &[
    UpdateGate::CurrentSourceAndResolutionOwners,
    UpdateGate::SupportedSourceAndCompleteClaims,
    UpdateGate::VerifiedImageOriginAndPlatform,
    UpdateGate::FreshAuthorityAndRecurringPolicy,
    UpdateGate::CommonLocksAndStorageReadiness,
    UpdateGate::DurableJobAndAtomicManagedFileRecovery,
    UpdateGate::IndependentRuntimeAndHealthVerification,
    UpdateGate::TimezoneAndScheduleMigration,
];
/// Opaque proposal only: no Deserialize, effect method, approval flag or receipt.
pub struct UpdatePlan {
    generated_at: i64,
    engine_id: Option<String>,
    config: Option<UpdateConfig>,
    mode: RunMode,
    platform: Platform,
    before: StackInventory,
    rows: Vec<StackUpdate>,
}
impl fmt::Debug for UpdatePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdatePlan")
            .field("stacks", &self.rows.len())
            .finish_non_exhaustive()
    }
}
#[derive(Serialize)]
pub struct SelectedBefore<'a> {
    pub catalog: &'a SourceStatus,
    pub engine: &'a SourceStatus,
    pub engine_matches_catalog: Option<bool>,
    pub stacks: Vec<&'a StackView>,
}
#[derive(Serialize)]
pub struct UpdateSummary<'a> {
    pub version: u16,
    pub generated_at: i64,
    pub engine_id: Option<&'a str>,
    pub config: Option<&'a UpdateConfig>,
    pub mode: RunMode,
    pub platform: Platform,
    pub before: SelectedBefore<'a>,
    pub rows: &'a [StackUpdate],
    pub unresolved: &'static [UpdateGate],
}
impl UpdatePlan {
    pub fn summary(&self) -> UpdateSummary<'_> {
        UpdateSummary {
            version: 1,
            generated_at: self.generated_at,
            engine_id: self.engine_id.as_deref(),
            config: self.config.as_ref(),
            mode: self.mode,
            platform: self.platform,
            before: SelectedBefore {
                catalog: &self.before.catalog,
                engine: &self.before.engine,
                engine_matches_catalog: self.before.engine_matches_catalog,
                stacks: self
                    .before
                    .stacks
                    .iter()
                    .filter(|s| self.rows.iter().any(|row| row.stack == s.stack))
                    .collect(),
            },
            rows: &self.rows,
            unresolved: GATES,
        }
    }
}
fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn limit() -> Error {
    Error(ErrorCode::Unavailable)
}
fn alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(opaque_id)
}
fn repository(value: &str) -> bool {
    if value.is_empty() || value.len() > 192 || value.split('/').count() > 8 {
        return false;
    }
    let pieces: Vec<_> = value.split('/').collect();
    pieces.iter().enumerate().all(|(i, part)| {
        let name = if let Some((host, port)) = part.split_once(':') {
            if i != 0
                || pieces.len() < 2
                || port.is_empty()
                || !port.bytes().all(|b| b.is_ascii_digit())
                || !port.parse::<u16>().is_ok_and(|p| p != 0)
            {
                return false;
            }
            host
        } else {
            part
        };
        !matches!(name, "" | "." | "..")
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    })
}
fn charge(text: &mut usize, value: &str) -> Result<()> {
    *text = text.checked_add(value.len()).ok_or_else(limit)?;
    if *text > MAX_TEXT_BYTES {
        return Err(limit());
    }
    Ok(())
}
fn checked(text: &mut usize, value: &str, valid: bool) -> Result<()> {
    if !valid {
        return Err(invalid());
    }
    charge(text, value)
}
fn identity(project: &ProjectIdentity, text: &mut usize) -> Result<()> {
    checked(
        text,
        &project.name,
        alias(&project.name) && !project.name.bytes().any(|b| b.is_ascii_uppercase()),
    )?;
    if !(1..=4).contains(&project.config_files.len()) {
        return Err(limit());
    }
    for path in std::iter::once(&project.working_directory).chain(&project.config_files) {
        checked(
            text,
            path,
            path.starts_with('/')
                && path.len() <= 512
                && !path.contains([',', '$', '{', '}', '\\'])
                && !path.chars().any(char::is_control)
                && path[1..].split('/').count() <= 16
                && path[1..]
                    .split('/')
                    .all(|s| !matches!(s, "" | "." | "..") && s.len() <= 255),
        )?;
    }
    let unique: BTreeSet<_> = project.config_files.iter().collect();
    if unique.len() != project.config_files.len() {
        return Err(invalid());
    }
    Ok(())
}
fn image_source(source: &ImageSource, text: &mut usize) -> Result<()> {
    let (repo, selector, valid) = match source {
        ImageSource::Tag { repository, tag } => (
            repository,
            tag,
            !tag.is_empty()
                && tag.len() <= 128
                && tag.as_bytes()[0].is_ascii_alphanumeric()
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        ),
        ImageSource::Digest {
            repository,
            manifest_digest,
        } => (repository, manifest_digest, digest(manifest_digest)),
    };
    checked(text, repo, repository(repo))?;
    checked(text, selector, valid)
}
fn admit(input: &UpdateInput<'_>) -> Result<()> {
    if !(1..=MAX_STACKS).contains(&input.selected.len())
        || input.specs.len() > MAX_STACKS
        || input.images.len() > MAX_IMAGES
    {
        return Err(limit());
    }
    let mut text = 0;
    let mut selected = BTreeSet::new();
    for stack in input.selected {
        checked(&mut text, stack, alias(stack))?;
        if !selected.insert(stack) {
            return Err(invalid());
        }
    }
    if let Some(config) = input.config {
        if config.version != 1 {
            return Err(invalid());
        }
        if config.excluded_stacks.len() > MAX_STACKS {
            return Err(limit());
        }
        let mut excluded = BTreeSet::new();
        for stack in &config.excluded_stacks {
            checked(&mut text, stack, alias(stack))?;
            if !excluded.insert(stack) {
                return Err(invalid());
            }
        }
    }
    let mut specs = BTreeSet::new();
    let mut service_count = 0;
    for spec in input.specs {
        checked(&mut text, &spec.stack, alias(&spec.stack))?;
        if !specs.insert(&spec.stack) {
            return Err(invalid());
        }
        identity(&spec.project, &mut text)?;
        if !(1..=MAX_SERVICES).contains(&spec.services.len()) {
            return Err(limit());
        }
        if selected.contains(&spec.stack) {
            service_count += spec.services.len();
        }
        let mut names = BTreeSet::new();
        for service in &spec.services {
            checked(&mut text, &service.service, compose_name(&service.service))?;
            if !names.insert(&service.service) || !(1..=16).contains(&service.replicas) {
                return Err(invalid());
            }
            image_source(&service.source, &mut text)?;
        }
    }
    if service_count > MAX_SERVICES {
        return Err(limit());
    }
    let mut resolutions = BTreeSet::new();
    for image in input.images {
        checked(&mut text, &image.stack, alias(&image.stack))?;
        checked(&mut text, &image.service, compose_name(&image.service))?;
        checked(&mut text, &image.engine_id, alias(&image.engine_id))?;
        if !resolutions.insert((&image.stack, &image.service))
            || !(0..=MAX_TIME).contains(&image.sampled_at)
        {
            return Err(invalid());
        }
        identity(&image.project, &mut text)?;
        image_source(&image.source, &mut text)?;
        checked(
            &mut text,
            &image.manifest_digest,
            digest(&image.manifest_digest),
        )?;
        checked(
            &mut text,
            &image.config_digest,
            digest(&image.config_digest),
        )?;
    }
    Ok(())
}
fn same_project(view: &StackView, identity: &ProjectIdentity) -> bool {
    view.project == identity.name
        && view.working_directory == identity.working_directory
        && view.config_files == identity.config_files
}
fn targets(
    spec: &StackUpdateSpec,
    view: &StackView,
    images: &[ImageResolution],
    engine_id: &str,
    platform: Platform,
    now: i64,
) -> std::result::Result<Vec<ServiceTarget>, Issue> {
    if !same_project(view, &spec.project) || !view.issues.is_empty() {
        return Err(Issue::RegistrationDrift);
    }
    match view.files {
        FileStatus::Conflict { .. } => return Err(Issue::SourceConflict),
        FileStatus::Unsupported { .. } => return Err(Issue::UnsupportedSource),
        FileStatus::Unavailable => return Err(Issue::CatalogUnavailable),
        FileStatus::NotInspected => {} // Source admission stays an unresolved gate.
    }
    let declared: BTreeSet<_> = view
        .services
        .iter()
        .filter(|s| s.declared)
        .map(|s| &s.name)
        .collect();
    let tracked: BTreeSet<_> = spec.services.iter().map(|s| &s.service).collect();
    if declared != tracked {
        return Err(Issue::RegistrationDrift);
    }
    if view
        .services
        .iter()
        .flat_map(|s| &s.members)
        .any(|m| m.one_off && m.container.running)
    {
        return Err(Issue::ActiveOneOff);
    }
    let mut result = Vec::new();
    for track in &spec.services {
        if track.platform != platform {
            return Err(Issue::PlatformMismatch);
        }
        let service = view
            .services
            .iter()
            .find(|s| s.name == track.service)
            .ok_or(Issue::RegistrationDrift)?;
        let counts = service.counts.ok_or(Issue::MembershipUnknown)?;
        if counts.ordinary == 0 || counts.running != counts.ordinary {
            return Err(Issue::RuntimeNotRunning);
        }
        if counts.ordinary != track.replicas {
            return Err(Issue::ReplicaDrift);
        }
        let image = images
            .iter()
            .find(|i| i.stack == spec.stack && i.service == track.service)
            .ok_or(Issue::ResolutionUnavailable)?;
        if image.sampled_at > now || now - image.sampled_at > discovery::MAX_AGE_SECONDS {
            return Err(Issue::ResolutionStale);
        }
        if image.engine_id != engine_id
            || image.project != spec.project
            || image.source != track.source
        {
            return Err(Issue::ResolutionMismatch);
        }
        if image.platform != platform {
            return Err(Issue::PlatformMismatch);
        }
        if matches!(&track.source, ImageSource::Digest {manifest_digest,..} if *manifest_digest != image.manifest_digest)
        {
            return Err(Issue::PinnedSourceMoved);
        }
        result.push(ServiceTarget {
            service: track.service.clone(),
            source: track.source.clone(),
            platform,
            replicas: track.replicas,
            manifest_digest: image.manifest_digest.clone(),
            config_digest: image.config_digest.clone(),
            resolution_sampled_at: image.sampled_at,
            recreate: service
                .members
                .iter()
                .any(|m| !m.one_off && m.container.image != image.config_digest),
        });
    }
    result.sort_by(|a, b| a.service.cmp(&b.service));
    Ok(result)
}
fn failure(row: &mut StackUpdate, issue: Issue) {
    row.decision = if matches!(
        issue,
        Issue::ConfigUnknown
            | Issue::CatalogUnavailable
            | Issue::MembershipUnknown
            | Issue::ResolutionUnavailable
            | Issue::ResolutionStale
    ) {
        Decision::Unknown
    } else {
        Decision::Blocked
    };
    row.issue = Some(issue);
}
struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_REPORT_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("update report bound"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded<T: Serialize>(value: &T) -> Result<()> {
    serde_json::to_writer(Count(0), value).map_err(|_| limit())
}
pub fn plan_updates(input: UpdateInput<'_>) -> Result<UpdatePlan> {
    admit(&input)?;
    let engine_id = input.catalog.as_ref().map(|c| c.engine_id);
    let before = discovery::discover(input.now, input.catalog, input.engine)?;
    let mut rows = Vec::new();
    for stack in input.selected {
        let spec = input.specs.iter().find(|s| s.stack == *stack);
        let mut row = StackUpdate {
            stack: stack.clone(),
            project: spec.map(|s| s.project.clone()),
            decision: Decision::Unknown,
            issue: None,
            services: vec![],
        };
        match input.config {
            None => failure(&mut row, Issue::ConfigUnknown),
            Some(c) if c.excluded_stacks.contains(stack) => row.decision = Decision::Excluded,
            Some(c)
                if input.mode == RunMode::Scheduled
                    && (!c.enabled || c.schedule_preset == SchedulePreset::Disabled) =>
            {
                row.decision = Decision::ScheduleDisabled
            }
            Some(_) if before.catalog.state != SourceState::Fresh => {
                failure(&mut row, Issue::CatalogUnavailable)
            }
            Some(_)
                if before.engine.state != SourceState::Fresh
                    || before.engine_matches_catalog != Some(true) =>
            {
                failure(&mut row, Issue::MembershipUnknown)
            }
            Some(_) => {
                let view = before.stacks.iter().find(|s| s.stack == *stack);
                match (spec, view) {
                    (Some(spec), Some(view)) => match targets(
                        spec,
                        view,
                        input.images,
                        engine_id.unwrap(),
                        input.platform,
                        input.now,
                    ) {
                        Ok(services) => {
                            row.decision = if services.iter().any(|s| s.recreate) {
                                Decision::UpdateProposed
                            } else {
                                Decision::Current
                            };
                            row.services = services;
                        }
                        Err(issue) => failure(&mut row, issue),
                    },
                    _ => failure(&mut row, Issue::StackMissing),
                }
            }
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| a.stack.cmp(&b.stack));
    let mut config = input.config.cloned();
    if let Some(c) = &mut config {
        c.excluded_stacks.sort();
    }
    let plan = UpdatePlan {
        generated_at: input.now,
        engine_id: engine_id.map(Into::into),
        config,
        mode: input.mode,
        platform: input.platform,
        before,
        rows,
    };
    bounded(&plan.summary())?;
    Ok(plan)
}

pub struct PostInput<'a> {
    pub now: i64,
    pub not_before: i64,
    pub catalog: Option<CatalogInput<'a>>,
    pub engine: Option<EngineInput<'a>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    NotApplicable,
    Unknown,
    Diverged,
    ObservedConverged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PostIssue {
    Unavailable,
    PredatesDispatch,
    EngineChanged,
    IdentityDrift,
    MembershipDrift,
    StoppedReplica,
    ImageMismatch,
    OneOffChanged,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PostStackObservation {
    pub stack: String,
    pub status: ObservationStatus,
    pub issue: Option<PostIssue>,
}
#[derive(Serialize)]
pub struct PostAssessment {
    pub version: u16,
    pub generated_at: i64,
    pub catalog_sampled_at: Option<i64>,
    pub engine_sampled_at: Option<i64>,
    pub rows: Vec<PostStackObservation>,
    pub unresolved: &'static [UpdateGate],
}
fn compare_stack(
    row: &StackUpdate,
    before: &StackView,
    after: &StackView,
) -> std::result::Result<(), PostIssue> {
    if row.project.as_ref().is_none_or(|p| !same_project(after, p)) || !after.issues.is_empty() {
        return Err(PostIssue::IdentityDrift);
    }
    if !matches!(after.files, FileStatus::NotInspected) {
        return Err(PostIssue::IdentityDrift);
    }
    let declared: BTreeSet<_> = after
        .services
        .iter()
        .filter(|s| s.declared)
        .map(|s| &s.name)
        .collect();
    let expected: BTreeSet<_> = row.services.iter().map(|s| &s.service).collect();
    if declared != expected {
        return Err(PostIssue::MembershipDrift);
    }
    // Compare identities and complete selected snapshots, not counts alone.
    let old: Vec<_> = before
        .services
        .iter()
        .flat_map(|s| {
            s.members
                .iter()
                .filter(|m| m.one_off)
                .map(move |m| (&s.name, &m.container))
        })
        .collect();
    let new: Vec<_> = after
        .services
        .iter()
        .flat_map(|s| {
            s.members
                .iter()
                .filter(|m| m.one_off)
                .map(move |m| (&s.name, &m.container))
        })
        .collect();
    if old.len() != new.len() || old.iter().any(|value| !new.contains(value)) {
        return Err(PostIssue::OneOffChanged);
    }
    for target in &row.services {
        let service = after
            .services
            .iter()
            .find(|s| s.name == target.service)
            .ok_or(PostIssue::MembershipDrift)?;
        let counts = service.counts.ok_or(PostIssue::Unavailable)?;
        if counts.ordinary != target.replicas {
            return Err(PostIssue::MembershipDrift);
        }
        if counts.running != counts.ordinary {
            return Err(PostIssue::StoppedReplica);
        }
        if service
            .members
            .iter()
            .any(|m| !m.one_off && m.container.image != target.config_digest)
        {
            return Err(PostIssue::ImageMismatch);
        }
    }
    Ok(())
}
/// Pure comparison only. No process result, receipt or authority is accepted here.
pub fn assess_post_update(plan: &UpdatePlan, input: PostInput<'_>) -> Result<PostAssessment> {
    if input.not_before < plan.generated_at
        || input.not_before > input.now
        || !(0..=MAX_TIME).contains(&input.not_before)
    {
        return Err(invalid());
    }
    let engine_matches = input
        .catalog
        .as_ref()
        .is_some_and(|c| Some(c.engine_id) == plan.engine_id.as_deref())
        && input
            .engine
            .as_ref()
            .is_some_and(|e| Some(e.engine_id) == plan.engine_id.as_deref());
    let after = discovery::discover(input.now, input.catalog, input.engine)?;
    let fresh = after.catalog.state == SourceState::Fresh
        && after.engine.state == SourceState::Fresh
        && after.engine_matches_catalog == Some(true);
    let later = after.catalog.sampled_at.is_some_and(|t| {
        t >= input.not_before && plan.before.catalog.sampled_at.is_some_and(|old| t > old)
    }) && after.engine.sampled_at.is_some_and(|t| {
        t >= input.not_before && plan.before.engine.sampled_at.is_some_and(|old| t > old)
    });
    let mut rows = Vec::new();
    for row in &plan.rows {
        let mut observation = PostStackObservation {
            stack: row.stack.clone(),
            status: ObservationStatus::NotApplicable,
            issue: None,
        };
        if row.decision == Decision::UpdateProposed {
            let result = if !fresh {
                Err(PostIssue::Unavailable)
            } else if !engine_matches {
                Err(PostIssue::EngineChanged)
            } else if !later {
                Err(PostIssue::PredatesDispatch)
            } else {
                match (
                    plan.before.stacks.iter().find(|s| s.stack == row.stack),
                    after.stacks.iter().find(|s| s.stack == row.stack),
                ) {
                    (Some(before), Some(after)) => compare_stack(row, before, after),
                    _ => Err(PostIssue::IdentityDrift),
                }
            };
            match result {
                Ok(()) => observation.status = ObservationStatus::ObservedConverged,
                Err(issue) => {
                    observation.status =
                        if matches!(issue, PostIssue::Unavailable | PostIssue::PredatesDispatch) {
                            ObservationStatus::Unknown
                        } else {
                            ObservationStatus::Diverged
                        };
                    observation.issue = Some(issue);
                }
            }
        }
        rows.push(observation);
    }
    let report = PostAssessment {
        version: 1,
        generated_at: input.now,
        catalog_sampled_at: after.catalog.sampled_at,
        engine_sampled_at: after.engine.sampled_at,
        rows,
        unresolved: GATES,
    };
    bounded(&report)?;
    Ok(report)
}

#[cfg(test)]
mod tests;
