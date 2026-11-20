use super::*;
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    s.issue_bootstrap("token", 100).unwrap();
    s.enroll("token", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    (d, s, p)
}
#[test]
fn enrollment_once_expiry_and_no_default_user() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert!(s.login_record("admin").unwrap().is_none());
    s.issue_bootstrap("expired", 1).unwrap();
    assert!(s.enroll("expired", "a", "$argon2id$x", 901).is_err());
    s.issue_bootstrap("fresh", 902).unwrap();
    assert!(s.enroll("expired", "a", "$argon2id$x", 903).is_err());
    s.enroll("fresh", "a", "$argon2id$x", 903).unwrap();
    assert!(s.enroll("fresh", "b", "$argon2id$x", 904).is_err());
    assert!(s.issue_bootstrap("new", 904).is_err());
}
#[test]
fn sessions_csrf_expiry_revoke_and_grant_change() {
    let (_d, mut s, p) = setup();
    let record = s.login_record("alice").unwrap().unwrap();
    let csrf = limeos_identity::digest("csrf");
    s.create_session(&record, None, "session", &csrf, 200)
        .unwrap();
    assert_eq!(s.authenticate("session", Some(&csrf), 201).unwrap(), p);
    assert!(s.authenticate("session", Some("wrong"), 201).is_err());
    assert!(s.authenticate("session", None, 200 + 8 * 3600).is_err());
    s.revoke_session("session", 202).unwrap();
    assert!(s.authenticate("session", None, 203).is_err());
    s.create_session(&record, None, "other", &csrf, 204)
        .unwrap();
    s.revise_grants(&p.id, Role::Viewer, &[], 205).unwrap();
    assert!(s.authenticate("other", None, 206).is_err());
    assert!(
        s.create_session(&record, None, "racing", &csrf, 206)
            .is_err()
    );
}
#[test]
fn task_uid_task_scope_expiry_revocation_revision_and_restart() {
    let (d, mut s, p) = setup();
    let scope = Scope {
        operation: Operation::HealthRead,
        resource: "system".into(),
    };
    s.issue_task(
        &p,
        TaskGrant {
            service_uid: 1001,
            task: "task",
            scopes: std::slice::from_ref(&scope),
            expires: 400,
        },
        "token",
        200,
    )
    .unwrap();
    assert!(s.check_task("token", 1001, "task", &scope, 201).is_ok());
    assert!(s.check_task("token", 1002, "task", &scope, 201).is_err());
    assert!(s.check_task("token", 1001, "other", &scope, 201).is_err());
    assert!(
        s.check_task(
            "token",
            1001,
            "task",
            &Scope {
                resource: "disk".into(),
                ..scope.clone()
            },
            201
        )
        .is_err()
    );
    assert!(s.check_task("token", 1001, "task", &scope, 400).is_err());
    s.revoke_task("token", 202).unwrap();
    assert!(s.check_task("token", 1001, "task", &scope, 203).is_err());
    s.issue_task(
        &p,
        TaskGrant {
            service_uid: 1001,
            task: "task",
            scopes: std::slice::from_ref(&scope),
            expires: 400,
        },
        "next",
        204,
    )
    .unwrap();
    let generation = s.generation;
    drop(s);
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert!(s.generation > generation);
    assert!(s.check_task("next", 1001, "task", &scope, 205).is_err());
    s.issue_task(
        &p,
        TaskGrant {
            service_uid: 1001,
            task: "task",
            scopes: std::slice::from_ref(&scope),
            expires: 400,
        },
        "revision",
        206,
    )
    .unwrap();
    s.revise_grants(&p.id, Role::Viewer, &[], 207).unwrap();
    assert!(s.check_task("revision", 1001, "task", &scope, 208).is_err());
}
#[test]
fn committed_intent_idempotency_resource_lock_deadline_and_revocation() {
    let (_d, mut s, p) = setup();
    let intent = Intent::HealthProbe {
        resource: "system".into(),
    };
    let id = s.queue_job(&p, "key", &intent, 400, 200).unwrap();
    assert_eq!(id, s.queue_job(&p, "key", &intent, 400, 201).unwrap());
    assert!(
        s.queue_job(
            &p,
            "key",
            &Intent::HealthProbe {
                resource: "other".into()
            },
            400,
            201
        )
        .is_err()
    );
    s.transition(&id, JobState::Queued, JobState::Running, 201)
        .unwrap();
    let other = s.queue_job(&p, "other", &intent, 400, 200).unwrap();
    assert!(
        s.transition(&other, JobState::Queued, JobState::Running, 202)
            .is_err()
    );
    assert!(
        s.transition(&id, JobState::Running, JobState::Succeeded, 202)
            .is_err()
    );
    s.transition(&id, JobState::Running, JobState::Verifying, 203)
        .unwrap();
    s.transition(&id, JobState::Verifying, JobState::Succeeded, 204)
        .unwrap();
    let expired = s
        .queue_job(
            &p,
            "expired",
            &Intent::HealthProbe {
                resource: "expiry-test".into(),
            },
            201,
            200,
        )
        .unwrap();
    assert!(
        s.transition(&expired, JobState::Queued, JobState::Running, 201)
            .is_err()
    );
    s.revise_grants(&p.id, Role::Viewer, &[], 205).unwrap();
    assert!(
        s.transition(&other, JobState::Queued, JobState::Running, 206)
            .is_err()
    );
}
#[test]
fn failures_rollback_intent_events_and_never_report_a_queued_operation() {
    let (_d, mut s, p) = setup();
    let before: i64 = s
        .conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    s.conn.execute_batch("CREATE TRIGGER reject_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        s.queue_job(
            &p,
            "key",
            &Intent::HealthProbe {
                resource: "system".into()
            },
            400,
            200
        )
        .unwrap_err()
        .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    s.conn
        .execute_batch("DROP TRIGGER reject_event; UPDATE meta SET audit_bytes=67108864;")
        .unwrap();
    assert!(
        s.queue_job(
            &p,
            "full",
            &Intent::HealthProbe {
                resource: "system".into()
            },
            400,
            200
        )
        .is_err()
    );
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn budget_reservation_unknown_usage_retains_maximum_and_settlement_is_unique() {
    let (_d, mut s, _) = setup();
    s.set_budget("period", 100).unwrap();
    s.reserve("r1", "period", "t1", "price1", 80).unwrap();
    s.reserve("r1", "period", "t1", "price1", 80).unwrap();
    assert!(s.reserve("r1", "period", "t1", "price2", 80).is_err());
    assert!(s.reserve("r2", "period", "t1", "price1", 21).is_err());
    assert!(s.settle("r1", 81).is_err());
    s.settle("r1", 30).unwrap();
    s.settle("r1", 30).unwrap();
    assert!(s.settle("r1", 29).is_err());
    s.reserve("r2", "period", "t1", "price1", 70).unwrap();
    assert!(s.reserve("r3", "period", "t1", "price1", 1).is_err());
}
#[test]
fn single_core_and_future_schema_refuse_startup() {
    let (d, s, _) = setup();
    assert!(Store::open(&d.path().join("core.sqlite")).is_err());
    drop(s);
    let conn = Connection::open(d.path().join("core.sqlite")).unwrap();
    conn.pragma_update(None, "user_version", 999).unwrap();
    drop(conn);
    assert!(Store::open(&d.path().join("core.sqlite")).is_err());
}
#[test]
fn symlinked_core_lock_is_refused_without_creating_authority_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let target = dir.path().join("operator-file");
    std::fs::write(&target, b"preserve operator bytes").unwrap();
    std::os::unix::fs::symlink(&target, path.with_extension("lock")).unwrap();
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"preserve operator bytes");
}

