use super::*;
use limeos_domain::{ContainerJob, StackJob, StackSnapshot};

const LONG: &str = "Checked the engine by hand and accept the present state.";

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
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    }
}
fn restarted(s: &ContainerSnapshot) -> ContainerSnapshot {
    ContainerSnapshot {
        started_at: "2026-10-05T07:05:00.000000000Z".into(),
        ..s.clone()
    }
}
/// A claimed (running, locked) restart job on one container.
fn running(store: &mut Store, p: &Principal, key: &str, byte: &str) -> ContainerJob {
    let before = snapshot(byte);
    let plan = store.plan_restart(p, &before, 200).unwrap();
    let approval = store
        .approve_container(p, &plan.plan.id, &plan.digest, 201)
        .unwrap();
    let job = store
        .queue_container(p, key, &plan, &approval.token, &before, 202)
        .unwrap();
    store
        .claim_container(p, &job.id, &before, &"1".repeat(64), 203)
        .unwrap()
}
fn uncertain(store: &mut Store, job: &str) {
    store
        .conn
        .execute(
            "UPDATE jobs SET state='needs_intervention' WHERE id=?",
            [job],
        )
        .unwrap();
}
fn locks(store: &Store, job: &str) -> i64 {
    store
        .conn
        .query_row(
            "SELECT count(*) FROM resource_locks WHERE job=?",
            [job],
            |r| r.get(0),
        )
        .unwrap()
}
fn evidence(job: &ContainerJob, now: &ContainerSnapshot) -> ResolutionEvidence {
    ResolutionEvidence::observe(
        job.plan.operation,
        &[&job.plan.expected],
        std::slice::from_ref(&Some(now.clone())),
    )
    .unwrap()
}
fn request(
    store: &mut Store,
    p: &Principal,
    job: &str,
    declared: DeclaredOutcome,
    reason: &str,
    e: &ResolutionEvidence,
) -> Result<PendingResolution> {
    store.request_resolution(p, job, declared, reason, e, &e.digest().unwrap(), 300)
}
fn confirm(pending: &PendingResolution) -> ResolveReceipt {
    ResolveReceipt {
        kind: pending.kind,
        action: pending.request.job.clone(),
        evidence_digest: pending.request.evidence_digest.clone(),
        declared: pending.request.declared,
        resolved_by: pending.request.resolved_by.clone(),
    }
}
fn job_row(store: &Store, id: &str) -> (String, String, String, String, i64, i64, i64) {
    store
        .conn
        .query_row(
            "SELECT principal,idempotency,intent,digest,generation,revision,deadline FROM jobs WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap()
}

#[test]
fn only_an_uncertain_job_can_be_requested() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    let e = evidence(&job, &job.plan.expected);
    // A live running job is still owned by its worker.
    assert_eq!(
        request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // A running job left behind by an earlier core generation is uncertain.
    s.conn
        .execute("UPDATE jobs SET generation=0 WHERE id=?", [&job.id])
        .unwrap();
    request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    s.withdraw_resolution(&p, &job.id, 301).unwrap();
    uncertain(&mut s, &job.id);
    request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    // Queued, succeeded and unknown ids never qualify.
    assert_eq!(
        request(
            &mut s,
            &p,
            &"z".repeat(64),
            DeclaredOutcome::NotDone,
            "ok",
            &e
        )
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    let other = running(&mut s, &p, "b", "b");
    s.conn
        .execute("UPDATE jobs SET state='succeeded' WHERE id=?", [&other.id])
        .unwrap();
    assert_eq!(
        request(&mut s, &p, &other.id, DeclaredOutcome::NotDone, "ok", &e)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // Nothing but the audited request was written; the job and locks stay.
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(locks(&s, &job.id), 1);
    resources::check(&s.conn).unwrap();
}

#[test]
fn outcome_rules_are_enforced_against_the_supplied_evidence() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let unchanged = evidence(&job, &job.plan.expected);
    let done = evidence(&job, &restarted(&job.plan.expected));
    let conflict = |r: Result<PendingResolution>| r.unwrap_err().0;
    use DeclaredOutcome::*;
    assert_eq!(
        conflict(request(&mut s, &p, &job.id, VerifiedDone, "ok", &unchanged)),
        ErrorCode::Conflict
    );
    assert_eq!(
        conflict(request(&mut s, &p, &job.id, NotDone, "ok", &done)),
        ErrorCode::Conflict
    );
    // Divergence always needs a long reason, and a reason is always needed.
    assert_eq!(
        conflict(request(
            &mut s,
            &p,
            &job.id,
            AcceptedDivergence,
            "short",
            &done
        )),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        conflict(request(&mut s, &p, &job.id, NotDone, "", &unchanged)),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        conflict(request(
            &mut s,
            &p,
            &job.id,
            NotDone,
            &"x".repeat(501),
            &unchanged
        )),
        ErrorCode::InvalidInput
    );
    // Stale digest: the page must re-fetch.
    assert_eq!(
        conflict(s.request_resolution(
            &p,
            &job.id,
            NotDone,
            "ok",
            &unchanged,
            &done.digest().unwrap(),
            300
        )),
        ErrorCode::Conflict
    );
    // Evidence for some other container is not evidence for this job.
    let foreign = {
        let other = snapshot("c");
        ResolutionEvidence::observe(ContainerAction::Restart, &[&other], &[Some(other.clone())])
            .unwrap()
    };
    assert_eq!(
        conflict(request(&mut s, &p, &job.id, NotDone, "ok", &foreign)),
        ErrorCode::InvalidInput
    );
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM job_resolutions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    // Each declaration the evidence supports is accepted.
    request(&mut s, &p, &job.id, VerifiedDone, "ok", &done).unwrap();
    s.withdraw_resolution(&p, &job.id, 301).unwrap();
    request(&mut s, &p, &job.id, NotDone, "ok", &unchanged).unwrap();
    s.withdraw_resolution(&p, &job.id, 302).unwrap();
    request(&mut s, &p, &job.id, AcceptedDivergence, LONG, &done).unwrap();
}

#[test]
fn only_a_current_administrator_may_request_and_the_state_machine_refuses_the_generic_path() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    for role in [Role::Operator, Role::Viewer, Role::MediaRequester] {
        let demoted = Principal { role, ..p.clone() };
        assert!(
            request(
                &mut s,
                &demoted,
                &job.id,
                DeclaredOutcome::NotDone,
                "ok",
                &e
            )
            .is_err()
        );
        assert!(s.resolution_subject(&demoted, &job.id).is_err());
    }
    let stale = Principal {
        grant_revision: p.grant_revision + 1,
        ..p.clone()
    };
    assert!(request(&mut s, &stale, &job.id, DeclaredOutcome::NotDone, "ok", &e).is_err());
    assert_eq!(
        s.transition(
            &job.id,
            JobState::NeedsIntervention,
            JobState::Resolved,
            300
        )
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    // Direct SQL cannot skip the request/confirmation either.
    assert!(
        s.conn
            .execute("UPDATE jobs SET state='resolved' WHERE id=?", [&job.id])
            .is_err()
    );
    assert!(
        s.conn
            .execute("UPDATE jobs SET state='bogus' WHERE id=?", [&job.id])
            .is_err()
    );
    let subject = s.resolution_subject(&p, &job.id).unwrap();
    assert_eq!(subject.kind, ResolveKind::Container);
    assert_eq!(subject.targets, vec![job.plan.expected.clone()]);
    assert_eq!(subject.state, JobState::NeedsIntervention);
}

#[test]
fn completion_resolves_releases_locks_audits_and_keeps_every_original_row() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let before = job_row(&s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    let pending = request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    assert_eq!(pending.kind, ResolveKind::Container);
    assert_eq!(
        pending.plan_digest,
        limeos_identity::digest(&json(&job.plan).unwrap())
    );
    assert_eq!(s.pending_resolutions().unwrap(), vec![pending.clone()]);
    // The request alone changes nothing about the lock.
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(locks(&s, &job.id), 1);
    // A confirmation for something else, or none at all, is refused.
    for wrong in [
        ResolveReceipt {
            evidence_digest: "0".repeat(64),
            ..confirm(&pending)
        },
        ResolveReceipt {
            declared: DeclaredOutcome::VerifiedDone,
            ..confirm(&pending)
        },
        ResolveReceipt {
            kind: ResolveKind::Stack,
            ..confirm(&pending)
        },
        ResolveReceipt {
            resolved_by: "someone".into(),
            ..confirm(&pending)
        },
    ] {
        assert_eq!(
            s.complete_resolution(&job.id, &wrong, 310).unwrap_err().0,
            ErrorCode::Conflict
        );
    }
    assert_eq!(
        s.complete_resolution(&"y".repeat(64), &confirm(&pending), 310)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    let record = s
        .complete_resolution(&job.id, &confirm(&pending), 311)
        .unwrap();
    assert_eq!(record.resolved_at, 311);
    assert_eq!(record.resolved_by, p.id);
    assert_eq!(s.job_state(&job.id).unwrap(), "resolved");
    assert_eq!(locks(&s, &job.id), 0);
    assert!(s.pending_resolutions().unwrap().is_empty());
    assert_eq!(s.job_resolution(&p, &job.id).unwrap(), Some(record.clone()));
    // Original identity columns are untouched; only the state moved.
    assert_eq!(job_row(&s, &job.id), before);
    assert_eq!(s.container_job(&p, &job.id).unwrap().plan, job.plan);
    // Idempotent repeat returns the stored record; no second audit event.
    let events = s.container_events(&p, &job.id, 0).unwrap().len();
    assert_eq!(
        s.complete_resolution(&job.id, &confirm(&pending), 400)
            .unwrap(),
        record
    );
    assert_eq!(s.container_events(&p, &job.id, 0).unwrap().len(), events);
    assert!(
        s.container_events(&p, &job.id, 0)
            .unwrap()
            .iter()
            .any(|e| e.event
                == EventKind::JobResolved {
                    declared: DeclaredOutcome::NotDone
                })
    );
    // Terminal: not reopened, request not withdrawn or rewritten.
    assert!(s.withdraw_resolution(&p, &job.id, 401).is_err());
    assert!(
        s.conn
            .execute(
                "UPDATE jobs SET state='needs_intervention' WHERE id=?",
                [&job.id]
            )
            .is_err()
    );
    assert!(
        s.conn
            .execute(
                "UPDATE job_resolutions SET reason='edited' WHERE job=?",
                [&job.id]
            )
            .is_err()
    );
    assert!(
        s.conn
            .execute("DELETE FROM job_resolutions WHERE job=?", [&job.id])
            .is_err()
    );
    resources::check(&s.conn).unwrap();
    // The resource is free: a new action on the same container dispatches.
    let again = running(&mut s, &p, "again", "a");
    assert_eq!(again.state, JobState::Running);
    assert_eq!(locks(&s, &again.id), 1);
}

#[test]
fn request_then_crash_is_completed_after_restart_and_nothing_is_freed_before() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let (job, pending, p) = {
        let mut s = Store::open(&path).unwrap();
        s.issue_bootstrap("bootstrap", 100).unwrap();
        s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
            .unwrap();
        let p = s.login_record("alice").unwrap().unwrap().principal;
        let job = running(&mut s, &p, "a", "a");
        // Restart recovery itself makes the running job needs_intervention.
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
        let e = evidence(&job, &restarted(&job.plan.expected));
        let pending =
            request(&mut s, &p, &job.id, DeclaredOutcome::VerifiedDone, "ok", &e).unwrap();
        (job, pending, p)
    };
    // Crash after step 1: still held, request still pending.
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(locks(&s, &job.id), 1);
    assert_eq!(s.pending_resolutions().unwrap(), vec![pending.clone()]);
    // The executor answered before the crash; startup finishes step 3.
    s.complete_resolution(&job.id, &confirm(&pending), 500)
        .unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.job_state(&job.id).unwrap(), "resolved");
    assert_eq!(locks(&s, &job.id), 0);
    assert!(s.pending_resolutions().unwrap().is_empty());
    assert!(s.job_resolution(&p, &job.id).unwrap().is_some());
}

#[test]
fn a_second_request_conflicts_until_the_first_is_withdrawn() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    let first = request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "first", &e).unwrap();
    // An identical retry returns the open request.
    assert_eq!(
        request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "first", &e).unwrap(),
        first
    );
    for (declared, reason) in [
        (DeclaredOutcome::NotDone, "different reason"),
        (DeclaredOutcome::AcceptedDivergence, LONG),
    ] {
        assert_eq!(
            request(&mut s, &p, &job.id, declared, reason, &e)
                .unwrap_err()
                .0,
            ErrorCode::Conflict
        );
    }
    s.withdraw_resolution(&p, &job.id, 301).unwrap();
    assert_eq!(
        s.withdraw_resolution(&p, &job.id, 302).unwrap_err().0,
        ErrorCode::NotFound
    );
    request(
        &mut s,
        &p,
        &job.id,
        DeclaredOutcome::AcceptedDivergence,
        LONG,
        &e,
    )
    .unwrap();
}

