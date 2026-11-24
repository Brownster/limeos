use super::*;

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let initial = store.login_record("alice").unwrap().unwrap().principal;
    // Explicit fixture provisioning through the existing internal Store method.
    // No enrolled/default role implicitly acquires the new management grant.
    store
        .revise_grants(&initial.id, Role::Administrator, &scopes(), 102)
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    (dir, store, actor)
}
fn scopes() -> Vec<Scope> {
    vec![
        Scope {
            operation: Operation::HealthRead,
            resource: "host:system".into(),
        },
        Scope {
            operation: Operation::AutomationManage,
            resource: "automation:observe-health".into(),
        },
    ]
}
fn input(operation: ScheduledOperation) -> CreateSchedule {
    CreateSchedule {
        definition: ScheduleDefinition {
            operation,
            anchor: 0,
            interval_seconds: 30,
            grace_seconds: 5,
            missed: MissedRunPolicy::CoalesceLatest,
            window: None,
        },
    }
}
fn create(store: &mut Store, actor: &Principal) -> Schedule {
    store
        .automation_create(actor, &input(ScheduledOperation::ObserveHealth), 200)
        .unwrap()
        .schedule
}
fn settlement(schedule: &Schedule, now: i64) -> Settlement {
    proposed(schedule, now).unwrap().unwrap()
}
fn counts(store: &Store) -> (i64, i64, i64, i64) {
    store.conn.query_row("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_occurrences),(SELECT count(*) FROM events),audit_bytes FROM meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
fn watermark(store: &Store, id: &str) -> Option<i64> {
    load(&store.conn, id)
        .unwrap()
        .unwrap()
        .schedule
        .settled_through
}

#[test]
fn reservation_rechecks_time_and_deadline_after_actual_write_lock_wait() {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
        mpsc,
    };
    use std::time::{Duration, Instant};

    for (now, window, expired) in [
        (201, None, false),
        (231, None, false),
        (
            240,
            Some(MaintenanceWindow {
                start_minute: 0,
                end_minute: 4,
            }),
            false,
        ),
        (200, None, true),
    ] {
        let (dir, mut store, actor) = setup();
        let mut input = input(ScheduledOperation::ObserveHealth);
        input.definition.window = window;
        let row = store
            .automation_create(&actor, &input, 200)
            .unwrap()
            .schedule;
        let request = settlement(&row, 200);
        let before = counts(&store);
        let wall = Arc::new(AtomicI64::new(200));
        let moved = wall.clone();
        let calls = AtomicUsize::new(0);
        let path = dir.path().join("core.sqlite");
        let (ready, occupied) = mpsc::sync_channel(0);
        let writer = std::thread::spawn(move || {
            let writer = Connection::open(path).unwrap();
            writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(70));
            moved.store(now, Ordering::SeqCst);
            writer.execute_batch("COMMIT").unwrap();
        });
        occupied.recv().unwrap();
        let start = Instant::now();
        let deadline = start
            + if expired {
                Duration::from_millis(20)
            } else {
                Duration::from_secs(10)
            };
        let clock = || {
            calls.fetch_add(1, Ordering::SeqCst);
            wall.load(Ordering::SeqCst)
        };
        let result = store.automation_reserve_current(&row, &request, clock, deadline);
        writer.join().unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!expired));
        if now == 201 {
            let Reservation::Queued(job) = result.unwrap() else {
                panic!("current due occurrence was not reserved");
            };
            let deadline: i64 = store
                .conn
                .query_row("SELECT deadline FROM jobs WHERE id=?", [&job.id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(deadline, now + JOB_SECONDS);
            assert_eq!(watermark(&store, &row.id), Some(200));
            let after = counts(&store);
            assert_eq!(
                (after.0, after.1, after.2),
                (before.0 + 1, before.1 + 1, before.2 + 2)
            );
            assert!(after.3 > before.3);
        } else {
            assert_eq!(
                result,
                if expired {
                    Err(Error(ErrorCode::Expired))
                } else {
                    Ok(Reservation::Stale)
                }
            );
            assert_eq!(counts(&store), before);
            assert_eq!(watermark(&store, &row.id), None);
        }
    }
}

