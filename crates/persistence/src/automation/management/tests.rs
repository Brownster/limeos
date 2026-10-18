use super::*;
use std::time::Duration;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
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
fn session(store: &mut Store) -> Principal {
    let record = store.login_record("alice").unwrap().unwrap();
    store.revoke_session("session", 103).unwrap();
    store
        .create_session(&record, None, "session", "csrf", 103)
        .unwrap();
    record.principal
}
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    // Explicit internal fixture grant provisioning, never a role-only authority.
    store
        .revise_grants(&actor.id, Role::Administrator, &scopes(), 102)
        .unwrap();
    let actor = session(&mut store);
    (dir, store, actor)
}
fn input() -> CreateSchedule {
    CreateSchedule {
        definition: ScheduleDefinition {
            operation: ScheduledOperation::ObserveHealth,
            anchor: 0,
            interval_seconds: 30,
            grace_seconds: 5,
            missed: MissedRunPolicy::CoalesceLatest,
            window: None,
        },
    }
}
fn create(store: &mut Store) -> ScheduleView {
    store
        .observe_health_schedule_create("session", "csrf", &input(), || 200, deadline())
        .unwrap()
}
fn snapshot(store: &Store) -> Vec<String> {
    [
        "SELECT json_group_array(json_array(id,owner,policy,owner_revision,revision,operation,definition,paused,settled_through,created,updated)) FROM (SELECT * FROM automation_schedules ORDER BY id)",
        "SELECT json_group_array(json_array(schedule,revision,due,owner,policy,owner_revision,settlement,job,created)) FROM (SELECT * FROM automation_occurrences ORDER BY schedule,revision,due)",
        "SELECT json_group_array(json_array(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)) FROM (SELECT * FROM jobs ORDER BY id)",
        "SELECT json_group_array(json_array(resource,job)) FROM (SELECT * FROM resource_locks ORDER BY resource)",
        "SELECT json_group_array(json_array(cursor,principal,job,kind,created)) FROM (SELECT * FROM events ORDER BY cursor)",
        "SELECT json_array(audit_bytes,generation) FROM meta WHERE id=1",
    ].iter().map(|sql| store.conn.query_row(sql, [], |r| r.get(0)).unwrap()).collect()
}
fn job_state(store: &Store, id: &str) -> JobState {
    let value: String = store
        .conn
        .query_row("SELECT state FROM jobs WHERE id=?", [id], |r| r.get(0))
        .unwrap();
    serde_json::from_value(serde_json::Value::String(value)).unwrap()
}

#[test]
fn owner_operation_pages_are_query_only_bounded_and_survive_reopen() {
    let (dir, mut store, actor) = setup();
    let first = create(&mut store);
    assert_eq!(first.status, ScheduleStatus::AwaitingProducer);
    assert_eq!(first.schedule.definition.anchor, 200);
    for _ in 0..32 {
        create(&mut store);
    }
    let mut other = input();
    other.definition.operation = ScheduledOperation::Backup;
    let blocked = store.automation_create(&actor, &other, 200).unwrap();
    for _ in 0..32 {
        store.automation_create(&actor, &other, 200).unwrap();
    }
    let bob = store.import_legacy_user("bob", "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1", 104).unwrap();
    store
        .revise_grants(&bob.id, Role::Administrator, &scopes(), 105)
        .unwrap();
    let bob = store.login_record("bob").unwrap().unwrap().principal;
    let foreign = store.automation_create(&bob, &input(), 200).unwrap();
    let before = snapshot(&store);
    let changes = store.conn.total_changes();
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    let page = store
        .observe_health_schedule_page("session", None, || 201, deadline())
        .unwrap();
    assert_eq!(page.items.len(), 32);
    assert!(
        page.items
            .iter()
            .all(|row| row.status == ScheduleStatus::AwaitingProducer)
    );
    let tail = store
        .observe_health_schedule_page("session", page.next.as_deref(), || 201, deadline())
        .unwrap();
    assert_eq!(tail.items.len(), 1);
    assert!(tail.next.is_none());
    assert!(
        !page
            .items
            .iter()
            .chain(&tail.items)
            .any(|row| row.schedule.id == blocked.schedule.id)
    );
    assert!(
        !page
            .items
            .iter()
            .chain(&tail.items)
            .any(|row| row.schedule.id == foreign.schedule.id)
    );
    assert_eq!(store.conn.total_changes(), changes);
    assert_eq!(snapshot(&store), before);
    store.conn.execute_batch("PRAGMA query_only=OFF").unwrap();
    assert_eq!(
        store.observe_health_schedule_pause(
            "session",
            "csrf",
            &blocked.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: false
            },
            || 201,
            deadline()
        ),
        Err(Error(ErrorCode::NotFound))
    );
    drop(store);
    let mut reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        reopened
            .observe_health_schedule_page("session", None, || 202, deadline())
            .unwrap()
            .items,
        page.items
    );
    assert_eq!(
        reopened
            .conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn current_read_scope_and_authority_suspension_are_distinct_from_invalid_sessions() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store);
    store
        .revise_grants(&actor.id, Role::Viewer, &scopes()[..1], 201)
        .unwrap();
    assert_eq!(
        store.observe_health_schedule_page("session", None, || 202, deadline()),
        Err(Error(ErrorCode::Unauthenticated))
    );
    session(&mut store);
    let before = snapshot(&store);
    assert_eq!(
        store
            .observe_health_schedule_page("session", None, || 202, deadline())
            .unwrap()
            .items[0]
            .status,
        ScheduleStatus::AuthoritySuspended
    );
    assert_eq!(
        store.observe_health_schedule_create("session", "csrf", &input(), || 202, deadline()),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(
        store.observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: false
            },
            || 202,
            deadline()
        ),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(snapshot(&store), before);
    store
        .revise_grants(&actor.id, Role::Administrator, &scopes()[1..], 203)
        .unwrap();
    session(&mut store);
    assert_eq!(
        store.observe_health_schedule_page("session", None, || 204, deadline()),
        Err(Error(ErrorCode::Forbidden))
    );
}

