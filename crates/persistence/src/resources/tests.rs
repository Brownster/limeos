use super::*;
use limeos_domain::{ContainerAction, ContainerReceipt, ContainerSnapshot, ExecutionState};

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = store.login_record("alice").unwrap().unwrap().principal;
    (dir, store, p)
}
fn snapshot(byte: &str) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", byte.repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    }
}
fn queue(store: &mut Store, p: &Principal, key: &str, byte: &str) -> limeos_domain::ContainerJob {
    let before = snapshot(byte);
    let plan = store.plan_restart(p, &before, 200).unwrap();
    let approval = store
        .approve_container(p, &plan.plan.id, &plan.digest, 201)
        .unwrap();
    store
        .queue_container(p, key, &plan, &approval.token, &before, 202)
        .unwrap()
}
/// Models operation-owned dependencies inside the sole writer, not API input.
/// Current container operations still derive only their primary container ID.
fn dependencies(store: &mut Store, job: &str, resources: &[&str]) {
    store
        .write(|tx| {
            for resource in resources {
                tx.execute(
                    "INSERT INTO job_resources VALUES(?,?)",
                    params![job, resource],
                )
                .map_err(durable)?;
            }
            Ok(())
        })
        .unwrap();
}
fn locks(store: &Store, job: &str) -> Vec<String> {
    store
        .conn
        .prepare("SELECT resource FROM resource_locks WHERE job=? ORDER BY resource")
        .unwrap()
        .query_map([job], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn receipt(job: &limeos_domain::ContainerJob, state: ExecutionState) -> ContainerReceipt {
    ContainerReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&json(&job.plan).unwrap()),
        operation: ContainerAction::Restart,
        before: job.plan.expected.clone(),
        state,
        error: None,
    }
}

#[test]
fn complete_resource_sets_are_atomic_and_unrelated_jobs_can_dispatch() {
    let (_dir, mut s, p) = setup();
    let a = queue(&mut s, &p, "a", "a");
    let b = queue(&mut s, &p, "b", "b");
    let c = queue(&mut s, &p, "c", "c");
    dependencies(
        &mut s,
        &a.id,
        &["storage:uuid:disk", "storage:mount:/mnt/Data"],
    );
    dependencies(
        &mut s,
        &b.id,
        &["storage:uuid:disk", "storage:mount:/mnt/Other"],
    );
    dependencies(&mut s, &c.id, &["storage:uuid:another"]);
    s.claim_container(&p, &a.id, &snapshot("a"), 203).unwrap();
    let events = s.container_events(&p, &b.id, 0).unwrap();
    assert_eq!(
        s.claim_container(&p, &b.id, &snapshot("b"), 204)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(s.job_state(&b.id).unwrap(), "queued");
    assert!(locks(&s, &b.id).is_empty());
    assert_eq!(s.container_events(&p, &b.id, 0).unwrap(), events);
    assert_eq!(locks(&s, &a.id).len(), 3);
    s.claim_container(&p, &c.id, &snapshot("c"), 204).unwrap();
    assert_eq!(locks(&s, &c.id).len(), 2);
    check(&s.conn).unwrap();
}

#[test]
fn audit_failure_rolls_back_all_acquisitions_and_all_releases() {
    let (_dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    dependencies(
        &mut s,
        &job.id,
        &["storage:configuration", "storage:uuid:disk"],
    );
    s.conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.claim_container(&p, &job.id, &snapshot("a"), 203).is_err());
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
    assert!(locks(&s, &job.id).is_empty());
    s.conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    let job = s.claim_container(&p, &job.id, &snapshot("a"), 204).unwrap();
    let held = locks(&s, &job.id);
    s.conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let after = ContainerSnapshot {
        started_at: "2026-10-05T07:00:01Z".into(),
        ..snapshot("a")
    };
    assert!(
        s.record_container_result(
            &job,
            &receipt(&job, ExecutionState::Verified),
            Some(&after),
            205
        )
        .is_err()
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "running");
    assert_eq!(locks(&s, &job.id), held);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM container_results", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    s.conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    s.record_container_result(
        &job,
        &receipt(&job, ExecutionState::Verified),
        Some(&after),
        206,
    )
    .unwrap();
    assert!(locks(&s, &job.id).is_empty());
}

