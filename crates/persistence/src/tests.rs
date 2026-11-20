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

/// The revocation reader refuses a symlink at the database path, as the
/// store does, instead of opening whatever it points to.
#[test]
fn the_reader_refuses_a_symlinked_database() {
    let (d, s, _) = setup();
    let linked = d.path().join("linked.sqlite");
    std::os::unix::fs::symlink(d.path().join("core.sqlite"), &linked).unwrap();
    assert_eq!(
        open_reader(&linked).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        open_reader(&d.path().join("missing.sqlite"))
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    assert!(!d.path().join("missing.sqlite").exists());
    open_reader(&d.path().join("core.sqlite")).unwrap();
    drop(s);
}

/// The reader cannot write and, closed last, does not checkpoint: the WAL
/// it leaves is the writer's, untouched.
#[test]
fn the_reader_neither_writes_nor_checkpoints_on_close() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("core.sqlite");
    let wal = path.with_extension("sqlite-wal");
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE t(x); INSERT INTO t VALUES(1);")
        .unwrap();
    let reader = open_reader(&path).unwrap();
    assert!(reader.execute("INSERT INTO t VALUES(2)", []).is_err());
    let rows: i64 = reader
        .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    // The writer goes first without its own checkpoint, as after a crash.
    writer
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .unwrap();
    drop(writer);
    let frames = std::fs::metadata(&wal).unwrap().len();
    assert!(frames > 0);
    drop(reader);
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), frames);
}

