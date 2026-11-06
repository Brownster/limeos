use super::super::tests::{alert, batch, begin, counts, deadline, queue, setup};
use super::*;

fn schedule(store: &mut Store, actor: &Principal) -> String {
    store
        .automation_create(
            actor,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: 0,
                    interval_seconds: 30,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            120,
        )
        .unwrap()
        .schedule
        .id
}
fn lease(store: &mut Store, id: &str, at: i64) -> Box<ObservationPollLease> {
    let ObservationPollAdmission::Lease(lease) = store
        .automation_poll_begin(
            id,
            vec!["host:system".into(), "container:plex".into()],
            || at,
            deadline(),
        )
        .unwrap()
    else {
        panic!("poll lease")
    };
    lease
}
fn sample(
    store: &mut Store,
    id: &str,
    at: i64,
    status: SignalStatus,
    complete: bool,
) -> ObservationPollOutcome {
    let lease = lease(store, id, at);
    store
        .automation_poll_complete(&lease, &batch(status, complete), || at)
        .unwrap()
}
fn changes(store: &Store) -> i64 {
    store
        .conn
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn healthy_polls_are_read_only_even_when_audit_capacity_is_exhausted() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    assert!(matches!(
        sample(&mut store, &id, 120, SignalStatus::Healthy, true),
        ObservationPollOutcome::Committed(_)
    ));
    let stable = counts(&store);
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [AUDIT_LIMIT])
        .unwrap();
    let before = changes(&store);
    for at in (150..=1080).step_by(30) {
        assert_eq!(
            sample(&mut store, &id, at, SignalStatus::Healthy, true),
            ObservationPollOutcome::Unchanged {
                complete: true,
                sampled_at: at
            }
        );
        assert!(matches!(
            store
                .automation_poll_begin(&id, vec!["host:system".into()], || at, deadline())
                .unwrap(),
            ObservationPollAdmission::NotDue
        ));
    }
    assert_eq!(changes(&store), before);
    let after = counts(&store);
    assert_eq!(
        (after.0, after.1, after.2, after.3, after.4),
        (stable.0, stable.1, stable.2, stable.3, stable.4)
    );
    let poll = lease(&mut store, &id, 1110);
    assert_eq!(
        store.automation_poll_complete(&poll, &batch(SignalStatus::Unavailable, false), || 1110),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(changes(&store), before);
}