#[tokio::test]
async fn concurrent_resolutions_have_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let (job, p, e) = db
        .call(|s| {
            s.issue_bootstrap("bootstrap", 100)?;
            s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)?;
            let p = s.login_record("alice")?.unwrap().principal;
            let job = running(s, &p, "a", "a");
            uncertain(s, &job.id);
            let e = evidence(&job, &job.plan.expected);
            Ok((job, p, e))
        })
        .await
        .unwrap();
    let mut handles = vec![];
    for i in 0..8 {
        let (db, p, e, id) = (db.clone(), p.clone(), e.clone(), job.id.clone());
        handles.push(tokio::spawn(async move {
            db.call(move |s| {
                request(
                    s,
                    &p,
                    &id,
                    DeclaredOutcome::NotDone,
                    &format!("reason {i}"),
                    &e,
                )
            })
            .await
        }));
    }
    let mut won = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => won += 1,
            Err(e) => assert_eq!(e.0, ErrorCode::Conflict),
        }
    }
    assert_eq!(won, 1);
}

fn member(n: char, running: bool) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", n.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running,
    }
}
fn stack() -> StackSnapshot {
    StackSnapshot::new(
        "media",
        vec![member('a', true), member('b', true), member('c', false)],
    )
    .unwrap()
}

#[test]
fn stack_jobs_resolve_on_their_targets_and_free_every_member_lock() {
    let (_d, mut s, p) = setup();
    let proposal = s
        .plan_stack(&p, ContainerAction::Restart, "registered", &stack(), 200)
        .unwrap();
    let approval = s
        .approve_stack(&p, &proposal.plan.id, &proposal.digest, 201)
        .unwrap();
    let job: StackJob = s
        .queue_stack(&p, "restart", &proposal, &approval.token, &stack(), 202)
        .unwrap();
    let job = s
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 203)
        .unwrap();
    assert_eq!(locks(&s, &job.id), 5);
    uncertain(&mut s, &job.id);
    let subject = s.resolution_subject(&p, &job.id).unwrap();
    assert_eq!(subject.kind, ResolveKind::Stack);
    // Only the stopped-member-excluding targets are evidence.
    assert_eq!(subject.targets.len(), 2);
    let targets: Vec<_> = subject.targets.iter().collect();
    let half = vec![Some(restarted(&member('a', true))), Some(member('b', true))];
    let mixed = ResolutionEvidence::observe(ContainerAction::Restart, &targets, &half).unwrap();
    assert_eq!(
        request(
            &mut s,
            &p,
            &job.id,
            DeclaredOutcome::VerifiedDone,
            "ok",
            &mixed
        )
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &mixed)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    let pending = request(
        &mut s,
        &p,
        &job.id,
        DeclaredOutcome::AcceptedDivergence,
        LONG,
        &mixed,
    )
    .unwrap();
    assert_eq!(pending.kind, ResolveKind::Stack);
    s.complete_resolution(&job.id, &confirm(&pending), 320)
        .unwrap();
    assert_eq!(locks(&s, &job.id), 0);
    assert_eq!(s.job_state(&job.id).unwrap(), "resolved");
    assert_eq!(s.stack_job(&p, &job.id).unwrap().state, JobState::Resolved);
    resources::check(&s.conn).unwrap();
    // A single-container job on a former member now dispatches.
    let single = running(&mut s, &p, "single", "a");
    assert_eq!(single.state, JobState::Running);
}