/// Shutdown closes the reader on the writer thread, before the writer's
/// final checkpoint and before the core lock is released, even while a
/// revocation's blocking task still holds it: the writer's close is the
/// last one, so it removes the WAL.
#[tokio::test]
async fn shutdown_closes_the_reader_before_the_writer_checkpoints() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("core.sqlite");
    let db = Database::open(&path).unwrap();
    // An admission that finds nothing to revoke: the reader has read.
    db.revoke(
        Revocation::Task {
            digest: "missing".into(),
        },
        0,
    )
    .await
    .unwrap();
    let held = db.reader.clone();
    drop(db);
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .unwrap();
    let mut released = false;
    for _ in 0..500 {
        if lock.try_lock_exclusive().is_ok() {
            released = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(released);
    assert!(!path.with_extension("sqlite-wal").exists());
    // A late admission finds the reader gone instead of reopening the store.
    assert!(held.lock().unwrap().is_none());
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
    // With nothing to recover the same store opens, so reads and revocation
    // keep working while ordinary writes stay refused at the cap.
    conn.execute(
        "UPDATE jobs SET state='needs_intervention' WHERE state='running'",
        [],
    )
    .unwrap();
    drop(conn);
    let mut s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert_eq!(s.audit_usage().unwrap().level(), AuditLevel::Full);
    s.revoke_session("missing", 400).unwrap();
}

/// The reserve is full, a job was running, and nearly all history is
/// prunable: startup prunes in its ordinary batches until the recovery event
/// fits, instead of refusing until someone edits the database.
#[test]
fn startup_prunes_history_to_record_recovery_when_the_reserve_is_full() {
    let (d, mut s, p) = setup();
    let job = running(&mut s, &p, "interrupted");
    let rows = (AUDIT_LIMIT + retention::AUDIT_RESERVE) / 256;
    s.conn.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,'{\"kind\":\"session_created\"}',300 FROM c",params![rows,p.id]).unwrap();
    let generation = s.generation;
    drop(s);
    let s = Store::open(&d.path().join("core.sqlite")).unwrap();
    assert_eq!(s.generation, generation + 1);
    assert_eq!(s.job_state(&job).unwrap(), "needs_intervention");
    let recovered: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM events WHERE job=? AND kind=?",
            params![job, json(&EventKind::JobRecoveryRequired).unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recovered, 1);
    let usage = s.audit_usage().unwrap().bytes;
    assert_eq!(usage, events(&s) * 256);
    assert!(usage <= AUDIT_LIMIT + retention::AUDIT_RESERVE);
    // Only as much as recovery needed: one batch.
    assert!(usage >= AUDIT_LIMIT + retention::AUDIT_RESERVE - 1024 * 256);
}

/// `rows` job-less events of `kind`, inserted raw (startup reconciles the
/// counter).
fn raw_events(s: &Store, p: &Principal, kind: &str, rows: i64) {
    s.conn.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,?,300 FROM c",params![rows,p.id,kind]).unwrap();
}
/// `jobs` health probes left running, through the ordinary write paths
/// (two events each).
fn running_jobs(s: &mut Store, p: &Principal, jobs: usize) -> Vec<String> {
    s.conn.execute_batch("PRAGMA synchronous=OFF").unwrap();
    let ids = (0..jobs)
        .map(|i| running(s, p, &format!("interrupted-{i}")))
        .collect();
    s.conn.execute_batch("PRAGMA synchronous=FULL").unwrap();
    ids
}
fn authority_rows(conn: &Connection) -> Vec<i64> {
    conn.prepare(&format!(
        "SELECT cursor FROM events WHERE json_extract(kind,'$.kind') IN ({}) ORDER BY cursor",
        retention::AUTHORITY_KINDS
            .map(|kind| format!("'{kind}'"))
            .join(",")
    ))
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .collect::<std::result::Result<_, _>>()
    .unwrap()
}

/// The reviewer's store: the cap filled by 5,000 running jobs (10,000 pinned
/// events) and grant revisions, which the bulk rule never removes. Recovery
/// needs 904 rows beyond the reserve, which only the authority rule can free:
/// startup prunes the oldest revisions, down to what recovery needs and never
/// below the authority budget, and records every recovery event.
#[test]
fn startup_prunes_authority_events_when_bulk_history_cannot_make_room() {
    let (d, mut s, p) = setup();
    let path = d.path().join("core.sqlite");
    let jobs = running_jobs(&mut s, &p, 5_000);
    let total = AUDIT_LIMIT / 256;
    raw_events(
        &s,
        &p,
        &json(&EventKind::GrantsRevised).unwrap(),
        total - events(&s),
    );
    assert_eq!(events(&s), 262_144);
    let revisions = authority_rows(&s.conn);
    let generation = s.generation;
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.generation, generation + 1);
    for job in &jobs {
        assert_eq!(s.job_state(job).unwrap(), "needs_intervention");
    }
    let (held, recovered): (i64, i64) = s
        .conn
        .query_row(
            "SELECT count(*),count(*) FILTER (WHERE kind=?) FROM events WHERE job IS NOT NULL",
            [json(&EventKind::JobRecoveryRequired).unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((held, recovered), (15_000, 5_000));
    let usage = s.audit_usage().unwrap().bytes;
    assert_eq!(usage, events(&s) * 256);
    assert!(usage <= AUDIT_LIMIT + retention::AUDIT_RESERVE);
    // Only the oldest revisions went, in batches, no more than needed.
    let kept = authority_rows(&s.conn);
    let removed = revisions.len() - kept.len();
    assert!((904..904 + 256).contains(&removed), "{removed}");
    assert_eq!(kept, revisions[removed..]);
}

/// Both rules together may free what neither frees alone; when even both
/// cannot, startup refuses and the batches it ran are rolled back with the
/// rest of the startup transaction.
#[test]
fn startup_runs_every_events_rule_and_refuses_only_when_none_can_free_enough() {
    let total = (AUDIT_LIMIT + retention::AUDIT_RESERVE) / 256;
    for (bulk, revisions, opens) in [(500, 500, true), (250, 250, false)] {
        let (d, mut s, p) = setup();
        let path = d.path().join("core.sqlite");
        let job = running(&mut s, &p, "interrupted");
        // Startup needs 601 rows: 600 past the full reserve, then recovery.
        raw_events(&s, &p, &json(&EventKind::SessionCreated).unwrap(), bulk);
        raw_events(
            &s,
            &p,
            &json(&EventKind::GrantsRevised).unwrap(),
            4096 + revisions,
        );
        let pinned = total + 600 - events(&s);
        raw_events(
            &s,
            &p,
            r#"{"kind":"automation_schedule_changed","schedule":"s","revision":2}"#,
            pinned,
        );
        let (rows, generation) = (events(&s), s.generation);
        drop(s);
        match Store::open(&path) {
            Ok(s) => {
                assert!(opens);
                assert_eq!(s.job_state(&job).unwrap(), "needs_intervention");
                assert!(s.audit_usage().unwrap().bytes <= AUDIT_LIMIT + retention::AUDIT_RESERVE);
                // Not below the authority budget.
                assert!(authority_rows(&s.conn).len() >= 4096);
            }
            Err(error) => {
                assert!(!opens);
                assert_eq!(error.0, ErrorCode::StateNotDurable);
                let conn = Connection::open(&path).unwrap();
                let (after, generation_after): (i64, i64) = conn
                    .query_row(
                        "SELECT (SELECT count(*) FROM events),generation FROM meta WHERE id=1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!((after, generation_after), (rows, generation));
            }
        }
    }
}

/// Below the free-space reserve, startup gives back a WAL a crash left behind
/// before it refuses; it never writes or bumps the generation.
#[test]
fn startup_below_the_free_space_reserve_truncates_a_crash_wal_and_refuses() {
    let (d, s, _) = setup();
    let path = d.path().join("core.sqlite");
    let generation = s.generation;
    drop(s);
    // A connection that dies without its closing checkpoint leaves a WAL.
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA wal_autocheckpoint=0; CREATE TABLE crash_padding(x); INSERT INTO crash_padding SELECT zeroblob(65536) FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<16) SELECT x FROM c); DROP TABLE crash_padding;")
        .unwrap();
    std::mem::forget(conn);
    let wal = || std::fs::metadata(path.with_extension("sqlite-wal")).map_or(0, |m| m.len());
    assert!(wal() > 1024 * 1024);
    OPEN_FREE_BYTES.set(Some(MIN_FREE_BYTES - 1));
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(wal(), 0);
    OPEN_FREE_BYTES.set(None);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.generation, generation + 1);
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
fn revocation_succeeds_with_the_audit_cap_hit() {
    let (_d, mut s, p) = setup();
    revocable(&mut s, &p, "session");
    revocable(&mut s, &p, "late");
    s.conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    let intent = Intent::HealthProbe {
        resource: "system".into(),
    };
    assert_eq!(
        s.queue_job(&p, "refused", &intent, 400, 210).unwrap_err().0,
        ErrorCode::StateNotDurable
    );
    let before = events(&s);
    s.revoke_session("session", 211).unwrap();
    s.revoke_task("session-task", 211).unwrap();
    assert_eq!(live(&s, "session"), (false, false));
    assert_eq!(events(&s), before + 2);
    // Past the reserve too: authority is still removed, without its event.
    s.conn
        .execute(
            "UPDATE meta SET audit_bytes=?",
            [AUDIT_LIMIT + retention::AUDIT_RESERVE],
        )
        .unwrap();
    s.revoke_session("late", 212).unwrap();
    s.revoke_task("late-task", 212).unwrap();
    assert_eq!(live(&s, "late"), (false, false));
    assert_eq!(events(&s), before + 2);
}

/// A grant revision can promote a role or add scopes, so no gate of an
/// ordinary write is lifted for it: refused at the audit cap (it never uses
/// the revocation reserve) and below the free-space reserve.
#[test]
fn grant_revisions_take_the_ordinary_write_gates() {
    let (_d, mut s, p) = setup();
    let widen = [Scope {
        operation: Operation::AutomationManage,
        resource: "*".into(),
    }];
    let refused = |s: &mut Store| {
        let before = events(s);
        assert_eq!(
            s.revise_grants(&p.id, Role::Administrator, &widen, 213)
                .unwrap_err()
                .0,
            ErrorCode::StateNotDurable
        );
        assert_eq!(s.grants(&p).unwrap().len(), 5);
        assert_eq!(events(s), before);
    };
    s.conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    refused(&mut s);
    s.conn
        .execute(
            "UPDATE meta SET audit_bytes=?",
            [AUDIT_LIMIT + retention::AUDIT_RESERVE - 256],
        )
        .unwrap();
    refused(&mut s);
    s.conn
        .execute("UPDATE meta SET audit_bytes=?", [events(&s) * 256])
        .unwrap();
    s.free_bytes = Some(MIN_FREE_BYTES - 1);
    refused(&mut s);
    s.free_bytes = None;
    s.revise_grants(&p.id, Role::Administrator, &widen, 214)
        .unwrap();
}

#[test]
fn revocation_succeeds_below_the_free_space_reserve() {
    let (_d, mut s, p) = setup();
    revocable(&mut s, &p, "session");
    s.free_bytes = Some(MIN_FREE_BYTES - 1);
    let intent = Intent::HealthProbe {
        resource: "system".into(),
    };
    assert_eq!(
        s.queue_job(&p, "refused", &intent, 400, 210).unwrap_err().0,
        ErrorCode::StateNotDurable
    );
    let before = events(&s);
    s.revoke_session("session", 211).unwrap();
    s.revoke_task("session-task", 211).unwrap();
    assert_eq!(live(&s, "session"), (false, false));
    assert_eq!(events(&s), before + 2);
}

/// Worker threads make the order in which tasks run (and the two revocations
/// enter their lane) arbitrary; the outcome must not depend on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revocation_succeeds_with_the_queue_full_and_overtakes_it() {
    use std::sync::Mutex;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("core.sqlite");
    {
        let mut s = Store::open(&path).unwrap();
        s.issue_bootstrap("token", 100).unwrap();
        s.enroll("token", "alice", "$argon2id$fixture", 101)
            .unwrap();
        let p = s.login_record("alice").unwrap().unwrap().principal;
        revocable(&mut s, &p, "session");
    }
    let db = Database::open(&path).unwrap();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (busy, occupied) = oneshot::channel();
    let blocker = tokio::spawn({
        let db = db.clone();
        async move {
            db.call(move |_| {
                let _ = busy.send(());
                let _ = released.recv();
                Ok(())
            })
            .await
        }
    });
    occupied.await.unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut ordinary = Vec::new();
    for _ in 0..QUEUE_CAPACITY {
        let (db, seen) = (db.clone(), seen.clone());
        ordinary.push(tokio::spawn(async move {
            db.call(move |s| {
                // The reopen already retired the token (a new generation), so
                // its revocation shows in the row, not in `check_task`.
                let revoked: bool = s
                    .conn
                    .query_row(
                        "SELECT revoked FROM task_tokens WHERE digest='session-task'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(durable)?;
                let state = (s.authenticate("session", None, 300).is_ok(), revoked);
                seen.lock().unwrap().push(state);
                Ok(())
            })
            .await
        }));
    }
    while db.sender.capacity() > 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        db.call(|_| Ok(())).await.unwrap_err().0,
        ErrorCode::Overloaded
    );
    let revocations = [
        Revocation::Session {
            digest: "session".into(),
            csrf: "csrf".into(),
        },
        Revocation::Task {
            digest: "session-task".into(),
        },
    ]
    .map(|revocation| {
        let db = db.clone();
        tokio::spawn(async move { db.revoke(revocation, 250).await })
    });
    while db.priority.capacity() > PRIORITY_CAPACITY - 2 {
        tokio::task::yield_now().await;
    }
    release.send(()).unwrap();
    for revocation in revocations {
        revocation.await.unwrap().unwrap();
    }
    blocker.await.unwrap().unwrap();
    for call in ordinary {
        call.await.unwrap().unwrap();
    }
    // Every queued item ran after both revocations.
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert_eq!(seen.len(), QUEUE_CAPACITY);
    assert!(seen.iter().all(|state| *state == (false, true)), "{seen:?}");
    let events: i64 = db
        .call(|s| {
            s.conn
                .query_row("SELECT count(*) FROM events WHERE json_extract(kind,'$.kind') IN ('session_revoked','task_revoked') AND created=250", [], |r| r.get(0))
                .map_err(durable)
        })
        .await
        .unwrap();
    assert_eq!(events, 2);
}

