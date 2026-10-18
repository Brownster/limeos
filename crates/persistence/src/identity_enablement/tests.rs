use super::*;

fn setup() -> (tempfile::TempDir, Store, LoginRecord) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let record = session(&mut store);
    (dir, store, record)
}
fn session(store: &mut Store) -> LoginRecord {
    let record = store.login_record("alice").unwrap().unwrap();
    store.revoke_session("session", 102).unwrap();
    store
        .create_session(&record, None, "session", "csrf", 102)
        .unwrap();
    record
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    identity: (String, i64, String),
    grants: Vec<(String, String)>,
    events: i64,
    audit: i64,
    locks: Vec<(String, String)>,
}
fn snapshot(store: &Store) -> Snapshot {
    let identity = store
        .conn
        .query_row(
            "SELECT role,grant_revision,password_hash FROM users WHERE username='alice'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let grants = store
        .conn
        .prepare("SELECT operation,resource FROM grants ORDER BY operation,resource")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let (events, audit) = store
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM events),audit_bytes FROM meta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let locks = store
        .conn
        .prepare("SELECT resource,job FROM resource_locks ORDER BY resource")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    Snapshot {
        identity,
        grants,
        events,
        audit,
        locks,
    }
}

#[test]
fn fixed_grant_revision_audit_and_revocation_preserve_other_authority() {
    let (_dir, mut store, record) = setup();
    assert!(!enabled(&store.conn, &record.principal.id).unwrap());
    let job = store
        .queue_job(
            &record.principal,
            "existing-job",
            &Intent::HealthProbe {
                resource: "host:system".into(),
            },
            500,
            102,
        )
        .unwrap();
    store
        .transition(&job, JobState::Queued, JobState::Running, 103)
        .unwrap();
    let scope = Scope {
        operation: Operation::HealthRead,
        resource: "host:system".into(),
    };
    store
        .issue_task(
            &record.principal,
            TaskGrant {
                service_uid: 42,
                task: "health",
                scopes: std::slice::from_ref(&scope),
                expires: 300,
            },
            "task",
            103,
        )
        .unwrap();
    let before = snapshot(&store);
    let principal = store
        .enable_observe_health("session", "csrf", &record, None, || 104, deadline())
        .unwrap();
    let after = snapshot(&store);
    assert_eq!(principal.role, record.principal.role);
    assert_eq!(
        principal.grant_revision,
        record.principal.grant_revision + 1
    );
    assert!(
        before
            .grants
            .iter()
            .all(|grant| after.grants.contains(grant))
    );
    assert_eq!(after.grants.len(), before.grants.len() + 1);
    assert_eq!(after.events, before.events + 1);
    assert_eq!(after.audit, before.audit + 256);
    assert_eq!(after.locks, before.locks);
    assert_eq!(store.job_state(&job).unwrap(), "running");
    assert_eq!(
        store.authenticate("session", Some("csrf"), 104),
        Err(Error(ErrorCode::Unauthenticated))
    );
    assert_eq!(
        store.check_task("task", 42, "health", &scope, 104),
        Err(Error(ErrorCode::Expired))
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE principal=? AND kind=?",
                params![principal.id, json(&EventKind::GrantsRevised).unwrap()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn existing_exact_grant_is_query_only_noop_even_with_hash_upgrade_and_full_audit() {
    let (_dir, mut store, record) = setup();
    store
        .enable_observe_health("session", "csrf", &record, None, || 103, deadline())
        .unwrap();
    let current = session(&mut store);
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    let before = snapshot(&store);
    let writes = store.conn.total_changes();
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    assert_eq!(
        store
            .enable_observe_health(
                "session",
                "csrf",
                &current,
                Some("$argon2id$must-not-write"),
                || 104,
                deadline()
            )
            .unwrap(),
        current.principal
    );
    assert_eq!(snapshot(&store), before);
    assert_eq!(store.conn.total_changes(), writes);
}

#[test]
fn session_csrf_hash_role_and_grants_are_rechecked_after_verification() {
    for change in [
        "logout", "csrf", "hash", "role", "grants", "expiry", "deadline",
    ] {
        let (_dir, mut store, record) = setup();
        let mut now = 104;
        match change {
            "logout" => {
                store.revoke_session("session", 103).unwrap();
            }
            "csrf" => {
                store
                    .conn
                    .execute("UPDATE sessions SET csrf_digest='changed'", [])
                    .unwrap();
            }
            "hash" => {
                store
                    .conn
                    .execute("UPDATE users SET password_hash='$argon2id$changed'", [])
                    .unwrap();
            }
            "role" => {
                store
                    .revise_grants(&record.principal.id, Role::Operator, &[], 103)
                    .unwrap();
            }
            "grants" => {
                store
                    .revise_grants(&record.principal.id, Role::Administrator, &[], 103)
                    .unwrap();
            }
            "expiry" => now = 40000,
            _ => {}
        }
        let before = snapshot(&store);
        let deadline = if change == "deadline" {
            Instant::now()
        } else {
            deadline()
        };
        let expected = match change {
            "csrf" => ErrorCode::Forbidden,
            "hash" | "deadline" => ErrorCode::Expired,
            _ => ErrorCode::Unauthenticated,
        };
        assert_eq!(
            store.enable_observe_health("session", "csrf", &record, None, || now, deadline),
            Err(Error(expected)),
            "{change}"
        );
        assert_eq!(snapshot(&store), before, "{change}");
    }
}

#[test]
fn stored_identity_management_and_host_read_are_required_without_role_shortcut() {
    for (role, grants, allowed) in [
        (Role::Administrator, vec![], false),
        (
            Role::Administrator,
            vec![Scope {
                operation: Operation::HealthRead,
                resource: "*".into(),
            }],
            false,
        ),
        (
            Role::Administrator,
            vec![Scope {
                operation: Operation::IdentityManage,
                resource: "*".into(),
            }],
            false,
        ),
        (
            Role::Operator,
            vec![
                Scope {
                    operation: Operation::IdentityManage,
                    resource: "*".into(),
                },
                Scope {
                    operation: Operation::HealthRead,
                    resource: "*".into(),
                },
            ],
            false,
        ),
        (
            Role::Administrator,
            vec![
                Scope {
                    operation: Operation::IdentityManage,
                    resource: RESOURCE.into(),
                },
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
            ],
            true,
        ),
    ] {
        let (_dir, mut store, record) = setup();
        store
            .revise_grants(&record.principal.id, role, &grants, 103)
            .unwrap();
        session(&mut store);
        let before = snapshot(&store);
        assert_eq!(
            store
                .observe_health_reauthentication("session", "csrf", 104)
                .is_ok(),
            allowed
        );
        assert_eq!(snapshot(&store), before);
    }
}

#[test]
fn failures_after_identity_grant_or_audit_write_roll_back_every_change() {
    for trigger in [
        "CREATE TEMP TRIGGER fail AFTER UPDATE OF grant_revision ON users BEGIN SELECT RAISE(ABORT,'identity fault'); END",
        "CREATE TEMP TRIGGER fail AFTER INSERT ON grants WHEN json_extract(NEW.operation,'$')='automation_manage' BEGIN SELECT RAISE(ABORT,'grant fault'); END",
        "CREATE TEMP TRIGGER fail AFTER INSERT ON events WHEN json_extract(NEW.kind,'$.kind')='grants_revised' BEGIN SELECT RAISE(ABORT,'audit fault'); END",
    ] {
        let (_dir, mut store, record) = setup();
        store.conn.execute_batch(trigger).unwrap();
        let before = snapshot(&store);
        assert_eq!(
            store.enable_observe_health(
                "session",
                "csrf",
                &record,
                Some("$argon2id$upgraded"),
                || 104,
                deadline()
            ),
            Err(Error(ErrorCode::StateNotDurable))
        );
        assert_eq!(snapshot(&store), before);
    }
}

#[test]
fn exact_grant_and_audit_boundaries_refuse_whole_changes() {
    for count in [31, 32] {
        let (_dir, mut store, record) = setup();
        let mut grants = store.grants(&record.principal).unwrap();
        for index in grants.len()..count {
            grants.push(Scope {
                operation: Operation::HealthRead,
                resource: format!("fixture:{index}"),
            });
        }
        store
            .revise_grants(&record.principal.id, Role::Administrator, &grants, 103)
            .unwrap();
        let record = session(&mut store);
        let before = snapshot(&store);
        let result =
            store.enable_observe_health("session", "csrf", &record, None, || 104, deadline());
        if count == 31 {
            assert!(result.is_ok());
            assert_eq!(snapshot(&store).grants.len(), 32);
        } else {
            assert_eq!(result, Err(Error(ErrorCode::Overloaded)));
            assert_eq!(snapshot(&store), before);
        }
    }
    for audit in [AUDIT_LIMIT - 256, AUDIT_LIMIT - 255] {
        let (_dir, mut store, record) = setup();
        store
            .conn
            .execute("UPDATE meta SET audit_bytes=?", [audit])
            .unwrap();
        let before = snapshot(&store);
        let result =
            store.enable_observe_health("session", "csrf", &record, None, || 104, deadline());
        if audit == AUDIT_LIMIT - 256 {
            assert!(result.is_ok());
            assert_eq!(snapshot(&store).audit, AUDIT_LIMIT);
        } else {
            assert_eq!(result, Err(Error(ErrorCode::StateNotDurable)));
            assert_eq!(snapshot(&store), before);
        }
    }
}

#[test]
fn original_deadline_expires_while_waiting_for_actual_database_write_lock() {
    let (dir, mut store, record) = setup();
    let writer = Connection::open(dir.path().join("core.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = snapshot(&store);
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        writer.execute_batch("COMMIT").unwrap();
    });
    assert_eq!(
        store.enable_observe_health(
            "session",
            "csrf",
            &record,
            None,
            || 104,
            Instant::now() + Duration::from_millis(20)
        ),
        Err(Error(ErrorCode::Expired))
    );
    release.join().unwrap();
    assert_eq!(snapshot(&store), before);
}

#[test]
fn active_effect_claim_is_not_released_or_rewritten_by_self_enablement() {
    let (_dir, mut store, record) = setup();
    let observed = limeos_domain::ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    };
    let plan = store
        .plan_container(
            &record.principal,
            limeos_domain::ContainerAction::Stop,
            &observed,
            103,
        )
        .unwrap();
    let approval = store
        .approve_container(&record.principal, &plan.plan.id, &plan.digest, 104)
        .unwrap();
    let job = store
        .queue_container(
            &record.principal,
            "pending-effect",
            &plan,
            &approval.token,
            &observed,
            105,
        )
        .unwrap();
    let active = store
        .claim_container(&record.principal, &job.id, &observed, 106)
        .unwrap();
    let before = snapshot(&store);
    assert!(!before.locks.is_empty());
    store
        .enable_observe_health("session", "csrf", &record, None, || 107, deadline())
        .unwrap();
    assert_eq!(snapshot(&store).locks, before.locks);
    assert_eq!(store.job_state(&active.id).unwrap(), "running");
}

#[test]
fn deadline_after_real_transaction_writes_rolls_back_before_commit() {
    let (_dir, mut store, record) = setup();
    // An owned SQLite-only delay after the grant insert tests the final check,
    // with constant-memory work and no public test/clock/proof hook.
    store.conn.execute_batch("CREATE TEMP TRIGGER delay AFTER INSERT ON grants WHEN json_extract(NEW.operation,'$')='automation_manage' BEGIN SELECT sum(n) FROM (WITH RECURSIVE counter(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM counter WHERE n<10000000) SELECT n FROM counter); END").unwrap();
    let before = snapshot(&store);
    let writes = store.conn.total_changes();
    assert_eq!(
        store.enable_observe_health(
            "session",
            "csrf",
            &record,
            None,
            || 104,
            Instant::now() + Duration::from_millis(200)
        ),
        Err(Error(ErrorCode::Expired))
    );
    assert!(
        store.conn.total_changes() > writes,
        "fixture must reach transaction mutations before expiry"
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn session_expiring_during_real_write_lock_wait_uses_fresh_transaction_clock() {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
        mpsc,
    };
    let (dir, mut store, record) = setup();
    store
        .conn
        .execute("UPDATE sessions SET expires=105 WHERE digest='session'", [])
        .unwrap();
    let writer = Connection::open(dir.path().join("core.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = snapshot(&store);
    let wall = Arc::new(AtomicI64::new(104));
    let calls = AtomicUsize::new(0);
    let (sampled, first_sample) = mpsc::sync_channel(1);
    let advance = wall.clone();
    let release = std::thread::spawn(move || {
        first_sample.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        advance.store(106, Ordering::SeqCst);
        writer.execute_batch("COMMIT").unwrap();
    });
    let original_deadline = deadline();
    let result = store.enable_observe_health(
        "session",
        "csrf",
        &record,
        Some("$argon2id$must-not-write"),
        || {
            let observed = wall.load(Ordering::SeqCst);
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                sampled.send(()).unwrap();
            }
            observed
        },
        original_deadline,
    );
    release.join().unwrap();
    assert_eq!(result, Err(Error(ErrorCode::Unauthenticated)));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "read and acquired write transaction need distinct samples"
    );
    assert!(
        Instant::now() < original_deadline,
        "failure must be session expiry, not budget expiry"
    );
    assert_eq!(snapshot(&store), before);
}