#[test]
fn startup_refuses_resolution_state_that_disagrees_with_locks_or_requests() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    let pending = request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    s.complete_resolution(&job.id, &confirm(&pending), 320)
        .unwrap();
    resources::check(&s.conn).unwrap();
    // Corruption is injected with the guarding triggers lifted, then restored.
    let names = [
        "resource_locks_insert",
        "job_resolutions_update",
        "job_resolutions_delete",
        "jobs_resolved_update",
    ];
    let saved: Vec<String> = names
        .iter()
        .map(|n| {
            s.conn
                .query_row("SELECT sql FROM sqlite_schema WHERE name=?", [n], |r| {
                    r.get(0)
                })
                .unwrap()
        })
        .collect();
    let lift = |s: &Store| {
        for n in names {
            s.conn.execute_batch(&format!("DROP TRIGGER {n}")).unwrap();
        }
    };
    let restore = |s: &Store| {
        for sql in &saved {
            s.conn.execute_batch(sql).unwrap();
        }
    };
    // A resolved job that still holds a lock.
    lift(&s);
    s.conn
        .execute(
            "INSERT INTO resource_locks SELECT resource,job FROM job_resources WHERE job=?",
            [&job.id],
        )
        .unwrap();
    restore(&s);
    assert!(resources::check(&s.conn).is_err());
    s.conn
        .execute("DELETE FROM resource_locks WHERE job=?", [&job.id])
        .unwrap();
    resources::check(&s.conn).unwrap();
    // A resolved job without a completed request.
    lift(&s);
    s.conn
        .execute(
            "UPDATE job_resolutions SET confirmation=NULL,completed=NULL WHERE job=?",
            [&job.id],
        )
        .unwrap();
    restore(&s);
    assert!(resources::check(&s.conn).is_err());
}