/// A store with a live session and task token for `alice`, opened as the
/// daemon opens it.
fn database(d: &tempfile::TempDir) -> Database {
    let path = d.path().join("core.sqlite");
    {
        let mut s = Store::open(&path).unwrap();
        s.issue_bootstrap("token", 100).unwrap();
        s.enroll("token", "alice", "$argon2id$fixture", 101)
            .unwrap();
        let p = s.login_record("alice").unwrap().unwrap().principal;
        revocable(&mut s, &p, "session");
    }
    Database::open(&path).unwrap()
}
/// Proves a retention pass runs between ordinary calls: clears the pass time,
/// then sees it set again.
async fn maintained(db: &Database) -> bool {
    db.call(|s| {
        s.retained_at = None;
        Ok(())
    })
    .await
    .unwrap();
    db.call(|s| Ok(s.retained_at.is_some())).await.unwrap()
}

/// Logouts with well-formed but unknown cookies or a wrong CSRF value are
/// refused before the writer: none reaches the revocation lane or takes an
/// ordinary slot, so ordinary calls and retention keep running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unauthenticated_logouts_never_reach_the_writer() {
    let d = tempfile::tempdir().unwrap();
    let db = database(&d);
    let flood: Vec<_> = (0..2000)
        .map(|i| {
            let db = db.clone();
            tokio::spawn(async move {
                let revocation = if i % 2 == 0 {
                    Revocation::Session {
                        digest: format!("unknown-{i}"),
                        csrf: "csrf".into(),
                    }
                } else {
                    Revocation::Session {
                        digest: "session".into(),
                        csrf: format!("wrong-{i}"),
                    }
                };
                db.revoke(revocation, 250).await.unwrap_err().0
            })
        })
        .collect();
    for _ in 0..200 {
        db.call(|s| Ok(live(s, "session"))).await.unwrap();
    }
    assert!(maintained(&db).await);
    for (i, refused) in flood.into_iter().enumerate() {
        let expected = if i % 2 == 0 {
            ErrorCode::Unauthenticated
        } else {
            ErrorCode::Forbidden
        };
        assert_eq!(refused.await.unwrap(), expected);
    }
    assert_eq!(db.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(db.priority.capacity(), PRIORITY_CAPACITY);
    // Unknown or revoked task tokens need nothing either.
    db.revoke(
        Revocation::Task {
            digest: "unknown".into(),
        },
        250,
    )
    .await
    .unwrap();
    assert_eq!(db.admitted.load(Ordering::SeqCst), 0);
    // A genuine logout still goes through.
    db.revoke(
        Revocation::Session {
            digest: "session".into(),
            csrf: "csrf".into(),
        },
        251,
    )
    .await
    .unwrap();
    assert_eq!(db.admitted.load(Ordering::SeqCst), 1);
    assert!(!db.call(|s| Ok(live(s, "session").0)).await.unwrap());
}

