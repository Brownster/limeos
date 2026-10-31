use super::*;
use std::time::Duration;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
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
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            102,
        )
        .unwrap();
    let record = store.login_record("alice").unwrap().unwrap();
    store
        .create_session(&record, None, "session", "csrf", 110)
        .unwrap();
    (directory, store, record.principal)
}
fn replace(
    store: &mut Store,
    revision: i64,
    definition: HealthPolicyDefinition,
    now: i64,
) -> HealthPolicyView {
    store
        .observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision,
                definition,
            },
            || now,
            deadline(),
        )
        .unwrap()
}
fn definition(threshold: u8) -> HealthPolicyDefinition {
    HealthPolicyDefinition {
        fail_threshold: threshold,
        ..Default::default()
    }
}
fn counts(store: &Store) -> (i64, i64, i64, i64, i64) {
    store.conn.query_row("SELECT (SELECT count(*) FROM automation_health_policies),(SELECT count(*) FROM events),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_notifications),audit_bytes FROM meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
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
fn lease(store: &mut Store, job: &ObservationJob, now: i64) -> ObservationLease {
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(job, vec!["host:system".into()], || now, deadline())
        .unwrap()
    else {
        panic!("lease")
    };
    lease
}
fn batch() -> HealthObservation {
    HealthObservation {
        signals: vec![ScopedHealthSignal {
            resource: "host:system".into(),
            signal: AlertSignal {
                key: "host:load".into(),
                kind: AlertKind::Host,
                status: SignalStatus::Failing,
                severity: Severity::Warning,
                summary: "Fixture load assessment".into(),
            },
        }],
        complete: true,
    }
}
fn sample(store: &mut Store, actor: &Principal, now: i64) -> ObservationJob {
    let job = queue(store, actor, now);
    let lease = lease(store, &job, now);
    store
        .automation_observation_complete(&lease, &batch(), || now)
        .unwrap();
    job
}

#[test]
fn implicit_reads_are_query_only_and_explicit_replacement_is_cas_and_replay_safe() {
    let (_directory, mut store, _actor) = setup();
    let before = counts(&store);
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    let empty = store
        .observe_health_policy("session", || 200, deadline())
        .unwrap();
    assert_eq!(
        empty,
        HealthPolicyView {
            revision: 0,
            updated: None,
            definition: HealthPolicyDefinition::default()
        }
    );
    assert_eq!(counts(&store), before);
    store.conn.execute_batch("PRAGMA query_only=OFF").unwrap();
    let first = replace(&mut store, 0, definition(2), 200);
    assert_eq!(first.revision, 1);
    let before = counts(&store);
    assert_eq!(replace(&mut store, 0, definition(2), 201), first);
    assert_eq!(replace(&mut store, 1, definition(2), 201), first);
    assert_eq!(counts(&store), before);
    let second = replace(&mut store, 1, definition(3), 201);
    assert_eq!(second.revision, 2);
    for (revision, threshold) in [(0, 3), (1, 1), (2, 0)] {
        assert!(
            store
                .observe_health_policy_replace(
                    "session",
                    "csrf",
                    &ReplaceHealthPolicy {
                        revision,
                        definition: definition(threshold)
                    },
                    || 202,
                    deadline()
                )
                .is_err()
        );
    }
    let before = counts(&store);
    assert!(matches!(
        store.observe_health_policy_replace(
            "session",
            "bad-csrf",
            &ReplaceHealthPolicy {
                revision: 2,
                definition: definition(4)
            },
            || 202,
            deadline()
        ),
        Err(Error(ErrorCode::Forbidden))
    ));
    assert!(matches!(
        store.observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision: 2,
                definition: definition(4)
            },
            || 200,
            deadline()
        ),
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn changed_policy_expires_private_collection_and_discard_keeps_one_original_job() {
    let (directory, mut store, actor) = setup();
    replace(&mut store, 0, definition(1), 190);
    let job = queue(&mut store, &actor, 200);
    let admitted = lease(&mut store, &job, 200);
    assert_eq!(admitted.health_policy().revision, 1);
    assert_eq!(admitted.health_policy().definition, definition(1));
    replace(&mut store, 1, definition(3), 201);
    let before = counts(&store);
    assert!(matches!(
        store.automation_observation_complete(&admitted, &batch(), || 202),
        Err(Error(ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
    let receipt = store
        .automation_observation_discard(&job, || 202, deadline())
        .unwrap();
    assert_eq!(receipt.state, JobState::PreconditionChanged);
    assert_eq!(receipt.notifications, 0);
    let before = counts(&store);
    assert_eq!(
        store
            .automation_observation_discard(&job, || 203, deadline())
            .unwrap(),
        receipt
    );
    assert_eq!(counts(&store), before);
    let next = sample(&mut store, &actor, 210);
    assert_ne!(next.id, job.id);
    assert_eq!(
        counts(&store).3,
        0,
        "configured threshold 3, only first failure"
    );
    drop(store);
    let mut reopened = Store::open(&directory.path().join("core.sqlite")).unwrap();
    assert_eq!(
        reopened
            .observe_health_policy("session", || 211, deadline())
            .unwrap()
            .revision,
        2
    );
    assert_eq!(reopened.job_state(&job.id).unwrap(), "precondition_changed");
}

#[test]
fn queued_revision_survives_restart_and_refuses_changed_policy_before_collection() {
    let (directory, mut store, actor) = setup();
    let job = queue(&mut store, &actor, 200);
    replace(&mut store, 0, definition(1), 201);
    drop(store);
    let mut store = Store::open(&directory.path().join("core.sqlite")).unwrap();
    assert!(matches!(
        store.automation_observation_begin(&job, vec!["host:system".into()], || 202, deadline()),
        Err(Error(ErrorCode::Expired))
    ));
    assert_eq!(
        store
            .automation_observation_discard(&job, || 202, deadline())
            .unwrap()
            .state,
        JobState::PreconditionChanged
    );
}

#[test]
fn publication_rechecks_latest_category_and_exclusive_silence_at_one_transaction() {
    let (directory, mut store, actor) = setup();
    replace(&mut store, 0, definition(1), 190);
    sample(&mut store, &actor, 200);
    assert!(
        !store
            .observe_health_alert_page("session", None, || 200, deadline())
            .unwrap()
            .partial
    );
    let mut muted = definition(1);
    muted.disabled = vec![AlertKind::Host];
    replace(&mut store, 1, muted, 201);
    assert!(
        store
            .observe_health_alert_page("session", None, || 201, deadline())
            .unwrap()
            .partial
    );
    let publication = store
        .automation_publish_in_app(None, 32, || 201, deadline())
        .unwrap();
    assert_eq!((publication.published, publication.blocked), (0, 1));
    let mut silent = definition(1);
    silent.silences = vec![HealthPolicySilence {
        kind: AlertKind::Host,
        key: "host:load".into(),
        created_at: "1970-01-01T00:03:21.123456Z".into(),
        expires_at: Some("1970-01-01T00:03:30Z".into()),
        reason: "maintenance".into(),
    }];
    replace(&mut store, 2, silent, 201); // Equal wall time; durable event order decides revision.
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 209, deadline())
            .unwrap()
            .published,
        0
    );
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 210, deadline())
            .unwrap()
            .published,
        1
    );
    let before = counts(&store);
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 210, deadline())
            .unwrap()
            .published,
        0
    );
    assert_eq!(counts(&store), before);
    let page = store
        .observe_health_notification_page("session", None, || 210, deadline())
        .unwrap();
    assert!(matches!(
        page.items[0].delivery,
        DeliveryState::Delivered { at: 210 }
    ));
    drop(store);
    let mut store = Store::open(&directory.path().join("core.sqlite")).unwrap();
    assert_eq!(
        store
            .observe_health_policy("session", || 211, deadline())
            .unwrap()
            .definition
            .silences[0]
            .created_at,
        "1970-01-01T00:03:21.123456Z"
    );
    sample(&mut store, &actor, 210);
    assert!(
        !store
            .observe_health_alert_page("session", None, || 211, deadline())
            .unwrap()
            .partial
    );
}

#[test]
fn management_grant_is_required_for_configuration_and_expired_work_never_commits() {
    let (_directory, mut store, actor) = setup();
    let expired = Instant::now() - Duration::from_millis(1);
    let before = counts(&store);
    assert!(matches!(
        store.observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision: 0,
                definition: definition(3)
            },
            || 200,
            expired
        ),
        Err(Error(ErrorCode::Expired))
    ));
    assert_eq!(counts(&store), before);
    store
        .revise_grants(
            &actor.id,
            Role::Operator,
            &[Scope {
                operation: Operation::HealthRead,
                resource: "host:system".into(),
            }],
            201,
        )
        .unwrap();
    let record = store.login_record("alice").unwrap().unwrap();
    store
        .create_session(&record, None, "readonly", "csrf2", 202)
        .unwrap();
    assert!(matches!(
        store.observe_health_policy("readonly", || 203, deadline()),
        Err(Error(ErrorCode::Forbidden))
    ));
    assert!(matches!(
        store.observe_health_policy_replace(
            "readonly",
            "csrf2",
            &ReplaceHealthPolicy {
                revision: 0,
                definition: definition(3)
            },
            || 203,
            deadline()
        ),
        Err(Error(ErrorCode::Forbidden))
    ));
}