#[test]
fn current_reservation_replays_after_restart_and_rechecks_owner_revision() {
    use std::time::{Duration, Instant};

    let (dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    assert!(matches!(
        store.automation_reserve_current(
            &row,
            &request,
            || 200,
            Instant::now() + Duration::from_secs(10)
        ),
        Ok(Reservation::Queued(_))
    ));
    let before = counts(&store);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        store.automation_reserve_current(
            &row,
            &request,
            || 260,
            Instant::now() + Duration::from_secs(10)
        ),
        Ok(Reservation::Existing)
    );
    assert_eq!(counts(&store), before);
    store
        .revise_grants(&actor.id, Role::Operator, &scopes(), 261)
        .unwrap();
    let before = counts(&store);
    assert_eq!(
        store.automation_reserve_current(
            &row,
            &request,
            || 262,
            Instant::now() + Duration::from_secs(10)
        ),
        Ok(Reservation::Stale)
    );
    assert_eq!(counts(&store), before);
    assert_eq!(watermark(&store, &row.id), Some(200));
}

#[test]
fn current_reservation_expiring_inside_clock_refuses_before_any_insert() {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let before = counts(&store);
    store.conn.execute_batch("CREATE TEMP TRIGGER refuse_late_reservation BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT,'insert reached after deadline'); END").unwrap();
    let called = Cell::new(false);
    let deadline = Instant::now() + Duration::from_millis(200);
    assert_eq!(
        store.automation_reserve_current(
            &row,
            &request,
            || {
                called.set(true);
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
                200
            },
            deadline
        ),
        Err(Error(ErrorCode::Expired))
    );
    assert!(called.get());
    assert_eq!(counts(&store), before);
    assert_eq!(watermark(&store, &row.id), None);
}

#[test]
fn current_reservation_refuses_expired_deadlines_and_invalid_clock_values() {
    use std::time::{Duration, Instant};

    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let before = counts(&store);
    assert_eq!(
        store.automation_reserve_current(
            &row,
            &request,
            || panic!("expired reservation sampled its clock"),
            Instant::now()
        ),
        Err(Error(ErrorCode::Expired))
    );
    for now in [-1, MAX_TIME + 1] {
        assert_eq!(
            store.automation_reserve_current(
                &row,
                &request,
                || now,
                Instant::now() + Duration::from_secs(10)
            ),
            Err(Error(ErrorCode::InvalidInput))
        );
    }
    assert_eq!(counts(&store), before);
    assert_eq!(watermark(&store, &row.id), None);
}

#[test]
fn explicit_management_and_host_read_grants_are_both_required_without_role_inference() {
    let (_dir, mut store, actor) = setup();
    for (role, grants) in [
        (Role::Administrator, vec![scopes()[0].clone()]),
        (Role::Administrator, vec![scopes()[1].clone()]),
        (
            Role::Administrator,
            vec![
                scopes()[0].clone(),
                Scope {
                    resource: "*".into(),
                    ..scopes()[1].clone()
                },
            ],
        ),
        (Role::Viewer, scopes()),
        (Role::MediaRequester, scopes()),
    ] {
        store.revise_grants(&actor.id, role, &grants, 110).unwrap();
        let current = store.login_record("alice").unwrap().unwrap().principal;
        let before = counts(&store);
        assert_eq!(
            store.automation_create(&current, &input(ScheduledOperation::ObserveHealth), 200),
            Err(Error(ErrorCode::Forbidden))
        );
        assert_eq!(counts(&store), before);
    }
    store
        .revise_grants(&actor.id, Role::Operator, &scopes(), 111)
        .unwrap();
    let current = store.login_record("alice").unwrap().unwrap().principal;
    assert!(
        store
            .automation_create(&current, &input(ScheduledOperation::ObserveHealth), 200)
            .is_ok()
    );
    assert_eq!(
        store.automation_create(&actor, &input(ScheduledOperation::ObserveHealth), 200),
        Err(Error(ErrorCode::Expired))
    );
}