#[test]
fn transitions_emit_once_and_stable_failure_and_recovery_do_not_grow_history() {
    let (dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    sample(&mut store, &id, 120, SignalStatus::Healthy, true);
    assert!(matches!(
        sample(&mut store, &id, 150, SignalStatus::Failing, true),
        ObservationPollOutcome::Committed(_)
    ));
    let opened = sample(&mut store, &id, 180, SignalStatus::Failing, true);
    assert!(matches!(
        opened,
        ObservationPollOutcome::Committed(ObservationResult {
            notifications: 1,
            ..
        })
    ));
    let stable = counts(&store);
    assert!(matches!(alert(&store).incident, IncidentState::Open { .. }));
    for at in [210, 240, 270] {
        assert!(matches!(
            sample(&mut store, &id, at, SignalStatus::Failing, true),
            ObservationPollOutcome::Unchanged { .. }
        ));
    }
    assert_eq!(counts(&store), stable);
    assert!(matches!(
        sample(&mut store, &id, 300, SignalStatus::Unavailable, false),
        ObservationPollOutcome::Committed(ObservationResult {
            complete: false,
            notifications: 0,
            ..
        })
    ));
    assert!(matches!(alert(&store).incident, IncidentState::Open { .. }));
    let stable = counts(&store);
    assert!(matches!(
        sample(&mut store, &id, 330, SignalStatus::Unavailable, false),
        ObservationPollOutcome::Unchanged {
            complete: false,
            ..
        }
    ));
    assert_eq!(counts(&store), stable);
    assert!(matches!(
        sample(&mut store, &id, 360, SignalStatus::Healthy, true),
        ObservationPollOutcome::Committed(ObservationResult {
            notifications: 1,
            ..
        })
    ));
    assert!(matches!(
        sample(&mut store, &id, 390, SignalStatus::Healthy, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
    let stable = counts(&store);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    // The volatile watermark is gone; a fresh stable read repeats, with no delivery/job duplicate.
    assert!(matches!(
        sample(&mut store, &id, 390, SignalStatus::Healthy, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
    assert_eq!(counts(&store), stable);
}

#[test]
fn original_nonce_body_authority_and_deadline_fence_completion_and_replay() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let first = lease(&mut store, &id, 120);
    let receipt = store
        .automation_poll_complete(&first, &batch(SignalStatus::Healthy, true), || 126)
        .unwrap();
    assert!(
        matches!(receipt, ObservationPollOutcome::Committed(_)),
        "original admission can complete beyond five-second grace"
    );
    let before = changes(&store);
    assert_eq!(
        store
            .automation_poll_complete(&first, &batch(SignalStatus::Healthy, true), || 127)
            .unwrap(),
        receipt
    );
    assert_eq!(changes(&store), before);
    assert_eq!(
        store.automation_poll_complete(&first, &batch(SignalStatus::Failing, true), || 127),
        Err(Error(ErrorCode::Conflict))
    );
    let mut forged = (*first).clone();
    forged.nonce = limeos_identity::opaque().unwrap();
    assert_eq!(
        store.automation_poll_complete(&forged, &batch(SignalStatus::Healthy, true), || 128),
        Err(Error(ErrorCode::Conflict))
    );
    let expired = lease(&mut store, &id, 150);
    assert_eq!(
        store.automation_poll_complete(&expired, &batch(SignalStatus::Failing, true), || 161),
        Err(Error(ErrorCode::Expired))
    );
    let rolled = lease(&mut store, &id, 150);
    assert_eq!(
        store.automation_poll_complete(&rolled, &batch(SignalStatus::Failing, true), || 149),
        Err(Error(ErrorCode::Expired))
    );
    let revoked = lease(&mut store, &id, 150);
    store
        .automation_pause(
            &actor,
            &id,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            151,
        )
        .unwrap();
    let before = changes(&store);
    assert_eq!(
        store.automation_poll_complete(&revoked, &batch(SignalStatus::Failing, true), || 152),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(changes(&store), before);
}

#[test]
fn full_batch_validation_refuses_late_binding_conflicts_and_invalid_completeness() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let mut value = batch(SignalStatus::Failing, true);
    let mut invalid = value.signals[0].clone();
    invalid.signal.key = "container:unknown".into();
    invalid.signal.kind = AlertKind::Container;
    invalid.resource = "container:unknown".into();
    value.signals.push(invalid);
    let poll = lease(&mut store, &id, 120);
    let before = changes(&store);
    assert_eq!(
        store.automation_poll_complete(&poll, &value, || 120),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(changes(&store), before);
    let poll = lease(&mut store, &id, 120);
    assert_eq!(
        store.automation_poll_complete(&poll, &batch(SignalStatus::Unavailable, true), || 120),
        Err(Error(ErrorCode::InvalidInput))
    );
    assert_eq!(changes(&store), before);
    assert!(matches!(
        sample(&mut store, &id, 120, SignalStatus::Healthy, true),
        ObservationPollOutcome::Committed(_)
    ));
}

#[test]
fn poll_abandon_cannot_release_newer_poll_or_queued_observation() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let abandoned = lease(&mut store, &id, 120);
    store.automation_poll_abandon(&abandoned);
    let current = lease(&mut store, &id, 120);
    store.automation_poll_abandon(&abandoned);
    assert!(matches!(
        store.automation_poll_begin(&id, vec!["host:system".into()], || 120, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    store.automation_poll_abandon(&current);
    let queued = queue(&mut store, &actor, 120);
    assert!(matches!(
        store.automation_poll_begin(&id, vec!["host:system".into()], || 120, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    let old = begin(&mut store, &queued, 120);
    store.automation_poll_abandon(&abandoned);
    store
        .automation_observation_complete(&old, &batch(SignalStatus::Healthy, true), || 120)
        .unwrap();
    assert!(matches!(
        sample(&mut store, &id, 120, SignalStatus::Healthy, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
}

#[test]
fn transaction_failure_leaves_no_queued_poll_and_original_lease_is_consumed() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    store.conn.execute_batch("CREATE TRIGGER fixture_fail BEFORE INSERT ON automation_observation_results BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    let poll = lease(&mut store, &id, 120);
    let before = counts(&store);
    assert!(matches!(
        store.automation_poll_complete(&poll, &batch(SignalStatus::Failing, true), || 120),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    assert_eq!(counts(&store), before);
    assert!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items
            .is_empty()
    );
    store
        .conn
        .execute_batch("DROP TRIGGER fixture_fail")
        .unwrap();
    assert_eq!(
        store.automation_poll_complete(&poll, &batch(SignalStatus::Failing, true), || 120),
        Err(Error(ErrorCode::Expired))
    );
    assert!(matches!(
        sample(&mut store, &id, 120, SignalStatus::Failing, true),
        ObservationPollOutcome::Committed(_)
    ));
}

fn replace_policy(store: &mut Store, definition: HealthPolicyDefinition, at: i64) {
    let revision = store
        .observe_health_policy("session", || at, deadline())
        .unwrap()
        .revision;
    store
        .observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision,
                definition,
            },
            || at,
            deadline(),
        )
        .unwrap();
}

#[test]
fn current_policy_and_grants_are_rechecked_and_muted_incidents_announce_once() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let old = lease(&mut store, &id, 120);
    let muted = HealthPolicyDefinition {
        fail_threshold: 1,
        disabled: vec![AlertKind::Host],
        ..Default::default()
    };
    replace_policy(&mut store, muted, 121);
    let before = counts(&store);
    assert_eq!(
        store.automation_poll_complete(&old, &batch(SignalStatus::Failing, true), || 122),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(counts(&store), before);
    assert!(matches!(
        sample(&mut store, &id, 122, SignalStatus::Failing, true),
        ObservationPollOutcome::Committed(ObservationResult {
            notifications: 0,
            ..
        })
    ));
    assert!(matches!(
        alert(&store).incident,
        IncidentState::Open {
            announced: false,
            ..
        }
    ));
    let stable = counts(&store);
    assert!(matches!(
        sample(&mut store, &id, 150, SignalStatus::Failing, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
    assert_eq!(counts(&store), stable);
    replace_policy(
        &mut store,
        HealthPolicyDefinition {
            fail_threshold: 1,
            ..Default::default()
        },
        151,
    );
    assert!(matches!(
        sample(&mut store, &id, 180, SignalStatus::Failing, true),
        ObservationPollOutcome::Committed(ObservationResult {
            notifications: 1,
            ..
        })
    ));
    assert!(matches!(
        sample(&mut store, &id, 210, SignalStatus::Failing, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
    let revoked = lease(&mut store, &id, 240);
    store
        .revise_grants(
            &actor.id,
            Role::Operator,
            &[Scope {
                operation: Operation::HealthRead,
                resource: "host:system".into(),
            }],
            241,
        )
        .unwrap();
    let before = counts(&store);
    assert!(matches!(
        store.automation_poll_complete(&revoked, &batch(SignalStatus::Healthy, true), || 242),
        Err(Error(ErrorCode::Forbidden | ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn missing_and_corrupt_poll_proofs_refuse_before_startup_recovery() {
    for mutation in [
        "DROP TRIGGER automation_poll_origin_insert",
        "DROP TRIGGER automation_polled_result",
        "UPDATE events SET kind=json_set(kind,'$.digest',printf('%064d',0)) WHERE json_extract(kind,'$.kind')='automation_observation_polled'",
        "DELETE FROM events WHERE json_extract(kind,'$.kind')='automation_observation_polled'",
        "UPDATE events SET created=created+1 WHERE json_extract(kind,'$.kind')='automation_observation_polled'",
        "UPDATE events SET principal=NULL WHERE json_extract(kind,'$.kind')='automation_observation_polled'",
    ] {
        let (dir, mut store, actor) = setup();
        let id = schedule(&mut store, &actor);
        sample(&mut store, &id, 120, SignalStatus::Healthy, true);
        let generation = store.generation;
        store.conn.execute_batch(mutation).unwrap();
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
    }
    for mutation in [
        "admitted=admitted+1",
        "previous_due=120",
        "lease_digest=printf('%064d',0)",
        "definition=json_set(definition,'$.grace_seconds',0)",
    ] {
        let (dir, mut store, actor) = setup();
        let id = schedule(&mut store, &actor);
        sample(&mut store, &id, 122, SignalStatus::Healthy, true);
        let sql: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='automation_poll_origin_update'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute_batch("DROP TRIGGER automation_poll_origin_update")
            .unwrap();
        store
            .conn
            .execute_batch(&format!("UPDATE automation_poll_origins SET {mutation}"))
            .unwrap();
        store.conn.execute_batch(&sql).unwrap();
        drop(store);
        assert!(
            matches!(
                Store::open(&dir.path().join("core.sqlite")),
                Err(Error(ErrorCode::StateNotDurable))
            ),
            "{mutation}"
        );
    }
}

#[test]
fn schema_thirteen_upgrade_preserves_policy_history_and_legacy_queue() {
    let (dir, mut store, actor) = setup();
    replace_policy(
        &mut store,
        HealthPolicyDefinition {
            fail_threshold: 1,
            ..Default::default()
        },
        120,
    );
    let queued = queue(&mut store, &actor, 120);
    let read = begin(&mut store, &queued, 120);
    store
        .automation_observation_complete(&read, &batch(SignalStatus::Failing, true), || 120)
        .unwrap();
    let pending = queue(&mut store, &actor, 150);
    let before = counts(&store);
    let bytes: String = store
        .conn
        .query_row("SELECT intent FROM jobs WHERE id=?", [&pending.id], |r| {
            r.get(0)
        })
        .unwrap();
    crate::remove_v16_schema(&store.conn);
    store.conn.execute_batch("DROP TABLE stack_results; DROP TABLE stack_plans; DROP TRIGGER automation_polled_result; DROP TABLE automation_poll_origins; PRAGMA user_version=13").unwrap();
    let reference = Connection::open_in_memory().unwrap();
    reference
        .execute_batch(concat!(
            include_str!("../../../schema.sql"),
            include_str!("../../../migration-v2.sql"),
            include_str!("../../../migration-v3.sql"),
            include_str!("../../../migration-v4.sql"),
            include_str!("../../../migration-v5.sql"),
            include_str!("../../../migration-v6.sql"),
            include_str!("../../../migration-v7.sql"),
            include_str!("../../../migration-v8.sql"),
            include_str!("../../../migration-v9.sql"),
            include_str!("../../../migration-v10.sql"),
            include_str!("../../../migration-v11.sql"),
            include_str!("../../../migration-v12.sql"),
            include_str!("../../../migration-v13.sql")
        ))
        .unwrap();
    fn objects(conn: &Connection) -> Vec<(String, String, String)> {
        conn.prepare(
            "SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
    }
    assert_eq!(objects(&store.conn), objects(&reference));
    // Fail after the new table/first trigger have been created. Their DDL and
    // user_version must roll back while all old authority and history survive.
    let generation = store.generation;
    store.conn.execute_batch("CREATE TRIGGER automation_poll_origin_update BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT,'fixture migration collision'); END").unwrap();
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    conn.execute_batch("DROP TRIGGER automation_poll_origin_update")
        .unwrap();
    assert_eq!(objects(&conn), objects(&reference));
    drop(conn);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(counts(&store), before);
    assert_eq!(
        store
            .observe_health_policy("session", || 151, deadline())
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT intent FROM jobs WHERE id=?", [&pending.id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        bytes
    );
    let read = begin(&mut store, &pending, 151);
    store
        .automation_observation_complete(&read, &batch(SignalStatus::Healthy, true), || 151)
        .unwrap();
    let id = schedule(&mut store, &actor);
    assert!(matches!(
        sample(&mut store, &id, 180, SignalStatus::Healthy, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
    drop(store);
    Store::open(&dir.path().join("core.sqlite")).unwrap();
}

#[test]
fn volatile_watermarks_preserve_skip_and_clock_rules_without_duplicate_jobs() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    sample(&mut store, &id, 120, SignalStatus::Healthy, true);
    sample(&mut store, &id, 150, SignalStatus::Healthy, true);
    assert!(matches!(
        store
            .automation_poll_begin(&id, vec!["host:system".into()], || 140, deadline())
            .unwrap(),
        ObservationPollAdmission::ClockRollback
    ));
    let before = counts(&store);
    let ObservationPollAdmission::Skipped(key) = store
        .automation_poll_begin(&id, vec!["host:system".into()], || 186, deadline())
        .unwrap()
    else {
        panic!("skipped")
    };
    assert_eq!(key.due, 180);
    let after = counts(&store);
    assert_eq!(after.0, before.0);
    assert_eq!(after.4, before.4 + 1);
    let changes = changes(&store);
    assert!(matches!(
        store
            .automation_poll_begin(&id, vec!["host:system".into()], || 187, deadline())
            .unwrap(),
        ObservationPollAdmission::NotDue
    ));
    assert_eq!(self::changes(&store), changes);
    assert!(matches!(
        sample(&mut store, &id, 210, SignalStatus::Healthy, true),
        ObservationPollOutcome::Unchanged { .. }
    ));
}

#[test]
fn restart_rejects_old_lease_without_releasing_the_new_generations_poll() {
    let (dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let old = lease(&mut store, &id, 120);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let current = lease(&mut store, &id, 120);
    let before = counts(&store);
    assert_eq!(
        store.automation_poll_complete(&old, &batch(SignalStatus::Failing, true), || 121),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(counts(&store), before);
    assert!(matches!(
        store.automation_poll_begin(&id, vec!["host:system".into()], || 121, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    store
        .automation_poll_complete(&current, &batch(SignalStatus::Healthy, true), || 121)
        .unwrap();
}

#[test]
fn deferred_origin_constraint_prevents_a_committed_intermediate_queued_poll() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let schedule = load(&store.conn, &id).unwrap().unwrap().schedule;
    let DueDecision::Ready(key) = schedule.due(120).unwrap() else {
        panic!("due")
    };
    let before = counts(&store);
    let generation = store.generation;
    let tx = store
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let Reservation::Queued(job) = reserve_admitted_occurrence(
        &tx,
        &schedule,
        &Settlement::Observation(key),
        120,
        generation,
        Some((120, None)),
    )
    .unwrap() else {
        panic!("queued")
    };
    tx.execute(
        "INSERT INTO automation_poll_origins VALUES(?,?,?,?,?)",
        params![
            job.id,
            json(&schedule.definition).unwrap(),
            120,
            Option::<i64>::None,
            limeos_identity::digest("fixture")
        ],
    )
    .unwrap();
    assert!(tx.commit().is_err());
    assert_eq!(counts(&store), before);
    assert!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn legal_pruning_of_old_occurrence_detail_keeps_poll_proofs_valid() {
    let (dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    sample(&mut store, &id, 120, SignalStatus::Healthy, true);
    sample(&mut store, &id, 150, SignalStatus::Failing, true);
    assert_eq!(
        store
            .conn
            .execute(
                "DELETE FROM automation_occurrences WHERE schedule=? AND due=120",
                [&id]
            )
            .unwrap(),
        1
    );
    let before = counts(&store);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(counts(&store), before);
    sample(&mut store, &id, 180, SignalStatus::Failing, true);
}

#[test]
fn fresh_reader_deadline_is_rechecked_after_the_actual_sqlite_write_lock_wait() {
    let (dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let poll = lease(&mut store, &id, 120);
    let blocker = Connection::open(dir.path().join("core.sqlite")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = counts(&store);
    let deadline = Instant::now() + Duration::from_millis(80);
    let thread = std::thread::spawn(move || {
        let answer = store.automation_poll_complete_before(
            &poll,
            &batch(SignalStatus::Failing, true),
            || 120,
            deadline,
        );
        (store, answer)
    });
    std::thread::sleep(Duration::from_millis(120));
    blocker.execute_batch("ROLLBACK").unwrap();
    let (mut store, answer) = thread.join().unwrap();
    assert_eq!(answer, Err(Error(ErrorCode::Expired)));
    assert_eq!(counts(&store), before);
    assert!(matches!(
        sample(&mut store, &id, 120, SignalStatus::Healthy, true),
        ObservationPollOutcome::Committed(_)
    ));
}

#[test]
fn a_later_read_deadline_cannot_extend_the_original_poll_lease() {
    let (_dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    let ObservationPollAdmission::Lease(poll) = store
        .automation_poll_begin(
            &id,
            vec!["host:system".into()],
            || 120,
            Instant::now() + Duration::from_millis(20),
        )
        .unwrap()
    else {
        panic!("lease")
    };
    let before = counts(&store);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        store.automation_poll_complete_before(
            &poll,
            &batch(SignalStatus::Failing, true),
            || 120,
            Instant::now() + Duration::from_secs(60)
        ),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn container_source_wall_age_is_checked_again_in_the_completion_transaction() {
    for (sampled, now) in [(120, 126), (121, 120)] {
        let (_dir, mut store, actor) = setup();
        let id = schedule(&mut store, &actor);
        let ObservationPollAdmission::Lease(poll) = store
            .automation_poll_begin(&id, vec!["host:system".into()], || 120, deadline())
            .unwrap()
        else {
            panic!("lease")
        };
        let source =
            ContainerHealthCohort::new("a".repeat(64), "fixture-engine".into(), sampled, vec![])
                .unwrap();
        let before = counts(&store);
        assert_eq!(
            store.automation_poll_containers_complete_before(&poll, &source, || now, deadline()),
            Err(Error(ErrorCode::Conflict))
        );
        assert_eq!(counts(&store), before);
    }
}

fn process_write_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("write_bytes: ")
                .map(|v| v.trim().parse().unwrap())
        })
        .unwrap()
}
fn database_bytes(dir: &tempfile::TempDir) -> (Vec<u8>, Vec<u8>) {
    (
        std::fs::read(dir.path().join("core.sqlite")).unwrap(),
        std::fs::read(dir.path().join("core.sqlite-wal")).unwrap(),
    )
}

#[test]
fn isolated_idle_io_helper() {
    if std::env::var_os("LIMEOS_TEST_POLL_IO_HELPER").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let (dir, mut store, actor) = setup();
    let id = schedule(&mut store, &actor);
    sample(&mut store, &id, 120, SignalStatus::Healthy, true);
    let before = counts(&store);
    let changes_before = changes(&store);
    let bytes = database_bytes(&dir);
    let io = process_write_bytes();
    for poll in 1..=2880 {
        assert!(matches!(
            sample(
                &mut store,
                &id,
                120 + poll * 30,
                SignalStatus::Healthy,
                true
            ),
            ObservationPollOutcome::Unchanged { .. }
        ));
    }
    let writes = process_write_bytes() - io;
    assert_eq!(counts(&store), before);
    assert_eq!(changes(&store), changes_before);
    assert_eq!(database_bytes(&dir), bytes);
    assert_eq!(writes, 0);
    println!(
        "LIMEOS_TRANSITION_IDLE {}",
        serde_json::json!({"polls":2880,"sqlite_changes":0,"database_bytes_changed":false,"wal_bytes_changed":false,"process_write_bytes":writes,"scope":"isolated real Store; simulated day; normalized healthy fixture; not installed 24-hour evidence"})
    );
}

#[test]
fn simulated_day_of_unchanged_polls_has_zero_database_and_process_writes() {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "automation::completion::polling::tests::isolated_idle_io_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LIMEOS_TEST_POLL_IO_HELPER", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("isolated I/O helper timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.stdout.len() < 8192 && output.stderr.len() < 8192);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("LIMEOS_TRANSITION_IDLE "));
    print!("{stdout}");
}
