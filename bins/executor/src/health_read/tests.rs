use super::*;
use limeos_domain::automation::{self, SignalStatus};
use limeos_executor_container::docker::health_evidence::{
    ContainerHealth, HealthStatus, RestartMode,
};

/// Container `index`: running unless `stopped`, `restart: always` (so it is
/// alerted on), with the given check result.
fn row(index: usize, stopped: bool, health: HealthStatus) -> ContainerHealth {
    let id = format!("{index:064x}");
    let domain = match health {
        HealthStatus::Healthy => automation::ContainerHealth::Healthy,
        HealthStatus::Unhealthy => automation::ContainerHealth::Unhealthy,
        HealthStatus::Starting => automation::ContainerHealth::Starting,
        HealthStatus::NotConfigured => automation::ContainerHealth::Unreported,
        HealthStatus::Unavailable => automation::ContainerHealth::Unavailable,
    };
    let mut signal = automation::container_signal(
        &id,
        Some(!stopped),
        domain,
        automation::RestartPolicy::Always,
    )
    .unwrap();
    let name = format!("svc{index}");
    if let Some(signal) = &mut signal {
        signal.summary = signal.summary.replacen(&id, &name, 1);
    }
    ContainerHealth {
        resource: format!("container:{id}"),
        name,
        running: !stopped,
        restart: RestartMode::Always,
        health,
        signal,
    }
}
fn snapshot(count: usize, truncated: u16) -> EngineHealthSnapshot {
    EngineHealthSnapshot {
        version: 1,
        engine_id: "fixture-engine".into(),
        containers_observed_at: 120,
        complete: truncated == 0,
        containers: (0..count)
            .map(|i| match i % 4 {
                0 => row(i, false, HealthStatus::Healthy),
                1 => row(i, true, HealthStatus::Unhealthy),
                2 => row(i, false, HealthStatus::Unhealthy),
                _ => row(i, false, HealthStatus::NotConfigured),
            })
            .collect(),
        truncated,
    }
}
fn nonce() -> String {
    "a".repeat(64)
}

#[test]
fn sixteen_seventeen_and_sixty_four_containers_make_a_whole_cohort() {
    for count in [16, 17, 18, 64] {
        let value = cohort(&nonce(), &snapshot(count, 0), &[]).unwrap();
        assert_eq!(value.members().len(), count);
        assert_eq!(value.truncated(), 0);
        assert!(value.complete());
        // Stopped and unhealthy members are failing, never left out.
        let failing = value
            .members()
            .iter()
            .filter(|m| m.signal.as_ref().unwrap().status == SignalStatus::Failing)
            .count();
        assert_eq!(
            failing,
            (0..count).filter(|i| matches!(i % 4, 1 | 2)).count()
        );
        assert!(value.encode().unwrap().len() <= limeos_domain::automation::CONTAINER_HEALTH_BYTES);
    }
}

#[test]
fn past_the_bound_the_cohort_counts_the_rest_and_is_never_complete() {
    let value = cohort(&nonce(), &snapshot(64, 1), &[]).unwrap();
    assert_eq!(value.members().len(), 64);
    assert_eq!(value.truncated(), 1);
    assert!(!value.complete());
    let decoded = ContainerHealthCohort::decode(&value.encode().unwrap()).unwrap();
    assert_eq!(decoded.truncated(), 1);
    // A 65th member is never sent: the snapshot past the bound is refused.
    assert!(cohort(&nonce(), &snapshot(65, 0), &[]).is_err());
}

#[test]
fn a_container_changed_since_collection_is_unavailable_not_healthy_or_gone() {
    let snap = snapshot(18, 0);
    let changed = vec![snap.containers[4].resource.clone()];
    let value = cohort(&nonce(), &snap, &changed).unwrap();
    assert_eq!(value.members().len(), 18);
    let member = value
        .members()
        .iter()
        .find(|m| m.resource == changed[0])
        .unwrap();
    let signal = member.signal.as_ref().unwrap();
    assert_eq!(signal.status, SignalStatus::Unavailable);
    assert_eq!(signal.summary, "svc4 observation unavailable");
    assert!(!value.complete());
}

#[test]
fn every_reason_survives_the_worker_frame_and_supervision_keeps_it() {
    use HealthReadFailure::*;
    let all = [
        ReaderBudget,
        WorkerBusy,
        WorkerDeadline,
        WorkerCapability,
        WorkerFailed,
        WorkerCleanup,
        WorkerMemory,
        WorkerDescriptors,
        EngineForbidden,
        EngineUnavailable,
        EngineOverloaded,
        EngineChanged,
        CohortBound,
    ];
    for failure in all {
        let byte = failure_byte(failure);
        assert_eq!(failure_from_byte(byte), Some(failure));
        assert_eq!(supervision(Refusal::WorkerFailed(byte)), failure);
        assert!(!failure.message().is_empty());
    }
    for byte in [0, 14, 255] {
        assert_eq!(failure_from_byte(byte), None);
        assert_eq!(supervision(Refusal::WorkerFailed(byte)), WorkerFailed);
    }
    assert_eq!(supervision(Refusal::Busy), WorkerBusy);
    assert_eq!(supervision(Refusal::Deadline), WorkerDeadline);
    assert_eq!(supervision(Refusal::CleanupUnconfirmed), WorkerCleanup);
    assert_eq!(supervision(Refusal::WorkerCapability), WorkerCapability);
    assert_eq!(supervision(Refusal::WorkerRefused), WorkerFailed);
    assert_eq!(engine(Error(ErrorCode::Forbidden)), EngineForbidden);
    assert_eq!(engine(Error(ErrorCode::Unavailable)), EngineUnavailable);
    assert_eq!(engine(Error(ErrorCode::Overloaded)), EngineOverloaded);
    assert_eq!(engine(Error(ErrorCode::Conflict)), EngineChanged);
}

#[test]
fn the_journal_gets_one_line_per_change_of_reason_and_one_on_recovery() {
    let state = Mutex::new(None);
    let unavailable = Err(HealthReadFailure::EngineUnavailable);
    let budget = Err(HealthReadFailure::WorkerMemory);
    let logged: Vec<bool> = [
        unavailable,
        unavailable,
        budget,
        budget,
        Ok(()),
        Ok(()),
        unavailable,
    ]
    .into_iter()
    .map(|result| note_in(&state, result))
    .collect();
    assert_eq!(logged, [true, false, true, false, true, false, true]);
}