#[test]
fn migration_from_version_15_widens_the_job_states_and_keeps_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let job = {
        let mut s = Store::open(&path).unwrap();
        s.issue_bootstrap("bootstrap", 100).unwrap();
        s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
            .unwrap();
        let p = s.login_record("alice").unwrap().unwrap().principal;
        let job = running(&mut s, &p, "a", "a");
        // Rebuild the version-15 shape: no table, triggers or widened CHECK.
        s.conn
            .execute_batch(
                "ALTER TABLE meta DROP COLUMN executor_epoch; DROP INDEX events_notification; DROP INDEX events_kind; DROP INDEX events_job; DROP TRIGGER jobs_resolved_update; DROP TRIGGER jobs_resolved_insert; DROP TABLE job_resolutions;
                 PRAGMA writable_schema=ON;
                 UPDATE sqlite_schema SET sql=replace(sql,',''resolved'')',')') WHERE type='table' AND name='jobs';
                 PRAGMA writable_schema=RESET;
                 PRAGMA user_version=15;",
            )
            .unwrap();
        assert!(
            s.conn
                .execute("UPDATE jobs SET state='resolved' WHERE id=?", [&job.id])
                .is_err()
        );
        job
    };
    let mut s = Store::open(&path).unwrap();
    let version: u32 = s
        .conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    // The recovered job is still held; the new state exists and is guarded.
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(locks(&s, &job.id), 1);
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let e = evidence(&job, &job.plan.expected);
    let pending = request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    s.complete_resolution(&job.id, &confirm(&pending), 320)
        .unwrap();
    assert_eq!(s.job_state(&job.id).unwrap(), "resolved");
    assert!(
        s.conn
            .execute("UPDATE jobs SET state='bogus' WHERE id=?", [&job.id])
            .is_err()
    );
    drop(s);
    Store::open(&path).unwrap();
}

