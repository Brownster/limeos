//! Fresh, nonce-bound health collection under the existing independent supervisor.
//!
//! Every failure has a closed, non-secret reason (`HealthReadFailure`): the
//! worker sends its own as one byte through its guardian, the reader logs it
//! when it changes and sends it to core with the refusal, so "unavailable" is
//! diagnosable on a host (`limeosctl reader-status --read container-health`,
//! `journalctl -u limeos-storage-reader`).
use crate::{
    read_budget::Sample,
    read_launch::ProtectedHealthLaunch,
    read_supervision::{Refusal, Supervisor},
};
use limeos_domain::{
    Error, ErrorCode,
    automation::{ContainerHealthCohort, ContainerHealthMember},
};
use limeos_executor_container::docker::health_evidence::{
    EngineHealthSnapshot, inspect_container_health,
};
use limeos_executor_protocol::{CONTAINER_HEALTH_ENGINE_BUDGET, HealthReadFailure};
use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

const MIB: usize = 1024 * 1024;
/// The reader before it starts a worker, as for the stack read.
const MANAGER_RSS: usize = 12 * MIB;
/// The worker's baseline and its peak with a whole Engine read. The Engine
/// collection is the stack worker's (`EngineWideV1`: 64 containers, a 4 MiB
/// counted ledger), which runs under the same 20 MiB, and the cohort adds at
/// most `CONTAINER_HEALTH_BYTES` (56 KiB). The scale suite reads 30, 64 and
/// 65 containers through this worker with these samples enforced.
const WORKER_BASELINE_RSS: usize = 16 * MIB;
const WORKER_RSS: usize = 20 * MIB;
/// The worker's start-up and its answer back through the guardian, between
/// the end of its Engine budget and the end of its supervision window.
pub(crate) const WORKER_MARGIN: Duration = Duration::from_millis(500);

/// A refused read: the code core sees, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed {
    pub error: Error,
    pub failure: HealthReadFailure,
}
impl Failed {
    fn unavailable(failure: HealthReadFailure) -> Self {
        Self {
            error: Error(ErrorCode::Unavailable),
            failure,
        }
    }
}

fn sample(rss: usize, fds: usize, soft: u64) -> Result<(), HealthReadFailure> {
    let s = Sample::read().map_err(|_| HealthReadFailure::WorkerFailed)?;
    if s.rss_bytes > rss {
        return Err(HealthReadFailure::WorkerMemory);
    }
    if s.open_fds > fds || s.soft_nofile != Some(soft) || !(1..=4).contains(&s.threads) {
        return Err(HealthReadFailure::WorkerDescriptors);
    }
    Ok(())
}

/// The worker's reason as its frame byte, and back. Exhaustive both ways.
pub fn failure_byte(failure: HealthReadFailure) -> u8 {
    use HealthReadFailure::*;
    match failure {
        ReaderBudget => 1,
        WorkerBusy => 2,
        WorkerDeadline => 3,
        WorkerCapability => 4,
        WorkerFailed => 5,
        WorkerCleanup => 6,
        WorkerMemory => 7,
        WorkerDescriptors => 8,
        EngineForbidden => 9,
        EngineUnavailable => 10,
        EngineOverloaded => 11,
        EngineChanged => 12,
        CohortBound => 13,
        EngineSlow => 14,
    }
}
pub(crate) fn failure_from_byte(byte: u8) -> Option<HealthReadFailure> {
    use HealthReadFailure::*;
    Some(match byte {
        1 => ReaderBudget,
        2 => WorkerBusy,
        3 => WorkerDeadline,
        4 => WorkerCapability,
        5 => WorkerFailed,
        6 => WorkerCleanup,
        7 => WorkerMemory,
        8 => WorkerDescriptors,
        9 => EngineForbidden,
        10 => EngineUnavailable,
        11 => EngineOverloaded,
        12 => EngineChanged,
        13 => CohortBound,
        14 => EngineSlow,
        _ => return None,
    })
}
/// The Engine read's refusal as a reason.
fn engine(error: Error) -> HealthReadFailure {
    match error.0 {
        ErrorCode::Forbidden => HealthReadFailure::EngineForbidden,
        ErrorCode::Overloaded => HealthReadFailure::EngineOverloaded,
        ErrorCode::Conflict => HealthReadFailure::EngineChanged,
        // Only the Engine evidence's own lifetime expires; the worker's
        // budget, shorter, normally ends a slow read first.
        ErrorCode::Expired => HealthReadFailure::EngineSlow,
        _ => HealthReadFailure::EngineUnavailable,
    }
}
/// The supervisor's refusal as a reason, including the worker's own.
fn supervision(refusal: Refusal) -> HealthReadFailure {
    match refusal {
        Refusal::Busy => HealthReadFailure::WorkerBusy,
        Refusal::Deadline => HealthReadFailure::WorkerDeadline,
        Refusal::WorkerCapability => HealthReadFailure::WorkerCapability,
        Refusal::CleanupUnconfirmed => HealthReadFailure::WorkerCleanup,
        Refusal::WorkerFailed(byte) => {
            failure_from_byte(byte).unwrap_or(HealthReadFailure::WorkerFailed)
        }
        Refusal::Unavailable
        | Refusal::WorkerRefused
        | Refusal::Cancelled
        | Refusal::InvalidOutput
        | Refusal::WorkerExit => HealthReadFailure::WorkerFailed,
    }
}