#[test]
fn server_anchor_bounded_read_pages_and_noop_pause_do_not_write() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    assert_eq!(row.definition.anchor, 200);
    for _ in 0..32 {
        create(&mut store, &actor);
    }
    let before = store.conn.total_changes();
    let page = store.automation_schedules(&actor, None).unwrap();
    assert_eq!(page.items.len(), 32);
    assert!(page.next.is_some());
    assert_eq!(
        store
            .automation_schedules(&actor, page.next.as_deref())
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(store.automation_schedule_page(None, 32).unwrap().len(), 32);
    assert!(store.automation_schedule_page(None, 33).is_err());
    assert!(
        store
            .automation_schedules(&actor, Some("wrong-cursor"))
            .is_err()
    );
    store
        .automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: 1,
                paused: false,
            },
            201,
        )
        .unwrap();
    assert_eq!(store.conn.total_changes(), before);
}

#[test]
fn lost_reply_after_grace_next_due_and_refreshed_watermark_replays_original_job() {
    let (dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let Reservation::Queued(job) = store.automation_reserve(&row, &request, 200).unwrap() else {
        panic!("expected one queued observation");
    };
    let before = counts(&store);
    for now in [200, 206, 230, 1000] {
        assert_eq!(
            store.automation_reserve(&row, &request, now).unwrap(),
            Reservation::Existing
        );
    }
    let refreshed = store.automation_schedule_page(None, 1).unwrap().remove(0);
    assert_eq!(
        store.automation_reserve(&refreshed, &request, 231).unwrap(),
        Reservation::Existing
    );
    assert_eq!(counts(&store), before);
    drop(store);
    let mut reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        reopened
            .automation_reserve(&refreshed, &request, 1000)
            .unwrap(),
        Reservation::Existing
    );
    assert_eq!(
        reopened.automation_schedules(&actor, None).unwrap().items[0]
            .last_occurrence
            .as_ref()
            .unwrap()
            .outcome,
        OccurrenceOutcome::Job {
            id: job.id,
            state: JobState::Queued
        }
    );
}

#[test]
fn every_dispatch_boundary_refuses_reserved_or_forged_observation_intent() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let Reservation::Queued(job) = store.automation_reserve(&row, &request, 200).unwrap() else {
        panic!("queued");
    };
    let intent = Intent::AutomationObserveHealth {
        occurrence: job.occurrence.clone(),
    };
    assert_eq!(
        store.queue_job(&actor, "forged", &intent, 250, 201),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(
        store.transition(&job.id, JobState::Queued, JobState::Running, 201),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(
        store.transition(&job.id, JobState::Queued, JobState::Canceled, 201),
        Err(Error(ErrorCode::Forbidden))
    );
    assert!(
        store
            .conn
            .execute("UPDATE jobs SET state='running' WHERE id=?", [&job.id])
            .is_err()
    );
    assert_eq!(store.job_state(&job.id).unwrap(), "queued");
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT count(*) FROM resource_locks WHERE job=?",
                [&job.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn pause_cancels_only_queued_observations_and_resume_is_new_reviewed_revision() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let Reservation::Queued(job) = store.automation_reserve(&row, &request, 200).unwrap() else {
        panic!("queued");
    };
    let before = counts(&store);
    assert_eq!(
        store.automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: 2,
                paused: true
            },
            201
        ),
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(counts(&store), before);
    let paused = store
        .automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            201,
        )
        .unwrap();
    assert_eq!(paused.status, ScheduleStatus::Paused);
    assert_eq!(store.job_state(&job.id).unwrap(), "canceled");
    let resumed = store
        .automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: 2,
                paused: false,
            },
            240,
        )
        .unwrap();
    assert_eq!(
        (
            resumed.schedule.revision,
            resumed.schedule.definition.anchor,
            resumed.schedule.settled_through
        ),
        (3, 240, None)
    );
    assert_eq!(
        store.automation_reserve(&row, &request, 241).unwrap(),
        Reservation::Stale
    );
}

