//! Authenticated Engine-declared Compose membership, never trusted stack files.
use super::{
    Docker,
    process_evidence::{self, EnginePeerPolicy, EngineStorageProcessEvidence},
    read_admission::Scope,
};
use limeos_domain::{ContainerSnapshot, Error, ErrorCode, Result};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU8, Ordering};

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
    pub async fn revalidate(&self) -> Result<()> {
        self.inner.revalidate().await
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
    state: AtomicU8,
}
impl Capture {
    pub(super) fn new(snapshot: EngineComposeSnapshot) -> Self {
        Self {
            snapshot,
            state: AtomicU8::new(0),
        }
    }
    pub(super) fn begin(&self) -> Result<Validation<'_>> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.state.store(2, Ordering::Release);
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Validation {
            capture: self,
            finished: false,
        })
    }
}
pub(super) struct Validation<'a> {
    capture: &'a Capture,
    finished: bool,
}
impl Validation<'_> {
    pub(super) fn finish(mut self, result: Result<()>) -> Result<()> {
        let result = match result {
            Ok(())
                if self
                    .capture
                    .state
                    .compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok() =>
            {
                Ok(())
            }
            other => {
                self.capture.state.store(2, Ordering::Release);
                other.and(Err(Error(ErrorCode::Conflict)))
            }
        };
        self.finished = true;
        result
    }
}
impl Drop for Validation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.capture.state.store(2, Ordering::Release);
        }
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
pub(super) fn parse(
    value: &Value,
    container: &ContainerSnapshot,
    scope: &mut Scope,
) -> Result<ComposeContainerMembership> {
    let config = value
        .get("Config")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let labels = config.get("Labels").ok_or_else(refused)?;
    let ownership = if labels.is_null() {
        ComposeOwnership::NonCompose
    } else {
        let labels = labels.as_object().ok_or_else(refused)?;
        if labels.values().any(|v| !v.is_string()) {
            return Err(refused());
        }
        if !labels.keys().any(|key| key.starts_with(PREFIX)) {
            ComposeOwnership::NonCompose
        } else {
            // Special hook/relay execution semantics are outside the supported subset.
            if labels.contains_key("com.docker.compose.hook")
                || labels.contains_key("com.docker.compose.relay")
            {
                return Err(refused());
            }
            let text = |key| labels.get(key).and_then(Value::as_str).ok_or_else(refused);
            let project = text(PROJECT)?;
            let service = text(SERVICE)?;
            let directory = text(DIRECTORY)?;
            let files = text(FILES)?;
            let one_off = match text(ONEOFF)? {
                "True" => true,
                "False" => false,
                _ => return Err(refused()),
            };
            if !name(project, true)
                || !name(service, false)
                || !path(directory)
                || files.len() > 4 * 512 + 3
            {
                return Err(refused());
            }
            let mut selected = Vec::new();
            for file in files.split(',') {
                if selected.len() == 4 || !path(file) || selected.iter().any(|p| p == file) {
                    return Err(refused());
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
pub(super) fn consistent(members: &[ComposeContainerMembership]) -> Result<()> {
    for (i, member) in members.iter().enumerate() {
        let ComposeOwnership::Compose { project, .. } = &member.ownership else {
            continue;
        };
        for other in &members[..i] {
            if let ComposeOwnership::Compose {
                project: previous, ..
            } = &other.ownership
            {
                if previous.name == project.name && previous != project {
                    return Err(refused());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