/// The latest failure logged; `None` after a success.
static LOGGED: Mutex<Option<HealthReadFailure>> = Mutex::new(None);
/// Logs a failure when its reason differs from the last one logged, and the
/// first success after a failure, so a 1 Hz reader cannot flood the journal.
fn note(result: Result<(), HealthReadFailure>) {
    note_in(&LOGGED, result);
}
/// As `note`, against `logged`; returns whether it logged (for tests).
fn note_in(
    logged: &Mutex<Option<HealthReadFailure>>,
    result: Result<(), HealthReadFailure>,
) -> bool {
    let mut last = logged.lock().unwrap_or_else(PoisonError::into_inner);
    let now = result.err();
    if *last == now {
        return false;
    }
    match now {
        Some(failure) => tracing::warn!(
            reason = ?failure,
            "container-health read unavailable: {}",
            failure.message()
        ),
        None => tracing::info!("container-health reads answer again"),
    }
    *last = now;
    true
}

/// One read for a request that arrived when `deadline` minus
/// `CONTAINER_HEALTH_READER_WINDOW` was now.
pub fn refresh(
    nonce: &str,
    manager: &Supervisor,
    gate: tokio::sync::OwnedMutexGuard<()>,
    deadline: Instant,
) -> Result<ContainerHealthCohort, Failed> {
    let answer = read(nonce, manager, gate, deadline);
    note(answer.as_ref().map(drop).map_err(|failed| failed.failure));
    answer
}
fn read(
    nonce: &str,
    manager: &Supervisor,
    gate: tokio::sync::OwnedMutexGuard<()>,
    deadline: Instant,
) -> Result<ContainerHealthCohort, Failed> {
    // A request that waited for the read slot past this cannot give its
    // worker the whole Engine budget before core stops waiting.
    if Instant::now() + CONTAINER_HEALTH_ENGINE_BUDGET + WORKER_MARGIN > deadline {
        return Err(Failed::unavailable(HealthReadFailure::WorkerBusy));
    }
    sample(MANAGER_RSS, 24, 256)
        .map_err(|_| Failed::unavailable(HealthReadFailure::ReaderBudget))?;
    let launch = ProtectedHealthLaunch::prepare(nonce, deadline)
        .map_err(|_| Failed::unavailable(HealthReadFailure::WorkerFailed))?;
    let report = launch
        .start_with(manager, gate)
        .and_then(|ticket| ticket.wait())
        .map_err(|refusal| {
            crate::read_supervision::note_refusal(&refusal);
            Failed::unavailable(supervision(refusal))
        })?;
    let cohort = ContainerHealthCohort::decode(report.as_bytes()).map_err(|error| {
        Failed::unavailable(if error.0 == ErrorCode::Overloaded {
            HealthReadFailure::CohortBound
        } else {
            HealthReadFailure::WorkerFailed
        })
    })?;
    if cohort.nonce() != nonce {
        return Err(Failed {
            error: Error(ErrorCode::Conflict),
            failure: HealthReadFailure::WorkerFailed,
        });
    }
    Ok(cohort)
}

/// The cohort of one Engine read: every member read, a container that
/// changed since collection with an unavailable signal (never healthy or
/// gone), and the containers past the bound counted, not dropped.
pub(crate) fn cohort(
    nonce: &str,
    snapshot: &EngineHealthSnapshot,
    changed: &[String],
) -> limeos_domain::Result<ContainerHealthCohort> {
    let mut members = Vec::new();
    members
        .try_reserve_exact(snapshot.containers.len())
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    for member in &snapshot.containers {
        members.push(ContainerHealthMember {
            resource: member.resource.clone(),
            signal: if changed.contains(&member.resource) {
                member.unavailable_signal()?
            } else {
                member.signal.clone()
            },
        });
    }
    members.sort_by(|a, b| a.resource.cmp(&b.resource));
    ContainerHealthCohort::with_truncated(
        nonce.into(),
        snapshot.engine_id.clone(),
        snapshot.containers_observed_at,
        members,
        snapshot.truncated,
    )
}

pub(crate) async fn collect(
    nonce: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, HealthReadFailure> {
    if !limeos_domain::opaque_id(nonce) {
        return Err(HealthReadFailure::WorkerFailed);
    }
    sample(WORKER_BASELINE_RSS, 16, 226)?;
    // A cancelled worker's answer is discarded by its guardian.
    if cancelled() {
        return Err(HealthReadFailure::WorkerFailed);
    }
    // The whole Engine read has its own budget, inside the worker's
    // supervision window, so a slow Engine is reported as slow.
    let (engine, changed) = tokio::time::timeout(CONTAINER_HEALTH_ENGINE_BUDGET, async {
        let engine = inspect_container_health().await.map_err(engine)?;
        sample(WORKER_RSS, 49, 226)?;
        // Alerting reads tolerate one container changing since collection:
        // it is reported with an unavailable signal, never healthy or gone.
        let changed = engine.revalidate_display().await.map_err(self::engine)?;
        Ok((engine, changed))
    })
    .await
    .map_err(|_| HealthReadFailure::EngineSlow)??;
    let bytes = cohort(nonce, engine.snapshot(), &changed)
        .and_then(|cohort| cohort.encode())
        .map_err(|_| HealthReadFailure::CohortBound)?;
    sample(WORKER_RSS, 49, 226)?;
    if cancelled() {
        return Err(HealthReadFailure::WorkerFailed);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
