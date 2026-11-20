//! Authenticated Engine-declared Compose membership, never trusted stack files.
use super::{
    Docker,
    process_evidence::{self, EnginePeerPolicy, EngineStorageProcessEvidence},
    read_admission::{Scope, Validation, ValidationGate},
    storage::Refusal,
};
use limeos_domain::{
    ContainerSnapshot, Error, ErrorCode, Result,
    deployment::discovery::{OmissionReason, OmittedContainer},
};
use serde::Serialize;
use serde_json::Value;

const PREFIX: &str = "com.docker.compose.";
const PROJECT: &str = "com.docker.compose.project";
const SERVICE: &str = "com.docker.compose.service";
const DIRECTORY: &str = "com.docker.compose.project.working_dir";
const FILES: &str = "com.docker.compose.project.config_files";
const ONEOFF: &str = "com.docker.compose.oneoff";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeProjectIdentity {
    pub name: String,
    pub working_directory: String,
    /// Ordered names declared in the comma-delimited Engine label, not file authority.
    pub config_files: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComposeOwnership {
    NonCompose,
    Compose {
        project: ComposeProjectIdentity,
        service: String,
        one_off: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeContainerMembership {
    pub container: ContainerSnapshot,
    pub ownership: ComposeOwnership,
}
/// Past selected metadata cannot reconstruct retained Engine observations.
/// ```compile_fail
/// use limeos_executor_container::docker::compose_evidence::EngineComposeSnapshot;
/// let imported: EngineComposeSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct EngineComposeSnapshot {
    pub version: u16,
    pub engine_id: String,
    pub containers_observed_at: i64,
    pub containers: Vec<ComposeContainerMembership>,
    /// Engine containers beyond the bound, running ones kept first; not inspected.
    pub truncated: u16,
    /// Containers inspected but left out: changing between the two passes, or
    /// of a shape this reader does not model. Never in `containers`.
    pub omitted: Vec<OmittedContainer>,
}
impl EngineComposeSnapshot {
    /// Every Engine container is in `containers`: nothing truncated or omitted.
    pub fn complete(&self) -> bool {
        self.truncated == 0 && self.omitted.is_empty()
    }
}
/// Fixed-endpoint opaque owner. JSON cannot restore its socket, ledger or lifetime.
/// ```compile_fail
/// use limeos_executor_container::docker::compose_evidence::EngineComposeEvidence;
/// let imported: EngineComposeEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct EngineComposeEvidence {
    inner: EngineStorageProcessEvidence,
}
impl EngineComposeEvidence {
    pub(super) fn from_storage(inner: EngineStorageProcessEvidence) -> Result<Self> {
        if inner.compose_snapshot().is_none() {
            return Err(refused());
        }
        Ok(Self { inner })
    }
    pub fn snapshot(&self) -> &EngineComposeSnapshot {
        self.inner
            .compose_snapshot()
            .expect("private Compose constructor")
    }
    /// The genuine base from the same original collection; validation also checks Compose.
    pub fn storage(&self) -> &EngineStorageProcessEvidence {
        &self.inner
    }
    /// Strict: any difference from the original collection refuses.
    pub async fn revalidate(&self) -> Result<()> {
        self.inner.revalidate().await
    }
    /// For display only: the resources whose facts changed since collection,
    /// which a reader must show as changing. Engine identity, the container
    /// set and the socket must still match.
    pub async fn revalidate_display(&self) -> Result<Vec<String>> {
        self.inner.revalidate_display().await
    }
}

/// Unregistered UID-0 reader. No caller-selected endpoint/policy/profile/clock/cap.
pub async fn inspect_compose_membership() -> Result<EngineComposeEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    let inner = process_evidence::inspect_compose(
        &Docker::new("/run/docker.sock".into()),
        EnginePeerPolicy::root(),
    )
    .await?;
    EngineComposeEvidence::from_storage(inner)
}