#[test]
fn changed_current_grant_revision_cannot_reserve_or_appear_active() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    store
        .revise_grants(&actor.id, Role::Operator, &scopes(), 201)
        .unwrap();
    let current = store.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        store.automation_reserve(&row, &request, 201).unwrap(),
        Reservation::Stale
    );
    assert_eq!(
        store.automation_schedules(&current, None).unwrap().items[0].status,
        ScheduleStatus::AuthoritySuspended
    );
    let rebound = store
        .automation_pause(
            &current,
            &row.id,
            &PauseSchedule {
                revision: 1,
                paused: false,
            },
            202,
        )
        .unwrap();
    assert_eq!(
        (
            rebound.schedule.revision,
            rebound.schedule.definition.anchor
        ),
        (2, 202)
    );
    assert_eq!(
        store.automation_reserve(&row, &request, 203).unwrap(),
        Reservation::Stale
    );
}

#[test]
fn unsupported_effects_remain_blocked_without_jobs_or_human_approvals() {
    let (_dir, mut store, actor) = setup();
    for operation in [
        ScheduledOperation::Backup,
        ScheduledOperation::ProtectionSync,
        ScheduledOperation::ProtectionScrub,
        ScheduledOperation::ContainerUpdate,
        ScheduledOperation::PackageReconcile,
    ] {
        let row = store
            .automation_create(&actor, &input(operation), 200)
            .unwrap();
        assert_eq!(row.status, ScheduleStatus::Blocked);
        let request = settlement(&row.schedule, 200);
        assert_eq!(
            store
                .automation_reserve(&row.schedule, &request, 200)
                .unwrap(),
            Reservation::Blocked
        );
    }
    assert_eq!(counts(&store).0, 0);
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM storage_mount_plans", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(serde_json::from_str::<CreateSchedule>(r#"{"definition":{"operation":"storage_mount","anchor":200,"interval_seconds":30,"grace_seconds":5,"missed":"skip","window":null}}"#).is_err());
}

#[test]
fn failure_after_job_or_watermark_rolls_back_every_reservation_write() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    for trigger in [
        "CREATE TEMP TRIGGER injected_failure BEFORE INSERT ON automation_occurrences BEGIN SELECT RAISE(ABORT,'after job'); END;",
        "CREATE TEMP TRIGGER injected_failure BEFORE INSERT ON events WHEN json_extract(NEW.kind,'$.kind')='automation_occurrence_settled' BEGIN SELECT RAISE(ABORT,'after watermark and first audit'); END;",
    ] {
        store.conn.execute_batch(trigger).unwrap();
        let before = counts(&store);
        assert_eq!(
            store.automation_reserve(&row, &request, 200),
            Err(Error(ErrorCode::StateNotDurable))
        );
        assert_eq!(counts(&store), before);
        assert_eq!(watermark(&store, &row.id), None);
        store
            .conn
            .execute_batch("DROP TRIGGER injected_failure")
            .unwrap();
    }
}