#[test]
fn pause_cas_noop_resume_and_cancellation_preserve_unrelated_jobs_and_claims() {
    let (_dir, mut store, actor) = setup();
    let row = create(&mut store);
    let other = create(&mut store);
    let queued = |store: &mut Store, row: &Schedule| {
        let settlement = proposed(row, 200).unwrap().unwrap();
        let Reservation::Queued(job) = store.automation_reserve(row, &settlement, 200).unwrap()
        else {
            panic!("job")
        };
        job.id
    };
    let own_job = queued(&mut store, &row.schedule);
    let other_job = queued(&mut store, &other.schedule);
    let effect = store
        .queue_job(
            &actor,
            "unrelated",
            &Intent::HealthProbe {
                resource: "host:system".into(),
            },
            500,
            200,
        )
        .unwrap();
    store
        .transition(&effect, JobState::Queued, JobState::Running, 200)
        .unwrap();
    let claims: Vec<(String, String)> = store
        .conn
        .prepare("SELECT resource,job FROM resource_locks ORDER BY resource")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let same = store.conn.total_changes();
    store
        .observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: false,
            },
            || 201,
            deadline(),
        )
        .unwrap();
    assert_eq!(store.conn.total_changes(), same);
    let paused = store
        .observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            || 201,
            deadline(),
        )
        .unwrap();
    assert_eq!(paused.status, ScheduleStatus::Paused);
    assert_eq!(paused.schedule.definition.anchor, 200);
    assert_eq!(job_state(&store, &own_job), JobState::Canceled);
    assert_eq!(job_state(&store, &other_job), JobState::Queued);
    let before = snapshot(&store);
    assert_eq!(
        store.observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: true
            },
            || 202,
            deadline()
        ),
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(snapshot(&store), before);
    let resumed = store
        .observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 2,
                paused: false,
            },
            || 250,
            deadline(),
        )
        .unwrap();
    assert_eq!(
        (
            resumed.schedule.revision,
            resumed.schedule.definition.anchor,
            resumed.schedule.settled_through
        ),
        (3, 250, None)
    );
    assert_eq!(resumed.status, ScheduleStatus::AwaitingProducer);
    assert_eq!(job_state(&store, &effect), JobState::Running);
    assert_eq!(
        store
            .conn
            .prepare("SELECT resource,job FROM resource_locks ORDER BY resource")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<(String, String)>, _>>()
            .unwrap(),
        claims
    );
}

