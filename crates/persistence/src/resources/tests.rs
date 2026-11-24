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
    s.claim_container(&p, &a.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
    let events = s.container_events(&p, &b.id, 0).unwrap();
    assert_eq!(
        s.claim_container(&p, &b.id, &snapshot("b"), &"1".repeat(64), 204)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(s.job_state(&b.id).unwrap(), "queued");
    assert!(locks(&s, &b.id).is_empty());
    assert_eq!(s.container_events(&p, &b.id, 0).unwrap(), events);
    assert_eq!(locks(&s, &a.id).len(), 3);
    s.claim_container(&p, &c.id, &snapshot("c"), &"1".repeat(64), 204)
        .unwrap();
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
    assert!(
        s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
            .is_err()
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
    assert!(locks(&s, &job.id).is_empty());
    s.conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    let job = s
        .claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 204)
        .unwrap();
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
    let a = s
        .claim_container(&p, &a.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
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
    assert!(
        s.claim_container(&p, &b.id, &snapshot("b"), &"1".repeat(64), 206)
            .is_err()
    );
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
    s.claim_container(&p, &b.id, &snapshot("b"), &"1".repeat(64), 208)
        .unwrap();
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
    let job = s
        .claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
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
    s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
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
        s.claim_container(&p, &job.id, &snapshot(byte), &"1".repeat(64), 203)
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
    s.claim_container(&p, &queued.id, &snapshot("e"), &"1".repeat(64), 205)
        .unwrap();
    assert_eq!(locks(&s, &queued.id).len(), 1);
    check(&s.conn).unwrap();
}

#[test]
fn failed_v7_migration_preserves_v6_jobs_authority_and_generation() {
    let (dir, mut s, p) = setup();
    let job = queue(&mut s, &p, "job", "a");
    s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
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
        s.claim_container(&p, &active.id, &snapshot("a"), &"1".repeat(64), 203)
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
    s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
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

#[test]
fn v16_upgrade_adds_the_event_indexes_and_keeps_every_row() {
    let (dir, mut s, p) = setup();
    let path = dir.path().join("core.sqlite");
    let job = queue(&mut s, &p, "upgraded", "a");
    s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
        .unwrap();
    let rows = |conn: &Connection| -> (i64, i64, i64, i64) {
        conn.query_row("SELECT (SELECT count(*) FROM events),(SELECT count(*) FROM jobs),(SELECT count(*) FROM resource_locks),(SELECT audit_bytes FROM meta)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
    };
    let before = rows(&s.conn);
    crate::remove_v17_schema(&s.conn);
    s.conn.execute_batch("PRAGMA user_version=16").unwrap();
    // The reference check compares the index as it does triggers.
    assert!(check(&s.conn).is_err());
    drop(s);
    let s = Store::open(&path).unwrap();
    let version: u32 = s
        .conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    check(&s.conn).unwrap();
    // Startup recovered the claimed job: one more event, nothing lost.
    let after = rows(&s.conn);
    assert_eq!(after, (before.0 + 1, before.1, before.2, before.3 + 256));
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    drop(s);
    Store::open(&path).unwrap();
}

/// On a nearly full disk the v17 upgrade is refused before it builds its
/// indexes: nothing is written, the store stays at version 16, and
/// `upgrade_space` says how much space the upgrade needs.
#[test]
fn v16_upgrade_on_a_nearly_full_disk_refuses_before_building_indexes() {
    let (dir, mut s, p) = setup();
    let path = dir.path().join("core.sqlite");
    crate::scale::fill(&mut s, &p.id, 100_000, 300);
    crate::remove_v17_schema(&s.conn);
    s.conn.execute_batch("PRAGMA user_version=16").unwrap();
    drop(s);
    crate::OPEN_FREE_BYTES.set(Some(0));
    let needed = crate::upgrade_space(&path).unwrap().needed;
    crate::OPEN_FREE_BYTES.set(Some(needed - 1));
    let space = crate::upgrade_space(&path).unwrap();
    assert_eq!(space.from, 16);
    assert_eq!(space.available, needed - 1);
    assert_eq!(space.needed, needed);
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    let conn = Connection::open(&path).unwrap();
    let (version, indexes): (u32, i64) = conn
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version),(SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('events_job','events_kind','events_notification'))",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((version, indexes), (16, 0));
    drop(conn);
    // What the refused open measured is what core logs.
    assert_eq!(crate::upgrade_space(&path), Some(space));
    // With the space it asked for, the upgrade goes through.
    crate::OPEN_FREE_BYTES.set(Some(space.needed));
    assert_eq!(crate::upgrade_space(&path), None);
    Store::open(&path).unwrap();
    assert_eq!(crate::upgrade_space(&path), None);
}

/// A version-16 store as an older build left it, with `rows` events added:
/// of a settled job (64-character id) when `long`, else job-less. No event
/// indexes, and no free pages for the upgrade to reuse.
fn v16_store(long: bool, rows: i64) -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, mut s, p) = setup();
    let path = dir.path().join("core.sqlite");
    let job = queue(&mut s, &p, "settled", "a");
    s.cancel_container_job(&p, &job.id, 203).unwrap();
    assert_eq!(job.id.len(), 64);
    let (owner, kind) = if long {
        (
            Some(job.id),
            json(&EventKind::JobTransition {
                state: JobState::Canceled,
            })
            .unwrap(),
        )
    } else {
        (None, json(&EventKind::SessionCreated).unwrap())
    };
    s.write(|tx| {
        let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,?,?,300 FROM c", params![rows, p.id, owner, kind]).map_err(durable)?;
        crate::retention::account(tx, added as i64)
    })
    .unwrap();
    crate::remove_v17_schema(&s.conn);
    s.conn
        .execute_batch("PRAGMA user_version=16; VACUUM;")
        .unwrap();
    drop(s);
    (dir, path)
}
/// Bytes of the database, its WAL and its pre-upgrade snapshot.
fn on_disk(path: &std::path::Path) -> u64 {
    [
        path.to_path_buf(),
        path.with_extension("sqlite-wal"),
        crate::migrations::snapshot::snapshot_path(path, 16).unwrap(),
    ]
    .iter()
    .map(|file| std::fs::metadata(file).map_or(0, |m| m.len()))
    .sum()
}

/// The upgrade's estimate follows the data: admitted with exactly the space
/// it asks for, the snapshot and the index build (new database pages, and
/// the same pages in the WAL) leave the 8 MiB write reserve free, with long job ids and with
/// job-less rows; and it asks for no more than three times what it uses.
#[test]
fn v16_upgrade_space_estimate_covers_the_index_build_and_the_reserve() {
    for long in [true, false] {
        let (_dir, path) = v16_store(long, 65_536);
        crate::OPEN_FREE_BYTES.set(Some(0));
        let needed = crate::upgrade_space(&path).unwrap().needed;
        crate::OPEN_FREE_BYTES.set(Some(needed));
        assert_eq!(crate::upgrade_space(&path), None);
        let before = on_disk(&path);
        let s = Store::open(&path).unwrap();
        // The commit's checkpoint has copied the WAL into the database, and
        // the WAL keeps its size until the next write: the peak.
        let used = on_disk(&path) - before;
        drop(s);
        crate::OPEN_FREE_BYTES.set(None);
        assert!(
            used + MIN_FREE_BYTES <= needed,
            "long ids {long}: used {used}, allowed {}",
            needed - MIN_FREE_BYTES
        );
        assert!(
            needed - MIN_FREE_BYTES <= 3 * used,
            "long ids {long}: used {used}, allowed {}",
            needed - MIN_FREE_BYTES
        );
    }
}

/// The index build runs out of space although the check passed (here a page
/// limit stands in for a full disk): the upgrade rolls back, and
/// `upgrade_space` still reports a shortfall, so core logs the same
/// actionable message instead of a bare refusal.
#[test]
fn v16_upgrade_that_runs_out_of_space_still_says_how_much_it_needs() {
    let (_dir, path) = v16_store(true, 65_536);
    let pages: i64 = Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    crate::UPGRADE_MAX_PAGES.set(Some(pages + 8));
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    crate::UPGRADE_MAX_PAGES.set(None);
    let conn = Connection::open(&path).unwrap();
    let (version, indexes): (u32, i64) = conn
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version),(SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('events_job','events_kind','events_notification'))",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((version, indexes), (16, 0));
    drop(conn);
    let space = crate::upgrade_space(&path).unwrap();
    assert_eq!(space.from, 16);
    assert!(space.needed > space.available);
    // Reported once; the next start measures afresh and goes through.
    assert_eq!(crate::upgrade_space(&path), None);
    Store::open(&path).unwrap();
}