#[test]
fn only_bound_terminal_proof_releases_dependencies_and_retains_required_set() {
    let (_dir, mut s, p) = setup();
    let a = queue(&mut s, &p, "a", "a");
    let b = queue(&mut s, &p, "b", "b");
    dependencies(&mut s, &a.id, &["storage:configuration"]);
    dependencies(&mut s, &b.id, &["storage:configuration"]);
    let a = s.claim_container(&p, &a.id, &snapshot("a"), 203).unwrap();
    let held = locks(&s, &a.id);
    assert!(
        s.transition(&a.id, JobState::Running, JobState::Failed, 204)
            .is_err()
    );
    assert!(s.cancel_container_job(&p, &a.id, 204).is_err());
    let after = ContainerSnapshot {
        started_at: "2026-10-05T07:00:01Z".into(),
        ..snapshot("a")
    };
    let mut proof = receipt(&a, ExecutionState::Verified);
    proof.action = b.id.clone();
    assert!(
        s.record_container_result(&a, &proof, Some(&after), 204)
            .is_err()
    );
    assert_eq!(locks(&s, &a.id), held);
    assert_eq!(
        s.record_container_result(&a, &receipt(&a, ExecutionState::EffectAccepted), None, 205)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(locks(&s, &a.id), held);
    assert!(s.claim_container(&p, &b.id, &snapshot("b"), 206).is_err());
    s.record_container_result(
        &a,
        &receipt(&a, ExecutionState::Verified),
        Some(&after),
        207,
    )
    .unwrap();
    assert!(locks(&s, &a.id).is_empty());
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM job_resources WHERE job=?",
                [&a.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    s.claim_container(&p, &b.id, &snapshot("b"), 208).unwrap();
    assert_eq!(locks(&s, &b.id).len(), 2);
}

#[test]
fn uncertainty_expiry_revocation_and_restart_keep_the_entire_set() {
    let (dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    dependencies(
        &mut s,
        &job.id,
        &["storage:configuration", "storage:mount:/mnt/TV"],
    );
    let job = s.claim_container(&p, &job.id, &snapshot("a"), 203).unwrap();
    let held = locks(&s, &job.id);
    s.transition(&job.id, JobState::Running, JobState::OutcomeUnknown, 204)
        .unwrap();
    s.revise_grants(&p.id, Role::Viewer, &[], 205).unwrap();
    drop(s);
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(s.job_state(&job.id).unwrap(), "outcome_unknown");
    assert_eq!(locks(&s, &job.id), held);
    // Authenticated executor proof can reconcile expired/revoked work; it does
    // not grant authority for a new effect or refresh the old approval.
    s.record_container_result(
        &job,
        &receipt(&job, ExecutionState::PreconditionChanged),
        None,
        1000,
    )
    .unwrap();
    assert!(locks(&s, &job.id).is_empty());
}

#[test]
fn active_sets_and_job_identity_cannot_be_mutated_or_stolen() {
    let (_dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    dependencies(&mut s, &job.id, &["storage:configuration"]);
    s.claim_container(&p, &job.id, &snapshot("a"), 203).unwrap();
    let held = locks(&s, &job.id);
    for sql in [
        "INSERT INTO job_resources VALUES(?,'storage:uuid:new')",
        "DELETE FROM job_resources WHERE job=?",
        "UPDATE job_resources SET resource='storage:uuid:new' WHERE job=?",
        "DELETE FROM resource_locks WHERE job=?",
        "UPDATE resource_locks SET resource='storage:uuid:new' WHERE job=?",
        "UPDATE jobs SET resource='container:other' WHERE id=?",
        "UPDATE jobs SET deadline=999999 WHERE id=?",
        "UPDATE jobs SET digest='substituted' WHERE id=?",
    ] {
        assert!(s.conn.execute(sql, [&job.id]).is_err(), "{sql}");
    }
    assert_eq!(locks(&s, &job.id), held);
    check(&s.conn).unwrap();
}

#[test]
fn resource_bounds_and_canceled_queue_do_not_leave_locks() {
    let (_dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    for value in [
        String::new(),
        "x".repeat(769),
        "nul\0value".into(),
        "new\nline".into(),
        "carriage\rreturn".into(),
    ] {
        assert!(
            s.conn
                .execute(
                    "INSERT INTO job_resources VALUES(?,?)",
                    params![job.id, value]
                )
                .is_err()
        );
    }
    for i in 0..127 {
        dependencies(&mut s, &job.id, &[&format!("storage:uuid:fixture-{i}")]);
    }
    assert!(
        s.conn
            .execute("INSERT INTO job_resources VALUES(?,'overflow')", [&job.id])
            .is_err()
    );
    assert!(
        s.conn
            .execute(
                "DELETE FROM job_resources WHERE job=? AND resource=?",
                params![job.id, job.plan.expected.resource]
            )
            .is_err()
    );
    s.cancel_container_job(&p, &job.id, 203).unwrap();
    assert!(locks(&s, &job.id).is_empty());
    assert!(
        s.conn
            .execute(
                "INSERT INTO job_resources VALUES(?,'after-cancel')",
                [&job.id]
            )
            .is_err()
    );
    check(&s.conn).unwrap();
}

#[test]
fn v6_upgrade_preserves_approval_bytes_and_all_old_active_locks() {
    let (dir, mut s, p) = setup();
    let queued = queue(&mut s, &p, "queued", "e");
    let plan_bytes: (String, String, String) = s
        .conn
        .query_row(
            "SELECT body,digest,approval_digest FROM container_plans WHERE job=?",
            [&queued.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let mut jobs = vec![];
    for (byte, state) in [
        ("a", JobState::Running),
        ("b", JobState::Verifying),
        ("c", JobState::OutcomeUnknown),
        ("d", JobState::NeedsIntervention),
    ] {
        let job = queue(&mut s, &p, byte, byte);
        s.claim_container(&p, &job.id, &snapshot(byte), 203)
            .unwrap();
        if state != JobState::Running {
            s.transition(&job.id, JobState::Running, state, 204)
                .unwrap();
        }
        jobs.push(job);
    }
    remove_v7_schema(&s.conn);
    s.conn.pragma_update(None, "user_version", 6).unwrap();
    drop(s);
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        s.conn
            .query_row(
                "SELECT body,digest,approval_digest FROM container_plans WHERE job=?",
                [&queued.id],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?
                ))
            )
            .unwrap(),
        plan_bytes
    );
    for job in &jobs {
        assert_eq!(locks(&s, &job.id), vec![job.plan.expected.resource.clone()]);
    }
    assert!(locks(&s, &queued.id).is_empty());
    s.claim_container(&p, &queued.id, &snapshot("e"), 205)
        .unwrap();
    assert_eq!(locks(&s, &queued.id).len(), 1);
    check(&s.conn).unwrap();
}

#[test]
fn failed_v7_migration_preserves_v6_jobs_authority_and_generation() {
    let (dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    s.claim_container(&p, &job.id, &snapshot("a"), 203).unwrap();
    remove_v7_schema(&s.conn);
    s.conn
        .execute_batch("PRAGMA user_version=6; CREATE INDEX resource_locks_job ON users(username);")
        .unwrap();
    let generation = s.generation;
    drop(s);
    assert_eq!(
        Store::open(&dir.path().join("core.sqlite"))
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        generation
    );
    assert_eq!(
        conn.query_row("SELECT state FROM jobs WHERE id=?", [&job.id], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "running"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='job_resources'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn missing_or_extra_locks_refuse_startup_without_repair_or_recovery_events() {
    for fault in [
        "missing-lock",
        "inactive-lock",
        "missing-primary",
        "foreign-owner",
    ] {
        let (dir, mut s, p) = setup();
        let active = queue(&mut s, &p, "active", "a");
        let queued = queue(&mut s, &p, "queued", "b");
        s.claim_container(&p, &active.id, &snapshot("a"), 203)
            .unwrap();
        let generation = s.generation;
        let events: i64 = s
            .conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        let guards: Vec<String> = s.conn
            .prepare("SELECT sql FROM sqlite_schema WHERE type='trigger' AND name IN ('resource_locks_delete','resource_locks_insert','job_resources_delete')")
            .unwrap().query_map([], |r|r.get(0)).unwrap()
            .collect::<std::result::Result<_, _>>().unwrap();
        s.conn.execute_batch("DROP TRIGGER resource_locks_delete; DROP TRIGGER resource_locks_insert; DROP TRIGGER job_resources_delete;").unwrap();
        match fault {
            "missing-lock" => {
                s.conn
                    .execute("DELETE FROM resource_locks WHERE job=?", [&active.id])
                    .unwrap();
            }
            "inactive-lock" => {
                s.conn
                    .execute(
                        "INSERT INTO resource_locks VALUES(?,?)",
                        params![queued.plan.expected.resource, queued.id],
                    )
                    .unwrap();
            }
            "missing-primary" => {
                s.conn
                    .execute("DELETE FROM job_resources WHERE job=?", [&queued.id])
                    .unwrap();
            }
            _ => {
                s.conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
                s.conn
                    .execute("INSERT INTO resource_locks VALUES('orphan','absent')", [])
                    .unwrap();
            }
        }
        for guard in guards {
            s.conn.execute_batch(&guard).unwrap();
        }
        drop(s);
        assert_eq!(
            Store::open(&dir.path().join("core.sqlite"))
                .err()
                .unwrap()
                .0,
            ErrorCode::StateNotDurable,
            "{fault}"
        );
        let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            generation
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            events
        );
        assert_eq!(
            conn.query_row("SELECT state FROM jobs WHERE id=?", [&active.id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "running"
        );
    }
}

#[test]
fn missing_or_changed_dispatch_guards_refuse_startup_with_healthy_pages() {
    for changed in [false, true] {
        let (dir, mut s, p) = setup();
        let queued = queue(&mut s, &p, "queued", "a");
        let generation = s.generation;
        s.conn
            .execute_batch("DROP TRIGGER jobs_acquire_resources")
            .unwrap();
        if changed {
            s.conn.execute_batch("CREATE TRIGGER jobs_acquire_resources AFTER UPDATE OF state ON jobs BEGIN SELECT 1; END;").unwrap();
        }
        assert_eq!(
            s.conn
                .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        drop(s);
        assert_eq!(
            Store::open(&dir.path().join("core.sqlite"))
                .err()
                .unwrap()
                .0,
            ErrorCode::StateNotDurable
        );
        let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            generation
        );
        assert_eq!(
            conn.query_row("SELECT state FROM jobs WHERE id=?", [&queued.id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "queued"
        );
    }
}

#[test]
fn kill_helper() {
    let Some(path) = std::env::var_os("LIMEOS_RESOURCE_KILL_TEST_PATH") else {
        return;
    };
    let mut s = Store::open(Path::new(&path)).unwrap();
    s.issue_bootstrap("bootstrap", 100).unwrap();
    s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let job = queue(&mut s, &p, "killed", "a");
    dependencies(
        &mut s,
        &job.id,
        &[
            "storage:configuration",
            "storage:uuid:disk",
            "storage:mount:/mnt/TV",
        ],
    );
    s.claim_container(&p, &job.id, &snapshot("a"), 203).unwrap();
    std::fs::write(Path::new(&path).with_extension("ready"), &job.id).unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn real_process_kill_preserves_every_committed_claim() {
    use std::os::unix::process::ExitStatusExt;
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let ready = path.with_extension("ready");
    let mut child = ChildGuard(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resources::tests::kill_helper"])
            .env("LIMEOS_RESOURCE_KILL_TEST_PATH", &path)
            .spawn()
            .unwrap(),
    );
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    let id = std::fs::read_to_string(ready).unwrap();
    let s = Store::open(&path).unwrap();
    assert_eq!(s.job_state(&id).unwrap(), "needs_intervention");
    assert_eq!(locks(&s, &id).len(), 4);
    check(&s.conn).unwrap();
}