#[test]
fn reauthentication_needs_a_live_administrator_session_with_its_csrf() {
    let (_d, mut s, p) = setup();
    let record = s.login_record("alice").unwrap().unwrap();
    s.create_session(&record, None, "session", "csrf", 400)
        .unwrap();
    let found = s
        .resolution_reauthentication("session", "csrf", 401)
        .unwrap();
    assert_eq!(found.principal, p);
    assert_eq!(found.password_hash, "$argon2id$fixture");
    assert_eq!(
        s.resolution_reauthentication("session", "other", 401)
            .err()
            .unwrap(),
        Error(ErrorCode::Forbidden)
    );
    assert_eq!(
        s.resolution_reauthentication("missing", "csrf", 401)
            .err()
            .unwrap(),
        Error(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        s.resolution_reauthentication("session", "csrf", 400 + 9 * 3600)
            .err()
            .unwrap(),
        Error(ErrorCode::Unauthenticated)
    );
    s.conn
        .execute("UPDATE users SET role='\"operator\"'", [])
        .unwrap();
    assert!(
        s.resolution_reauthentication("session", "csrf", 401)
            .is_err()
    );
}

#[test]
fn principal_lookup_and_member_outcomes_follow_the_stored_receipt() {
    let (_d, mut s, p) = setup();
    assert_eq!(s.resolution_principal(&p.id).unwrap(), p);
    assert_eq!(
        s.resolution_principal(&"f".repeat(32)).unwrap_err(),
        Error(ErrorCode::NotFound)
    );
    let job = running(&mut s, &p, "a", "a");
    assert_eq!(s.resolution_members(&p, &job.id).unwrap(), None);
    let receipt = limeos_domain::RestartReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&json(&job.plan).unwrap()),
        operation: job.plan.operation,
        before: job.plan.expected.clone(),
        state: limeos_domain::ExecutionState::OutcomeUnknown,
        error: Some(ErrorCode::Unavailable),
    };
    s.record_container_result(&job, &receipt, None, 250)
        .unwrap();
    let members = s.resolution_members(&p, &job.id).unwrap().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].container, "a".repeat(64));
    assert_eq!(
        members[0].state,
        limeos_domain::ExecutionState::OutcomeUnknown
    );
    assert_eq!(members[0].error, Some(ErrorCode::Unavailable));
}