#[test]
fn missing_or_changed_event_indexes_refuse_startup() {
    for tamper in [
        "DROP INDEX events_job",
        "DROP INDEX events_job; CREATE INDEX events_job ON events(job)",
        "DROP INDEX events_kind",
        "DROP INDEX events_notification",
        "DROP INDEX events_notification; CREATE INDEX events_notification ON events(json_extract(kind,'$.notification'))",
    ] {
        let (dir, s, _) = setup();
        s.conn.execute_batch(tamper).unwrap();
        drop(s);
        assert_eq!(
            Store::open(&dir.path().join("core.sqlite"))
                .err()
                .unwrap()
                .0,
            ErrorCode::StateNotDurable
        );
    }
}

#[test]
fn event_reads_by_job_and_kind_use_the_indexes() {
    let (_dir, s, _) = setup();
    for (query, index) in [
        (
            "SELECT cursor,kind,created FROM events WHERE job=? AND principal=? AND cursor>? ORDER BY cursor LIMIT 128",
            "events_job",
        ),
        (
            "SELECT count(*),coalesce(min(created),-1),coalesce(sum(principal IS NOT ?),0) FROM events WHERE job=? AND kind=?",
            "events_job",
        ),
        ("SELECT min(created) FROM events WHERE job=?", "events_job"),
        (
            "SELECT coalesce(max(json_extract(kind,'$.revision')),0) FROM events WHERE principal=? AND job IS NULL AND cursor<? AND json_extract(kind,'$.kind')='automation_health_policy_changed'",
            "events_kind",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM events WHERE json_extract(kind,'$.kind')='automation_notification_created' GROUP BY json_extract(kind,'$.notification') HAVING count(*)<>1)",
            "events_kind",
        ),
    ] {
        let plan: Vec<String> = s
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap()
            .query_map(
                rusqlite::params_from_iter(std::iter::repeat_n("x", query.matches('?').count())),
                |r| r.get(3),
            )
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|step| step.contains(&format!("INDEX {index} "))),
            "{query}: {plan:?}"
        );
    }
}

/// The index sort stays in memory for a table at the audit cap and its
/// reserve even when every event has a job (the largest keys), and only a
/// table far past it spills to temporary files, which the estimate counts.
#[test]
fn the_v17_index_sort_stays_in_memory_up_to_the_audit_cap() {
    let at_cap = ((crate::AUDIT_LIMIT + crate::retention::AUDIT_RESERVE) / 256) as u64;
    let build = |rows: u64| crate::IndexBuild::of([(rows, rows * 64), (rows, rows * 30), (0, 0)]);
    let full = build(at_cap);
    assert!(full.in_memory(), "{full:?}");
    assert!(full.sort <= 32 * 1024 * 1024);
    let past = build(2 * at_cap);
    assert!(!past.in_memory());
    assert_eq!(
        past.needed(),
        MIN_FREE_BYTES + crate::UPGRADE_FIXED_BYTES + past.entries * 4 / 3 * 2 + past.sort * 2
    );
}

/// Even a store with no events writes the new indexes' root pages and their
/// WAL frames, so the estimate asks for more than the bare write reserve.
#[test]
fn the_v17_estimate_for_an_empty_store_exceeds_the_write_reserve() {
    let empty = crate::IndexBuild::of([(0, 0), (0, 0), (0, 0)]);
    assert!(empty.in_memory());
    assert_eq!(empty.needed(), MIN_FREE_BYTES + crate::UPGRADE_FIXED_BYTES);
}

/// Runs `corrupt` on `conn` with the named triggers dropped, then puts them
/// back exactly as they were: a store whose data no guarded write could
/// leave, with a schema the startup check accepts.
fn unguarded(conn: &Connection, triggers: &[&str], corrupt: impl FnOnce(&Connection)) {
    let guards: Vec<String> = triggers
        .iter()
        .map(|name| {
            conn.query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        })
        .collect();
    for name in triggers {
        conn.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    }
    corrupt(conn);
    for guard in guards {
        conn.execute_batch(&guard).unwrap();
    }
}

