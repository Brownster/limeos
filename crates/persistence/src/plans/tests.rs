use super::*;

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let principal = store.login_record("alice").unwrap().unwrap().principal;
    (dir, store, principal)
}
fn snapshot() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    }
}
fn approved(store: &mut Store, p: &Principal, at: i64) -> (PlannedRestart, PlanApproval) {
    let proposal = store.plan_restart(p, &snapshot(), at).unwrap();
    let approval = store
        .approve_restart(p, &proposal.plan.id, &proposal.digest, at + 1)
        .unwrap();
    (proposal, approval)
}

#[test]
fn proof_is_bound_to_the_persisted_intent_and_accepted_effect_is_not_success() {
    use limeos_domain::{ExecutionState, RestartReceipt};
    let (_dir, mut store, principal) = setup();
    let (proposal, approval) = approved(&mut store, &principal, 200);
    let job = store
        .queue_restart(
            &principal,
            "proof",
            &proposal,
            &approval.token,
            &snapshot(),
            202,
        )
        .unwrap();
    let job = store
        .claim_restart(&principal, &job.id, &snapshot(), 203)
        .unwrap();
    store
        .transition(&job.id, JobState::Running, JobState::Verifying, 204)
        .unwrap();
    assert_eq!(
        store
            .transition(&job.id, JobState::Verifying, JobState::Succeeded, 204)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let mut receipt = RestartReceipt {
        action: job.id.clone(),
        plan_digest: proposal.digest,
        before: snapshot(),
        state: ExecutionState::EffectAccepted,
        error: None,
    };
    assert_eq!(
        store
            .record_restart_result(&job, &receipt, None, 204)
            .unwrap(),
        JobState::NeedsIntervention
    );
    let retained: String = store
        .conn
        .query_row(
            "SELECT receipt FROM restart_results WHERE job=?",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<RestartReceipt>(&retained).unwrap(),
        receipt
    );
    receipt.state = ExecutionState::Verified;
    assert!(
        store
            .record_restart_result(&job, &receipt, Some(&snapshot()), 205)
            .is_err()
    );
    let changed = ContainerSnapshot {
        started_at: "2026-10-05T07:01:00Z".into(),
        ..snapshot()
    };
    let mut forged = job.clone();
    forged.plan.principal = "forged".into();
    assert!(
        store
            .record_restart_result(&forged, &receipt, Some(&changed), 205)
            .is_err()
    );
    assert_eq!(
        store
            .record_restart_result(&job, &receipt, Some(&changed), 205)
            .unwrap(),
        JobState::Succeeded
    );
    assert_eq!(
        store
            .record_restart_result(&job, &receipt, Some(&changed), 206)
            .unwrap(),
        JobState::Succeeded
    );
}

#[test]
fn v2_to_v3_failure_preserves_old_authority_and_schema() {
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn.execute_batch(include_str!("../migration-v2.sql"))
            .unwrap();
        if fail {
            conn.execute_batch("CREATE TABLE restart_results(dummy TEXT);")
                .unwrap();
        }
        drop(conn);
        let result = Store::open(&path);
        if fail {
            assert!(result.is_err());
            let conn = Connection::open(path).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                2
            );
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn approval_is_plan_bound_hashed_single_use_and_idempotency_survives_expiry() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let stored: String = s
        .conn
        .query_row("SELECT approval_digest FROM restart_plans", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, limeos_identity::digest(&approval.token));
    assert_ne!(stored, approval.token);
    let job = s
        .queue_restart(
            &p,
            "request-1",
            &proposal,
            &approval.token,
            &snapshot(),
            202,
        )
        .unwrap();
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(
        s.queue_restart(
            &p,
            "request-2",
            &proposal,
            &approval.token,
            &snapshot(),
            203
        )
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.approve_restart(&p, &proposal.plan.id, &proposal.digest, 204)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // A lost response can be replayed after expiry or a changed host observation.
    // It returns the original result and does not grant another dispatch.
    let changed = ContainerSnapshot {
        running: false,
        ..snapshot()
    };
    let duplicate = s
        .queue_restart(&p, "request-1", &proposal, &approval.token, &changed, 501)
        .unwrap();
    assert_eq!(duplicate.id, job.id);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(s.restart_events(&p, &job.id, 0).unwrap().len(), 1);
}

#[test]
fn modified_plan_wrong_nonce_stale_resource_and_reused_key_do_not_consume_approval() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let mut altered = proposal.clone();
    altered.plan.expected.resource = format!("container:{}", "d".repeat(64));
    assert_eq!(
        s.queue_restart(&p, "one", &altered, &approval.token, &snapshot(), 202)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.queue_restart(&p, "one", &proposal, &"e".repeat(64), &snapshot(), 202)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let changed = ContainerSnapshot {
        started_at: "2026-10-05T07:00:01Z".into(),
        ..snapshot()
    };
    assert_eq!(
        s.queue_restart(&p, "one", &proposal, &approval.token, &changed, 202)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    s.queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 203)
        .unwrap();
    let (next, nonce) = approved(&mut s, &p, 204);
    assert_eq!(
        s.queue_restart(&p, "one", &next, &nonce.token, &snapshot(), 206)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    s.queue_restart(&p, "two", &next, &nonce.token, &snapshot(), 207)
        .unwrap();
    let (expired, token) = approved(&mut s, &p, 208);
    assert_eq!(
        s.queue_restart(&p, "three", &expired, &token.token, &snapshot(), 508)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
}

#[test]
fn changed_grants_forged_role_and_unscoped_callers_cannot_propose_or_dispatch() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let job = s
        .queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 202)
        .unwrap();
    s.revise_grants(&p.id, Role::Viewer, &[snapshot().scope()], 203)
        .unwrap();
    assert_eq!(
        s.claim_restart(&p, &job.id, &snapshot(), 204)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    let viewer = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.plan_restart(&viewer, &snapshot(), 204).unwrap_err().0,
        ErrorCode::Forbidden
    );
    let forged = Principal {
        role: Role::Administrator,
        ..viewer.clone()
    };
    assert_eq!(
        s.plan_restart(&forged, &snapshot(), 204).unwrap_err().0,
        ErrorCode::Expired
    );
    s.revise_grants(
        &p.id,
        Role::Operator,
        &[Scope {
            operation: Operation::ContainerManage,
            resource: format!("container:{}", "d".repeat(64)),
        }],
        205,
    )
    .unwrap();
    let operator = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.plan_restart(&operator, &snapshot(), 206).unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
}

#[test]
fn approvals_and_events_are_owned_and_hidden_from_other_principals() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let job = s
        .queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 202)
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO users VALUES('bob','bob','$argon2id$fixture','\"administrator\"',1)",
            [],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO grants VALUES('bob','\"container_manage\"','*')",
            [],
        )
        .unwrap();
    let bob = s.login_record("bob").unwrap().unwrap().principal;
    assert_eq!(
        s.restart_plan(&bob, &proposal.plan.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.restart_job(&bob, &job.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.restart_events(&bob, &job.id, 0).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.approve_restart(&p, &proposal.plan.id, &"d".repeat(64), 203)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
}

#[test]
fn canceled_proposals_and_queued_jobs_cannot_be_dispatched_or_resurrected() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    s.cancel_restart_plan(&p, &proposal.plan.id, 202).unwrap();
    assert_eq!(
        s.queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 203)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let (proposal, approval) = approved(&mut s, &p, 204);
    let job = s
        .queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 206)
        .unwrap();
    s.cancel_restart_job(&p, &job.id, 207).unwrap();
    s.cancel_restart_job(&p, &job.id, 208).unwrap();
    assert_eq!(
        s.claim_restart(&p, &job.id, &snapshot(), 209)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 210)
            .unwrap()
            .state,
        JobState::Canceled
    );
    let events = s.restart_events(&p, &job.id, 0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        s.restart_events(&p, &job.id, events[0].cursor).unwrap(),
        events[1..]
    );
}