/// A second user with container rights but only the operator role.
fn operator(store: &Store) -> Principal {
    store
        .conn
        .execute(
            "INSERT INTO users VALUES('bob','bob','$argon2id$fixture','\"operator\"',1)",
            [],
        )
        .unwrap();
    store
        .conn
        .execute(
            "INSERT INTO grants VALUES('bob','\"container_manage\"','*')",
            [],
        )
        .unwrap();
    store.login_record("bob").unwrap().unwrap().principal
}

#[test]
fn an_administrator_sees_every_principals_stuck_jobs_and_others_keep_their_own() {
    let (_d, mut s, alice) = setup();
    let bob = operator(&s);
    let mine = running(&mut s, &alice, "a", "a");
    let theirs = running(&mut s, &bob, "b", "b");
    let live = running(&mut s, &bob, "c", "c");
    uncertain(&mut s, &mine.id);
    uncertain(&mut s, &theirs.id);
    // `live` is running under the current core generation: not stuck.
    let seen = s.needs_attention_jobs(&alice).unwrap();
    assert_eq!(
        seen.iter().map(|j| j.job.as_str()).collect::<Vec<_>>(),
        [mine.id.as_str(), theirs.id.as_str()]
    );
    let other = &seen[1];
    assert_eq!(other.owner, "bob");
    assert_eq!(other.kind, ResolveKind::Container);
    assert_eq!(other.operation, ContainerAction::Restart);
    assert_eq!(other.state, JobState::NeedsIntervention);
    assert_eq!(other.resource, theirs.plan.expected.resource);
    assert_eq!(other.targets, [theirs.plan.expected.resource.clone()]);
    assert!(other.created_at > 0);
    assert!(seen.iter().all(|j| j.job != live.id));
    // The operator is refused the listing and sees only their own jobs.
    assert_eq!(
        s.needs_attention_jobs(&bob).unwrap_err().0,
        ErrorCode::Forbidden
    );
    let own: Vec<_> = s
        .container_jobs(&bob)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect();
    assert!(own.contains(&theirs.id) && !own.contains(&mine.id));
    // Role and grant revision are checked against the store.
    for role in [Role::Operator, Role::Viewer, Role::MediaRequester] {
        let demoted = Principal {
            role,
            ..alice.clone()
        };
        assert!(s.needs_attention_jobs(&demoted).is_err());
    }
    let stale = Principal {
        grant_revision: alice.grant_revision + 1,
        ..alice.clone()
    };
    assert!(s.needs_attention_jobs(&stale).is_err());
    // An administrator can read and decide another principal's job.
    let subject = s.resolution_subject(&alice, &theirs.id).unwrap();
    assert_eq!(subject.targets, [theirs.plan.expected.clone()]);
    let e = evidence(&theirs, &theirs.plan.expected);
    let pending = request(
        &mut s,
        &alice,
        &theirs.id,
        DeclaredOutcome::NotDone,
        "ok",
        &e,
    )
    .unwrap();
    s.complete_resolution(&theirs.id, &confirm(&pending), 320)
        .unwrap();
    assert_eq!(locks(&s, &theirs.id), 0);
    let record = s.job_resolution(&alice, &theirs.id).unwrap().unwrap();
    assert_eq!(record.resolved_by, alice.id);
    let remaining = s.needs_attention_jobs(&alice).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].job, mine.id);
}

#[test]
fn the_listing_includes_jobs_left_running_by_an_earlier_generation_and_stack_jobs() {
    let (_d, mut s, alice) = setup();
    let bob = operator(&s);
    let stale = running(&mut s, &bob, "d", "d");
    // The same test `request_resolution` applies: an older generation.
    s.conn
        .execute("UPDATE jobs SET generation=0 WHERE id=?", [&stale.id])
        .unwrap();
    let seen = s.needs_attention_jobs(&alice).unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].state, JobState::Running);
    s.conn
        .execute("UPDATE jobs SET state='verifying' WHERE id=?", [&stale.id])
        .unwrap();
    assert_eq!(
        s.needs_attention_jobs(&alice).unwrap()[0].state,
        JobState::Verifying
    );
    // A stack job lists the headline stack and only the members it acts on.
    let proposal = s
        .plan_stack(&bob, ContainerAction::Restart, "registered", &stack(), 200)
        .unwrap();
    let approval = s
        .approve_stack(&bob, &proposal.plan.id, &proposal.digest, 201)
        .unwrap();
    let job = s
        .queue_stack(&bob, "restart", &proposal, &approval.token, &stack(), 202)
        .unwrap();
    let job = s
        .claim_stack(&bob, &job.id, &stack(), &"1".repeat(64), 203)
        .unwrap();
    uncertain(&mut s, &job.id);
    let seen = s.needs_attention_jobs(&alice).unwrap();
    let listed = seen.iter().find(|j| j.job == job.id).unwrap();
    assert_eq!(listed.kind, ResolveKind::Stack);
    assert_eq!(listed.owner, "bob");
    assert_eq!(listed.resource, job.plan.expected.resource);
    assert_eq!(listed.targets.len(), 2);
    assert!(s.stack_jobs(&alice).unwrap().is_empty());
}

