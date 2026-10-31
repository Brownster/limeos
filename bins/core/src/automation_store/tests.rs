use super::*;
use limeos_domain::automation::{
    AlertKind, AlertSignal, CreateSchedule, MissedRunPolicy, PauseSchedule, Schedule,
    ScheduleDefinition, ScheduledOperation, Severity, SignalStatus,
};
use limeos_domain::{JobState, Operation, Principal, Role, Scope};
use limeos_persistence::ScopedHealthSignal;
use rusqlite::{Connection, OpenFlags};
use std::{
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn setup() -> (
    tempfile::TempDir,
    ObservationStoreBridge,
    Principal,
    Arc<AtomicI64>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let initial = store.login_record("alice").unwrap().unwrap().principal;
    store
        .revise_grants(
            &initial.id,
            Role::Operator,
            &[
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
                Scope {
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            102,
        )
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    drop(store);
    let mut bridge = ObservationStoreBridge::new(Database::open(&path).unwrap());
    let clock = Arc::new(AtomicI64::new(200));
    let sampled = clock.clone();
    bridge.clock = Arc::new(move || sampled.load(Ordering::SeqCst));
    (dir, bridge, actor, clock)
}
async fn schedule(
    bridge: &ObservationStoreBridge,
    actor: &Principal,
    operation: ScheduledOperation,
) -> Schedule {
    let actor = actor.clone();
    let clock = bridge.clock.clone();
    bridge
        .db
        .call(move |store| {
            store
                .automation_create(
                    &actor,
                    &CreateSchedule {
                        definition: ScheduleDefinition {
                            operation,
                            anchor: 0,
                            interval_seconds: 30,
                            grace_seconds: 5,
                            missed: MissedRunPolicy::CoalesceLatest,
                            window: None,
                        },
                    },
                    clock(),
                )
                .map(|view| view.schedule)
        })
        .await
        .unwrap()
}
fn counts(dir: &tempfile::TempDir) -> (i64, i64, i64, i64, i64, i64) {
    let conn = Connection::open_with_flags(
        dir.path().join("core.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_occurrences),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_notifications),(SELECT count(*) FROM events),audit_bytes FROM meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
async fn reopen(dir: &tempfile::TempDir) -> Database {
    for _ in 0..50 {
        match Database::open(&dir.path().join("core.sqlite")) {
            Ok(db) => return db,
            Err(Error(ErrorCode::Conflict)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("unexpected reopen refusal: {error:?}"),
        }
    }
    panic!("prior database worker did not exit")
}
fn batch(status: SignalStatus, complete: bool) -> HealthObservation {
    HealthObservation {
        complete,
        signals: vec![ScopedHealthSignal {
            resource: "host:system".into(),
            signal: AlertSignal {
                key: "host:load".into(),
                kind: AlertKind::Host,
                status,
                severity: Severity::Warning,
                summary: "Fixture load assessment".into(),
            },
        }],
    }
}

async fn poll(bridge: &ObservationStoreBridge, id: &str) -> ObservationPollLease {
    let ObservationPollAdmission::Lease(lease) = bridge
        .poll_begin(id.into(), vec!["host:system".into()], deadline())
        .await
        .unwrap()
    else {
        panic!("poll lease");
    };
    *lease
}

#[tokio::test]
async fn transition_poll_bridge_commits_once_and_restart_repeats_only_a_read() {
    let (dir, bridge, actor, clock) = setup();
    clock.store(210, Ordering::SeqCst);
    let row = schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let lease = poll(&bridge, &row.id).await;
    let receipt = bridge
        .poll_complete(lease.clone(), batch(SignalStatus::Healthy, true))
        .await
        .unwrap();
    assert!(matches!(receipt, ObservationPollOutcome::Committed(_)));
    let before = counts(&dir);
    assert_eq!(
        bridge
            .poll_complete(lease, batch(SignalStatus::Healthy, true))
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(counts(&dir), before);
    clock.store(240, Ordering::SeqCst);
    let read = poll(&bridge, &row.id).await;
    assert_eq!(
        bridge
            .poll_complete(read, batch(SignalStatus::Healthy, true))
            .await
            .unwrap(),
        ObservationPollOutcome::Unchanged {
            complete: true,
            sampled_at: 240
        }
    );
    assert_eq!(counts(&dir), before);
    drop(bridge);
    let mut bridge = ObservationStoreBridge::new(reopen(&dir).await);
    bridge.clock = Arc::new(|| 240);
    let read = poll(&bridge, &row.id).await;
    assert!(matches!(
        bridge
            .poll_complete(read, batch(SignalStatus::Healthy, true))
            .await
            .unwrap(),
        ObservationPollOutcome::Unchanged { .. }
    ));
    assert_eq!(counts(&dir), before);
}

#[tokio::test]
async fn concurrent_poll_admission_has_one_private_lease_and_no_queued_job() {
    let (dir, bridge, actor, clock) = setup();
    clock.store(210, Ordering::SeqCst);
    let row = schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let before = counts(&dir);
    let (one, two) = tokio::join!(
        bridge.poll_begin(row.id.clone(), vec!["host:system".into()], deadline()),
        bridge.poll_begin(row.id.clone(), vec!["host:system".into()], deadline())
    );
    let lease = match (one, two) {
        (Ok(ObservationPollAdmission::Lease(lease)), Err(Error(ErrorCode::Conflict)))
        | (Err(Error(ErrorCode::Conflict)), Ok(ObservationPollAdmission::Lease(lease))) => lease,
        other => panic!("unexpected admissions: {other:?}"),
    };
    assert_eq!(counts(&dir), before);
    assert!(
        bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    bridge.poll_abandon(*lease, deadline()).await.unwrap();
    assert_eq!(counts(&dir), before);
    let fresh = poll(&bridge, &row.id).await;
    bridge
        .poll_complete(fresh, batch(SignalStatus::Failing, true))
        .await
        .unwrap();
    assert!(
        bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn expired_poll_reply_cannot_receive_a_new_worker_deadline() {
    let (dir, bridge, actor, clock) = setup();
    clock.store(210, Ordering::SeqCst);
    let row = schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let short = Instant::now() + Duration::from_millis(50);
    let ObservationPollAdmission::Lease(lease) = bridge
        .poll_begin(row.id.clone(), vec!["host:system".into()], short)
        .await
        .unwrap()
    else {
        panic!("lease")
    };
    let before = counts(&dir);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        bridge
            .poll_complete((*lease).clone(), batch(SignalStatus::Failing, true))
            .await,
        Err(Error(ErrorCode::Expired))
    );
    bridge.poll_abandon(*lease, deadline()).await.unwrap();
    assert_eq!(counts(&dir), before);
    let read = poll(&bridge, &row.id).await;
    bridge
        .poll_complete(read, batch(SignalStatus::Healthy, true))
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_passes_and_restart_discover_only_original_queued_jobs() {
    let (dir, bridge, actor, _) = setup();
    for _ in 0..2 {
        schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    }
    let (first, second) = tokio::join!(
        bridge.reserve_page(None, deadline()),
        bridge.reserve_page(None, deadline())
    );
    assert_eq!(
        first.unwrap().queued.len() + second.unwrap().queued.len(),
        2
    );
    let original = bridge.queued_page(None, deadline()).await.unwrap();
    assert_eq!(original.items.len(), 2);
    assert_eq!(counts(&dir).0, 2);
    let before = counts(&dir);
    drop(bridge);
    let mut bridge = ObservationStoreBridge::new(reopen(&dir).await);
    bridge.clock = Arc::new(|| 200);
    assert_eq!(
        bridge.queued_page(None, deadline()).await.unwrap(),
        original
    );
    assert!(
        bridge
            .reserve_page(None, deadline())
            .await
            .unwrap()
            .queued
            .is_empty()
    );
    assert_eq!(counts(&dir), before);
}

#[tokio::test]
async fn timeout_after_committed_reservation_is_recovered_without_replacement() {
    let (dir, bridge, actor, _) = setup();
    let row = schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let DueDecision::Ready(key) = row.due(200).unwrap() else {
        panic!("due")
    };
    let (committed, received) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let original_deadline = Instant::now() + Duration::from_millis(300);
    let worker = bridge.clone();
    let request = tokio::spawn(async move {
        worker
            .call(original_deadline, move |store| {
                let result = store.automation_reserve_current(
                    &row,
                    &Settlement::Observation(key),
                    || 200,
                    original_deadline,
                )?;
                committed.send(()).unwrap();
                hold.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(result)
            })
            .await
    });
    received.await.unwrap();
    assert_eq!(request.await.unwrap(), Err(Error(ErrorCode::Expired)));
    release.send(()).unwrap();
    bridge.db.call(|_| Ok(())).await.unwrap();
    let jobs = bridge.queued_page(None, deadline()).await.unwrap();
    assert_eq!(jobs.items.len(), 1);
    let before = counts(&dir);
    assert!(
        bridge
            .reserve_page(None, deadline())
            .await
            .unwrap()
            .queued
            .is_empty()
    );
    assert_eq!(bridge.queued_page(None, deadline()).await.unwrap(), jobs);
    assert_eq!(counts(&dir), before);
}

#[tokio::test]
async fn expired_abandoned_worker_request_cannot_retire_the_queued_job() {
    let (dir, bridge, actor, clock) = setup();
    schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let job = bridge
        .reserve_page(None, deadline())
        .await
        .unwrap()
        .queued
        .remove(0);
    clock.store(260, Ordering::SeqCst);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let db = bridge.db.clone();
    let blocked = tokio::spawn(async move {
        db.call(move |_| {
            entered.send(()).unwrap();
            hold.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
        .await
    });
    ready.await.unwrap();
    let before = counts(&dir);
    assert_eq!(
        bridge
            .discard(job.clone(), Instant::now() + Duration::from_millis(80))
            .await,
        Err(Error(ErrorCode::Expired))
    );
    release.send(()).unwrap();
    blocked.await.unwrap().unwrap();
    bridge.db.call(|_| Ok(())).await.unwrap();
    assert_eq!(counts(&dir), before);
    assert_eq!(
        bridge.queued_page(None, deadline()).await.unwrap().items,
        vec![job.clone()]
    );
    assert_eq!(
        bridge.discard(job, deadline()).await.unwrap().state,
        JobState::Failed
    );
    assert_eq!(counts(&dir).2, 1);
}

#[tokio::test]
async fn actual_worker_transfers_private_leases_and_settles_in_app_recovery_once() {
    let (dir, bridge, actor, clock) = setup();
    schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    for (at, status, complete) in [
        (200, SignalStatus::Failing, true),
        (230, SignalStatus::Failing, true),
        (260, SignalStatus::Unavailable, false),
        (290, SignalStatus::Healthy, true),
    ] {
        clock.store(at, Ordering::SeqCst);
        let job = bridge
            .reserve_page(None, deadline())
            .await
            .unwrap()
            .queued
            .remove(0);
        let ObservationAdmission::Lease(lease) = bridge
            .begin(job.clone(), vec!["host:system".into()], deadline())
            .await
            .unwrap()
        else {
            panic!("lease")
        };
        let value = batch(status, complete);
        let receipt = bridge.complete(lease.clone(), value.clone()).await.unwrap();
        assert_eq!(receipt.complete, complete);
        let before = counts(&dir);
        assert_eq!(bridge.complete(lease, value).await.unwrap(), receipt);
        assert_eq!(counts(&dir), before);
        assert!(
            matches!(bridge.begin(job,vec!["host:system".into()],deadline()).await.unwrap(),ObservationAdmission::Existing(original) if original==receipt)
        );
    }
    assert_eq!(counts(&dir).3, 2);
    let mut delivered = 0;
    for _ in 0..3 {
        delivered += bridge
            .publish_page(None, deadline())
            .await
            .unwrap()
            .published;
    }
    assert_eq!(delivered, 2);
    clock.store(290 + 7 * 24 * 3600 + 1, Ordering::SeqCst);
    let mut pruned = 0;
    for _ in 0..3 {
        pruned += bridge.prune_page(None, deadline()).await.unwrap().0;
    }
    assert_eq!(pruned, 2);
    assert_eq!(counts(&dir).3, 0);
    let before = counts(&dir);
    drop(bridge);
    let bridge = ObservationStoreBridge::new(reopen(&dir).await);
    assert!(
        bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(counts(&dir), before);
}

#[tokio::test]
async fn current_grants_resource_scopes_and_pause_are_rechecked_on_the_worker() {
    let (dir, bridge, actor, _) = setup();
    schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let job = bridge
        .reserve_page(None, deadline())
        .await
        .unwrap()
        .queued
        .remove(0);
    assert!(matches!(
        bridge
            .begin(job.clone(), vec!["container:private".into()], deadline())
            .await,
        Err(Error(ErrorCode::Forbidden))
    ));
    let ObservationAdmission::Lease(lease) = bridge
        .begin(job.clone(), vec!["host:system".into()], deadline())
        .await
        .unwrap()
    else {
        panic!("lease")
    };
    let owner = actor.id;
    bridge
        .db
        .call(move |store| {
            store.revise_grants(
                &owner,
                Role::Operator,
                &[Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                }],
                201,
            )
        })
        .await
        .unwrap();
    let before = counts(&dir);
    assert_eq!(
        bridge
            .complete(lease, batch(SignalStatus::Healthy, true))
            .await,
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(counts(&dir), before);
    assert_eq!(
        bridge.discard(job, deadline()).await.unwrap().state,
        JobState::PreconditionChanged
    );

    let (_dir, bridge, actor, _) = setup();
    let row = schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    let job = bridge
        .reserve_page(None, deadline())
        .await
        .unwrap()
        .queued
        .remove(0);
    bridge
        .db
        .call(move |store| {
            store.automation_pause(
                &actor,
                &row.id,
                &PauseSchedule {
                    revision: 1,
                    paused: true,
                },
                201,
            )
        })
        .await
        .unwrap();
    assert!(
        bridge
            .begin(job, vec!["host:system".into()], deadline())
            .await
            .is_err()
    );
    assert!(
        bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn bounded_pass_cursors_cover_every_schedule_and_effects_stay_blocked() {
    let (dir, bridge, actor, _) = setup();
    for _ in 0..33 {
        schedule(&bridge, &actor, ScheduledOperation::ObserveHealth).await;
    }
    for operation in [
        ScheduledOperation::Backup,
        ScheduledOperation::ProtectionSync,
        ScheduledOperation::ProtectionScrub,
        ScheduledOperation::ContainerUpdate,
        ScheduledOperation::PackageReconcile,
    ] {
        schedule(&bridge, &actor, operation).await;
    }
    let mut after = None;
    let (mut scanned, mut queued, mut blocked) = (0, 0, 0);
    loop {
        let pass = bridge.reserve_page(after, deadline()).await.unwrap();
        assert!(pass.scanned <= 32 && pass.queued.len() <= 32);
        scanned += pass.scanned;
        queued += pass.queued.len();
        blocked += pass.blocked;
        after = pass.next;
        if after.is_none() {
            break;
        }
    }
    assert_eq!((scanned, queued, blocked), (38, 33, 5));
    assert_eq!((counts(&dir).0, counts(&dir).1), (33, 38));
    let first = bridge.queued_page(None, deadline()).await.unwrap();
    assert_eq!(first.items.len(), 32);
    let last = bridge.queued_page(first.next, deadline()).await.unwrap();
    assert_eq!(last.items.len(), 1);
    assert_eq!(last.next, None);
    assert!(first.items.last().unwrap().id < last.items[0].id);
    let before = counts(&dir);
    assert_eq!(
        bridge.reserve_page(Some("*".into()), deadline()).await,
        Err(Error(ErrorCode::InvalidInput))
    );
    assert_eq!(counts(&dir), before);
}