#[test]
fn tampered_policy_observation_and_publication_proofs_refuse_restart() {
    for mutation in [
        "DROP TRIGGER automation_health_policy_update; UPDATE automation_health_policies SET body=json_set(body,'$.fail_threshold',3)",
        "DELETE FROM events WHERE json_extract(kind,'$.kind')='automation_health_policy_changed'",
        "DROP TRIGGER automation_occurrence_identity; UPDATE automation_occurrences SET policy_revision=0",
        "DROP TRIGGER automation_result_update; UPDATE automation_observation_results SET policy_revision=0",
        "DROP TRIGGER automation_stream_identity; UPDATE automation_streams SET policy_revision=0",
        "UPDATE events SET kind=json_set(kind,'$.revision',2) WHERE json_extract(kind,'$.kind')='automation_notification_policy_checked'",
        "DELETE FROM events WHERE json_extract(kind,'$.kind')='automation_notification_policy_checked'",
    ] {
        let (directory, mut store, actor) = setup();
        replace(&mut store, 0, definition(1), 190);
        sample(&mut store, &actor, 200);
        store
            .automation_publish_in_app(None, 32, || 201, deadline())
            .unwrap();
        let triggers = store
            .conn
            .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger'")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        store.conn.execute_batch(mutation).unwrap();
        for (name, sql) in triggers {
            let present: bool = store
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?)",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            if !present {
                store.conn.execute_batch(&sql).unwrap();
            }
        }
        drop(store);
        assert!(
            matches!(
                Store::open(&directory.path().join("core.sqlite")),
                Err(Error(ErrorCode::StateNotDurable))
            ),
            "{mutation}"
        );
    }
}