#[test]
fn full_queue_audit_and_unrelated_idempotency_do_not_advance_watermark() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let request = settlement(&row, 200);
    let key = job_key(request.key()).unwrap();
    let old = store
        .queue_job(
            &actor,
            &key,
            &Intent::HealthProbe {
                resource: "host:system".into(),
            },
            300,
            200,
        )
        .unwrap();
    assert_eq!(
        store.automation_reserve(&row, &request, 200),
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(store.job_state(&old).unwrap(), "queued");
    assert_eq!(watermark(&store, &row.id), None);
    for i in 1..64 {
        store
            .queue_job(
                &actor,
                &format!("full-{i}"),
                &Intent::HealthProbe {
                    resource: "host:system".into(),
                },
                300,
                200,
            )
            .unwrap();
    }
    let before = counts(&store);
    assert_eq!(
        store.automation_reserve(&row, &request, 200),
        Err(Error(ErrorCode::Overloaded))
    );
    assert_eq!(counts(&store), before);
    assert_eq!(watermark(&store, &row.id), None);
    store
        .conn
        .execute(
            "UPDATE meta SET audit_bytes=? WHERE id=1",
            [AUDIT_LIMIT - 256],
        )
        .unwrap();
    assert_eq!(
        store.automation_reserve(&row, &request, 200),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(watermark(&store, &row.id), None);
}

#[test]
fn watermark_and_canceled_job_identity_survive_history_pruning_and_restart() {
    let (dir, mut store, actor) = setup();
    let row = store
        .automation_create(&actor, &input(ScheduledOperation::Backup), 200)
        .unwrap()
        .schedule;
    let first = settlement(&row, 200);
    store.automation_reserve(&row, &first, 200).unwrap();
    let next = store.automation_schedule_page(None, 1).unwrap().remove(0);
    store
        .automation_reserve(&next, &settlement(&next, 230), 230)
        .unwrap();
    store
        .conn
        .execute(
            "DELETE FROM automation_occurrences WHERE schedule=? AND due=200",
            [&row.id],
        )
        .unwrap();
    assert_eq!(
        store.automation_reserve(&row, &first, 240).unwrap(),
        Reservation::Existing
    );
    let observation = create(&mut store, &actor);
    let request = settlement(&observation, 200);
    store
        .automation_reserve(&observation, &request, 200)
        .unwrap();
    store
        .automation_pause(
            &actor,
            &observation.id,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            201,
        )
        .unwrap();
    store
        .conn
        .execute(
            "DELETE FROM automation_occurrences WHERE schedule=?",
            [&observation.id],
        )
        .unwrap();
    drop(store);
    let mut reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        reopened.automation_reserve(&row, &first, 240).unwrap(),
        Reservation::Existing
    );
    assert_eq!(
        reopened
            .automation_reserve(&observation, &request, 240)
            .unwrap(),
        Reservation::Stale
    );
}

#[test]
fn reciprocal_occurrence_to_ordinary_job_corruption_refuses_before_startup_mutation() {
    let (dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    let ordinary = store
        .queue_job(
            &actor,
            "ordinary",
            &Intent::HealthProbe {
                resource: "host:system".into(),
            },
            300,
            200,
        )
        .unwrap();
    store.conn.execute("INSERT INTO automation_occurrences VALUES(?,1,200,?,'health-v1',?,'observation',?,200,0)",params![row.id,actor.id,actor.grant_revision,ordinary]).unwrap();
    store
        .conn
        .execute(
            "UPDATE automation_schedules SET settled_through=200 WHERE id=?",
            [&row.id],
        )
        .unwrap();
    let generation = store.generation;
    let before = counts(&store);
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before.2
    );
}

#[test]
fn altered_job_key_digest_resource_or_occurrence_revision_refuses_startup() {
    for mutation in [
        "UPDATE jobs SET digest=printf('%064d',0) WHERE id=?",
        "UPDATE jobs SET resource='host:system' WHERE id=?",
        "UPDATE jobs SET intent=json_set(intent,'$.occurrence.due',230) WHERE id=?",
        "UPDATE automation_occurrences SET owner_revision=owner_revision+1 WHERE job=?",
    ] {
        let (dir, mut store, actor) = setup();
        let schedule = create(&mut store, &actor);
        let Reservation::Queued(job) = store
            .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
            .unwrap()
        else {
            panic!("queued");
        };
        let name = if mutation.contains("automation_occurrences") {
            "automation_occurrence_identity"
        } else {
            "jobs_identity_immutable"
        };
        let original: String = store
            .conn
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?", [name], |r| {
                r.get(0)
            })
            .unwrap();
        // Build a corrupt fixture, restoring the exact guard before startup, so
        // refusal cannot be attributed merely to a missing/changed trigger.
        store
            .conn
            .execute_batch(&format!("DROP TRIGGER {name}"))
            .unwrap();
        store.conn.execute(mutation, [&job.id]).unwrap();
        store.conn.execute_batch(&original).unwrap();
        let generation = store.generation;
        let before = counts(&store);
        drop(store);
        assert!(
            matches!(
                Store::open(&dir.path().join("core.sqlite")),
                Err(Error(ErrorCode::StateNotDurable))
            ),
            "{mutation}"
        );
        let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            generation
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before.2
        );
    }
}