/// `copies` settled copies of job `template` (its row with new ids and
/// idempotency keys, queued as every job is, which gives each its primary
/// resource, then canceled), numbered from `from`.
fn settled_copies(conn: &Connection, template: &str, from: i64, copies: i64) {
    conn.execute(
        "WITH RECURSIVE n(i) AS (SELECT ?2 UNION ALL SELECT i+1 FROM n WHERE i<?2+?3-1)
         INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
         SELECT printf('%064d',i),principal,'copy-'||i,intent,digest,resource,'queued',generation,revision,deadline
         FROM jobs, n WHERE id=?1",
        params![template, from, copies],
    )
    .unwrap();
    conn.execute(
        "UPDATE jobs SET state='canceled' WHERE state='queued' AND idempotency LIKE 'copy-%'",
        [],
    )
    .unwrap();
}

/// What the open refused, and that it changed nothing: the generation is
/// the one before it.
pub(crate) fn refused_unchanged(dir: &tempfile::TempDir, generation: i64, case: &str) {
    let path = dir.path().join("core.sqlite");
    assert_eq!(
        Store::open(&path).err().map(|e| e.0),
        Some(ErrorCode::StateNotDurable),
        "{case}"
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        generation,
        "{case}"
    );
}

/// Startup holds settled jobs to their resource set's invariant, as it holds
/// queued ones (`missing_or_extra_locks_refuse_startup_without_repair_or_recovery_events`),
/// and refuses a lock held by a settled job: since the walk of every job
/// moved out of the lock invariants (`JOB_WALK`), and those start from the
/// jobs that hold locks, the locks and the resolutions, a settled job must
/// still be judged by each.
#[test]
fn settled_jobs_keep_their_resource_sets_and_hold_no_lock_at_startup() {
    for fault in ["missing-primary", "oversized-set", "settled-lock"] {
        let (dir, mut s, p) = setup();
        let settled = queue(&mut s, &p, "settled", "a");
        queue(&mut s, &p, "queued", "b");
        s.conn
            .execute("UPDATE jobs SET state='canceled' WHERE id=?", [&settled.id])
            .unwrap();
        settled_copies(&s.conn, &settled.id, 1, 200);
        check(&s.conn).unwrap();
        let generation = s.generation;
        let resource = settled.plan.expected.resource.clone();
        unguarded(
            &s.conn,
            &[
                "job_resources_delete",
                "job_resources_insert",
                "resource_locks_insert",
            ],
            |conn| match fault {
                "missing-primary" => {
                    conn.execute("DELETE FROM job_resources WHERE job=?", [&settled.id])
                        .unwrap();
                }
                "oversized-set" => {
                    for n in 0..128 {
                        conn.execute(
                            "INSERT INTO job_resources VALUES(?,?)",
                            params![settled.id, format!("storage:extra:{n}")],
                        )
                        .unwrap();
                    }
                }
                _ => {
                    conn.execute(
                        "INSERT INTO resource_locks VALUES(?,?)",
                        params![resource, settled.id],
                    )
                    .unwrap();
                }
            },
        );
        assert!(check(&s.conn).is_err(), "{fault}");
        drop(s);
        refused_unchanged(&dir, generation, fault);
    }
}

/// A store with 300 settled copies of a container job and, between them, a
/// settled job of `operation` with id `FORGED` that no queue path writes
/// (an intent with no plan), with resource `system`, or with the copies'
/// resource when `shared`. Returns the store's directory, its generation
/// and the copies' resource.
fn forged_among_settled(operation: &str, shared: Option<()>) -> (tempfile::TempDir, i64, String) {
    let (dir, mut s, p) = setup();
    let template = queue(&mut s, &p, "template", "a");
    s.conn
        .execute(
            "UPDATE jobs SET state='canceled' WHERE id=?",
            [&template.id],
        )
        .unwrap();
    settled_copies(&s.conn, &template.id, 1, 150);
    let resource = template.plan.expected.resource.clone();
    s.conn
        .execute(
            "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
             VALUES(?1,?2,'forged',?3,?4,?5,'queued',1,1,1000)",
            params![
                FORGED,
                p.id,
                serde_json::json!({ "operation": operation }).to_string(),
                "0".repeat(64),
                if shared.is_some() { resource.as_str() } else { "system" }
            ],
        )
        .unwrap();
    s.conn
        .execute(
            "UPDATE jobs SET state='canceled' WHERE idempotency='forged'",
            [],
        )
        .unwrap();
    settled_copies(&s.conn, &template.id, 151, 150);
    let generation = s.generation;
    drop(s);
    (dir, generation, resource)
}
const FORGED: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

/// Startup's walk lists every job of an operation an owner checks, wherever
/// it is among many settled jobs of other operations, and that owner's check
/// then refuses one it cannot validate (here an intent with no plan, which
/// no queue path writes). Before 24 November each owner walked `jobs` to
/// find them; now only the walk does, so a job it missed would go unchecked.
/// The same row under an operation no owner checks opens: the refusal is
/// the owner's.
#[test]
fn startup_checks_every_job_of_a_checked_operation_among_settled_jobs() {
    for operation in CHECKED_OPERATIONS
        .iter()
        .copied()
        .chain(["no_such_operation"])
    {
        let (dir, generation, _) = forged_among_settled(operation, None);
        if operation == "no_such_operation" {
            Store::open(&dir.path().join("core.sqlite")).unwrap();
        } else {
            refused_unchanged(&dir, generation, operation);
        }
    }
}

