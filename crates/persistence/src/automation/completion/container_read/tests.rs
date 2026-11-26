use super::super::tests::{counts, deadline};
use super::*;
use std::thread;

fn resource() -> String {
    format!("container:{:064x}", 0)
}
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let (dir, mut store, actor) = super::super::tests::setup();
    store
        .revise_grants(
            &actor.id,
            Role::Operator,
            &[
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
                Scope {
                    operation: Operation::HealthRead,
                    resource: resource(),
                },
                Scope {
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            115,
        )
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    (dir, store, actor)
}
fn cohort(at: i64) -> ContainerHealthCohort {
    ContainerHealthCohort::new(
        "a".repeat(64),
        "fixture-engine".into(),
        at,
        vec![ContainerHealthMember {
            resource: resource(),
            signal: Some(AlertSignal {
                key: resource(),
                kind: AlertKind::Container,
                status: SignalStatus::Failing,
                severity: Severity::Warning,
                summary: "Fixture stopped".into(),
            }),
        }],
    )
    .unwrap()
}
fn schedule(store: &mut Store, actor: &Principal, at: i64) -> Schedule {
    store
        .automation_create(
            actor,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: at,
                    interval_seconds: 30,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            at,
        )
        .unwrap()
        .schedule
}
fn queue(store: &mut Store, actor: &Principal, at: i64) -> ObservationJob {
    let schedule = schedule(store, actor, at);
    let key = OccurrenceKey {
        schedule: schedule.id.clone(),
        revision: schedule.revision,
        due: at,
    };
    let Reservation::Queued(job) = store
        .automation_reserve_current(&schedule, &Settlement::Observation(key), || at, deadline())
        .unwrap()
    else {
        panic!("queued")
    };
    job
}
fn permit(admission: ContainerReadAdmission) -> ContainerReadPermit {
    let ContainerReadAdmission::Read(permit) = admission else {
        panic!("read permit")
    };
    *permit
}

#[test]
fn preflight_reuses_owner_exclusion_and_stable_poll_watermarks_without_writes() {
    let (_dir, mut store, actor) = setup();
    let row = schedule(&mut store, &actor, 120);
    let before = counts(&store);
    let read = permit(
        store
            .automation_container_poll_begin(&row.id, || 120, deadline())
            .unwrap(),
    );
    assert_eq!(counts(&store), before);
    assert!(matches!(
        store.automation_container_poll_begin(&row.id, || 120, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    assert!(matches!(
        store
            .automation_container_read_complete(&read, &cohort(120), || 120, deadline())
            .unwrap(),
        ContainerReadOutcome::Poll(ObservationPollOutcome::Committed(ObservationResult {
            complete: false,
            ..
        }))
    ));
    let stable = counts(&store);
    assert!(matches!(
        store
            .automation_container_poll_begin(&row.id, || 120, deadline())
            .unwrap(),
        ContainerReadAdmission::NotDue
    ));
    assert!(matches!(
        store
            .automation_container_poll_begin(&row.id, || 119, deadline())
            .unwrap(),
        ContainerReadAdmission::ClockRollback
    ));
    assert_eq!(counts(&store), stable);
    store
        .automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: row.revision,
                paused: true,
            },
            121,
        )
        .unwrap();
    let paused = counts(&store);
    assert!(matches!(
        store
            .automation_container_poll_begin(&row.id, || 122, deadline())
            .unwrap(),
        ContainerReadAdmission::Paused
    ));
    assert_eq!(counts(&store), paused);
}

#[test]
fn queued_original_identity_survives_restart_and_completion_returns_its_existing_receipt() {
    let (dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 120);
    let old = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    let before = counts(&store);
    assert!(matches!(
        store.automation_container_read_complete(&old, &cohort(121), || 121, deadline()),
        Err(Error(ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
    // An old permit's release cannot free this generation's active admission.
    assert!(matches!(
        store.automation_container_job_begin(&job, || 121, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    let ContainerReadOutcome::Queued(receipt) = store
        .automation_container_read_complete(&read, &cohort(121), || 121, deadline())
        .unwrap()
    else {
        panic!("queued receipt")
    };
    assert_eq!(receipt.job, job.id);
    assert!(!receipt.complete);
    let after = counts(&store);
    assert_eq!(after.0, before.0 + 1);
    let ContainerReadAdmission::Existing(again) = store
        .automation_container_job_begin(&job, || 122, deadline())
        .unwrap()
    else {
        panic!("existing")
    };
    assert_eq!(again, receipt);
    assert_eq!(counts(&store), after);
    assert!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn concrete_grants_are_resolved_after_read_and_failures_release_only_the_original_permit() {
    let (_dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 120);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    let source = ContainerHealthCohort::new(
        "a".repeat(64),
        "fixture-engine".into(),
        121,
        vec![ContainerHealthMember {
            resource: format!("container:{:064x}", 1),
            signal: None,
        }],
    )
    .unwrap();
    let before = counts(&store);
    assert!(matches!(
        store.automation_container_read_complete(&read, &source, || 121, deadline()),
        Err(Error(ErrorCode::Forbidden))
    ));
    assert_eq!(counts(&store), before);
    let fresh = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    store.automation_container_read_abandon(&read);
    assert!(matches!(
        store.automation_container_job_begin(&job, || 121, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    store.automation_container_read_abandon(&fresh);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    store
        .revise_grants(&actor.id, Role::Viewer, &[], 122)
        .unwrap();
    let before = counts(&store);
    assert!(matches!(
        store.automation_container_read_complete(&read, &cohort(121), || 122, deadline()),
        Err(Error(ErrorCode::Forbidden | ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn queued_completion_cannot_outlive_the_fresh_source_or_the_original_admission() {
    let (dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 120);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    let before = counts(&store);
    let writer = Connection::open(dir.path().join("core.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let held = thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        writer.execute_batch("ROLLBACK").unwrap();
    });
    assert!(matches!(
        store.automation_container_read_complete(
            &read,
            &cohort(121),
            || 121,
            Instant::now() + Duration::from_millis(30)
        ),
        Err(Error(ErrorCode::Expired))
    ));
    held.join().unwrap();
    assert_eq!(counts(&store), before);
    let read = permit(
        store
            .automation_container_job_begin(
                &job,
                || 121,
                Instant::now() + Duration::from_millis(20),
            )
            .unwrap(),
    );
    thread::sleep(Duration::from_millis(25));
    assert!(matches!(
        store.automation_container_read_complete(&read, &cohort(121), || 121, deadline()),
        Err(Error(ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    assert!(matches!(
        store.automation_container_read_complete(&read, &cohort(121), || 127, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn queued_partial_collection_cannot_silently_retire_a_retained_container() {
    let (_dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 120);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    store
        .automation_container_read_complete(&read, &cohort(121), || 121, deadline())
        .unwrap();
    let job = queue(&mut store, &actor, 122);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 123, deadline())
            .unwrap(),
    );
    let before = counts(&store);
    let empty =
        ContainerHealthCohort::new("a".repeat(64), "fixture-engine".into(), 123, vec![]).unwrap();
    assert!(matches!(
        store.automation_container_read_complete(&read, &empty, || 123, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items,
        [job]
    );
}

#[test]
fn a_truncated_cohort_never_retires_or_clears_a_container_it_left_out() {
    // An Engine past 64 containers: the alerted container is one of those
    // the read left out. It is unknown, not healthy and not gone, so the pass
    // may not settle as if it were (both completion paths share
    // `polling::container_bindings`).
    let (_dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 120);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 121, deadline())
            .unwrap(),
    );
    store
        .automation_container_read_complete(&read, &cohort(121), || 121, deadline())
        .unwrap();
    let left_out = ContainerHealthCohort::with_truncated(
        "a".repeat(64),
        "fixture-engine".into(),
        123,
        vec![],
        1,
    )
    .unwrap();
    assert!(!left_out.complete());
    let job = queue(&mut store, &actor, 122);
    let read = permit(
        store
            .automation_container_job_begin(&job, || 123, deadline())
            .unwrap(),
    );
    let before = counts(&store);
    assert!(matches!(
        store.automation_container_read_complete(&read, &left_out, || 123, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items,
        [job]
    );
}