#[test]
fn audit_and_actual_sql_failures_rollback_creation_and_queued_cancellation() {
    let (_dir, mut store, _actor) = setup();
    let before = snapshot(&store);
    store.conn.execute_batch("CREATE TEMP TRIGGER refuse_created AFTER INSERT ON automation_schedules BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(
        store.observe_health_schedule_create("session", "csrf", &input(), || 200, deadline()),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(snapshot(&store), before);
    store
        .conn
        .execute_batch("DROP TRIGGER refuse_created;")
        .unwrap();
    let row = create(&mut store);
    let settlement = proposed(&row.schedule, 200).unwrap().unwrap();
    store
        .automation_reserve(&row.schedule, &settlement, 200)
        .unwrap();
    let before = snapshot(&store);
    store.conn.execute_batch("CREATE TEMP TRIGGER refuse_paused AFTER UPDATE ON automation_schedules BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(
        store.observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: true
            },
            || 201,
            deadline()
        ),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(snapshot(&store), before);
    store
        .conn
        .execute_batch("DROP TRIGGER refuse_paused;")
        .unwrap();
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [AUDIT_LIMIT])
        .unwrap();
    let before = snapshot(&store);
    assert_eq!(
        store.observe_health_schedule_pause(
            "session",
            "csrf",
            &row.schedule.id,
            &PauseSchedule {
                revision: 1,
                paused: true
            },
            || 201,
            deadline()
        ),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn creation_cap_bad_credentials_and_internal_effect_input_leave_no_partial_rows() {
    let (_dir, mut store, _actor) = setup();
    let mut effect = input();
    effect.definition.operation = ScheduledOperation::Backup;
    let before = snapshot(&store);
    for (token, csrf, value, error) in [
        ("missing", "csrf", input(), ErrorCode::Unauthenticated),
        ("session", "wrong", input(), ErrorCode::Forbidden),
        ("session", "csrf", effect, ErrorCode::InvalidInput),
    ] {
        assert_eq!(
            store.observe_health_schedule_create(token, csrf, &value, || 200, deadline()),
            Err(Error(error))
        );
        assert_eq!(snapshot(&store), before);
    }
    for _ in 0..128 {
        create(&mut store);
    }
    let before = snapshot(&store);
    assert_eq!(
        store.observe_health_schedule_create("session", "csrf", &input(), || 200, deadline()),
        Err(Error(ErrorCode::Overloaded))
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn write_lock_wait_rechecks_session_clock_and_preserves_original_deadline() {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
        mpsc,
    };
    for pause in [false, true] {
        for expire_session in [false, true] {
            let (dir, mut store, _actor) = setup();
            let row = create(&mut store);
            store
                .conn
                .execute("UPDATE sessions SET expires=205 WHERE digest='session'", [])
                .unwrap();
            let before = snapshot(&store);
            let wall = Arc::new(AtomicI64::new(204));
            let calls = AtomicUsize::new(0);
            let moved = wall.clone();
            let path = dir.path().join("core.sqlite");
            let (ready, occupied) = mpsc::sync_channel(0);
            let writer = std::thread::spawn(move || {
                let writer = Connection::open(path).unwrap();
                writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                ready.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(70));
                if expire_session {
                    moved.store(206, Ordering::SeqCst);
                }
                writer.execute_batch("COMMIT").unwrap();
            });
            occupied.recv().unwrap();
            let start = Instant::now();
            let original = if expire_session {
                deadline()
            } else {
                start + Duration::from_millis(20)
            };
            let clock = || {
                calls.fetch_add(1, Ordering::SeqCst);
                wall.load(Ordering::SeqCst)
            };
            let result = if pause {
                store.observe_health_schedule_pause(
                    "session",
                    "csrf",
                    &row.schedule.id,
                    &PauseSchedule {
                        revision: 1,
                        paused: true,
                    },
                    clock,
                    original,
                )
            } else {
                store.observe_health_schedule_create("session", "csrf", &input(), clock, original)
            };
            writer.join().unwrap();
            assert!(start.elapsed() >= Duration::from_millis(50));
            assert_eq!(
                result,
                Err(Error(if expire_session {
                    ErrorCode::Unauthenticated
                } else {
                    ErrorCode::Expired
                }))
            );
            assert_eq!(calls.load(Ordering::SeqCst), usize::from(expire_session));
            if expire_session {
                assert!(Instant::now() < original);
            }
            assert_eq!(snapshot(&store), before);
        }
    }
}