/// Revocation work that keeps its lane full forever (each item queues the
/// next) still lets ordinary calls and the retention pass through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_revocation_lane_cannot_starve_ordinary_work() {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    fn chain(lane: mpsc::Sender<Work>, stop: Arc<AtomicBool>, runs: Arc<AtomicUsize>) -> Work {
        Box::new(move |_| {
            runs.fetch_add(1, Ordering::SeqCst);
            if !stop.load(Ordering::SeqCst) {
                let next = chain(lane.clone(), stop.clone(), runs.clone());
                let _ = lane.try_send(next);
            }
        })
    }
    let d = tempfile::tempdir().unwrap();
    let db = database(&d);
    let (stop, runs) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    for _ in 0..PRIORITY_BURST {
        db.priority
            .send(chain(db.priority.clone(), stop.clone(), runs.clone()))
            .await
            .unwrap();
    }
    db.bell.ring();
    let ordinary = async {
        for _ in 0..50 {
            db.call(|s| Ok(live(s, "session"))).await.unwrap();
        }
        maintained(&db).await
    };
    let progressed = tokio::time::timeout(Duration::from_secs(20), ordinary).await;
    stop.store(true, Ordering::SeqCst);
    assert_eq!(progressed, Ok(true));
    // The revocation lane was served throughout, not parked.
    assert!(runs.load(Ordering::SeqCst) >= 50 * PRIORITY_BURST);
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

