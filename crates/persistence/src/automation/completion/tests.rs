use super::*;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let initial = store.login_record("alice").unwrap().unwrap().principal;
    let scopes = vec![
        Scope {
            operation: Operation::HealthRead,
            resource: "host:system".into(),
        },
        Scope {
            operation: Operation::HealthRead,
            resource: "container:plex".into(),
        },
        Scope {
            operation: Operation::AutomationManage,
            resource: "automation:observe-health".into(),
        },
    ];
    store
        .revise_grants(&initial.id, Role::Operator, &scopes, 102)
        .unwrap();
    let record = store.login_record("alice").unwrap().unwrap();
    store
        .create_session(&record, None, "session", "csrf", 110)
        .unwrap();
    (dir, store, record.principal)
}
fn queue(store: &mut Store, actor: &Principal, now: i64) -> ObservationJob {
    let row = store
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
            now,
        )
        .unwrap()
        .schedule;
    let request = proposed(&row, now).unwrap().unwrap();
    let Reservation::Queued(job) = store
        .automation_reserve_current(&row, &request, || now, deadline())
        .unwrap()
    else {
        panic!("queued")
    };
    job
}
fn begin(store: &mut Store, job: &ObservationJob, now: i64) -> ObservationLease {
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(
            job,
            vec!["host:system".into(), "container:plex".into()],
            || now,
            deadline(),
        )
        .unwrap()
    else {
        panic!("read lease")
    };
    lease
}
fn batch(status: SignalStatus, complete: bool) -> HealthObservation {
    HealthObservation {
        signals: vec![ScopedHealthSignal {
            resource: "host:system".into(),
            signal: AlertSignal {
                key: "host:load".into(),
                kind: AlertKind::Host,
                status,
                severity: Severity::Warning,
                summary: "Host load assessment".into(),
            },
        }],
        complete,
    }
}
fn sample(
    store: &mut Store,
    actor: &Principal,
    now: i64,
    value: &HealthObservation,
) -> (ObservationJob, ObservationLease, ObservationResult) {
    let job = queue(store, actor, now);
    let lease = begin(store, &job, now);
    let receipt = store
        .automation_observation_complete(&lease, value, || now)
        .unwrap();
    (job, lease, receipt)
}
fn counts(store: &Store) -> (i64, i64, i64, i64, i64, i64) {
    store.conn.query_row("SELECT (SELECT count(*) FROM automation_observation_results),(SELECT coalesce(sum(sequence),0) FROM automation_streams),(SELECT count(*) FROM automation_alerts),(SELECT count(*) FROM automation_notifications),(SELECT count(*) FROM events),audit_bytes FROM meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
fn alert(store: &Store) -> AlertState {
    parse(
        &store
            .conn
            .query_row(
                "SELECT body FROM automation_alerts WHERE key='host:load'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn queued_pages_are_query_only_bounded_and_do_not_recreate_terminal_jobs() {
    let (_dir, mut store, actor) = setup();
    let mut jobs: Vec<_> = (0..33).map(|_| queue(&mut store, &actor, 200)).collect();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    let before = counts(&store);
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    let first = store.automation_queued_observation_page(None, 32).unwrap();
    assert_eq!(first.items, jobs[..32]);
    assert_eq!(first.next.as_deref(), Some(jobs[31].id.as_str()));
    let last = store
        .automation_queued_observation_page(first.next.as_deref(), 32)
        .unwrap();
    assert_eq!(last.items, jobs[32..]);
    assert_eq!(last.next, None);
    assert!(!first.partial && !last.partial);
    for limit in [0, 33, usize::MAX] {
        assert!(matches!(
            store.automation_queued_observation_page(None, limit),
            Err(Error(ErrorCode::InvalidInput))
        ));
    }
    assert!(matches!(
        store.automation_queued_observation_page(Some("*"), 32),
        Err(Error(ErrorCode::InvalidInput))
    ));
    assert_eq!(counts(&store), before);
    store.conn.execute_batch("PRAGMA query_only=OFF").unwrap();
    let lease = begin(&mut store, &jobs[0], 201);
    store
        .automation_observation_complete(
            &lease,
            &HealthObservation {
                signals: vec![],
                complete: false,
            },
            || 201,
        )
        .unwrap();
    store
        .automation_pause(
            &actor,
            &jobs[1].occurrence.schedule,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            202,
        )
        .unwrap();
    let before = counts(&store);
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    assert_eq!(
        store
            .automation_queued_observation_page(None, 32)
            .unwrap()
            .items,
        jobs[2..]
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn owner_sequences_partial_observation_transition_outbox_and_ack_survive_restart() {
    let (dir, mut store, actor) = setup();
    sample(&mut store, &actor, 200, &batch(SignalStatus::Failing, true));
    let (_, _, opening) = sample(&mut store, &actor, 201, &batch(SignalStatus::Failing, true));
    assert_eq!(opening.sequence, 2);
    let material = store
        .conn
        .query_row("SELECT body FROM automation_alerts", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    for (now, observation) in [
        (202, batch(SignalStatus::Failing, true)),
        (203, batch(SignalStatus::Unavailable, false)),
        (
            204,
            HealthObservation {
                signals: vec![],
                complete: false,
            },
        ),
    ] {
        sample(&mut store, &actor, now, &observation);
        assert_eq!(
            store
                .conn
                .query_row("SELECT body FROM automation_alerts", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            material
        );
    }
    assert!(
        store
            .observe_health_alert_page("session", None, || 204, deadline())
            .unwrap()
            .partial
    );
    let (job, lease, receipt) =
        sample(&mut store, &actor, 205, &batch(SignalStatus::Healthy, true));
    assert_eq!(receipt.sequence, 6);
    assert!(matches!(
        alert(&store).incident,
        IncidentState::Clear { failures: 0 }
    ));
    assert_eq!(counts(&store).3, 2);
    let before = counts(&store);
    assert_eq!(
        store
            .automation_observation_complete(&lease, &batch(SignalStatus::Healthy, true), || 206)
            .unwrap(),
        receipt
    );
    assert_eq!(counts(&store), before);
    let mut after = None;
    let mut published = 0;
    for _ in 0..8 {
        let page = store
            .automation_publish_in_app(after.as_deref(), 1, || 206, deadline())
            .unwrap();
        published += page.published;
        after = page.next;
    }
    assert_eq!(published, 2);
    let page = store
        .observe_health_notification_page("session", None, || 207, deadline())
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(!page.partial);
    assert!(
        page.items
            .iter()
            .all(|n| matches!(n.delivery, DeliveryState::Delivered { .. })
                && n.attempts == 1
                && n.revision == 3)
    );
    let id = &page.items[0].id;
    let input = AcknowledgeNotification { revision: 3 };
    assert!(
        store
            .observe_health_notification_acknowledge(
                "session",
                "bad-csrf",
                id,
                &input,
                || 208,
                deadline()
            )
            .is_err()
    );
    let ack = store
        .observe_health_notification_acknowledge("session", "csrf", id, &input, || 208, deadline())
        .unwrap();
    let before = counts(&store);
    assert_eq!(
        store
            .observe_health_notification_acknowledge(
                "session",
                "csrf",
                id,
                &input,
                || 209,
                deadline()
            )
            .unwrap(),
        ack
    );
    assert_eq!(counts(&store), before);
    drop(store);
    let mut reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(counts(&reopened), before);
    assert!(
        matches!(reopened.automation_observation_begin(&job,vec!["host:system".into()],||300,deadline()).unwrap(),ObservationAdmission::Existing(value) if value==receipt)
    );
    assert_eq!(
        reopened
            .automation_publish_in_app(None, 32, || 300, deadline())
            .unwrap()
            .published,
        0
    );
}

#[test]
fn collection_is_serialized_per_owner_and_old_results_cannot_replace_fresh_leases() {
    let (dir, mut store, actor) = setup();
    let first = queue(&mut store, &actor, 200);
    let second = queue(&mut store, &actor, 201);
    assert!(matches!(
        store.automation_observation_begin(&second, vec!["host:system".into()], || 201, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    let old = begin(&mut store, &first, 201);
    assert!(matches!(
        store.automation_observation_begin(&first, vec!["host:system".into()], || 201, deadline()),
        Err(Error(ErrorCode::Conflict))
    ));
    let before = counts(&store);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let fresh = begin(&mut store, &first, 202);
    assert_eq!(
        store.automation_observation_complete(&old, &batch(SignalStatus::Failing, true), || 202),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(counts(&store), before);
    assert_eq!(
        store
            .automation_observation_complete(&fresh, &batch(SignalStatus::Failing, true), || 202)
            .unwrap()
            .sequence,
        1
    );
    let next = begin(&mut store, &second, 203);
    assert_eq!(
        store
            .automation_observation_complete(&next, &batch(SignalStatus::Failing, true), || 203)
            .unwrap()
            .sequence,
        2
    );
    assert_eq!(counts(&store).3, 1);
}

#[test]
fn result_alert_outbox_job_and_audit_are_one_rollback_boundary() {
    let (_dir, mut store, actor) = setup();
    sample(&mut store, &actor, 200, &batch(SignalStatus::Failing, true));
    let job = queue(&mut store, &actor, 201);
    let lease = begin(&mut store, &job, 201);
    for trigger in [
        "CREATE TEMP TRIGGER fail_settlement BEFORE INSERT ON automation_observation_results BEGIN SELECT RAISE(ABORT,'result'); END",
        "CREATE TEMP TRIGGER fail_settlement BEFORE INSERT ON automation_alerts BEGIN SELECT RAISE(ABORT,'alert'); END",
        "CREATE TEMP TRIGGER fail_settlement BEFORE INSERT ON automation_notifications BEGIN SELECT RAISE(ABORT,'outbox'); END",
        "CREATE TEMP TRIGGER fail_settlement BEFORE INSERT ON events WHEN json_extract(NEW.kind,'$.kind')='automation_observation_settled' BEGIN SELECT RAISE(ABORT,'audit'); END",
    ] {
        store.conn.execute_batch(trigger).unwrap();
        let before = counts(&store);
        let material = alert(&store);
        assert_eq!(
            store.automation_observation_complete(
                &lease,
                &batch(SignalStatus::Failing, true),
                || 201
            ),
            Err(Error(ErrorCode::StateNotDurable))
        );
        assert_eq!(counts(&store), before);
        assert_eq!(alert(&store), material);
        assert_eq!(store.job_state(&job.id).unwrap(), "queued");
        store
            .conn
            .execute_batch("DROP TRIGGER fail_settlement")
            .unwrap();
    }
    store
        .automation_observation_complete(&lease, &batch(SignalStatus::Failing, true), || 201)
        .unwrap();
    let before = counts(&store);
    store.conn.execute_batch("CREATE TEMP TRIGGER fail_publish BEFORE INSERT ON events WHEN json_extract(NEW.kind,'$.kind')='automation_notification_published' BEGIN SELECT RAISE(ABORT,'publish'); END").unwrap();
    assert_eq!(
        store.automation_publish_in_app(None, 32, || 202, deadline()),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(counts(&store), before);
    assert!(matches!(
        store
            .observe_health_notification_page("session", None, || 202, deadline())
            .unwrap()
            .items[0]
            .delivery,
        DeliveryState::Pending
    ));
    store
        .conn
        .execute_batch("DROP TRIGGER fail_publish")
        .unwrap();
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 202, deadline())
            .unwrap()
            .published,
        1
    );
}

#[test]
fn authority_resource_binding_and_original_deadlines_are_rechecked() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    let (dir, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 200);
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(&job, vec!["host:system".into()], || 200, deadline())
        .unwrap()
    else {
        panic!("lease")
    };
    let outside = HealthObservation {
        signals: vec![ScopedHealthSignal {
            resource: "container:plex".into(),
            signal: container_signal(
                "plex",
                Some(false),
                ContainerHealth::Unreported,
                RestartPolicy::Always,
            )
            .unwrap()
            .unwrap(),
        }],
        complete: true,
    };
    let before = counts(&store);
    assert_eq!(
        store.automation_observation_complete(&lease, &outside, || 201),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(counts(&store), before);
    let original = Instant::now() + Duration::from_millis(80);
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(&job, vec!["host:system".into()], || 201, original)
        .unwrap()
    else {
        panic!("lease")
    };
    let path = dir.path().join("core.sqlite");
    let (ready, occupied) = mpsc::sync_channel(0);
    let writer = std::thread::spawn(move || {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ready.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        conn.execute_batch("COMMIT").unwrap();
    });
    occupied.recv().unwrap();
    let calls = AtomicUsize::new(0);
    assert_eq!(
        store.automation_observation_complete(&lease, &batch(SignalStatus::Failing, true), || {
            calls.fetch_add(1, Ordering::SeqCst);
            201
        }),
        Err(Error(ErrorCode::Expired))
    );
    writer.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(counts(&store), before);
    let lease = begin(&mut store, &job, 201);
    store
        .conn
        .execute(
            "DELETE FROM grants WHERE principal=? AND resource='container:plex'",
            [&actor.id],
        )
        .unwrap();
    assert_eq!(
        store.automation_observation_complete(&lease, &batch(SignalStatus::Failing, true), || 202),
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn expiry_and_changed_authority_retire_only_oldest_jobs_without_observations() {
    let (dir, mut store, actor) = setup();
    let first = queue(&mut store, &actor, 200);
    let second = queue(&mut store, &actor, 201);
    assert_eq!(
        store.automation_observation_discard(&first, || 201, deadline()),
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(
        store.automation_observation_discard(&second, || 300, deadline()),
        Err(Error(ErrorCode::Conflict))
    );
    let receipt = store
        .automation_observation_discard(&first, || 260, deadline())
        .unwrap();
    assert_eq!(receipt.state, JobState::Failed);
    assert!(!receipt.complete);
    assert_eq!(
        store
            .automation_observation_discard(&first, || 300, deadline())
            .unwrap(),
        receipt
    );
    store
        .revise_grants(&actor.id, Role::Viewer, &[], 261)
        .unwrap();
    let receipt = store
        .automation_observation_discard(&second, || 261, deadline())
        .unwrap();
    assert_eq!(receipt.state, JobState::PreconditionChanged);
    assert_eq!(receipt.sequence, 2);
    assert_eq!((counts(&store).2, counts(&store).3), (0, 0));
    drop(store);
    Store::open(&dir.path().join("core.sqlite")).unwrap();
}

#[test]
fn outbox_alert_and_audit_capacity_failures_leave_no_consumption() {
    let (_dir, mut store, actor) = setup();
    let wide = |status| HealthObservation {
        signals: (0..MAX_ALERTS)
            .map(|n| ScopedHealthSignal {
                resource: "host:system".into(),
                signal: AlertSignal {
                    key: format!("host:capacity-{n:03}"),
                    kind: AlertKind::Host,
                    status,
                    severity: Severity::Warning,
                    summary: "Capacity fixture".into(),
                },
            })
            .collect(),
        complete: true,
    };
    for (now, status) in [
        (200, SignalStatus::Failing),
        (201, SignalStatus::Failing),
        (202, SignalStatus::Healthy),
        (203, SignalStatus::Failing),
        (204, SignalStatus::Failing),
        (205, SignalStatus::Healthy),
        (206, SignalStatus::Failing),
    ] {
        sample(&mut store, &actor, now, &wide(status));
    }
    assert_eq!(counts(&store).2, MAX_ALERTS as i64);
    assert_eq!(counts(&store).3, MAX_NOTIFICATIONS as i64);
    let job = queue(&mut store, &actor, 207);
    let lease = begin(&mut store, &job, 207);
    let before = counts(&store);
    assert_eq!(
        store.automation_observation_complete(&lease, &wide(SignalStatus::Failing), || 207),
        Err(Error(ErrorCode::Overloaded))
    );
    assert_eq!(counts(&store), before);
    assert_eq!(store.job_state(&job.id).unwrap(), "queued");
    assert_eq!(
        store.automation_observation_complete(&lease, &batch(SignalStatus::Failing, true), || 207),
        Err(Error(ErrorCode::Overloaded))
    );
    assert_eq!(counts(&store), before);
    store
        .conn
        .execute(
            "UPDATE meta SET audit_bytes=? WHERE id=1",
            [AUDIT_LIMIT - 256],
        )
        .unwrap();
    let before = counts(&store);
    assert_eq!(
        store.automation_observation_complete(&lease, &wide(SignalStatus::Healthy), || 207),
        Err(Error(ErrorCode::StateNotDurable))
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn owners_have_independent_sequences_and_blocked_delivery_does_not_starve_later_rows() {
    let (_dir, mut store, alice) = setup();
    let bob=store.import_legacy_user("bob","pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1",103).unwrap();
    store
        .revise_grants(&bob.id, Role::Operator, &store.grants(&alice).unwrap(), 104)
        .unwrap();
    let record = store.login_record("bob").unwrap().unwrap();
    store
        .create_session(&record, None, "bob-session", "bob-csrf", 110)
        .unwrap();
    let bob = record.principal;
    for now in [200, 201] {
        let a = queue(&mut store, &alice, now);
        let b = queue(&mut store, &bob, now);
        let a_lease = begin(&mut store, &a, now);
        let b_lease = begin(&mut store, &b, now);
        let a_receipt = store
            .automation_observation_complete(&a_lease, &batch(SignalStatus::Failing, true), || now)
            .unwrap();
        let b_receipt = store
            .automation_observation_complete(&b_lease, &batch(SignalStatus::Failing, true), || now)
            .unwrap();
        assert_eq!(a_receipt.sequence, now - 199);
        assert_eq!(b_receipt.sequence, now - 199);
    }
    let a = store
        .observe_health_notification_page("session", None, || 202, deadline())
        .unwrap();
    let b = store
        .observe_health_notification_page("bob-session", None, || 202, deadline())
        .unwrap();
    assert_eq!((a.items.len(), b.items.len()), (1, 1));
    assert_ne!(a.items[0].id, b.items[0].id);
    assert_eq!(
        store.observe_health_notification_acknowledge(
            "session",
            "csrf",
            &b.items[0].id,
            &AcknowledgeNotification { revision: 1 },
            || 202,
            deadline()
        ),
        Err(Error(ErrorCode::NotFound))
    );
    let first_owner: String = store
        .conn
        .query_row(
            "SELECT owner FROM automation_notifications ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    store
        .revise_grants(
            &first_owner,
            Role::Viewer,
            &[Scope {
                operation: Operation::HealthRead,
                resource: "host:system".into(),
            }],
            203,
        )
        .unwrap();
    let mut after = None;
    let mut delivered = 0;
    let mut blocked = 0;
    for _ in 0..5 {
        let page = store
            .automation_publish_in_app(after.as_deref(), 1, || 204, deadline())
            .unwrap();
        delivered += page.published;
        blocked += page.blocked;
        after = page.next;
    }
    assert_eq!(delivered, 1);
    assert!(blocked >= 1);
}

#[test]
fn compact_result_watermark_alert_outbox_and_publication_corruption_refuse_before_generation() {
    for (trigger, mutation, publish) in [
        (
            Some("automation_result_delete"),
            "DELETE FROM automation_observation_results WHERE sequence=1",
            false,
        ),
        (
            Some("automation_stream_identity"),
            "UPDATE automation_streams SET sequence=sequence+1",
            false,
        ),
        (
            Some("automation_alert_identity"),
            "UPDATE automation_alerts SET body=json_set(body,'$.last_sequence',100)",
            false,
        ),
        (
            Some("automation_alert_delete"),
            "DELETE FROM automation_alerts",
            false,
        ),
        (
            Some("automation_notification_delete"),
            "DELETE FROM automation_notifications WHERE phase='recovery'",
            false,
        ),
        (
            Some("automation_notification_identity"),
            "UPDATE automation_notifications SET body=json_set(body,'$.event.summary','altered')",
            false,
        ),
        (
            Some("automation_notification_identity"),
            "UPDATE automation_notifications SET generation=generation+10,body=json_set(body,'$.event.generation',generation+10)",
            false,
        ),
        (
            None,
            "DELETE FROM events WHERE json_extract(kind,'$.kind')='automation_observation_settled' AND json_extract(kind,'$.sequence')=2",
            false,
        ),
        (
            None,
            "DELETE FROM events WHERE json_extract(kind,'$.kind')='automation_notification_published'",
            true,
        ),
    ] {
        let (dir, mut store, actor) = setup();
        sample(&mut store, &actor, 200, &batch(SignalStatus::Failing, true));
        sample(&mut store, &actor, 201, &batch(SignalStatus::Failing, true));
        sample(&mut store, &actor, 202, &batch(SignalStatus::Healthy, true));
        if publish {
            store
                .automation_publish_in_app(None, 32, || 202, deadline())
                .unwrap();
        }
        let restore = trigger.map(|name| {
            let sql: String = store
                .conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            store
                .conn
                .execute_batch(&format!("DROP TRIGGER {name}"))
                .unwrap();
            sql
        });
        store.conn.execute_batch(mutation).unwrap();
        if let Some(sql) = restore {
            store.conn.execute_batch(&sql).unwrap();
        }
        let generation = store.generation;
        drop(store);
        assert!(
            matches!(
                Store::open(&dir.path().join("core.sqlite")),
                Err(Error(ErrorCode::StateNotDurable))
            ),
            "{mutation}"
        );
        let raw = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            raw.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            generation
        );
    }
}

#[test]
fn migration_from_compiled_schema_eleven_preserves_queued_and_canceled_jobs() {
    let (dir, mut store, actor) = setup();
    let queued = queue(&mut store, &actor, 200);
    let canceled = queue(&mut store, &actor, 201);
    store
        .automation_pause(
            &actor,
            &canceled.occurrence.schedule,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            202,
        )
        .unwrap();
    let reference = Connection::open_in_memory().unwrap();
    reference
        .execute_batch(concat!(
            include_str!("../../schema.sql"),
            include_str!("../../migration-v2.sql"),
            include_str!("../../migration-v3.sql"),
            include_str!("../../migration-v4.sql"),
            include_str!("../../migration-v5.sql"),
            include_str!("../../migration-v6.sql"),
            include_str!("../../migration-v7.sql"),
            include_str!("../../migration-v8.sql"),
            include_str!("../../migration-v9.sql"),
            include_str!("../../migration-v10.sql"),
            include_str!("../../migration-v11.sql")
        ))
        .unwrap();
    let objects = |conn: &Connection, kind: &str| {
        conn.prepare(
            "SELECT name,sql FROM sqlite_schema WHERE type=? AND sql IS NOT NULL ORDER BY name",
        )
        .unwrap()
        .query_map([kind], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()
        .unwrap()
    };
    let expected = objects(&reference, "trigger");
    for (name, sql) in objects(&store.conn, "trigger") {
        if expected.get(&name) != Some(&sql) {
            store
                .conn
                .execute_batch(&format!("DROP TRIGGER {name}"))
                .unwrap();
            if let Some(old) = expected.get(&name) {
                store.conn.execute_batch(old).unwrap();
            }
        }
    }
    for table in [
        "automation_notifications",
        "automation_alerts",
        "automation_observation_results",
        "automation_streams",
    ] {
        store
            .conn
            .execute_batch(&format!("DROP TABLE {table}"))
            .unwrap();
    }
    store.conn.pragma_update(None, "user_version", 11).unwrap();
    assert_eq!(objects(&store.conn, "table"), objects(&reference, "table"));
    assert_eq!(objects(&store.conn, "trigger"), expected);
    let before = counts_jobs(&store);
    drop(store);
    let reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(counts_jobs(&reopened), before);
    assert_eq!(reopened.job_state(&queued.id).unwrap(), "queued");
    assert_eq!(reopened.job_state(&canceled.id).unwrap(), "canceled");
    assert_eq!(
        reopened
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        12
    );
}
fn counts_jobs(store: &Store) -> String {
    store.conn.query_row("SELECT group_concat(id||':'||state||':'||intent||':'||digest||':'||revision||':'||deadline,'|') FROM (SELECT * FROM jobs ORDER BY id)",[],|r|r.get(0)).unwrap()
}

#[test]
fn retention_keeps_openings_and_proves_pruned_rows_without_recreating_delivery() {
    let (dir, mut store, actor) = setup();
    sample(&mut store, &actor, 200, &batch(SignalStatus::Failing, true));
    sample(&mut store, &actor, 201, &batch(SignalStatus::Failing, true));
    let week = DELIVERED_RETENTION_SECONDS;
    assert_eq!(
        store
            .automation_prune_notifications(None, 32, || 202 + week, deadline())
            .unwrap()
            .0,
        0
    );
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 202, deadline())
            .unwrap()
            .published,
        1
    );
    assert_eq!(
        store
            .automation_prune_notifications(None, 32, || 202 + week, deadline())
            .unwrap()
            .0,
        0
    );
    let (job, _, receipt) = sample(
        &mut store,
        &actor,
        202 + week,
        &batch(SignalStatus::Healthy, true),
    );
    assert_eq!(
        store
            .automation_prune_notifications(None, 32, || 203 + week, deadline())
            .unwrap()
            .0,
        0
    );
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 203 + week, deadline())
            .unwrap()
            .published,
        1
    );
    let mut removed = 0;
    for _ in 0..3 {
        removed += store
            .automation_prune_notifications(None, 32, || 204 + 2 * week, deadline())
            .unwrap()
            .0;
    }
    assert_eq!(removed, 2);
    assert_eq!(counts(&store).3, 0);
    let before = counts(&store);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(counts(&store), before);
    assert!(
        matches!(store.automation_observation_begin(&job,vec!["host:system".into()],||205+2*week,deadline()).unwrap(),ObservationAdmission::Existing(value) if value==receipt)
    );
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 205 + 2 * week, deadline())
            .unwrap()
            .published,
        0
    );
}
