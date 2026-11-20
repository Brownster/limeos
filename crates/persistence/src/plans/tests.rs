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
        .approve_container(p, &proposal.plan.id, &proposal.digest, at + 1)
        .unwrap();
    (proposal, approval)
}
#[test]
fn lifecycle_approval_binds_action_and_all_actions_share_the_resource_lock() {
    use limeos_domain::{ContainerAction, ContainerReceipt, ExecutionState};
    let (_dir, mut store, principal) = setup();
    let before = snapshot();
    let proposal = store
        .plan_container(&principal, ContainerAction::Stop, &before, 200)
        .unwrap();
    let approval = store
        .approve_container(&principal, &proposal.plan.id, &proposal.digest, 201)
        .unwrap();
    let mut changed = proposal.clone();
    changed.plan.operation = ContainerAction::Restart;
    assert!(
        store
            .queue_container(
                &principal,
                "wrong-action",
                &changed,
                &approval.token,
                &before,
                202
            )
            .is_err()
    );
    let job = store
        .queue_container(&principal, "stop", &proposal, &approval.token, &before, 202)
        .unwrap();
    let job = store
        .claim_container(&principal, &job.id, &before, &"1".repeat(64), 203)
        .unwrap();
    let competing = approved(&mut store, &principal, 203);
    let next = store
        .queue_container(
            &principal,
            "restart",
            &competing.0,
            &competing.1.token,
            &before,
            205,
        )
        .unwrap();
    assert!(
        store
            .claim_container(&principal, &next.id, &before, &"1".repeat(64), 206)
            .is_err()
    );
    let after = ContainerSnapshot {
        running: false,
        ..before.clone()
    };
    let mut receipt = ContainerReceipt {
        action: job.id.clone(),
        plan_digest: proposal.digest,
        operation: ContainerAction::Restart,
        before,
        state: ExecutionState::Verified,
        error: None,
    };
    assert!(
        store
            .record_container_result(&job, &receipt, Some(&after), 207)
            .is_err()
    );
    receipt.operation = ContainerAction::Stop;
    assert_eq!(
        store
            .record_container_result(&job, &receipt, Some(&after), 207)
            .unwrap(),
        JobState::Succeeded
    );
    let start = store
        .plan_container(&principal, ContainerAction::Start, &after, 208)
        .unwrap();
    let token = store
        .approve_container(&principal, &start.plan.id, &start.digest, 209)
        .unwrap();
    let next = store
        .queue_container(&principal, "start", &start, &token.token, &after, 210)
        .unwrap();
    assert!(
        store
            .claim_container(&principal, &next.id, &after, &"1".repeat(64), 211)
            .is_ok()
    );
}
#[test]
fn v3_migration_preserves_restart_approval_bytes_and_rolls_back_table_renames() {
    for fail in [false, true] {
        let (dir, mut store, principal) = setup();
        let (proposal, approval) = approved(&mut store, &principal, 200);
        remove_v7_schema(&store.conn);
        store.conn.execute_batch("DROP TABLE storage_plans; DROP TABLE compose_plans; ALTER TABLE container_plans RENAME TO restart_plans; ALTER TABLE container_results RENAME TO restart_results; PRAGMA user_version=3;").unwrap();
        if fail {
            store
                .conn
                .execute_batch("CREATE TABLE container_results(conflict TEXT);")
                .unwrap();
        }
        drop(store);
        let path = dir.path().join("core.sqlite");
        let reopened = Store::open(&path);
        if fail {
            assert!(reopened.is_err());
            let conn = Connection::open(&path).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                3
            );
            assert_eq!(
                conn.query_row("SELECT digest FROM restart_plans", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                proposal.digest
            );
        } else {
            let mut store = reopened.unwrap();
            assert_eq!(
                store.container_plan(&principal, &proposal.plan.id).unwrap(),
                proposal
            );
            assert!(
                store
                    .queue_container(
                        &principal,
                        "legacy",
                        &proposal,
                        &approval.token,
                        &snapshot(),
                        202
                    )
                    .is_ok()
            );
        }
    }
}