/// The owners' checks read the jobs the walk listed through the primary
/// key, and the lock invariants start from the jobs that hold locks: none
/// of them walks `jobs`, so the walk is the only statement of the open
/// whose reads grow with every retained job. The walk itself is one scan of
/// `jobs` with each job's resource set looked up by its key.
#[test]
fn only_the_walk_reads_every_job_at_startup() {
    let (_dir, s, _p) = setup();
    let plan = |sql: &str| -> Vec<String> {
        s.conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(["[]"], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    let walks_jobs = |steps: &[String]| {
        steps
            .iter()
            .any(|step| step == "SCAN j" || step == "SCAN jobs" || step.starts_with("SCAN j USING"))
    };
    for (operation, sql) in listed_job_queries() {
        assert!(CHECKED_OPERATIONS.contains(&operation));
        assert!(sql.contains(&format!("='{operation}'")), "{sql}");
        let steps = plan(sql);
        assert!(!walks_jobs(&steps), "{steps:?}: {sql}");
        assert!(
            steps
                .iter()
                .any(|step| step == "SEARCH j USING INDEX sqlite_autoindex_jobs_1 (id=?)"),
            "{steps:?}: {sql}"
        );
        assert!(
            !steps
                .iter()
                .any(|step| step == "SCAN r" && sql.contains("job_resources r")),
            "{steps:?}: {sql}"
        );
    }
    // The accounting reads by key exactly as the owners do, so a job it
    // finds at the walk's row is one every owner reads there.
    let steps = plan(LISTED_BY_KEY);
    assert!(!walks_jobs(&steps), "{steps:?}");
    assert!(
        steps
            .iter()
            .any(|step| step == "SEARCH j USING INDEX sqlite_autoindex_jobs_1 (id=?)"),
        "{steps:?}"
    );
    let steps = s
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {LOCK_INVARIANTS}"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!walks_jobs(&steps), "{steps:?}");
    let steps = s
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {JOB_WALK}"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        steps
            .iter()
            .filter(|step| step.starts_with("SCAN"))
            .collect::<Vec<_>>(),
        ["SCAN j"],
        "{steps:?}"
    );
    // It returns every row: the accounting needs each one.
    assert!(!JOB_WALK.contains("WHERE json_extract"), "{JOB_WALK}");
}

/// Rewrites every occurrence of `from` with `to` (as long) on the pages of
/// the b-tree `btree` in the closed store at `path`, after folding its WAL
/// in, as a lost or misdirected page write can leave a b-tree: structurally
/// sound, so `quick_check` passes, but no longer agreeing with the others
/// (`quick_check` counts an index's entries; it does not compare them with
/// the rows: `accounting`). Returns how many it rewrote.
pub(crate) fn rewrite_in(path: &std::path::Path, btree: &str, from: &str, to: &str) -> usize {
    assert_eq!(from.len(), to.len());
    let (size, pages) = pages_of(path, btree);
    let mut bytes = std::fs::read(path).unwrap();
    let mut rewritten = 0;
    for page in pages {
        let page = &mut bytes[(page - 1) * size..page * size];
        let mut at = 0;
        while let Some(found) = page[at..]
            .windows(from.len())
            .position(|w| w == from.as_bytes())
        {
            let start = at + found;
            page[start..start + to.len()].copy_from_slice(to.as_bytes());
            rewritten += 1;
            at = start + to.len();
        }
    }
    std::fs::write(path, bytes).unwrap();
    rewritten
}

/// The page size and the pages of the b-tree `btree` of the closed store at
/// `path`, after folding its WAL in (so the file is the whole store).
fn pages_of(path: &std::path::Path, btree: &str) -> (usize, Vec<usize>) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let size = conn
        .query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
        .unwrap() as usize;
    let pages: Vec<usize> = conn
        .prepare("SELECT pageno FROM dbstat WHERE name=?")
        .unwrap()
        .query_map([btree], |r| Ok(r.get::<_, i64>(0)? as usize))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    drop(conn);
    assert!(!std::fs::metadata(path.with_extension("sqlite-wal")).is_ok_and(|m| m.len() > 0));
    (size, pages)
}

/// A SQLite varint at the start of `bytes`: its value and length.
fn varint(bytes: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    for (n, byte) in bytes.iter().take(9).enumerate() {
        if n == 8 {
            return ((value << 8) | u64::from(*byte), 9);
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return (value, n + 1);
        }
    }
    (value, bytes.len())
}

/// On the leaf page of the index `btree` that holds the last entry whose
/// record contains `marker`, moves that entry's cell pointer to the front of
/// the page's pointer array: every entry keeps its bytes and the page its
/// count, only the order of the entries changes (`quick_check` checks key
/// order in table b-trees only). Panics unless it moved one.
pub(crate) fn move_entry_first(path: &std::path::Path, btree: &str, marker: &[u8]) {
    let (size, pages) = pages_of(path, btree);
    let mut bytes = std::fs::read(path).unwrap();
    let mut moved = false;
    for page in pages {
        let page = &mut bytes[(page - 1) * size..page * size];
        // An index leaf: its header is 8 bytes, then the pointer array.
        if page[0] != 0x0a {
            continue;
        }
        let cells = usize::from(u16::from_be_bytes([page[3], page[4]]));
        let pointer = |page: &[u8], n: usize| {
            usize::from(u16::from_be_bytes([page[8 + 2 * n], page[9 + 2 * n]]))
        };
        let holding = (0..cells).rev().find(|n| {
            let at = pointer(page, *n);
            let (payload, length) = varint(&page[at..]);
            let end = (at + length + payload as usize).min(size);
            page[at..end].windows(marker.len()).any(|w| w == marker)
        });
        let Some(n) = holding else { continue };
        assert!(n > 0, "the entry is first on its page already");
        let pointers: Vec<[u8; 2]> = (0..cells)
            .map(|i| [page[8 + 2 * i], page[9 + 2 * i]])
            .collect();
        let mut order = vec![pointers[n]];
        order.extend(
            pointers
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != n)
                .map(|(_, p)| *p),
        );
        for (i, p) in order.iter().enumerate() {
            page[8 + 2 * i..10 + 2 * i].copy_from_slice(p);
        }
        moved = true;
        break;
    }
    assert!(moved, "no leaf of {btree} holds the entry");
    std::fs::write(path, bytes).unwrap();
}

/// What `quick_check` and the foreign-key check make of a store: both pass
/// on the stores the next tests build.
pub(crate) fn whole_file_checks_pass(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let violations: Vec<(String, i64, String)> = conn
        .prepare("SELECT \"table\",rowid,parent FROM pragma_foreign_key_check")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(violations.is_empty(), "{violations:?}");
}

/// A job row the primary key's index does not find, as a partial write can
/// leave `jobs` (here the row's id rewritten on its table page, and its
/// resource's index entry with it, so the resource-set and foreign-key
/// checks still pass): `quick_check` passes, the walk lists the row from
/// the table, and the owners, reading by key, would find nothing to check.
/// The open refuses: the key's index disagrees with the table
/// (`accounting`), and the listed job is not found by key (`LISTED_BY_KEY`),
/// as the eight walks this replaced refused it for its missing plan. Until
/// the accounting, the same rewrite on a job no owner checks opened.
#[test]
fn a_listed_job_the_key_does_not_find_refuses_startup() {
    let moved = format!("{}e", &FORGED[..63]);
    for operation in CHECKED_OPERATIONS
        .iter()
        .copied()
        .chain(["no_such_operation"])
    {
        let (dir, generation, _) = forged_among_settled(operation, None);
        let path = dir.path().join("core.sqlite");
        // A page may also hold stale copies of a cell in its free space.
        assert!(
            rewrite_in(&path, "jobs", FORGED, &moved) >= 1,
            "{operation}"
        );
        assert!(
            rewrite_in(&path, "sqlite_autoindex_job_resources_1", FORGED, &moved) >= 1,
            "{operation}"
        );
        whole_file_checks_pass(&path);
        // The disagreement built: the table holds the row, the key finds none.
        let conn = Connection::open(&path).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [&moved], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM jobs NOT INDEXED WHERE id=?"), 1);
        assert_eq!(
            count("SELECT count(*) FROM jobs INDEXED BY sqlite_autoindex_jobs_1 WHERE id=?"),
            0
        );
        drop(conn);
        // Since the accounting of the key's index (`accounting`), the same
        // disagreement on a job no owner checks is refused too.
        refused_unchanged(&dir, generation, operation);
    }
}

