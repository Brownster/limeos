use crate::{ComposeFileName, ContainerSnapshot, Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Write};

pub const MAX_STACKS: usize = 8;
pub const MAX_SERVICES: usize = 16;
pub const MAX_MEMBERS: usize = 16;
pub const MAX_INPUT_TEXT: usize = 48 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedSource {
    Yaml,
    Environment,
    Include,
    Interpolation,
    Other,
}

/// Diagnostic facts only. No variant makes stack actions executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileStatus {
    NotInspected,
    Conflict { candidates: Vec<ComposeFileName> },
    Unsupported { reason: UnsupportedSource },
    Unavailable,
}

/// Plain input values, not retained evidence or a wire admission decoder.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub stack: String,
    pub project: String,
    pub working_directory: String,
    pub config_files: Vec<String>,
    pub services: Vec<String>,
    pub files: FileStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectIdentity {
    pub name: String,
    pub working_directory: String,
    pub config_files: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Membership {
    Standalone {
        container: ContainerSnapshot,
    },
    Compose {
        container: ContainerSnapshot,
        project: ProjectIdentity,
        service: String,
        one_off: bool,
    },
}
impl Membership {
    fn container(&self) -> &ContainerSnapshot {
        match self {
            Self::Standalone { container } | Self::Compose { container, .. } => container,
        }
    }
}

pub struct CatalogInput<'a> {
    pub engine_id: &'a str,
    pub sampled_at: i64,
    pub stacks: &'a [Declaration],
}
pub struct EngineInput<'a> {
    pub engine_id: &'a str,
    pub sampled_at: i64,
    /// Complete membership, including stopped, one-off and standalone containers.
    pub members: &'a [Membership],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Stale,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceStatus {
    pub state: SourceState,
    pub sampled_at: Option<i64>,
    pub max_age_seconds: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Unknown,
    NoContainers,
    Stopped,
    Running,
    Partial,
    Conflict,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub ordinary: u16,
    pub running: u16,
    pub one_offs: u16,
    pub one_off_running: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemberView {
    pub container: ContainerSnapshot,
    pub one_off: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceView {
    pub name: String,
    pub declared: bool,
    pub runtime: RuntimeStatus,
    pub counts: Option<Counts>,
    pub members: Vec<MemberView>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StackIssue {
    IdentityConflict,
    OverlappingProject,
    UndeclaredServices,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StackView {
    pub stack: String,
    pub project: String,
    pub working_directory: String,
    pub config_files: Vec<String>,
    pub files: FileStatus,
    pub runtime: RuntimeStatus,
    pub counts: Option<Counts>,
    pub missing_services: Option<u16>,
    pub services: Vec<ServiceView>,
    pub issues: Vec<StackIssue>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationRelation {
    NotRegistered,
    CatalogUnknown,
    EngineMismatch,
    IdentityConflict,
    Overlapping,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OtherProjectView {
    pub project: ProjectIdentity,
    pub relation: RegistrationRelation,
    pub services: Vec<ServiceView>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StackInventory {
    pub version: u16,
    pub generated_at: i64,
    pub catalog: SourceStatus,
    pub engine: SourceStatus,
    pub engine_matches_catalog: Option<bool>,
    pub stacks: Vec<StackView>,
    pub other_projects: Vec<OtherProjectView>,
    pub standalone_containers: Option<u16>,
}

fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn limit() -> Error {
    Error(ErrorCode::Unavailable)
}
fn name(value: &str, project: bool) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|b| {
            (b.is_ascii_alphanumeric() && (!project || !b.is_ascii_uppercase()))
                || b"_-".contains(&b)
                || (!project && b == b'.')
        })
}
fn path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 512
        && !value.contains(',')
        && !value.chars().any(char::is_control)
        && value[1..].split('/').count() <= 16
        && value[1..]
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
}
fn overlap(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|s| s.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|s| s.starts_with('/'))
}
fn files_valid(files: &[String]) -> bool {
    (1..=4).contains(&files.len())
        && files
            .iter()
            .enumerate()
            .all(|(i, f)| path(f) && !files[..i].contains(f))
}
fn matches(declaration: &Declaration, project: &ProjectIdentity) -> bool {
    declaration.project == project.name
        && declaration.working_directory == project.working_directory
        && declaration.config_files == project.config_files
}
fn overlaps(declaration: &Declaration, project: &ProjectIdentity) -> bool {
    overlap(&declaration.working_directory, &project.working_directory)
        || project
            .config_files
            .iter()
            .any(|f| overlap(f, &declaration.working_directory))
        || declaration
            .config_files
            .iter()
            .any(|f| overlap(f, &project.working_directory))
}
fn source(at: Option<i64>, now: i64) -> Result<SourceStatus> {
    if at.is_some_and(|at| !(0..=MAX_TIME).contains(&at)) {
        return Err(invalid());
    }
    Ok(SourceStatus {
        state: match at {
            None => SourceState::Unavailable,
            Some(at) if at > now || now - at > MAX_AGE_SECONDS => SourceState::Stale,
            Some(_) => SourceState::Fresh,
        },
        sampled_at: at,
        max_age_seconds: MAX_AGE_SECONDS,
    })
}
fn admit(catalog: Option<&CatalogInput<'_>>, engine: Option<&EngineInput<'_>>) -> Result<()> {
    let mut bytes = 0_usize;
    let mut charge = |value: &str| -> Result<()> {
        bytes = bytes.checked_add(value.len()).ok_or_else(limit)?;
        if bytes > MAX_INPUT_TEXT {
            return Err(limit());
        }
        Ok(())
    };
    if let Some(catalog) = catalog {
        if catalog.stacks.len() > MAX_STACKS {
            return Err(limit());
        }
        if catalog.engine_id.is_empty()
            || catalog.engine_id.len() > 128
            || catalog.engine_id.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        charge(catalog.engine_id)?;
        for (i, stack) in catalog.stacks.iter().enumerate() {
            if !name(&stack.stack, false)
                || !name(&stack.project, true)
                || !path(&stack.working_directory)
                || !files_valid(&stack.config_files)
                || stack.services.is_empty()
                || stack.services.len() > MAX_SERVICES
                || catalog.stacks[..i].iter().any(|s| {
                    s.stack == stack.stack
                        || s.project == stack.project
                        || overlap(&s.working_directory, &stack.working_directory)
                })
            {
                return Err(invalid());
            }
            if let FileStatus::Conflict { candidates } = &stack.files
                && (!(2..=4).contains(&candidates.len())
                    || candidates
                        .iter()
                        .enumerate()
                        .any(|(i, c)| candidates[..i].contains(c)))
            {
                return Err(invalid());
            }
            for field in [&stack.stack, &stack.project, &stack.working_directory] {
                charge(field)?;
            }
            for file in &stack.config_files {
                charge(file)?;
            }
            for (i, service) in stack.services.iter().enumerate() {
                if !name(service, false) || stack.services[..i].contains(service) {
                    return Err(invalid());
                }
                charge(service)?;
            }
        }
    }
    if let Some(engine) = engine {
        if engine.members.len() > MAX_MEMBERS {
            return Err(limit());
        }
        if engine.engine_id.is_empty()
            || engine.engine_id.len() > 128
            || engine.engine_id.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        charge(engine.engine_id)?;
        for (i, member) in engine.members.iter().enumerate() {
            let container = member.container();
            container.validate()?;
            if engine.members[..i]
                .iter()
                .any(|m| m.container().resource == container.resource)
            {
                return Err(invalid());
            }
            for field in [&container.resource, &container.image, &container.started_at] {
                charge(field)?;
            }
            if let Membership::Compose {
                project, service, ..
            } = member
            {
                if !name(&project.name, true) || !name(service, false) || !path(&project.working_directory) || !files_valid(&project.config_files)
                    || engine.members[..i].iter().any(|m| matches!(m, Membership::Compose { project: previous, .. } if previous.name == project.name && previous != project)) { return Err(invalid()); }
                for field in [&project.name, &project.working_directory, service] {
                    charge(field)?;
                }
                for file in &project.config_files {
                    charge(file)?;
                }
            }
        }
    }
    Ok(())
}
fn counts(members: &[MemberView]) -> Counts {
    let mut value = Counts::default();
    for member in members {
        if member.one_off {
            value.one_offs += 1;
            value.one_off_running += u16::from(member.container.running);
        } else {
            value.ordinary += 1;
            value.running += u16::from(member.container.running);
        }
    }
    value
}
fn runtime(counts: Counts, missing: usize, drift: bool) -> RuntimeStatus {
    if counts.ordinary == 0 {
        RuntimeStatus::NoContainers
    } else if counts.running == 0 {
        RuntimeStatus::Stopped
    } else if counts.running == counts.ordinary && missing == 0 && !drift {
        RuntimeStatus::Running
    } else {
        RuntimeStatus::Partial
    }
}
fn services(
    declared: &[String],
    project: Option<&ProjectIdentity>,
    members: &[Membership],
    known: bool,
) -> Vec<ServiceView> {
    let mut names: BTreeSet<&str> = declared.iter().map(String::as_str).collect();
    if let Some(project) = project {
        names.extend(members.iter().filter_map(|m| match m {
            Membership::Compose {
                project: p,
                service,
                ..
            } if p == project => Some(service.as_str()),
            _ => None,
        }));
    }
    names
        .into_iter()
        .map(|name| {
            let mut rows: Vec<_> = members
                .iter()
                .filter_map(|m| match m {
                    Membership::Compose {
                        container,
                        project: p,
                        service,
                        one_off,
                    } if Some(p) == project && service == name => Some(MemberView {
                        container: container.clone(),
                        one_off: *one_off,
                    }),
                    _ => None,
                })
                .collect();
            rows.sort_by(|a, b| a.container.resource.cmp(&b.container.resource));
            let count = known.then(|| counts(&rows));
            ServiceView {
                name: name.into(),
                declared: declared.iter().any(|d| d == name),
                runtime: count.map_or(RuntimeStatus::Unknown, |c| runtime(c, 0, false)),
                counts: count,
                members: rows,
            }
        })
        .collect()
}
fn totals(services: &[ServiceView]) -> Counts {
    services
        .iter()
        .filter_map(|s| s.counts)
        .fold(Counts::default(), |a, b| Counts {
            ordinary: a.ordinary + b.ordinary,
            running: a.running + b.running,
            one_offs: a.one_offs + b.one_offs,
            one_off_running: a.one_off_running + b.one_off_running,
        })
}

pub fn discover(
    now: i64,
    catalog: Option<CatalogInput<'_>>,
    engine: Option<EngineInput<'_>>,
) -> Result<StackInventory> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(invalid());
    }
    admit(catalog.as_ref(), engine.as_ref())?;
    let catalog_status = source(catalog.as_ref().map(|c| c.sampled_at), now)?;
    let engine_status = source(engine.as_ref().map(|e| e.sampled_at), now)?;
    let catalog_fresh = catalog_status.state == SourceState::Fresh;
    let engine_fresh = engine_status.state == SourceState::Fresh;
    let engine_matches_catalog = if catalog_fresh && engine_fresh {
        Some(catalog.as_ref().unwrap().engine_id == engine.as_ref().unwrap().engine_id)
    } else {
        None
    };
    let joined = engine_matches_catalog == Some(true);
    let declarations = catalog.as_ref().map_or(&[][..], |c| c.stacks);
    let members = engine
        .as_ref()
        .filter(|_| engine_fresh)
        .map_or(&[][..], |e| e.members);
    let mut projects: Vec<&ProjectIdentity> = Vec::new();
    for member in members {
        if let Membership::Compose { project, .. } = member
            && !projects.contains(&project)
        {
            projects.push(project);
        }
    }
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    let mut stacks = Vec::new();
    for declaration in declarations {
        let mut issues = Vec::new();
        if joined {
            if projects
                .iter()
                .any(|p| p.name == declaration.project && !matches(declaration, p))
            {
                issues.push(StackIssue::IdentityConflict);
            }
            if projects
                .iter()
                .any(|p| p.name != declaration.project && overlaps(declaration, p))
            {
                issues.push(StackIssue::OverlappingProject);
            }
        }
        let conflict = !issues.is_empty();
        let known = joined && !conflict;
        let selected = known
            .then(|| projects.iter().copied().find(|p| matches(declaration, p)))
            .flatten();
        let service_rows = services(&declaration.services, selected, members, known);
        let drift = service_rows.iter().any(|s| !s.declared);
        if drift {
            issues.push(StackIssue::UndeclaredServices);
        }
        let count = known.then(|| totals(&service_rows));
        let missing = known.then(|| {
            service_rows
                .iter()
                .filter(|s| s.declared && s.counts.is_some_and(|c| c.ordinary == 0))
                .count() as u16
        });
        stacks.push(StackView {
            stack: declaration.stack.clone(),
            project: declaration.project.clone(),
            working_directory: declaration.working_directory.clone(),
            config_files: declaration.config_files.clone(),
            files: declaration.files.clone(),
            runtime: if conflict {
                RuntimeStatus::Conflict
            } else {
                count.map_or(RuntimeStatus::Unknown, |c| {
                    runtime(c, missing.unwrap_or(0).into(), drift)
                })
            },
            counts: count,
            missing_services: missing,
            services: service_rows,
            issues,
        });
    }
    stacks.sort_by(|a, b| a.stack.cmp(&b.stack));
    let mut other_projects = Vec::new();
    for project in projects {
        if joined && declarations.iter().any(|d| matches(d, project)) {
            continue;
        }
        let relation = if engine_matches_catalog == Some(false) {
            RegistrationRelation::EngineMismatch
        } else if !catalog_fresh {
            RegistrationRelation::CatalogUnknown
        } else if declarations.iter().any(|d| d.project == project.name) {
            RegistrationRelation::IdentityConflict
        } else if declarations.iter().any(|d| overlaps(d, project)) {
            RegistrationRelation::Overlapping
        } else {
            RegistrationRelation::NotRegistered
        };
        other_projects.push(OtherProjectView {
            project: project.clone(),
            relation,
            services: services(&[], Some(project), members, true),
        });
    }
    let report = StackInventory {
        version: 1,
        generated_at: now,
        catalog: catalog_status,
        engine: engine_status,
        engine_matches_catalog,
        stacks,
        other_projects,
        standalone_containers: engine_fresh.then(|| {
            members
                .iter()
                .filter(|m| matches!(m, Membership::Standalone { .. }))
                .count() as u16
        }),
    };
    report.check_size()?;
    Ok(report)
}

impl StackInventory {
    /// Streaming admission before serialization/transport allocates a response.
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_REPORT_BYTES - self.0 {
                    return Err(std::io::Error::other("stack report bound"));
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