#[test]
fn proof_is_bound_to_the_persisted_intent_and_accepted_effect_is_not_success() {
    use limeos_domain::{ExecutionState, RestartReceipt};
    let (_dir, mut store, principal) = setup();
    let (proposal, approval) = approved(&mut store, &principal, 200);
    let job = store
        .queue_container(
            &principal,
            "proof",
            &proposal,
            &approval.token,
            &snapshot(),
            202,
        )
        .unwrap();
    let job = store
        .claim_container(&principal, &job.id, &snapshot(), &"1".repeat(64), 203)
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
        operation: limeos_domain::ContainerAction::Restart,
        action: job.id.clone(),
        plan_digest: proposal.digest,
        before: snapshot(),
        state: ExecutionState::EffectAccepted,
        error: None,
    };
    assert_eq!(
        store
            .record_container_result(&job, &receipt, None, 204)
            .unwrap(),
        JobState::NeedsIntervention
    );
    let retained: String = store
        .conn
        .query_row(
            "SELECT receipt FROM container_results WHERE job=?",
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
            .record_container_result(&job, &receipt, Some(&snapshot()), 205)
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
            .record_container_result(&forged, &receipt, Some(&changed), 205)
            .is_err()
    );
    assert_eq!(
        store
            .record_container_result(&job, &receipt, Some(&changed), 205)
            .unwrap(),
        JobState::Succeeded
    );
    assert_eq!(
        store
            .record_container_result(&job, &receipt, Some(&changed), 206)
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
        .query_row("SELECT approval_digest FROM container_plans", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, limeos_identity::digest(&approval.token));
    assert_ne!(stored, approval.token);
    let job = s
        .queue_container(
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
        s.queue_container(
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
        s.approve_container(&p, &proposal.plan.id, &proposal.digest, 204)
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
        .queue_container(&p, "request-1", &proposal, &approval.token, &changed, 501)
        .unwrap();
    assert_eq!(duplicate.id, job.id);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(s.container_events(&p, &job.id, 0).unwrap().len(), 1);
}

#[test]
fn modified_plan_wrong_nonce_stale_resource_and_reused_key_do_not_consume_approval() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let mut altered = proposal.clone();
    altered.plan.expected.resource = format!("container:{}", "d".repeat(64));
    assert_eq!(
        s.queue_container(&p, "one", &altered, &approval.token, &snapshot(), 202)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.queue_container(&p, "one", &proposal, &"e".repeat(64), &snapshot(), 202)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let changed = ContainerSnapshot {
        started_at: "2026-10-05T07:00:01Z".into(),
        ..snapshot()
    };
    assert_eq!(
        s.queue_container(&p, "one", &proposal, &approval.token, &changed, 202)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    s.queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 203)
        .unwrap();
    let (next, nonce) = approved(&mut s, &p, 204);
    assert_eq!(
        s.queue_container(&p, "one", &next, &nonce.token, &snapshot(), 206)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    s.queue_container(&p, "two", &next, &nonce.token, &snapshot(), 207)
        .unwrap();
    let (expired, token) = approved(&mut s, &p, 208);
    assert_eq!(
        s.queue_container(&p, "three", &expired, &token.token, &snapshot(), 508)
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
        .queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 202)
        .unwrap();
    s.revise_grants(&p.id, Role::Viewer, &[snapshot().scope()], 203)
        .unwrap();
    assert_eq!(
        s.claim_container(&p, &job.id, &snapshot(), &"1".repeat(64), 204)
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
        .queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 202)
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
        s.container_plan(&bob, &proposal.plan.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.container_job(&bob, &job.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.container_events(&bob, &job.id, 0).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        s.approve_container(&p, &proposal.plan.id, &"d".repeat(64), 203)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
}

#[test]
fn canceled_proposals_and_queued_jobs_cannot_be_dispatched_or_resurrected() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    s.cancel_container_plan(&p, &proposal.plan.id, 202).unwrap();
    assert_eq!(
        s.queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 203)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let (proposal, approval) = approved(&mut s, &p, 204);
    let job = s
        .queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 206)
        .unwrap();
    s.cancel_container_job(&p, &job.id, 207).unwrap();
    s.cancel_container_job(&p, &job.id, 208).unwrap();
    assert_eq!(
        s.claim_container(&p, &job.id, &snapshot(), &"1".repeat(64), 209)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 210)
            .unwrap()
            .state,
        JobState::Canceled
    );
    let events = s.container_events(&p, &job.id, 0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        s.container_events(&p, &job.id, events[0].cursor).unwrap(),
        events[1..]
    );
}

#[test]
fn uncertain_dispatch_keeps_resource_lock_across_restart_and_never_reports_canceled() {
    let (dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    let first = s
        .queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 202)
        .unwrap();
    assert_eq!(
        s.transition(&first.id, JobState::Queued, JobState::Running, 203)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    s.claim_container(&p, &first.id, &snapshot(), &"1".repeat(64), 203)
        .unwrap();
    assert_eq!(
        s.transition(&first.id, JobState::Running, JobState::Canceled, 204)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        s.cancel_container_job(&p, &first.id, 204).unwrap_err().0,
        ErrorCode::Conflict
    );
    let (proposal, approval) = approved(&mut s, &p, 204);
    let second = s
        .queue_container(&p, "two", &proposal, &approval.token, &snapshot(), 206)
        .unwrap();
    assert_eq!(
        s.claim_container(&p, &second.id, &snapshot(), &"1".repeat(64), 207)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    drop(s);
    let mut recovered = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        recovered.container_job(&p, &first.id).unwrap().state,
        JobState::NeedsIntervention
    );
    assert_eq!(
        recovered
            .claim_container(&p, &second.id, &snapshot(), &"1".repeat(64), 208)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        recovered.container_job(&p, &second.id).unwrap().state,
        JobState::Queued
    );
}

#[test]
fn failed_audit_commit_rolls_back_intent_and_approval_claim() {
    let (_dir, mut s, p) = setup();
    let (proposal, approval) = approved(&mut s, &p, 200);
    s.conn.execute_batch("CREATE TRIGGER force_failure BEFORE INSERT ON events WHEN NEW.job IS NOT NULL BEGIN SELECT RAISE(FAIL,'fixture audit failure'); END;").unwrap();
    assert_eq!(
        s.queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 202)
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
            .query_row("SELECT job FROM container_plans", [], |r| r
                .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    s.conn.execute_batch("DROP TRIGGER force_failure;").unwrap();
    s.queue_container(&p, "one", &proposal, &approval.token, &snapshot(), 203)
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
            db.call(move |s| s.queue_container(&owner, "one", &plan, &nonce, &snapshot(), 202));
        let owner = p.clone();
        let second = db.call(move |s| {
            s.queue_container(
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
            db.call(move |s| Ok(s.container_events(&p, &a.id, 0)?.len()))
                .await
                .unwrap(),
            1
        );
    }
}

#[test]
fn every_held_container_job_is_listed_however_many_older_ones_are_held() {
    let (_dir, mut store, p) = setup();
    let batch = crate::RECOVERY_BATCH;
    let total = batch + 3;
    let mut held = std::collections::BTreeSet::new();
    for i in 0..total {
        let target = ContainerSnapshot {
            resource: format!("container:{:064x}", i + 1),
            ..snapshot()
        };
        let at = 200 + i as i64;
        let proposal = store.plan_restart(&p, &target, at).unwrap();
        let approval = store
            .approve_container(&p, &proposal.plan.id, &proposal.digest, at)
            .unwrap();
        let job = store
            .queue_container(
                &p,
                &format!("k{i}"),
                &proposal,
                &approval.token,
                &target,
                at,
            )
            .unwrap();
        store
            .claim_container(&p, &job.id, &target, &"1".repeat(64), at)
            .unwrap();
        held.insert(job.id);
    }
    let first = store.container_candidates().unwrap();
    assert_eq!(first.len(), batch);
    let last = first.last().unwrap().1.id.clone();
    let second = store.container_candidates_after(&last).unwrap();
    assert_eq!(second.len(), total - batch);
    let seen: std::collections::BTreeSet<String> = first
        .into_iter()
        .chain(second)
        .map(|(_, job)| job.id)
        .collect();
    assert_eq!(seen, held);
}

/// A claimed restart job on the fixture container and its closure receipt,
/// as a closing lookup answers it for an action it never recorded.
fn claimed_with_closure(
    store: &mut Store,
    p: &Principal,
    key: &str,
    at: i64,
) -> (RestartJob, limeos_domain::RestartReceipt) {
    let (proposal, approval) = approved(store, p, at);
    let job = store
        .queue_container(p, key, &proposal, &approval.token, &snapshot(), at + 2)
        .unwrap();
    let job = store
        .claim_container(p, &job.id, &snapshot(), &"1".repeat(64), at + 3)
        .unwrap();
    let closure = limeos_domain::RestartReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&json(&job.plan).unwrap()),
        operation: job.plan.operation,
        before: job.plan.expected.clone(),
        state: limeos_domain::ExecutionState::PreconditionChanged,
        error: Some(ErrorCode::Expired),
    };
    (job, closure)
}
fn count_events(store: &Store, job: &str, kind: &EventKind) -> i64 {
    store
        .conn
        .query_row(
            "SELECT count(*) FROM events WHERE job=? AND kind=?",
            params![job, json(kind).unwrap()],
            |r| r.get(0),
        )
        .unwrap()
}
fn held_locks(store: &Store, job: &str) -> i64 {
    store
        .conn
        .query_row(
            "SELECT count(*) FROM resource_locks WHERE job=?",
            [job],
            |r| r.get(0),
        )
        .unwrap()
}

/// With nothing against it, a closure releases the job like any refusal,
/// and a repeat changes nothing.
#[test]
fn an_undisputed_closure_releases_the_job_once() {
    let (_dir, mut store, p) = setup();
    store.observe_executor_epoch(&"1".repeat(64), 200).unwrap();
    let (job, closure) = claimed_with_closure(&mut store, &p, "closed", 200);
    for _ in 0..2 {
        assert_eq!(
            store.record_container_closure(&job, &closure, 210).unwrap(),
            JobState::PreconditionChanged
        );
    }
    assert_eq!(held_locks(&store, &job.id), 0);
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorRecordDisagrees),
        0
    );
}

/// Core already holds a result for the job: the executor ran it and then
/// lost its store. A closure saying it never recorded the action never
/// overwrites that result or releases the job.
#[test]
fn a_closure_never_overwrites_a_result_core_already_holds() {
    use limeos_domain::ExecutionState;
    let (_dir, mut store, p) = setup();
    let (job, closure) = claimed_with_closure(&mut store, &p, "ran", 200);
    let ran = limeos_domain::RestartReceipt {
        state: ExecutionState::OutcomeUnknown,
        error: Some(ErrorCode::Unavailable),
        ..closure.clone()
    };
    assert_eq!(
        store
            .record_container_result(&job, &ran, None, 205)
            .unwrap(),
        JobState::NeedsIntervention
    );
    let stored = |store: &Store| -> String {
        store
            .conn
            .query_row(
                "SELECT receipt FROM container_results WHERE job=?",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let before = stored(&store);
    let mut held = job.clone();
    held.state = JobState::NeedsIntervention;
    for _ in 0..2 {
        assert_eq!(
            store
                .record_container_closure(&held, &closure, 210)
                .unwrap(),
            JobState::NeedsIntervention
        );
    }
    assert_eq!(stored(&store), before);
    assert_eq!(store.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(held_locks(&store, &job.id), 1);
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorRecordDisagrees),
        1
    );
}

/// The executor's store changed (a new epoch) while the job was held: its
/// closure is not accepted, the job stays held, and a job dispatched after
/// the change is released as usual.
#[test]
fn a_new_store_epoch_marks_held_jobs_and_their_closures_are_refused() {
    let (_dir, mut store, p) = setup();
    // The first epoch seen, with nothing held, marks nothing; repeating it
    // writes nothing.
    assert_eq!(
        store.observe_executor_epoch(&"1".repeat(64), 200).unwrap(),
        0
    );
    let (job, closure) = claimed_with_closure(&mut store, &p, "before", 200);
    assert_eq!(
        store.observe_executor_epoch(&"1".repeat(64), 205).unwrap(),
        0
    );
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorStoreChanged),
        0
    );
    // A new epoch: the held job is marked once, however often it changes.
    assert_eq!(
        store.observe_executor_epoch(&"2".repeat(64), 206).unwrap(),
        1
    );
    assert_eq!(
        store.observe_executor_epoch(&"3".repeat(64), 207).unwrap(),
        0
    );
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorStoreChanged),
        1
    );
    assert_eq!(
        store.record_container_closure(&job, &closure, 210).unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(held_locks(&store, &job.id), 1);
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorRecordDisagrees),
        1
    );
    assert!(store.observe_executor_epoch("not an epoch", 211).is_err());
}

