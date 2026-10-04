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
fn startup_refuses_recovery_without_audit_capacity() {
    let (d, s, _) = setup();
    s.conn
        .execute("UPDATE meta SET audit_bytes=67108864", [])
        .unwrap();
    let generation = s.generation;
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