#[test]
fn missing_automation_dispatch_guard_refuses_startup() {
    let (dir, store, _actor) = setup();
    let generation = store.generation;
    store
        .conn
        .execute_batch("DROP TRIGGER automation_job_update_state")
        .unwrap();
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
}

#[test]
fn unexplained_current_revision_cancellation_and_missing_pause_audit_refuse_startup() {
    for remove_real_audit in [false, true] {
        let (dir, mut store, actor) = setup();
        let schedule = create(&mut store, &actor);
        let Reservation::Queued(job) = store
            .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
            .unwrap()
        else {
            panic!("queued");
        };
        if remove_real_audit {
            store
                .automation_pause(
                    &actor,
                    &schedule.id,
                    &PauseSchedule {
                        revision: 1,
                        paused: true,
                    },
                    201,
                )
                .unwrap();
            store
                .conn
                .execute(
                    "DELETE FROM events WHERE job=? AND kind=?",
                    params![
                        job.id,
                        json(&EventKind::JobTransition {
                            state: JobState::Canceled
                        })
                        .unwrap()
                    ],
                )
                .unwrap();
        } else {
            let refreshed = store.automation_schedule_page(None, 1).unwrap().remove(0);
            store
                .automation_reserve(&refreshed, &settlement(&refreshed, 230), 230)
                .unwrap();
            store
                .conn
                .execute("UPDATE jobs SET state='canceled' WHERE id=?", [&job.id])
                .unwrap();
        }
        store
            .conn
            .execute("DELETE FROM automation_occurrences WHERE job=?", [&job.id])
            .unwrap();
        let generation = store.generation;
        drop(store);
        assert!(matches!(
            Store::open(&dir.path().join("core.sqlite")),
            Err(Error(ErrorCode::StateNotDurable))
        ));
        let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            generation
        );
    }
}

fn assert_startup_refuses_without_mutation(dir: tempfile::TempDir, store: Store) {
    let generation = store.generation;
    let before = counts(&store);
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before.2
    );
}

#[test]
fn startup_rejects_wrong_missing_or_extra_queued_audit_owner() {
    for mutation in ["wrong_owner", "null_owner", "extra_wrong_owner"] {
        let (dir, mut store, actor) = setup();
        let schedule = create(&mut store, &actor);
        let Reservation::Queued(job) = store
            .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
            .unwrap()
        else {
            panic!("queued");
        };
        let other = limeos_identity::opaque().unwrap();
        store.conn.execute("INSERT INTO users SELECT ?,'bob',password_hash,role,grant_revision FROM users WHERE id=?",params![other,actor.id]).unwrap();
        let kind = json(&EventKind::JobQueued).unwrap();
        match mutation {
            "wrong_owner" => {
                store
                    .conn
                    .execute(
                        "UPDATE events SET principal=? WHERE job=? AND kind=?",
                        params![other, job.id, kind],
                    )
                    .unwrap();
            }
            "null_owner" => {
                store
                    .conn
                    .execute(
                        "UPDATE events SET principal=NULL WHERE job=? AND kind=?",
                        params![job.id, kind],
                    )
                    .unwrap();
            }
            _ => {
                store.conn.execute("INSERT INTO events(principal,job,kind,created) SELECT ?,job,kind,created FROM events WHERE job=? AND kind=?",params![other,job.id,kind]).unwrap();
                store
                    .conn
                    .execute("UPDATE meta SET audit_bytes=audit_bytes+256 WHERE id=1", [])
                    .unwrap();
            }
        }
        assert_startup_refuses_without_mutation(dir, store);
    }
}