/// The first epoch core ever records (a fresh install, or the first answer
/// after the upgrade that added it) is a change like any other: a job held
/// then was dispatched under no known epoch, so its store may since have been
/// replaced, and its closure is not accepted. Once per job, as for any change.
#[test]
fn the_first_epoch_recorded_marks_every_job_already_held() {
    let (_dir, mut store, p) = setup();
    let (job, closure) = claimed_with_closure(&mut store, &p, "unknown", 200);
    // Claimed before any epoch was known, as by a core before the upgrade.
    store
        .conn
        .execute("UPDATE meta SET executor_epoch=NULL WHERE id=1", [])
        .unwrap();
    assert_eq!(
        store.observe_executor_epoch(&"1".repeat(64), 205).unwrap(),
        1
    );
    assert_eq!(
        store.observe_executor_epoch(&"1".repeat(64), 206).unwrap(),
        0
    );
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorStoreChanged),
        1
    );
    assert_eq!(
        store.record_container_closure(&job, &closure, 210).unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(held_locks(&store, &job.id), 1);
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorRecordDisagrees),
        1
    );
}

/// A claim records the epoch it dispatches under in its own transaction,
/// before it takes the job: a job already held is marked by a new epoch, and
/// the job claimed under it is not, so its closure releases it while the
/// other's is refused.
#[test]
fn a_claim_records_its_epoch_before_it_takes_the_job() {
    let (_dir, mut store, p) = setup();
    let (earlier, earlier_closure) = claimed_with_closure(&mut store, &p, "earlier", 200);
    let other = ContainerSnapshot {
        resource: format!("container:{}", "c".repeat(64)),
        ..snapshot()
    };
    let proposal = store.plan_restart(&p, &other, 210).unwrap();
    let approval = store
        .approve_container(&p, &proposal.plan.id, &proposal.digest, 211)
        .unwrap();
    let queued = store
        .queue_container(&p, "later", &proposal, &approval.token, &other, 212)
        .unwrap();
    let later = store
        .claim_container(&p, &queued.id, &other, &"2".repeat(64), 213)
        .unwrap();
    assert_eq!(
        count_events(&store, &earlier.id, &EventKind::ExecutorStoreChanged),
        1
    );
    assert_eq!(
        count_events(&store, &later.id, &EventKind::ExecutorStoreChanged),
        0
    );
    let later_closure = limeos_domain::RestartReceipt {
        action: later.id.clone(),
        plan_digest: limeos_identity::digest(&json(&later.plan).unwrap()),
        before: later.plan.expected.clone(),
        ..earlier_closure.clone()
    };
    assert_eq!(
        store
            .record_container_closure(&later, &later_closure, 214)
            .unwrap(),
        JobState::PreconditionChanged
    );
    assert_eq!(
        store
            .record_container_closure(&earlier, &earlier_closure, 214)
            .unwrap(),
        JobState::NeedsIntervention
    );
    // A claim refused by a held lock records nothing: the job stays queued
    // and no epoch changes.
    let (proposal, approval) = approved(&mut store, &p, 220);
    let blocked = store
        .queue_container(&p, "blocked", &proposal, &approval.token, &snapshot(), 222)
        .unwrap();
    assert!(
        store
            .claim_container(&p, &blocked.id, &snapshot(), &"3".repeat(64), 223)
            .is_err()
    );
    assert_eq!(
        store.observe_executor_epoch(&"2".repeat(64), 224).unwrap(),
        0
    );
}