#[test]
fn the_listing_is_bounded() {
    let (_d, mut s, alice) = setup();
    let bob = operator(&s);
    let mut first = String::new();
    for n in 0..70 {
        let before = ContainerSnapshot {
            resource: format!("container:{n:064x}"),
            ..snapshot("a")
        };
        let plan = s.plan_restart(&bob, &before, 200).unwrap();
        let approval = s
            .approve_container(&bob, &plan.plan.id, &plan.digest, 201)
            .unwrap();
        let job = s
            .queue_container(&bob, &format!("k{n}"), &plan, &approval.token, &before, 202)
            .unwrap();
        let job = s
            .claim_container(&bob, &job.id, &before, &"1".repeat(64), 203)
            .unwrap();
        uncertain(&mut s, &job.id);
        if n == 0 {
            first = job.id;
        }
    }
    let seen = s.needs_attention_jobs(&alice).unwrap();
    assert_eq!(seen.len(), 64);
    // Oldest first: the first job leads.
    assert_eq!(seen[0].job, first);
}

/// What recovery records when its closing lookup finds no receipt: the
/// closure, a `precondition_changed` receipt, which settles the job.
fn settle_by_closure(store: &mut Store, job: &ContainerJob, now: i64) {
    let receipt = limeos_domain::RestartReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&json(&job.plan).unwrap()),
        operation: job.plan.operation,
        before: job.plan.expected.clone(),
        state: limeos_domain::ExecutionState::PreconditionChanged,
        error: Some(ErrorCode::Expired),
    };
    assert_eq!(
        store
            .record_container_result(job, &receipt, None, now)
            .unwrap(),
        JobState::PreconditionChanged
    );
}
fn superseded_events(store: &Store, p: &Principal, job: &str) -> Vec<EventKind> {
    store
        .container_events(p, job, 0)
        .unwrap()
        .into_iter()
        .map(|e| e.event)
        .filter(|e| matches!(e, EventKind::JobResolutionSuperseded { .. }))
        .collect()
}

/// Recovery settles the job while an operator's request is open: the
/// request can never complete, so it is closed as superseded, durably and
/// once, with an event naming what the job settled as, and the job's history
/// is no longer pinned by an open request.
#[test]
fn a_request_overtaken_by_recovery_is_superseded_once_with_an_event() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    let pending = request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    settle_by_closure(&mut s, &job, 310);
    // Completion cannot apply any more, and the request is not listed for retry.
    assert_eq!(
        s.complete_resolution(&job.id, &confirm(&pending), 311)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert!(s.pending_resolutions().unwrap().is_empty());
    assert_eq!(
        s.supersede_resolutions(312).unwrap(),
        vec![(job.id.clone(), JobState::PreconditionChanged)]
    );
    assert!(s.supersede_resolutions(313).unwrap().is_empty());
    assert_eq!(s.supersede_resolution(&job.id, 314).unwrap(), None);
    assert_eq!(
        superseded_events(&s, &p, &job.id),
        vec![EventKind::JobResolutionSuperseded {
            state: JobState::PreconditionChanged
        }]
    );
    let open: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM job_resolutions WHERE job=?",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 0);
    assert_eq!(s.job_state(&job.id).unwrap(), "precondition_changed");
    resources::check(&s.conn).unwrap();
}

/// The completion that loses the race supersedes the request itself; a
/// request whose job is still held is never superseded.
#[test]
fn only_a_request_whose_job_has_settled_is_superseded() {
    let (_d, mut s, p) = setup();
    let job = running(&mut s, &p, "a", "a");
    uncertain(&mut s, &job.id);
    let e = evidence(&job, &job.plan.expected);
    request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    assert_eq!(s.supersede_resolution(&job.id, 311).unwrap(), None);
    assert!(s.supersede_resolutions(311).unwrap().is_empty());
    assert_eq!(s.pending_resolutions().unwrap().len(), 1);
    settle_by_closure(&mut s, &job, 312);
    assert_eq!(
        s.supersede_resolution(&job.id, 313).unwrap(),
        Some(JobState::PreconditionChanged)
    );
    assert_eq!(s.supersede_resolution(&job.id, 314).unwrap(), None);
    assert_eq!(superseded_events(&s, &p, &job.id).len(), 1);
}

