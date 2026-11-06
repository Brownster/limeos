//! Genuine selected container health. Reports cannot restore retained authority.
use super::{
    Docker,
    process_evidence::{self, EnginePeerPolicy, EngineStorageProcessEvidence},
    read_admission::{Scope, Validation, ValidationGate},
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
    pub async fn revalidate(&self) -> Result<()> {
        self.inner.revalidate().await
    }
}

/// Fixed trusted reader. Runtime registration and independent supervision remain required.
/// Kept on `CombinedV1` (16 containers, 8 running): its only consumer is the
/// alert/health-policy automation behind `allow_container_health_read`, which
/// the basic cutover leaves off. Raise it with an `EngineWideV1`-style profile
/// before enabling that read on an Engine of more than 16 containers.
pub async fn inspect_container_health() -> Result<EngineHealthEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
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

pub(super) fn parse(
    value: &Value,
    container: &ContainerSnapshot,
    scope: &mut Scope,
) -> Result<ContainerHealth> {
    let state = value
        .get("State")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let running = container.running
        & !state
            .get("Paused")
            .and_then(Value::as_bool)
            .ok_or_else(refused)?
        & !state
            .get("Restarting")
            .and_then(Value::as_bool)
            .ok_or_else(refused)?
        & !state
            .get("Dead")
            .and_then(Value::as_bool)
            .ok_or_else(refused)?;
    let config = value
        .get("Config")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let restart = value
        .get("HostConfig")
        .and_then(Value::as_object)
        .and_then(|v| v.get("RestartPolicy"))
        .and_then(Value::as_object)
        .and_then(|v| v.get("Name"))
        .and_then(Value::as_str)
        .ok_or_else(refused)?;
    let restart = match restart {
        "always" => RestartMode::Always,
        "unless-stopped" => RestartMode::UnlessStopped,
        "no" | "" => RestartMode::No,
        "on-failure" => RestartMode::OnFailure,
        _ => return Err(refused()),
    };
    let name = value
        .get("Name")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix('/'))
        .ok_or_else(refused)?;
    if name.is_empty()
        || name.len() > 255
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(refused());
    }
    let configured = match config.get("Healthcheck") {
        None | Some(Value::Null) => false,
        Some(Value::Object(check)) => match check.get("Test") {
            Some(Value::Array(test)) => {
                if test.iter().any(|v| !v.is_string()) {
                    return Err(refused());
                }
                match test.first().and_then(Value::as_str) {
                    Some("NONE") if test.len() == 1 => false,
                    None | Some("CMD" | "CMD-SHELL") => true,
                    _ => return Err(refused()),
                }
            }
            None | Some(Value::Null) => true,
            _ => return Err(refused()),
        },
        _ => return Err(refused()),
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
                return Err(refused());
            }
            match result
                .get("Status")
                .and_then(Value::as_str)
                .ok_or_else(refused)?
            {
                "healthy" => HealthStatus::Healthy,
                "unhealthy" => HealthStatus::Unhealthy,
                "starting" => HealthStatus::Starting,
                _ => HealthStatus::Unavailable,
            }
        }
        _ => return Err(refused()),
    };
    let id = container
        .resource
        .strip_prefix("container:")
        .ok_or_else(refused)?;
    // Bound all domain signal/string-format temporaries before constructing them.
    // This is counted payload allowance, not allocator/cgroup heap accounting.
    scope.charge(2048)?;
    let mut signal =
        automation::container_signal(id, Some(running), health.domain(), restart.domain())?;
    if let Some(signal) = &mut signal {
        let suffix = signal.summary.strip_prefix(id).ok_or_else(refused)?;
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