#[test]
fn startup_rejects_out_of_domain_deadlines_in_retained_and_pruned_history() {
    for pruned in [false, true] {
        let (dir, mut store, actor) = setup();
        let schedule = create(&mut store, &actor);
        let Reservation::Queued(job) = store
            .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
            .unwrap()
        else {
            panic!("queued");
        };
        if pruned {
            store
                .automation_pause(
                    &actor,
                    &schedule.id,
                    &PauseSchedule {
                        revision: 1,
                        paused: true,
                    },
                    201,
                )
                .unwrap();
            store
                .conn
                .execute("DELETE FROM automation_occurrences WHERE job=?", [&job.id])
                .unwrap();
        }
        let guard: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='jobs_identity_immutable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute_batch("DROP TRIGGER jobs_identity_immutable")
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET deadline=? WHERE id=?",
                params![MAX_TIME + 1, job.id],
            )
            .unwrap();
        store.conn.execute_batch(&guard).unwrap();
        let changed = json(&EventKind::AutomationScheduleChanged {
            schedule: schedule.id,
            revision: 2,
        })
        .unwrap();
        store
            .conn
            .execute(
                "UPDATE events SET created=? WHERE job=? OR kind=?",
                params![MAX_TIME - 59, job.id, changed],
            )
            .unwrap();
        if !pruned {
            let guard: String = store
                .conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name='automation_occurrence_identity'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            store
                .conn
                .execute_batch("DROP TRIGGER automation_occurrence_identity")
                .unwrap();
            store
                .conn
                .execute(
                    "UPDATE automation_occurrences SET created=? WHERE job=?",
                    params![MAX_TIME - 59, job.id],
                )
                .unwrap();
            store.conn.execute_batch(&guard).unwrap();
        }
        assert_startup_refuses_without_mutation(dir, store);
    }
}

#[test]
fn startup_rejects_observation_jobs_bound_to_effect_schedule_history() {
    for pruned in [false, true] {
        let (dir, mut store, actor) = setup();
        let schedule = create(&mut store, &actor);
        let Reservation::Queued(job) = store
            .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
            .unwrap()
        else {
            panic!("queued");
        };
        if pruned {
            store
                .automation_pause(
                    &actor,
                    &schedule.id,
                    &PauseSchedule {
                        revision: 1,
                        paused: true,
                    },
                    201,
                )
                .unwrap();
            store
                .conn
                .execute("DELETE FROM automation_occurrences WHERE job=?", [&job.id])
                .unwrap();
        }
        let guard: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='automation_schedule_identity'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute_batch("DROP TRIGGER automation_schedule_identity")
            .unwrap();
        store.conn.execute("UPDATE automation_schedules SET operation='backup',definition=json_set(definition,'$.operation','backup') WHERE id=?", [&schedule.id]).unwrap();
        store.conn.execute_batch(&guard).unwrap();
        assert_startup_refuses_without_mutation(dir, store);
    }
}