fn running(s: &mut Store, p: &Principal, key: &str) -> String {
    let id = s
        .queue_job(
            p,
            key,
            &Intent::HealthProbe {
                resource: key.into(),
            },
            400,
            200,
        )
        .unwrap();
    s.transition(&id, JobState::Queued, JobState::Running, 201)
        .unwrap();
    id
}
fn events(s: &Store) -> i64 {
    s.conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn startup_reconciles_a_wrong_audit_counter_instead_of_refusing() {
    let (d, mut s, p) = setup();
    let job = running(&mut s, &p, "interrupted");
    s.conn
        .execute("UPDATE meta SET audit_bytes=67108864", [])
        .unwrap();
    drop(s);
    let s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert_eq!(s.job_state(&job).unwrap(), "needs_intervention");
    assert_eq!(s.audit_usage().unwrap().bytes, events(&s) * 256);
}

#[test]
fn startup_refuses_only_when_recovery_cannot_be_recorded_even_in_the_reserve() {
    let (d, mut s, p) = setup();
    running(&mut s, &p, "interrupted");
    let generation = s.generation;
    // History that no retention rule may remove (schedule-change proofs)
    // filling the cap and the reserve.
    let rows = (AUDIT_LIMIT + retention::AUDIT_RESERVE) / 256;
    s.conn.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,'{\"kind\":\"automation_schedule_changed\",\"schedule\":\"s\",\"revision\":2}',300 FROM c",params![rows,p.id]).unwrap();
    drop(s);
    assert_eq!(
        Store::open(&d.path().join("core.sqlite")).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    let conn = Connection::open(d.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    // With nothing to recover the same store opens, so reads keep working
    // while ordinary writes stay refused at the cap.
    conn.execute(
        "UPDATE jobs SET state='needs_intervention' WHERE state='running'",
        [],
    )
    .unwrap();
    drop(conn);
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert_eq!(s.audit_usage().unwrap().level(), AuditLevel::Full);
    assert!(s.login_record("alice").unwrap().is_some());
    assert!(s.set_budget("period", 1).is_err());
}

/// A live session and task token for `p`, for the revocation tests.
fn revocable(s: &mut Store, p: &Principal, name: &str) {
    let record = s.login_record("alice").unwrap().unwrap();
    s.create_session(&record, None, name, "csrf", 200).unwrap();
    s.issue_task(
        p,
        TaskGrant {
            service_uid: 1001,
            task: "task",
            scopes: &[Scope {
                operation: Operation::HealthRead,
                resource: "system".into(),
            }],
            expires: 3000,
        },
        &format!("{name}-task"),
        200,
    )
    .unwrap();
}
fn live(s: &Store, name: &str) -> (bool, bool) {
    let scope = Scope {
        operation: Operation::HealthRead,
        resource: "system".into(),
    };
    (
        s.authenticate(name, None, 300).is_ok(),
        s.check_task(&format!("{name}-task"), 1001, "task", &scope, 300)
            .is_ok(),
    )
}

#[test]
fn revocations_that_match_nothing_record_nothing() {
    let (_d, mut s, p) = setup();
    revocable(&mut s, &p, "session");
    let before = events(&s);
    s.revoke_session("missing", 210).unwrap();
    s.revoke_task("missing", 210).unwrap();
    assert_eq!(events(&s), before);
    s.revoke_session("session", 211).unwrap();
    s.revoke_task("session-task", 211).unwrap();
    assert_eq!(events(&s), before + 2);
    assert_eq!(live(&s, "session"), (false, false));
    s.revoke_session("session", 212).unwrap();
    s.revoke_task("session-task", 212).unwrap();
    assert_eq!(events(&s), before + 2);
}

#[test]
fn crash_helper() {
    let Some(path) = std::env::var_os("LIMEOS_CRASH_TEST_PATH") else {
        return;
    };
    let mut s = Store::open(Path::new(&path)).unwrap();
    s.issue_bootstrap("token", 100).unwrap();
    s.enroll("token", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let id = s
        .queue_job(
            &p,
            "crash",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            400,
            200,
        )
        .unwrap();
    s.transition(&id, JobState::Queued, JobState::Running, 201)
        .unwrap();
    std::process::exit(77); // No destructors or graceful checkpoint.
}
#[test]
fn process_crash_preserves_committed_intent_and_requires_reconciliation() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("core.sqlite");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::crash_helper"])
        .env("LIMEOS_CRASH_TEST_PATH", &path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(77));
    let mut s = Store::open(&path).unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let id = s
        .queue_job(
            &p,
            "crash",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            400,
            202,
        )
        .unwrap();
    assert_eq!(s.job_state(&id).unwrap(), "needs_intervention");
    let kind: String = s
        .conn
        .query_row(
            "SELECT kind FROM events WHERE job=? ORDER BY cursor DESC LIMIT 1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        parse::<EventKind>(&kind).unwrap(),
        EventKind::JobRecoveryRequired
    );
    assert!(
        s.queue_job(
            &p,
            "crash",
            &Intent::HealthProbe {
                resource: "other".into()
            },
            400,
            202
        )
        .is_err()
    );
    let next = s
        .queue_job(
            &p,
            "next",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            400,
            202,
        )
        .unwrap();
    assert!(
        s.transition(&next, JobState::Queued, JobState::Running, 203)
            .is_err()
    );
}
#[tokio::test]
async fn bounded_worker_serializes_concurrent_budget_reservations() {
    let d = tempfile::tempdir().unwrap();
    let db = Database::open(&d.path().join("core.sqlite")).unwrap();
    db.call(|s| s.set_budget("period", 100)).await.unwrap();
    let mut handles = vec![];
    for i in 0..20 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            db.call(move |s| s.reserve(&format!("r{i}"), "period", "task", "price", 10))
                .await
        }));
    }
    let mut success = 0;
    for h in handles {
        if h.await.unwrap().is_ok() {
            success += 1;
        }
    }
    assert_eq!(success, 10);
}
#[test]
fn symlinked_database_lock_is_refused_on_every_architecture() {
    let d = tempfile::tempdir().unwrap();
    let elsewhere = d.path().join("elsewhere.lock");
    std::os::unix::fs::symlink(&elsewhere, d.path().join("core.lock")).unwrap();
    assert!(Store::open(&d.path().join("core.sqlite")).is_err());
    assert!(!elsewhere.exists());
}
const SCRYPT: &str = "scrypt:32768:8:1$vdYT4OKICpm89RVk$04c86b2538e3f92ba1e4d2b9a2423d3f75bda2c7be27d67f30cda40d7638089134fee1f549b2968273fe31fbf30da2b9b3f0a4d5560077b0376cfca0ab2f2397";
const PBKDF2: &str = "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1";
#[test]
fn legacy_batch_is_atomic_idempotent_and_never_overwrites() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    let both = [
        ("holly".to_owned(), SCRYPT.to_owned()),
        ("bob".to_owned(), PBKDF2.to_owned()),
    ];
    assert_eq!(s.import_legacy_users(&both, 10).unwrap(), (2, vec![]));
    assert_eq!(
        s.import_legacy_users(&both, 11).unwrap(),
        (0, vec!["holly".to_owned(), "bob".to_owned()])
    );
    let count = |s: &Store, table: &str| -> i64 {
        s.conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&s, "users"), 2);
    assert_eq!(count(&s, "grants"), 10);
    // A mixed batch imports only the new user and never touches the existing one.
    let mixed = [
        ("carol".to_owned(), SCRYPT.to_owned()),
        ("bob".to_owned(), SCRYPT.to_owned()),
    ];
    assert_eq!(
        s.import_legacy_users(&mixed, 12).unwrap(),
        (1, vec!["bob".to_owned()])
    );
    assert_eq!(
        s.login_record("carol").unwrap().unwrap().password_hash,
        SCRYPT
    );
    assert_eq!(
        s.login_record("bob").unwrap().unwrap().password_hash,
        PBKDF2
    );
    assert_eq!(count(&s, "users"), 3);
    // An invalid entry anywhere aborts the whole batch.
    let events = count(&s, "events");
    let tainted = [
        ("dave".to_owned(), SCRYPT.to_owned()),
        ("eve".to_owned(), "plaintext".to_owned()),
    ];
    assert_eq!(
        s.import_legacy_users(&tainted, 13).unwrap_err().0,
        ErrorCode::InvalidInput
    );
    assert_eq!(count(&s, "events"), events);
    // Invalid, duplicate and empty batches are refused before any write.
    for bad in [
        vec![],
        vec![("a".to_owned(), "plaintext".to_owned())],
        vec![(String::new(), SCRYPT.to_owned())],
        vec![("a".to_owned(), SCRYPT.to_owned()); 2],
        vec![
            ("good".to_owned(), SCRYPT.to_owned()),
            ("bad name".to_owned(), SCRYPT.to_owned()),
        ],
    ] {
        assert_eq!(
            s.import_legacy_users(&bad, 13).unwrap_err().0,
            ErrorCode::InvalidInput
        );
    }
    assert_eq!(count(&s, "users"), 3);
}