/// An index entry that leads to another job's row: the forged job's id and
/// a settled container job's id swapped on their table pages (both with
/// the same resource, so the resource sets still agree), so the key finds
/// each job at the other's row. `quick_check` and the foreign-key check
/// pass; the walk lists the forged row under the container job's id, and by
/// key that id reads the container row, which no owner checks as one of
/// theirs. The open refuses: the key does not lead to the row the walk
/// read. Until the accounting, a job no owner checks, swapped the same way,
/// opened; now it is refused as well.
#[test]
fn a_listed_job_the_key_finds_at_another_row_refuses_startup() {
    // Ids no other bytes can run into: a zero-padded copy id can also match
    // where a shorter one is followed by a principal id starting with zeros.
    let copy = "c".repeat(64);
    let swap = "g".repeat(64);
    for operation in CHECKED_OPERATIONS
        .iter()
        .copied()
        .chain(["no_such_operation"])
    {
        let (dir, generation, _) = forged_among_settled(operation, Some(()));
        let path = dir.path().join("core.sqlite");
        // One more settled copy of the container job, under `copy`.
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
             SELECT ?1,principal,'copy-c',intent,digest,resource,'queued',generation,revision,deadline FROM jobs WHERE idempotency='copy-1'",
            [&copy],
        )
        .unwrap();
        conn.execute("UPDATE jobs SET state='canceled' WHERE id=?", [&copy])
            .unwrap();
        drop(conn);
        assert!(rewrite_in(&path, "jobs", FORGED, &swap) >= 1, "{operation}");
        assert!(rewrite_in(&path, "jobs", &copy, FORGED) >= 1, "{operation}");
        assert!(rewrite_in(&path, "jobs", &swap, &copy) >= 1, "{operation}");
        whole_file_checks_pass(&path);
        // The disagreement built: the table files the forged intent under the
        // copy's id, and the key leads from that id to a container job.
        let conn = Connection::open(&path).unwrap();
        let operation_of =
            |sql: &str| -> String { conn.query_row(sql, [&copy], |r| r.get(0)).unwrap() };
        assert_eq!(
            operation_of(
                "SELECT json_extract(intent,'$.operation') FROM jobs NOT INDEXED WHERE id=?"
            ),
            operation
        );
        assert_eq!(
            operation_of(
                "SELECT json_extract(intent,'$.operation') FROM jobs INDEXED BY sqlite_autoindex_jobs_1 WHERE id=?"
            ),
            "container_restart"
        );
        drop(conn);
        // Since the accounting of the key's index (`accounting`), the same
        // disagreement on a job no owner checks is refused too.
        refused_unchanged(&dir, generation, operation);
    }
}

/// One more container job, a copy of `copy-1` under `id` and the
/// idempotency key `key` (which must not contain `id`, so that rewriting
/// one leaves the other), with `copy-1`'s deadline or `deadline`, in the
/// closed store at `path`, in `state` (queued, then moved there).
fn another_copy(path: &std::path::Path, id: &str, key: &str, state: &str, deadline: Option<i64>) {
    assert!(!key.contains(id));
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
         SELECT ?1,principal,?2,intent,digest,resource,'queued',generation,revision,coalesce(?3,deadline) FROM jobs WHERE idempotency='copy-1'",
        params![id, key, deadline],
    )
    .unwrap();
    conn.execute("UPDATE jobs SET state=?2 WHERE id=?1", [id, state])
        .unwrap();
}

/// A primary key the table holds twice: a settled job's id rewritten on its
/// table page to another job's, with the key's index still filing it under
/// its own (`quick_check` counts entries, it does not compare them), or
/// rewritten there too, and in its resource set, so that the index holds
/// the key twice (`quick_check` does not check UNIQUE). Either way the
/// resource-set and foreign-key checks pass. A duplicated primary key is
/// corruption whatever the jobs' operations: before, a duplicate of an
/// observation job's id let the occurrence check pass where the eight walks
/// refused (review, 24 November). The open refuses all three: the table-only
/// duplicate by the tally of the key's index against the table, the
/// duplicate in the index too by the keys read in order (a key not after
/// the one before), and the same with the two entries apart (one moved to
/// the front of its page) likewise.
#[test]
fn a_primary_key_the_table_holds_twice_refuses_startup() {
    // Neighbours in key order that no other bytes run into (an id ending in
    // the same 63 characters, followed by a principal id, could).
    let first = "d".repeat(63) + "1";
    for (in_index, apart) in [(false, false), (true, false), (true, true)] {
        let (dir, generation, _) = forged_among_settled("no_such_operation", None);
        let path = dir.path().join("core.sqlite");
        // The second copy, under `first`'s neighbour in key order, so that
        // renaming `twice` to `first` keeps every index in order.
        let twice = "d".repeat(63) + "2";
        another_copy(&path, &first, "extra-1", "canceled", None);
        another_copy(&path, &twice, "extra-2", "canceled", None);
        let mut btrees = vec!["jobs"];
        if in_index {
            btrees.extend([
                "sqlite_autoindex_jobs_1",
                "job_resources",
                "sqlite_autoindex_job_resources_1",
            ]);
        }
        for btree in btrees {
            assert!(rewrite_in(&path, btree, &twice, &first) >= 1, "{btree}");
        }
        if apart {
            // The second entry of the key moved away from the first, to the
            // front of its page: no longer neighbours, and out of order. That
            // also hides the page's first entry from a search, so the
            // foreign-key check (whose parent lookups search this index) may
            // refuse it as well: only `quick_check` is asserted to pass.
            move_entry_first(&path, "sqlite_autoindex_jobs_1", first.as_bytes());
            let conn = Connection::open(&path).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        } else {
            whole_file_checks_pass(&path);
        }
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM jobs NOT INDEXED WHERE id=?",
                [&first],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        drop(conn);
        refused_unchanged(
            &dir,
            generation,
            &format!("in the index too: {in_index}, apart: {apart}"),
        );
    }
}