/// A writer stuck on one item (a hung fsync, a lock held from outside) no
/// longer freezes its callers: each call gives up at its deadline with
/// `Unavailable`, and how long the writer has been on that item, and how
/// many calls wait behind it, can be read without queueing.
#[tokio::test]
async fn a_stalled_writer_fails_calls_at_their_deadline_and_shows_how_long_it_has_been_stuck() {
    let d = tempfile::tempdir().unwrap();
    let mut db = Database::open(&d.path().join("core.sqlite")).unwrap();
    db.deadline = Duration::from_millis(300);
    assert_eq!(db.call(|_| Ok(1)).await, Ok(1));
    let idle = db.writer_activity();
    assert_eq!((idle.busy_for, idle.queued), (None, 0));
    let (release, stuck) = std::sync::mpsc::channel::<()>();
    let (entered, started) = std::sync::mpsc::channel::<()>();
    let blocker = db.clone();
    let blocked = tokio::spawn(async move {
        blocker
            .call(move |_| {
                entered.send(()).unwrap();
                stuck.recv().unwrap();
                Ok(())
            })
            .await
    });
    tokio::task::spawn_blocking(move || started.recv().unwrap())
        .await
        .unwrap();
    let asked = Instant::now();
    assert_eq!(db.call(|_| Ok(2)).await, Err(Error(ErrorCode::Unavailable)));
    let waited = asked.elapsed();
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
    // The blocked call itself gave up too, though its work still runs.
    assert_eq!(blocked.await.unwrap(), Err(Error(ErrorCode::Unavailable)));
    let stalled = db.writer_activity();
    assert!(
        stalled
            .busy_for
            .is_some_and(|busy| busy >= Duration::from_millis(250)),
        "{stalled:?}"
    );
    // The abandoned call still waits in the queue.
    assert_eq!(stalled.queued, 1);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while db.writer_activity() != WriterActivity::default() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the writer goes idle once the item returns");
    assert_eq!(db.call(|_| Ok(3)).await, Ok(3));
}