#[test]
fn last_supported_observation_deadline_is_exact_and_overflow_has_no_writes() {
    let (dir, mut store, actor) = setup();
    let accepted = store
        .automation_create(
            &actor,
            &input(ScheduledOperation::ObserveHealth),
            MAX_TIME - 60,
        )
        .unwrap()
        .schedule;
    let Reservation::Queued(job) = store
        .automation_reserve(
            &accepted,
            &settlement(&accepted, MAX_TIME - 60),
            MAX_TIME - 60,
        )
        .unwrap()
    else {
        panic!("queued");
    };
    assert_eq!(
        store
            .conn
            .query_row("SELECT deadline FROM jobs WHERE id=?", [&job.id], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        MAX_TIME
    );
    let refused = store
        .automation_create(
            &actor,
            &input(ScheduledOperation::ObserveHealth),
            MAX_TIME - 59,
        )
        .unwrap()
        .schedule;
    let before = counts(&store);
    assert_eq!(
        store.automation_reserve(
            &refused,
            &settlement(&refused, MAX_TIME - 59),
            MAX_TIME - 59
        ),
        Err(Error(ErrorCode::InvalidInput))
    );
    assert_eq!(counts(&store), before);
    assert_eq!(watermark(&store, &refused.id), None);
    drop(store);
    assert!(Store::open(&dir.path().join("core.sqlite")).is_ok());
}

#[test]
fn an_administrator_is_told_to_pause_instead_and_the_observation_is_untouched() {
    let (dir, mut store, actor) = setup();
    let schedule = create(&mut store, &actor);
    let Reservation::Queued(job) = store
        .automation_reserve(&schedule, &settlement(&schedule, 200), 200)
        .unwrap()
    else {
        panic!("queued");
    };
    let changes = store.conn.total_changes();
    let answer = store.operator_cancel_job(&actor, &job.id, 201).unwrap();
    assert_eq!(
        answer,
        limeos_domain::OperatorCancel {
            job: job.id.clone(),
            family: limeos_domain::JobFamily::AutomationObservation,
            state: JobState::Queued,
            outcome: limeos_domain::CancelOutcome::Refused,
            refusal: Some(limeos_domain::CancelRefusal::PauseScheduleInstead),
        }
    );
    assert_eq!(store.conn.total_changes(), changes);
    assert_eq!(store.job_state(&job.id).unwrap(), "queued");
    let listed = store
        .operator_jobs(&crate::job_admin::tests::only(&[JobState::Queued]), 64)
        .unwrap()
        .jobs
        .remove(0);
    assert_eq!(
        listed.family,
        limeos_domain::JobFamily::AutomationObservation
    );
    assert_eq!(listed.resource, "system");
    drop(store);
    Store::open(&dir.path().join("core.sqlite")).unwrap();
}

/// An occurrence whose table row names an ordinary job while its entry in
/// `automation_occurrences`' job index still names the observation job it
/// was reserved for (the job id rewritten on the table page). Every other
/// check reads the occurrence through that index, as the occurrence check
/// did before it read the table (`NOT INDEXED`), and passes. Refused by the
/// occurrence check.
#[test]
fn an_occurrence_naming_an_ordinary_job_on_its_table_page_refuses_startup() {
    use std::time::{Duration, Instant};

    let (dir, mut store, actor) = setup();
    let row = create(&mut store, &actor);
    assert!(matches!(
        store.automation_reserve_current(
            &row,
            &settlement(&row, 200),
            || 200,
            Instant::now() + Duration::from_secs(10)
        ),
        Ok(Reservation::Queued(_))
    ));
    let ordinary = store
        .queue_job(
            &actor,
            "ordinary",
            &Intent::HealthProbe {
                resource: "host:system".into(),
            },
            300,
            200,
        )
        .unwrap();
    let observed: String = store
        .conn
        .query_row("SELECT job FROM automation_occurrences", [], |r| r.get(0))
        .unwrap();
    drop(store);
    let path = dir.path().join("core.sqlite");
    // The store as built opens.
    let generation = Store::open(&path).unwrap().generation;
    assert_eq!(observed.len(), ordinary.len());
    assert!(
        crate::resources::tests::rewrite_in(&path, "automation_occurrences", &observed, &ordinary)
            >= 1
    );
    crate::resources::tests::whole_file_checks_pass(&path);
    let conn = Connection::open(&path).unwrap();
    let job = |sql: &str| -> String { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        job("SELECT job FROM automation_occurrences NOT INDEXED"),
        ordinary
    );
    assert_eq!(
        job(
            "SELECT job FROM automation_occurrences INDEXED BY sqlite_autoindex_automation_occurrences_1"
        ),
        observed
    );
    drop(conn);
    crate::resources::tests::refused_unchanged(&dir, generation, "occurrence");
}