/// A settled job the table now says is `verifying`, while `jobs_state` (and
/// the partial `jobs_resource_lock`) still file it as `succeeded`: what a
/// lost write of an index page leaves for a job interrupted in flight.
/// `quick_check` and the foreign-key check pass, the lock invariants and
/// startup recovery (which find such jobs through `jobs_state`) would see
/// no interrupted job, and the job would keep its unlocked resources, open
/// to another dispatch. The open refuses, through two accountings at once
/// (`jobs_state`'s and the partial index's); each alone is isolated below.
#[test]
fn a_job_state_its_index_does_not_hold_refuses_startup() {
    let (dir, generation, _) = forged_among_settled("no_such_operation", None);
    let path = dir.path().join("core.sqlite");
    another_copy(&path, &"c".repeat(64), "extra-c", "succeeded", None);
    assert!(rewrite_in(&path, "jobs", "succeeded", "verifying") >= 1);
    whole_file_checks_pass(&path);
    let conn = Connection::open(&path).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM jobs NOT INDEXED WHERE state='verifying'"),
        1
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM jobs INDEXED BY jobs_state WHERE state IN ('running','verifying')"
        ),
        0
    );
    drop(conn);
    refused_unchanged(
        &dir,
        generation,
        "verifying in the table, succeeded in jobs_state",
    );
}

/// A running job's resource set and locks, with a resource renamed on a
/// table page only: in `job_resources` (the index still files the old
/// name, which its lock and the lock invariants find), or two of its locks'
/// resources swapped in `resource_locks` (each found through the index at
/// the other's row). `quick_check`, the foreign-key check and the lock
/// invariants pass both; the next dispatch would lock what the index says.
/// The open refuses: the unsettled jobs' resource sets are held to their
/// index, and the lock tables pass `integrity_check` or the store is
/// refused.
#[test]
fn a_running_jobs_resources_or_locks_their_index_disagrees_with_refuse_startup() {
    for case in ["job_resources", "resource_locks"] {
        let (dir, mut s, p) = setup();
        let job = queue(&mut s, &p, "running", "a");
        dependencies(&mut s, &job.id, &["storage:uuid:aaaa", "storage:uuid:bbbb"]);
        s.claim_container(&p, &job.id, &snapshot("a"), &"1".repeat(64), 203)
            .unwrap();
        assert_eq!(locks(&s, &job.id).len(), 3);
        let generation = s.generation;
        drop(s);
        let path = dir.path().join("core.sqlite");
        if case == "job_resources" {
            assert!(rewrite_in(&path, case, "storage:uuid:bbbb", "storage:uuid:cccc") >= 1);
        } else {
            assert!(rewrite_in(&path, case, "storage:uuid:aaaa", "storage:uuid:xxxx") >= 1);
            assert!(rewrite_in(&path, case, "storage:uuid:bbbb", "storage:uuid:aaaa") >= 1);
            assert!(rewrite_in(&path, case, "storage:uuid:xxxx", "storage:uuid:bbbb") >= 1);
        }
        whole_file_checks_pass(&path);
        let conn = Connection::open(&path).unwrap();
        assert!(
            !conn
                .query_row(LOCK_INVARIANTS, [], |r| r.get::<_, bool>(0))
                .unwrap()
        );
        drop(conn);
        refused_unchanged(&dir, generation, case);
    }
}