/// An execute request that never ran (never sent, or refused before the
/// executor recorded anything) releases its just-claimed job as a refusal;
/// nothing else qualifies.
#[test]
fn an_unsent_execute_releases_only_a_just_claimed_job() {
    let (_dir, mut store, p) = setup();
    let (job, _) = claimed_with_closure(&mut store, &p, "unsent", 200);
    store.record_unsent_execute(&job.id, 205).unwrap();
    assert_eq!(store.job_state(&job.id).unwrap(), "precondition_changed");
    assert_eq!(held_locks(&store, &job.id), 0);
    assert_eq!(
        store.record_unsent_execute(&job.id, 206).unwrap_err().0,
        ErrorCode::Conflict
    );
    // A job held for reconciliation may have been sent: never released so.
    let (held, _) = claimed_with_closure(&mut store, &p, "held", 210);
    store
        .transition(
            &held.id,
            JobState::Running,
            JobState::NeedsIntervention,
            211,
        )
        .unwrap();
    assert_eq!(
        store.record_unsent_execute(&held.id, 212).unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(held_locks(&store, &held.id), 1);
}

#[test]
fn a_store_epoch_change_leaves_queued_and_settled_jobs_alone() {
    let (_dir, mut store, p) = setup();
    store.observe_executor_epoch(&"1".repeat(64), 200).unwrap();
    let (settled, closure) = claimed_with_closure(&mut store, &p, "settled", 200);
    store
        .record_container_closure(&settled, &closure, 205)
        .unwrap();
    let (proposal, approval) = approved(&mut store, &p, 206);
    let queued = store
        .queue_container(&p, "queued", &proposal, &approval.token, &snapshot(), 208)
        .unwrap();
    assert_eq!(
        store.observe_executor_epoch(&"2".repeat(64), 209).unwrap(),
        0
    );
    for job in [&settled.id, &queued.id] {
        assert_eq!(
            count_events(&store, job, &EventKind::ExecutorStoreChanged),
            0
        );
    }
}

/// Schema 18 adds the epoch column; an upgraded store has seen none yet and
/// keeps every row. The first epoch it records marks the job held across the
/// upgrade, which was dispatched under none.
#[test]
fn schema_seventeen_upgrade_adds_the_executor_epoch_and_keeps_every_row() {
    let (dir, mut store, p) = setup();
    let path = dir.path().join("core.sqlite");
    let (job, _) = claimed_with_closure(&mut store, &p, "kept", 200);
    let jobs = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
            .unwrap()
    };
    let before = jobs(&store);
    crate::remove_v18_schema(&store.conn);
    store.conn.execute_batch("PRAGMA user_version=17").unwrap();
    drop(store);
    assert_eq!(crate::upgrade_space(&path), None);
    let mut store = Store::open(&path).unwrap();
    let version: u32 = store
        .conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(jobs(&store), before);
    let epoch: Option<String> = store
        .conn
        .query_row("SELECT executor_epoch FROM meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(epoch, None);
    assert_eq!(store.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(
        store.observe_executor_epoch(&"1".repeat(64), 300).unwrap(),
        1
    );
    assert_eq!(
        count_events(&store, &job.id, &EventKind::ExecutorStoreChanged),
        1
    );
}
