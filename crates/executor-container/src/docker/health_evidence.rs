//! Genuine selected container health. Reports cannot restore retained authority.
use super::{
    Docker,
    process_evidence::{self, EnginePeerPolicy, EngineStorageProcessEvidence},
    read_admission::{Scope, Validation, ValidationGate},
    storage::Refusal,
};
use limeos_domain::{
    ContainerSnapshot, Error, ErrorCode, Result,
    automation::{self, AlertSignal, SignalStatus},
};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartMode {
    Always,
    UnlessStopped,
    No,
    OnFailure,
}
impl RestartMode {
    fn domain(self) -> automation::RestartPolicy {
        match self {
            Self::Always => automation::RestartPolicy::Always,
            Self::UnlessStopped => automation::RestartPolicy::UnlessStopped,
            Self::No => automation::RestartPolicy::No,
            Self::OnFailure => automation::RestartPolicy::OnFailure,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Starting,
    NotConfigured,
    Unavailable,
}
impl HealthStatus {
    fn domain(self) -> automation::ContainerHealth {
        match self {
            Self::Healthy => automation::ContainerHealth::Healthy,
            Self::Unhealthy => automation::ContainerHealth::Unhealthy,
            Self::Starting => automation::ContainerHealth::Starting,
            Self::NotConfigured => automation::ContainerHealth::Unreported,
            Self::Unavailable => automation::ContainerHealth::Unavailable,
        }
    }
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ContainerHealth {
    pub resource: String,
    pub name: String,
    /// Effective running state excludes paused, restarting and dead containers.
    pub running: bool,
    pub restart: RestartMode,
    pub health: HealthStatus,
    /// Key and resource both bind the full Engine ID; names are display text.
    pub signal: Option<AlertSignal>,
}
impl ContainerHealth {
    /// The signal this container gives when its health cannot be read this
    /// round, as for one that changed since collection.
    pub fn unavailable_signal(&self) -> Result<Option<AlertSignal>> {
        let id = self
            .resource
            .strip_prefix("container:")
            .ok_or_else(refused)?;
        let mut signal = automation::container_signal(
            id,
            None,
            automation::ContainerHealth::Unavailable,
            self.restart.domain(),
        )?;
        if let Some(signal) = &mut signal {
            let suffix = signal.summary.strip_prefix(id).ok_or_else(refused)?;
            signal.summary = format!("{}{suffix}", self.name);
            signal.validate()?;
        }
        Ok(signal)
    }
}
/// Serialize-only past observations. Completeness covers container signals only.
/// ```compile_fail
/// use limeos_executor_container::docker::health_evidence::EngineHealthSnapshot;
/// let imported: EngineHealthSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct EngineHealthSnapshot {
    pub version: u16,
    pub engine_id: String,
    pub containers_observed_at: i64,
    pub complete: bool,
    pub containers: Vec<ContainerHealth>,
}
/// The retained socket, original clocks, ledger and validation gate stay private.
/// ```compile_fail
/// use limeos_executor_container::docker::health_evidence::EngineHealthEvidence;
/// let imported: EngineHealthEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct EngineHealthEvidence {
    inner: EngineStorageProcessEvidence,
}
impl EngineHealthEvidence {
    pub(super) fn from_storage(inner: EngineStorageProcessEvidence) -> Result<Self> {
        if inner.health_snapshot().is_none() {
            return Err(refused());
        }
        Ok(Self { inner })
    }
    pub fn snapshot(&self) -> &EngineHealthSnapshot {
        self.inner
            .health_snapshot()
            .expect("private health constructor")
    }
    /// The same original complete storage/PID collection; validation checks health too.
    pub fn storage(&self) -> &EngineStorageProcessEvidence {
        &self.inner
    }
    /// Strict: any difference from the original collection refuses.
    pub async fn revalidate(&self) -> Result<()> {
        self.inner.revalidate().await
    }
    /// For alerting only: the resources whose facts changed since collection,
    /// which a reader must report as unavailable this round.
    pub async fn revalidate_display(&self) -> Result<Vec<String>> {
        Ok(self.inner.revalidate_display().await?.changed)
    }
}

/// Fixed trusted reader. Runtime registration and independent supervision remain required.
/// Kept on `CombinedV1` (16 containers, 8 running): its only consumer is the
/// alert/health-policy automation behind `allow_container_health_read`, which
/// the basic cutover leaves off. Raise it with an `EngineWideV1`-style profile
/// before enabling that read on an Engine of more than 16 containers.
/// Authenticated by the Engine's root-owned socket and root peer, like
/// `inspect_compose_membership`; GET requests only; runs in the reader's
/// health worker with the `docker` group and no capability (PR-32).
pub async fn inspect_container_health() -> Result<EngineHealthEvidence> {
    let inner = process_evidence::inspect_health(
        &Docker::new("/run/docker.sock".into()),
        EnginePeerPolicy::root(),
    )
    .await?;
    EngineHealthEvidence::from_storage(inner)
}
pub(super) struct Capture {
    pub(super) snapshot: EngineHealthSnapshot,
    gate: ValidationGate,
}
impl Capture {
    pub(super) fn new(snapshot: EngineHealthSnapshot) -> Self {
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

fn unsupported() -> Refusal {
    Refusal::Unsupported
}
/// A container whose health cannot be read this round: changing between the
/// two passes, or of a shape this reader does not model. Its signal is
/// `Unavailable`, so the cohort is incomplete rather than the container gone.
/// An unknown restart policy is monitored as `always`, never silently dropped.
pub(super) fn unavailable(
    resource: &str,
    known: Option<&ContainerHealth>,
    scope: &mut Scope,
) -> Result<ContainerHealth> {
    let id = resource.strip_prefix("container:").ok_or_else(refused)?;
    let restart = known.map_or(RestartMode::Always, |k| k.restart);
    scope.charge(2048)?;
    let mut signal = automation::container_signal(
        id,
        None,
        automation::ContainerHealth::Unavailable,
        restart.domain(),
    )?;
    let name = match known {
        Some(k) => scope.string(&k.name)?,
        None => scope.string(id.get(..12).unwrap_or(id))?,
    };
    if let Some(signal) = &mut signal {
        let suffix = signal.summary.strip_prefix(id).ok_or_else(refused)?;
        signal.summary = format!("{name}{suffix}");
        signal.validate()?;
    }
    Ok(ContainerHealth {
        resource: scope.string(resource)?,
        name,
        running: false,
        restart,
        health: HealthStatus::Unavailable,
        signal,
    })
}
pub(super) fn parse(
    value: &Value,
    container: &ContainerSnapshot,
    scope: &mut Scope,
) -> std::result::Result<ContainerHealth, Refusal> {
    let state = value
        .get("State")
        .and_then(Value::as_object)
        .ok_or_else(unsupported)?;
    let running = container.running
        & !state
            .get("Paused")
            .and_then(Value::as_bool)
            .ok_or_else(unsupported)?
        & !state
            .get("Restarting")
            .and_then(Value::as_bool)
            .ok_or_else(unsupported)?
        & !state
            .get("Dead")
            .and_then(Value::as_bool)
            .ok_or_else(unsupported)?;
    let config = value
        .get("Config")
        .and_then(Value::as_object)
        .ok_or_else(unsupported)?;
    let restart = value
        .get("HostConfig")
        .and_then(Value::as_object)
        .and_then(|v| v.get("RestartPolicy"))
        .and_then(Value::as_object)
        .and_then(|v| v.get("Name"))
        .and_then(Value::as_str)
        .ok_or_else(unsupported)?;
    let restart = match restart {
        "always" => RestartMode::Always,
        "unless-stopped" => RestartMode::UnlessStopped,
        "no" | "" => RestartMode::No,
        "on-failure" => RestartMode::OnFailure,
        _ => return Err(unsupported()),
    };
    let name = value
        .get("Name")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix('/'))
        .ok_or_else(unsupported)?;
    if name.is_empty()
        || name.len() > 255
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(unsupported());
    }
    let configured = match config.get("Healthcheck") {
        None | Some(Value::Null) => false,
        Some(Value::Object(check)) => match check.get("Test") {
            Some(Value::Array(test)) => {
                if test.iter().any(|v| !v.is_string()) {
                    return Err(unsupported());
                }
                match test.first().and_then(Value::as_str) {
                    Some("NONE") if test.len() == 1 => false,
                    None | Some("CMD" | "CMD-SHELL") => true,
                    _ => return Err(unsupported()),
                }
            }
            None | Some(Value::Null) => true,
            _ => return Err(unsupported()),
        },
        _ => return Err(unsupported()),
    };
    let health = match state.get("Health") {
        None | Some(Value::Null) => {
            if configured {
                HealthStatus::Unavailable
            } else {
                HealthStatus::NotConfigured
            }
        }
        Some(Value::Object(result)) => {
            // A populated result requires a configured check. No command/log is retained.
            if !configured {
                return Err(unsupported());
            }
            match result
                .get("Status")
                .and_then(Value::as_str)
                .ok_or_else(unsupported)?
            {
                "healthy" => HealthStatus::Healthy,
                "unhealthy" => HealthStatus::Unhealthy,
                "starting" => HealthStatus::Starting,
                _ => HealthStatus::Unavailable,
            }
        }
        _ => return Err(unsupported()),
    };
    let id = container
        .resource
        .strip_prefix("container:")
        .ok_or_else(unsupported)?;
    // Bound all domain signal/string-format temporaries before constructing them.
    // This is counted payload allowance, not allocator/cgroup heap accounting.
    scope.charge(2048)?;
    let mut signal =
        automation::container_signal(id, Some(running), health.domain(), restart.domain())?;
    if let Some(signal) = &mut signal {
        let suffix = signal.summary.strip_prefix(id).ok_or_else(unsupported)?;
        signal.summary = format!("{name}{suffix}");
        signal.validate()?;
    }
    Ok(ContainerHealth {
        resource: scope.string(&container.resource)?,
        name: scope.string(name)?,
        running,
        restart,
        health,
        signal,
    })
}
pub(super) fn complete(containers: &[ContainerHealth]) -> bool {
    containers.iter().all(|row| {
        row.signal
            .as_ref()
            .is_none_or(|signal| signal.status != SignalStatus::Unavailable)
    })
}