#[test]
fn uncertain_dispatch_keeps_resource_lock_across_restart_and_never_reports_canceled() {
    let (dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let first = s
        .queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 202)
        .unwrap();
    assert_eq!(
        s.transition(&first.id, JobState::Queued, JobState::Running, 203)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    s.claim_restart(&p, &first.id, &snapshot(), 203).unwrap();
    assert_eq!(
        s.transition(&first.id, JobState::Running, JobState::Canceled, 204)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        s.cancel_restart_job(&p, &first.id, 204).unwrap_err().0,
        ErrorCode::Conflict
    );
    let (proposal, approval) = approved(&mut s, &p, 204);
    let second = s
        .queue_restart(&p, "two", &proposal, &approval.token, &snapshot(), 206)
        .unwrap();
    assert_eq!(
        s.claim_restart(&p, &second.id, &snapshot(), 207)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    drop(s);
    let mut recovered = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        recovered.restart_job(&p, &first.id).unwrap().state,
        JobState::NeedsIntervention
    );
    assert_eq!(
        recovered
            .claim_restart(&p, &second.id, &snapshot(), 208)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        recovered.restart_job(&p, &second.id).unwrap().state,
        JobState::Queued
    );
}

#[test]
fn failed_audit_commit_rolls_back_intent_and_approval_claim() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    s.conn.execute_batch("CREATE TRIGGER force_failure BEFORE INSERT ON events WHEN NEW.job IS NOT NULL BEGIN SELECT RAISE(FAIL,'fixture audit failure'); END;").unwrap();
    assert_eq!(
        s.queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 202)
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
            .query_row("SELECT job FROM restart_plans", [], |r| r
                .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    s.conn.execute_batch("DROP TRIGGER force_failure;").unwrap();
    s.queue_restart(&p, "one", &proposal, &approval.token, &snapshot(), 203)
        .unwrap();
    let intent = Intent::ContainerRestart {
        plan: proposal.plan,
    };
    assert_eq!(
        s.queue_job(&p, "bypass", &intent, 400, 204).unwrap_err().0,
        ErrorCode::Forbidden
    );
}

#[test]
fn v1_migration_preserves_authority_and_failed_migration_rolls_back() {
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn.execute(
            "INSERT INTO users VALUES('alice','alice','$argon2id$fixture','\"administrator\"',1)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE meta SET generation=7", []).unwrap();
        if fail {
            conn.execute_batch("CREATE INDEX restart_plans_owner ON resources(id);")
                .unwrap();
        }
        drop(conn);
        let result = Store::open(&path);
        if fail {
            assert!(result.is_err());
            let conn = Connection::open(&path).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='restart_plans'",
                    [],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row("SELECT username FROM users", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "alice"
            );
        } else {
            let store = result.unwrap();
            assert_eq!(store.generation, 8);
            assert_eq!(
                store
                    .conn
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            assert_eq!(
                store.login_record("alice").unwrap().unwrap().password_hash,
                "$argon2id$fixture"
            );
        }
    }
}

#[tokio::test]
async fn concurrent_queueing_shares_a_job_or_consumes_approval_once() {
    for same_key in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
        let (p, proposal, approval) = db
            .call(|s| {
                s.issue_bootstrap("bootstrap", 100)?;
                s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)?;
                let p = s.login_record("alice")?.unwrap().principal;
                let (proposal, approval) = approved(s, &p, 200);
                Ok((p, proposal, approval))
            })
            .await
            .unwrap();
        let owner = p.clone();
        let plan = proposal.clone();
        let nonce = approval.token.clone();
        let first =
            db.call(move |s| s.queue_restart(&owner, "one", &plan, &nonce, &snapshot(), 202));
        let owner = p.clone();
        let second = db.call(move |s| {
            s.queue_restart(
                &owner,
                if same_key { "one" } else { "two" },
                &proposal,
                &approval.token,
                &snapshot(),
                202,
            )
        });
        let (a, b) = tokio::join!(first, second);
        let a = a.unwrap();
        if same_key {
            assert_eq!(a.id, b.unwrap().id);
        } else {
            assert_eq!(b.unwrap_err().0, ErrorCode::Conflict);
        }
        assert_eq!(
            db.call(move |s| Ok(s.restart_events(&p, &a.id, 0)?.len()))
                .await
                .unwrap(),
            1
        );
    }
}