pub(super) struct Capture {
    pub(super) snapshot: EngineComposeSnapshot,
    gate: ValidationGate,
}
impl Capture {
    pub(super) fn new(snapshot: EngineComposeSnapshot) -> Self {
        Self {
            snapshot,
            gate: ValidationGate::new(),
        }
    }
    pub(super) fn begin(&self) -> Result<Validation<'_>> {
        self.gate.begin()
    }
}
fn refused() -> Error {
    Error(ErrorCode::Unavailable)
}
fn name(value: &str, project: bool) -> bool {
    let first = value.as_bytes().first().copied();
    value.len() <= 64
        && first.is_some_and(|c| c.is_ascii_alphanumeric() && (!project || !c.is_ascii_uppercase()))
        && value.bytes().all(|c| {
            (c.is_ascii_alphanumeric() && (!project || !c.is_ascii_uppercase()))
                || b"_-".contains(&c)
                || (!project && c == b'.')
        })
}
fn path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && !value.contains(',')
        && value[1..].split('/').count() <= 16
        && value[1..]
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
}
fn unsupported() -> Refusal {
    Refusal::Unsupported
}
/// The Compose project an unsupported or changing container declares, when its
/// project label alone is well formed: the stack it belongs to is incomplete.
pub(super) fn declared_project(value: &Value, scope: &mut Scope) -> Result<Option<String>> {
    match value["Config"]["Labels"][PROJECT].as_str() {
        Some(project) if name(project, true) => scope.string(project).map(Some),
        _ => Ok(None),
    }
}
pub(super) fn omitted(
    container: &str,
    project: Option<String>,
    reason: OmissionReason,
    scope: &mut Scope,
) -> Result<OmittedContainer> {
    Ok(OmittedContainer {
        resource: scope.string(container)?,
        project,
        reason,
    })
}
pub(super) fn parse(
    value: &Value,
    container: &ContainerSnapshot,
    scope: &mut Scope,
) -> std::result::Result<ComposeContainerMembership, Refusal> {
    let config = value
        .get("Config")
        .and_then(Value::as_object)
        .ok_or_else(unsupported)?;
    let labels = config.get("Labels").ok_or_else(unsupported)?;
    let ownership = if labels.is_null() {
        ComposeOwnership::NonCompose
    } else {
        let labels = labels.as_object().ok_or_else(unsupported)?;
        if labels.values().any(|v| !v.is_string()) {
            return Err(unsupported());
        }
        if !labels.keys().any(|key| key.starts_with(PREFIX)) {
            ComposeOwnership::NonCompose
        } else {
            // Special hook/relay execution semantics are outside the supported subset.
            if labels.contains_key("com.docker.compose.hook")
                || labels.contains_key("com.docker.compose.relay")
            {
                return Err(unsupported());
            }
            let text = |key| {
                labels
                    .get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(unsupported)
            };
            let project = text(PROJECT)?;
            let service = text(SERVICE)?;
            let directory = text(DIRECTORY)?;
            let files = text(FILES)?;
            let one_off = match text(ONEOFF)? {
                "True" => true,
                "False" => false,
                _ => return Err(unsupported()),
            };
            if !name(project, true)
                || !name(service, false)
                || !path(directory)
                || files.len() > 4 * 512 + 3
            {
                return Err(unsupported());
            }
            let mut selected = Vec::new();
            for file in files.split(',') {
                if selected.len() == 4 || !path(file) || selected.iter().any(|p| p == file) {
                    return Err(unsupported());
                }
                scope.reserve(&mut selected, 1)?;
                selected.push(scope.string(file)?);
            }
            ComposeOwnership::Compose {
                project: ComposeProjectIdentity {
                    name: scope.string(project)?,
                    working_directory: scope.string(directory)?,
                    config_files: selected,
                },
                service: scope.string(service)?,
                one_off,
            }
        }
    };
    Ok(ComposeContainerMembership {
        container: ContainerSnapshot {
            resource: scope.string(&container.resource)?,
            image: scope.string(&container.image)?,
            started_at: scope.string(&container.started_at)?,
            running: container.running,
        },
        ownership,
    })
}
/// Project names that two members declare with different directories or files.
pub(super) fn conflicting(members: &[ComposeContainerMembership]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for (i, member) in members.iter().enumerate() {
        let ComposeOwnership::Compose { project, .. } = &member.ownership else {
            continue;
        };
        for other in members.iter().take(i) {
            if let ComposeOwnership::Compose {
                project: previous, ..
            } = &other.ownership
            {
                if previous.name == project.name
                    && previous != project
                    && !names.contains(&project.name.as_str())
                {
                    names.push(&project.name);
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests;