/// A hundred requests overtaken by recovery, older than a request still
/// waiting on its held job: the listing for retry skips them, superseding
/// drains them in bounded batches, and the newer request is listed at once.
#[test]
fn superseded_requests_never_hide_a_newer_request() {
    let (_d, mut s, p) = setup();
    for i in 0..100 {
        let job = running(&mut s, &p, &format!("k{i}"), "a");
        uncertain(&mut s, &job.id);
        let e = evidence(&job, &job.plan.expected);
        request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
        settle_by_closure(&mut s, &job, 310);
    }
    let held = running(&mut s, &p, "held", "a");
    uncertain(&mut s, &held.id);
    let e = evidence(&held, &held.plan.expected);
    let pending = request(&mut s, &p, &held.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
    assert_eq!(s.pending_resolutions().unwrap(), vec![pending.clone()]);
    assert_eq!(s.pending_resolution(&held.id).unwrap(), Some(pending));
    let drained: Vec<usize> = (0..3)
        .map(|_| s.supersede_resolutions(320).unwrap().len())
        .collect();
    assert_eq!(
        drained,
        [crate::RECOVERY_BATCH, 100 - crate::RECOVERY_BATCH, 0]
    );
}

/// Requests whose jobs are held are listed in pages by job ID after a
/// cursor, so a caller that rotates reaches every one however many there
/// are.
#[test]
fn pending_requests_are_listed_in_pages_after_a_cursor() {
    let (_d, mut s, p) = setup();
    let mut jobs = Vec::new();
    for (i, byte) in ["a", "c", "d"].into_iter().enumerate() {
        let job = running(&mut s, &p, &format!("k{i}"), byte);
        uncertain(&mut s, &job.id);
        let e = evidence(&job, &job.plan.expected);
        request(&mut s, &p, &job.id, DeclaredOutcome::NotDone, "ok", &e).unwrap();
        jobs.push(job.id);
    }
    jobs.sort();
    let listed = |s: &Store, cursor: &str| -> Vec<String> {
        s.pending_resolutions_after(cursor)
            .unwrap()
            .into_iter()
            .map(|p| p.request.job)
            .collect()
    };
    assert_eq!(listed(&s, ""), jobs);
    assert_eq!(listed(&s, &jobs[0]), jobs[1..]);
    assert!(listed(&s, &jobs[2]).is_empty());
}

/// A request on a job core still trusts the store for is undisputed; one on
/// a job held across a store change, or with a result core recorded, is
/// disputed, both as recorded and as listed for the retry loop.
#[test]
fn a_request_is_disputed_when_core_holds_a_record_a_closure_would_contradict() {
    let (_dir, mut store, p) = setup();
    let job = running(&mut store, &p, "trusted", "a");
    uncertain(&mut store, &job.id);
    let e = evidence(&job, &job.plan.expected);
    let pending = request(&mut store, &p, &job.id, DeclaredOutcome::NotDone, LONG, &e).unwrap();
    assert!(!pending.disputed);
    store.observe_executor_epoch(&"2".repeat(64), 301).unwrap();
    assert!(store.pending_resolution(&job.id).unwrap().unwrap().disputed);
    assert!(
        request(&mut store, &p, &job.id, DeclaredOutcome::NotDone, LONG, &e)
            .unwrap()
            .disputed
    );
    // A result core recorded disputes a closure too, with no store change.
    let ran = running(&mut store, &p, "ran", "c");
    let receipt = limeos_domain::RestartReceipt {
        action: ran.id.clone(),
        plan_digest: limeos_identity::digest(&json(&ran.plan).unwrap()),
        operation: ran.plan.operation,
        before: ran.plan.expected.clone(),
        state: limeos_domain::ExecutionState::OutcomeUnknown,
        error: Some(ErrorCode::Unavailable),
    };
    store
        .record_container_result(&ran, &receipt, None, 302)
        .unwrap();
    let e = evidence(&ran, &ran.plan.expected);
    assert!(
        request(&mut store, &p, &ran.id, DeclaredOutcome::NotDone, LONG, &e)
            .unwrap()
            .disputed
    );
}