/// The backup check's job statement on the jobs the walk lists: a backup
/// job in a state other than queued or canceled, or with an oversized
/// intent, is invalid by it alone (with the list), and passes with no list,
/// so a check given the wrong list would show here although the unlinked
/// check would still refuse the job. A resource of a bad length is invalid
/// too, but no store holding one passes `quick_check` first (the table's
/// CHECK).
#[test]
fn the_backup_job_statement_judges_the_listed_jobs() {
    for fault in ["state", "size", "resource"] {
        let (_dir, s, p) = setup();
        let id = "b".repeat(64);
        let intent = match fault {
            "size" => serde_json::json!({"operation": "backup_prepare_private", "pad": "x".repeat(17_000)}),
            _ => serde_json::json!({"operation": "backup_prepare_private"}),
        }
        .to_string();
        s.conn
            .execute(
                "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
                 VALUES(?1,?2,'backup',?3,?4,'storage:configuration','queued',1,1,1000)",
                params![id, p.id, intent, "0".repeat(64)],
            )
            .unwrap();
        unguarded(
            &s.conn,
            &["backup_preparation_update_state", "job_resources_insert"],
            |conn| match fault {
                "state" => {
                    conn.execute("UPDATE jobs SET state='succeeded' WHERE id=?", [&id])
                        .unwrap();
                }
                "resource" => {
                    conn.execute_batch("PRAGMA ignore_check_constraints=ON")
                        .unwrap();
                    conn.execute("INSERT INTO job_resources VALUES(?,'')", [&id])
                        .unwrap();
                    conn.execute_batch("PRAGMA ignore_check_constraints=OFF")
                        .unwrap();
                }
                _ => {}
            },
        );
        let invalid = |listed: &str| -> bool {
            s.conn
                .query_row(crate::backup_preparation::BACKUP_JOBS, [listed], |r| {
                    r.get(0)
                })
                .unwrap()
        };
        assert!(invalid(&serde_json::json!([id]).to_string()), "{fault}");
        assert!(!invalid("[]"), "{fault}");
        if fault == "resource" {
            assert_ne!(
                s.conn
                    .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        }
    }
}

/// A settled observation job without its result (which no guarded write
/// can leave: observation jobs are only ever queued or canceled) is refused
/// through the observation results statement on the listed jobs, and only
/// with the list.
#[test]
fn a_settled_observation_job_without_a_result_is_refused_on_the_list() {
    let (_dir, s, p) = setup();
    let id = "a".repeat(64);
    s.conn
        .execute(
            "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
             VALUES(?1,?2,'observe',?3,?4,'system','queued',1,1,1000)",
            params![
                id,
                p.id,
                serde_json::json!({"operation": "automation_observe_health"}).to_string(),
                "0".repeat(64)
            ],
        )
        .unwrap();
    unguarded(&s.conn, &["automation_job_update_state"], |conn| {
        conn.execute("UPDATE jobs SET state='succeeded' WHERE id=?", [&id])
            .unwrap();
    });
    let invalid = |listed: &str| -> bool {
        s.conn
            .query_row(crate::automation::OBSERVATION_RESULTS, [listed], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert!(invalid(&serde_json::json!([id]).to_string()));
    assert!(!invalid("[]"));
}

/// Every statement of the accounting reads only what it accounts for: the
/// index statements their index alone (no table row), and the table
/// statement the table (no index).
#[test]
fn the_accounting_reads_each_side_alone() {
    let (_dir, s, _p) = setup();
    let plan = |sql: &str| -> Vec<String> {
        let mut statement = s
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let params: &[&dyn rusqlite::ToSql] = if sql.contains("?1") { &[&"[]"] } else { &[] };
        statement
            .query_map(params, |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    for (sql, expected) in [
        (
            accounting::ID_ENTRIES,
            "SCAN jobs USING COVERING INDEX sqlite_autoindex_jobs_1",
        ),
        (
            accounting::IDEMPOTENCY_ENTRIES,
            "SCAN jobs USING COVERING INDEX sqlite_autoindex_jobs_2",
        ),
        (
            accounting::STATE_ENTRIES,
            "SCAN jobs USING COVERING INDEX jobs_state",
        ),
        (accounting::UNSETTLED_RESOURCES, "SCAN r"),
        (
            accounting::UNSETTLED_RESOURCE_ENTRIES,
            "SEARCH job_resources USING COVERING INDEX sqlite_autoindex_job_resources_1 (job=?)",
        ),
    ] {
        let steps = plan(sql);
        assert!(
            steps.iter().any(|step| step == expected),
            "{steps:?}: {sql}"
        );
    }
    // The indexes read whole are read as stored: no sorter, which would put
    // entries out of place back in order and hide them.
    for sql in [
        accounting::ID_ENTRIES,
        accounting::IDEMPOTENCY_ENTRIES,
        accounting::STATE_ENTRIES,
        accounting::LOCK_ENTRIES,
    ] {
        assert!(
            !plan(sql).iter().any(|step| step.contains("TEMP B-TREE")),
            "{sql}"
        );
        let opcodes: Vec<String> = s
            .conn
            .prepare(&format!("EXPLAIN {sql}"))
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            !opcodes.iter().any(|op| op.starts_with("Sorter")),
            "{sql}: {opcodes:?}"
        );
        // And forwards (`Rewind`/`Next`, never `Last`/`Prev`): the order
        // check expects ascending entries, so a backward scan (which
        // `PRAGMA reverse_unordered_selects`, set nowhere and not enabled in
        // the bundled build, would give these reads, having no ORDER BY)
        // would refuse every store, on a device nobody watches.
        assert!(
            !opcodes.iter().any(|op| op == "Last" || op == "Prev"),
            "{sql}: {opcodes:?}"
        );
        assert!(
            opcodes.iter().any(|op| op == "Rewind"),
            "{sql}: {opcodes:?}"
        );
    }
    // The partial index: the table's cursor is opened for a deferred seek
    // that no column needs, so the seek is never made.
    let root: i64 = s
        .conn
        .query_row(
            "SELECT rootpage FROM sqlite_schema WHERE type='table' AND name='jobs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let program: Vec<(String, i64, i64)> = s
        .conn
        .prepare(&format!("EXPLAIN {}", accounting::LOCK_ENTRIES))
        .unwrap()
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let table: Vec<i64> = program
        .iter()
        .filter(|(op, _, page)| op == "OpenRead" && *page == root)
        .map(|(_, cursor, _)| *cursor)
        .collect();
    assert!(
        !program
            .iter()
            .any(|(op, cursor, _)| matches!(op.as_str(), "Column" | "Rowid")
                && table.contains(cursor)),
        "{program:?}"
    );
    assert!(
        plan(accounting::LOCK_ENTRIES)
            .iter()
            .any(|step| step == "SCAN jobs USING INDEX jobs_resource_lock")
    );
}

/// The closed store at `dir` with jobs queued for the containers `bytes`
/// and claimed (`running`, holding their locks), opened and closed again.
/// Returns them and the store's generation after.
fn running_jobs(
    dir: &tempfile::TempDir,
    bytes: &[&str],
) -> (Vec<limeos_domain::ContainerJob>, i64) {
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let jobs: Vec<_> = bytes
        .iter()
        .map(|b| {
            let job = queue(&mut s, &p, &format!("running-{b}"), b);
            s.claim_container(&p, &job.id, &snapshot(b), &"1".repeat(64), 203)
                .unwrap();
            job
        })
        .collect();
    let generation = s.generation;
    drop(s);
    (jobs, generation)
}

/// Removes `job`'s locks, as the release trigger would, without moving its
/// state: a running job holding no lock (which the lock invariants refuse,
/// so long as they find the job).
fn without_locks(dir: &tempfile::TempDir, job: &str) {
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    unguarded(&conn, &["resource_locks_delete"], |conn| {
        conn.execute("DELETE FROM resource_locks WHERE job=?", [job])
            .unwrap();
    });
}

/// `jobs_state` alone: a job the table says is `queued` that `jobs_state`
/// still files as `failed` (neither a lock-holding state, so the partial
/// index has no say), and a deadline `jobs_state` holds differently. The
/// dispatcher finds queued jobs through `jobs_state`. Refused by its
/// accounting alone.
#[test]
fn a_state_or_deadline_jobs_state_does_not_hold_refuses_startup() {
    let resource = format!("container:{}", "a".repeat(64));
    for case in ["state", "deadline"] {
        let (dir, generation, _) = forged_among_settled("no_such_operation", None);
        let path = dir.path().join("core.sqlite");
        if case == "state" {
            another_copy(&path, &"c".repeat(64), "extra-c", "failed", None);
            let (from, to) = (format!("{resource}failed"), format!("{resource}queued"));
            assert!(rewrite_in(&path, "jobs", &from, &to) >= 1);
        } else {
            // "ABCDEFGH", big-endian: the largest deadline, so rewritten one
            // up it stays in order.
            another_copy(
                &path,
                &"c".repeat(64),
                "extra-c",
                "canceled",
                Some(0x4142434445464748),
            );
            assert!(rewrite_in(&path, "jobs_state", "ABCDEFGH", "ABCDEFGI") >= 1);
        }
        whole_file_checks_pass(&path);
        refused_unchanged(&dir, generation, case);
    }
}

/// `jobs_state` out of order: a running job whose locks are gone, its
/// `jobs_state` entry moved to the front of its page. The entries and their
/// count are unchanged, so `quick_check` and every tally pass, but a search
/// for `state=?` binary-searches past it: the lock invariants and startup
/// recovery find no running job, and it would stay `running`, holding no
/// lock, never marked for intervention (the reviewer's store, 24 November).
/// Refused by the order `jobs_state` is read in.
#[test]
fn a_jobs_state_entry_out_of_order_refuses_startup() {
    let (dir, _, _) = forged_among_settled("no_such_operation", None);
    let (jobs, generation) = running_jobs(&dir, &["b"]);
    without_locks(&dir, &jobs[0].id);
    let path = dir.path().join("core.sqlite");
    move_entry_first(&path, "jobs_state", b"running");
    whole_file_checks_pass(&path);
    let conn = Connection::open(&path).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM jobs NOT INDEXED WHERE state='running'"),
        1
    );
    assert_eq!(
        count("SELECT count(*) FROM jobs WHERE state IN ('running','verifying')"),
        0
    );
    assert!(
        !conn
            .query_row(LOCK_INVARIANTS, [], |r| r.get::<_, bool>(0))
            .unwrap()
    );
    drop(conn);
    refused_unchanged(&dir, generation, "running entry out of order");
}

/// The idempotency key's index alone: one entry's key rewritten (a retried
/// request would no longer find its job), or moved out of order (a search
/// for it misses it). Refused by that index's tally, and by the order it is
/// read in.
#[test]
fn an_idempotency_entry_that_disagrees_or_is_out_of_order_refuses_startup() {
    for case in ["key", "order"] {
        let (dir, generation, _) = forged_among_settled("no_such_operation", None);
        let path = dir.path().join("core.sqlite");
        another_copy(&path, &"c".repeat(64), "zzzz-1", "canceled", None);
        if case == "key" {
            assert!(rewrite_in(&path, "sqlite_autoindex_jobs_2", "zzzz-1", "zzzz-2") >= 1);
        } else {
            move_entry_first(&path, "sqlite_autoindex_jobs_2", b"zzzz-1");
        }
        whole_file_checks_pass(&path);
        refused_unchanged(&dir, generation, case);
    }
}

/// The partial `jobs_resource_lock` alone: an entry for a job that holds no
/// lock (a running job whose locks are gone and whose state the table and
/// `jobs_state` both now say is `planned`), or one lock-holding job's entry
/// filed under another's primary resource (so that resource twice, and the
/// other's missing). Refused by that index's accounting: the first by its
/// tally, the second by its tally and the order of its keys (each alone
/// would refuse it). A primary resource held twice in the table as well
/// never reaches it: the resource-set check, the lock invariants or the lock
/// table's `integrity_check` refuses that first.
#[test]
fn a_lock_index_entry_that_disagrees_refuses_startup() {
    let (b, c) = (
        format!("container:{}", "b".repeat(64)),
        format!("container:{}", "c".repeat(64)),
    );
    for case in ["no lock", "another resource"] {
        let (dir, _, _) = forged_among_settled("no_such_operation", None);
        let path = dir.path().join("core.sqlite");
        let generation = if case == "no lock" {
            let (jobs, generation) = running_jobs(&dir, &["b"]);
            without_locks(&dir, &jobs[0].id);
            let (from, to) = (format!("{b}running"), format!("{b}planned"));
            assert!(rewrite_in(&path, "jobs", &from, &to) >= 1);
            // The only `running` entry: `planned` sorts after every
            // `canceled` one too, so the index stays in order.
            assert!(rewrite_in(&path, "jobs_state", "running", "planned") >= 1);
            generation
        } else {
            let (_, generation) = running_jobs(&dir, &["b", "c"]);
            assert!(rewrite_in(&path, "jobs_resource_lock", &c, &b) >= 1);
            generation
        };
        whole_file_checks_pass(&path);
        refused_unchanged(&dir, generation, case);
    }
}

/// `job_resolutions` against its index: two running jobs' open resolution
/// requests with their jobs swapped on the table page (each found through
/// the index at the other's row). The lock invariants' resolution parts and
/// the foreign-key check pass. Refused by `integrity_check(job_resolutions)`.
#[test]
fn resolutions_their_index_disagrees_with_refuse_startup() {
    let (dir, _, _) = forged_among_settled("no_such_operation", None);
    let (jobs, generation) = running_jobs(&dir, &["b", "c"]);
    let path = dir.path().join("core.sqlite");
    let conn = Connection::open(&path).unwrap();
    let user: String = conn
        .query_row("SELECT id FROM users", [], |r| r.get(0))
        .unwrap();
    for job in &jobs {
        conn.execute(
            "INSERT INTO job_resolutions(job,resolved_by,declared,reason,evidence,evidence_digest,requested) VALUES(?,?,'not_done','r','e','d',1)",
            params![job.id, user],
        )
        .unwrap();
    }
    drop(conn);
    let swap = "g".repeat(64);
    assert!(rewrite_in(&path, "job_resolutions", &jobs[0].id, &swap) >= 1);
    assert!(rewrite_in(&path, "job_resolutions", &jobs[1].id, &jobs[0].id) >= 1);
    assert!(rewrite_in(&path, "job_resolutions", &swap, &jobs[1].id) >= 1);
    whole_file_checks_pass(&path);
    let conn = Connection::open(&path).unwrap();
    assert!(
        !conn
            .query_row(LOCK_INVARIANTS, [], |r| r.get::<_, bool>(0))
            .unwrap()
    );
    drop(conn);
    refused_unchanged(&dir, generation, "resolutions swapped");
}

/// A queued job's resource held twice, in `job_resources` and its index
/// alike (a UNIQUE violation `quick_check` does not check; both tallies
/// agree): the next dispatch would insert the same lock twice. Refused by
/// the unsettled jobs' entries read by seek, no two neighbours alike.
#[test]
fn a_resource_an_unsettled_job_holds_twice_refuses_startup() {
    let (dir, _, _) = forged_among_settled("no_such_operation", None);
    let path = dir.path().join("core.sqlite");
    let mut s = Store::open(&path).unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let job = queue(&mut s, &p, "queued-twice", "b");
    dependencies(&mut s, &job.id, &["storage:uuid:aaaa", "storage:uuid:aaab"]);
    let generation = s.generation;
    drop(s);
    // Renamed to its neighbour in key order: every index stays in order.
    for btree in ["job_resources", "sqlite_autoindex_job_resources_1"] {
        assert!(rewrite_in(&path, btree, "storage:uuid:aaab", "storage:uuid:aaaa") >= 1);
    }
    whole_file_checks_pass(&path);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM job_resources NOT INDEXED WHERE job=? AND resource='storage:uuid:aaaa'",
            [&job.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(conn);
    refused_unchanged(&dir, generation, "a resource held twice");
}