#[test]
fn schema_twelve_upgrade_preserves_default_history_pending_jobs_and_audit() {
    let (directory, mut store, actor) = setup();
    sample(&mut store, &actor, 200);
    sample(&mut store, &actor, 201);
    store
        .automation_publish_in_app(None, 32, || 202, deadline())
        .unwrap();
    let queued = queue(&mut store, &actor, 210);
    let before = counts(&store);
    store.conn.execute_batch("DROP TRIGGER automation_polled_result; DROP TABLE automation_poll_origins; DROP TRIGGER automation_result_policy; DROP TABLE automation_health_policies; ALTER TABLE automation_occurrences DROP COLUMN policy_revision; ALTER TABLE automation_observation_results DROP COLUMN policy_revision; ALTER TABLE automation_streams DROP COLUMN policy_revision; PRAGMA user_version=12").unwrap();
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
            include_str!("../../migration-v11.sql"),
            include_str!("../../migration-v12.sql"),
        ))
        .unwrap();
    let schema = |conn: &Connection| {
        conn.prepare(
            "SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    };
    assert_eq!(
        schema(&store.conn),
        schema(&reference),
        "actual compiled schema-12 object definitions"
    );
    drop(store);
    let mut store = Store::open(&directory.path().join("core.sqlite")).unwrap();
    assert_eq!(counts(&store), before);
    assert_eq!(
        store
            .observe_health_policy("session", || 211, deadline())
            .unwrap()
            .revision,
        0
    );
    assert_eq!(store.job_state(&queued.id).unwrap(), "queued");
    let lease = lease(&mut store, &queued, 211);
    assert_eq!(lease.health_policy().revision, 0);
    store
        .automation_observation_complete(&lease, &batch(), || 211)
        .unwrap();
    assert_eq!(
        store
            .automation_publish_in_app(None, 32, || 212, deadline())
            .unwrap()
            .published,
        0
    );
}

#[test]
fn full_policy_history_refuses_new_versions_without_partial_audit_or_row() {
    let (_directory, mut store, actor) = setup();
    let body = json(&HealthPolicyDefinition::default()).unwrap();
    let digest = limeos_identity::digest(&body);
    store
        .write(|tx| {
            for revision in 1..=MAX_VERSIONS {
                tx.execute(
                    "INSERT INTO automation_health_policies VALUES(?,?,?,?,?)",
                    params![actor.id, revision, body, digest, 190],
                )
                .map_err(durable)?;
                event(
                    tx,
                    Some(&actor.id),
                    None,
                    EventKind::AutomationHealthPolicyChanged {
                        revision,
                        digest: digest.clone(),
                    },
                    190,
                )?;
            }
            Ok(())
        })
        .unwrap();
    check(&store.conn).unwrap();
    let before = counts(&store);
    assert!(matches!(
        store.observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision: MAX_VERSIONS,
                definition: definition(3)
            },
            || 200,
            deadline()
        ),
        Err(Error(ErrorCode::Overloaded))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn configured_mutes_keep_incidents_and_only_announce_after_enablement_or_exact_expiry() {
    for category in [true, false] {
        let (directory, mut store, actor) = setup();
        let mut policy = definition(1);
        if category {
            policy.disabled = vec![AlertKind::Host];
        } else {
            policy.silences = vec![HealthPolicySilence {
                kind: AlertKind::Host,
                key: "host:load".into(),
                created_at: "1970-01-01T00:03:10Z".into(),
                expires_at: Some("1970-01-01T00:03:30Z".into()),
                reason: "fixture work".into(),
            }];
        }
        replace(&mut store, 0, policy, 190);
        sample(&mut store, &actor, 200);
        assert_eq!(counts(&store).3, 0);
        let state: AlertState = parse(
            &store
                .conn
                .query_row(
                    "SELECT body FROM automation_alerts WHERE owner=?",
                    [&actor.id],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            state.incident,
            IncidentState::Open {
                announced: false,
                ..
            }
        ));
        sample(&mut store, &actor, 209);
        assert_eq!(counts(&store).3, 0);
        if category {
            replace(&mut store, 1, definition(1), 210);
        }
        sample(&mut store, &actor, 210);
        assert_eq!(counts(&store).3, 1);
        assert_eq!(
            store
                .automation_publish_in_app(None, 32, || 210, deadline())
                .unwrap()
                .published,
            1
        );
        drop(store);
        Store::open(&directory.path().join("core.sqlite")).unwrap();
    }
}
