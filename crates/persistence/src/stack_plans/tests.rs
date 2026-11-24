use super::*;
use limeos_domain::{StackMemberReceipt, aggregate_state};

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
fn approved(
    store: &mut Store,
    p: &Principal,
    op: ContainerAction,
    at: i64,
) -> (PlannedStackAction, String) {
    let proposal = store.plan_stack(p, op, "registered", &stack(), at).unwrap();
    let approval = store
        .approve_stack(p, &proposal.plan.id, &proposal.digest, at + 1)
        .unwrap();
    (proposal, approval.token)
}
fn queued(store: &mut Store, p: &Principal, op: ContainerAction, key: &str, at: i64) -> StackJob {
    let (proposal, token) = approved(store, p, op, at);
    store
        .queue_stack(p, key, &proposal, &token, &stack(), at + 2)
        .unwrap()
}
fn receipt(job: &StackJob, states: &[ExecutionState]) -> StackReceipt {
    let members: Vec<_> = job
        .plan
        .targets()
        .iter()
        .zip(states)
        .map(|(t, s)| StackMemberReceipt {
            resource: t.resource.clone(),
            state: *s,
            error: None,
        })
        .collect();
    StackReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&json(&job.plan).unwrap()),
        operation: job.plan.operation,
        before: job.plan.expected.clone(),
        state: aggregate_state(&members),
        members,
        error: None,
    }
}
fn restarted(job: &StackJob) -> Vec<ContainerSnapshot> {
    job.plan
        .targets()
        .into_iter()
        .map(|t| ContainerSnapshot {
            started_at: "2026-10-05T07:05:00Z".into(),
            ..t.clone()
        })
        .collect()
}
fn locks(store: &Store, job: &str) -> Vec<String> {
    let mut stmt = store
        .conn
        .prepare("SELECT resource FROM resource_locks WHERE job=? ORDER BY resource")
        .unwrap();
    stmt.query_map([job], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

#[test]
fn approved_stack_job_locks_the_stack_and_every_member_then_verifies_each_target() {
    let (_dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "restart", 200);
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(store.stack_jobs(&p).unwrap().len(), 1);
    // The container job listing is unaffected by stack jobs.
    assert!(store.container_jobs(&p).unwrap().is_empty());
    let running = store
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    assert_eq!(running.state, JobState::Running);
    assert_eq!(locks(&store, &job.id).len(), 5);
    assert!(locks(&store, &job.id).contains(&"stack:media".to_string()));
    // The registered catalog stack is locked too, so deploy jobs exclude it.
    assert!(locks(&store, &job.id).contains(&"stack:registered".to_string()));
    let after = restarted(&job);
    let done = receipt(&job, &[ExecutionState::Verified; 2]);
    // A missing or unproven member never succeeds.
    assert!(
        store.record_stack_result(&job, &done, None, 206).unwrap() == JobState::NeedsIntervention
    );
    assert!(
        store
            .record_stack_result(&job, &done, Some(&after[..1]), 207)
            .is_err()
    );
    assert_eq!(
        store
            .record_stack_result(&job, &done, Some(&after), 208)
            .unwrap(),
        JobState::Succeeded
    );
    assert!(locks(&store, &job.id).is_empty());
    assert_eq!(
        store.stack_job(&p, &job.id).unwrap().state,
        JobState::Succeeded
    );
    // Recording the same result again is idempotent.
    assert_eq!(
        store
            .record_stack_result(&job, &done, Some(&after), 209)
            .unwrap(),
        JobState::Succeeded
    );
    let (stored, verification) = store.stack_result(&p, &job.id).unwrap().unwrap();
    assert_eq!(stored, done);
    assert_eq!(verification.unwrap(), after);
    assert!(store.stack_events(&p, &job.id, 0).unwrap().len() >= 3);
}

#[test]
fn partial_failure_keeps_per_member_evidence_and_all_locks() {
    let (_dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "restart", 200);
    store
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    let mut partial = receipt(
        &job,
        &[ExecutionState::Verified, ExecutionState::OutcomeUnknown],
    );
    partial.members[1].error = Some(ErrorCode::Unavailable);
    let after = restarted(&job);
    assert_eq!(
        store
            .record_stack_result(&job, &partial, Some(&after[..1]), 206)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(locks(&store, &job.id).len(), 5);
    let (stored, _) = store.stack_result(&p, &job.id).unwrap().unwrap();
    assert_eq!(stored.members[0].state, ExecutionState::Verified);
    assert_eq!(stored.members[1].error, Some(ErrorCode::Unavailable));
    // A later fresh proof of the second target may finish the job.
    let done = receipt(&job, &[ExecutionState::Verified; 2]);
    assert_eq!(
        store
            .record_stack_result(&job, &done, Some(&after), 207)
            .unwrap(),
        JobState::Succeeded
    );
    assert!(locks(&store, &job.id).is_empty());
}

#[test]
fn receipts_for_another_plan_digest_or_job_are_refused() {
    let (_dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "restart", 200);
    store
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    for mutate in [0, 1, 2, 3] {
        let mut r = receipt(&job, &[ExecutionState::Verified; 2]);
        match mutate {
            0 => r.action = "f".repeat(64),
            1 => r.plan_digest = "f".repeat(64),
            2 => r.operation = ContainerAction::Stop,
            _ => r.before.members[2].running = true,
        }
        assert!(
            store
                .record_stack_result(&job, &r, Some(&restarted(&job)), 206)
                .is_err()
        );
    }
    // A forged aggregate is refused before it reaches storage.
    let mut forged = receipt(&job, &[ExecutionState::OutcomeUnknown; 2]);
    forged.state = ExecutionState::Verified;
    assert!(
        store
            .record_stack_result(&job, &forged, Some(&restarted(&job)), 206)
            .is_err()
    );
    assert_eq!(
        store.stack_job(&p, &job.id).unwrap().state,
        JobState::Running
    );
}

#[test]
fn drift_after_planning_is_refused_at_queue_and_claim_without_partial_guesses() {
    let (_dir, mut store, p) = setup();
    let (proposal, token) = approved(&mut store, &p, ContainerAction::Restart, 200);
    for changed in [
        vec![member('a', true), member('b', true)],
        vec![
            member('a', true),
            member('b', true),
            member('c', false),
            member('d', true),
        ],
        vec![member('a', true), member('b', true), member('c', true)],
    ] {
        let current = StackSnapshot::new("media", changed).unwrap();
        assert_eq!(
            store
                .queue_stack(&p, "restart", &proposal, &token, &current, 203)
                .unwrap_err()
                .0,
            ErrorCode::Conflict
        );
    }
    let job = store
        .queue_stack(&p, "restart", &proposal, &token, &stack(), 204)
        .unwrap();
    let drifted = StackSnapshot::new(
        "media",
        vec![
            member('a', true),
            ContainerSnapshot {
                started_at: "2026-10-05T07:09:00Z".into(),
                ..member('b', true)
            },
            member('c', false),
        ],
    )
    .unwrap();
    assert_eq!(
        store
            .claim_stack(&p, &job.id, &drifted, &"1".repeat(64), 205)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        store.stack_job(&p, &job.id).unwrap().state,
        JobState::Queued
    );
    assert!(locks(&store, &job.id).is_empty());
    // Expiry also refuses the claim.
    assert_eq!(
        store
            .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 600)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
}

#[test]
fn stack_and_single_container_jobs_exclude_each_other_in_both_directions() {
    let (_dir, mut store, p) = setup();
    let first = queued(&mut store, &p, ContainerAction::Restart, "stack-one", 200);
    store
        .claim_stack(&p, &first.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    // A single-container job on a member cannot start while the stack runs.
    let single = member('a', true);
    let plan = store
        .plan_container(&p, ContainerAction::Restart, &single, 206)
        .unwrap();
    let token = store
        .approve_container(&p, &plan.plan.id, &plan.digest, 207)
        .unwrap();
    let container = store
        .queue_container(&p, "single", &plan, &token.token, &single, 208)
        .unwrap();
    assert!(
        store
            .claim_container(&p, &container.id, &single, &"1".repeat(64), 209)
            .is_err()
    );
    // A second stack job is excluded by the stack resource itself.
    let second = queued(&mut store, &p, ContainerAction::Stop, "stack-two", 210);
    assert!(
        store
            .claim_stack(&p, &second.id, &stack(), &"1".repeat(64), 215)
            .is_err()
    );
    assert_eq!(
        store.stack_job(&p, &second.id).unwrap().state,
        JobState::Queued
    );
    // Releasing the first (any terminal state) lets the queued work proceed.
    let done = receipt(&first, &[ExecutionState::Verified; 2]);
    store
        .record_stack_result(&first, &done, Some(&restarted(&first)), 216)
        .unwrap();
    store
        .claim_container(&p, &container.id, &single, &"1".repeat(64), 217)
        .unwrap();
    // Reverse direction: the running container job blocks the stack.
    assert!(
        store
            .claim_stack(&p, &second.id, &stack(), &"1".repeat(64), 218)
            .is_err()
    );
    assert_eq!(
        store.stack_job(&p, &second.id).unwrap().state,
        JobState::Queued
    );
}

#[test]
fn queue_replays_by_idempotency_key_and_rejects_reuse_for_other_plans() {
    let (_dir, mut store, p) = setup();
    let (proposal, token) = approved(&mut store, &p, ContainerAction::Restart, 200);
    assert!(
        store
            .replay_stack(&p, "key", &proposal, &token)
            .unwrap()
            .is_none()
    );
    let job = store
        .queue_stack(&p, "key", &proposal, &token, &stack(), 203)
        .unwrap();
    let replay = store
        .replay_stack(&p, "key", &proposal, &token)
        .unwrap()
        .unwrap();
    assert_eq!(replay.id, job.id);
    // The lost-response retry does not need a current inspection.
    let again = store
        .queue_stack(&p, "key", &proposal, &token, &stack(), 204)
        .unwrap();
    assert_eq!(again.id, job.id);
    assert_eq!(store.stack_jobs(&p).unwrap().len(), 1);
    let other = approved(&mut store, &p, ContainerAction::Stop, 205);
    assert!(store.replay_stack(&p, "key", &other.0, &other.1).is_err());
    assert!(
        store
            .queue_stack(&p, "key", &other.0, &other.1, &stack(), 207)
            .is_err()
    );
    // The approval is single use for its plan and bound to its holder.
    assert!(
        store
            .queue_stack(&p, "other-key", &proposal, &token, &stack(), 208)
            .is_err()
    );
    let (fresh, _) = approved(&mut store, &p, ContainerAction::Restart, 209);
    assert_eq!(
        store
            .queue_stack(&p, "wrong", &fresh, &"9".repeat(64), &stack(), 211)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let mut edited = fresh.clone();
    edited.plan.operation = ContainerAction::Stop;
    assert!(
        store
            .queue_stack(&p, "edited", &edited, &token, &stack(), 211)
            .is_err()
    );
}

#[test]
fn authorization_requires_the_stack_and_every_member_and_is_rechecked() {
    let (_dir, mut store, p) = setup();
    let grant = |resource: &str| Scope {
        operation: Operation::ContainerManage,
        resource: resource.into(),
    };
    let revise = |store: &mut Store, scopes: &[Scope], at| {
        store
            .revise_grants(&p.id, Role::Administrator, scopes, at)
            .unwrap();
        store.login_record("alice").unwrap().unwrap().principal
    };
    // The stack alone reaches no member.
    let stack_only = revise(&mut store, &[grant("stack:media")], 150);
    assert_eq!(
        store
            .plan_stack(
                &stack_only,
                ContainerAction::Restart,
                "registered",
                &stack(),
                200
            )
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    // Members alone do not reach the stack.
    let members_only = revise(
        &mut store,
        &[
            grant(&member('a', true).resource),
            grant(&member('b', true).resource),
            grant(&member('c', true).resource),
        ],
        151,
    );
    assert_eq!(
        store
            .plan_stack(
                &members_only,
                ContainerAction::Restart,
                "registered",
                &stack(),
                200
            )
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    // The registered catalog stack must be granted as well as the project.
    let mut no_catalog = vec![grant("stack:media")];
    no_catalog.extend(stack().members.iter().map(|m| grant(&m.resource)));
    let no_catalog = revise(&mut store, &no_catalog, 151);
    assert_eq!(
        store
            .plan_stack(
                &no_catalog,
                ContainerAction::Restart,
                "registered",
                &stack(),
                200
            )
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let mut all = vec![grant("stack:media"), grant("stack:registered")];
    all.extend(stack().members.iter().map(|m| grant(&m.resource)));
    let exact = revise(&mut store, &all, 152);
    let proposal = store
        .plan_stack(
            &exact,
            ContainerAction::Restart,
            "registered",
            &stack(),
            201,
        )
        .unwrap();
    let approval = store
        .approve_stack(&exact, &proposal.plan.id, &proposal.digest, 202)
        .unwrap();
    // A role downgrade or revoked grant invalidates the stored plan and job.
    let job = store
        .queue_stack(&exact, "key", &proposal, &approval.token, &stack(), 203)
        .unwrap();
    let revoked = revise(&mut store, &all[..2], 204);
    assert_eq!(
        store.stack_job(&revoked, &job.id).unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert!(store.stack_jobs(&revoked).unwrap().is_empty());
    assert_eq!(
        store
            .claim_stack(&revoked, &job.id, &stack(), &"1".repeat(64), 205)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let viewer = {
        store.revise_grants(&p.id, Role::Viewer, &all, 206).unwrap();
        store.login_record("alice").unwrap().unwrap().principal
    };
    assert!(
        store
            .plan_stack(
                &viewer,
                ContainerAction::Restart,
                "registered",
                &stack(),
                207
            )
            .is_err()
    );
    assert!(
        store
            .approve_stack(&viewer, &proposal.plan.id, &proposal.digest, 207)
            .is_err()
    );
}

#[test]
fn approval_digest_cancel_and_in_flight_cancellation_follow_the_container_rules() {
    let (_dir, mut store, p) = setup();
    let proposal = store
        .plan_stack(&p, ContainerAction::Stop, "registered", &stack(), 200)
        .unwrap();
    assert_eq!(
        store
            .approve_stack(&p, &proposal.plan.id, &"0".repeat(64), 201)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .approve_stack(&p, &proposal.plan.id, &proposal.digest, 500)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    store.cancel_stack_plan(&p, &proposal.plan.id, 202).unwrap();
    assert_eq!(
        store.stack_plan(&p, &proposal.plan.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    // A stack with nothing for the action is refused at the plan.
    let stopped = StackSnapshot::new("media", vec![member('a', false)]).unwrap();
    assert_eq!(
        store
            .plan_stack(&p, ContainerAction::Restart, "registered", &stopped, 203)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    let job = queued(&mut store, &p, ContainerAction::Restart, "one", 210);
    store.cancel_stack_job(&p, &job.id, 215).unwrap();
    assert_eq!(
        store.stack_job(&p, &job.id).unwrap().state,
        JobState::Canceled
    );
    let running = queued(&mut store, &p, ContainerAction::Restart, "two", 220);
    store
        .claim_stack(&p, &running.id, &stack(), &"1".repeat(64), 225)
        .unwrap();
    assert_eq!(
        store.cancel_stack_job(&p, &running.id, 226).unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(store.stack_candidates().unwrap().len(), 1);
}

#[test]
fn restart_recovery_makes_interrupted_stack_jobs_uncertain_and_never_queued() {
    let (dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "key", 200);
    store
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let candidates = store.stack_candidates().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].1.state, JobState::NeedsIntervention);
    assert_eq!(locks(&store, &job.id).len(), 5);
}

#[test]
fn schema_fourteen_upgrade_adds_stack_tables_and_preserves_container_plans() {
    let (dir, mut store, p) = setup();
    let container = store
        .plan_container(&p, ContainerAction::Restart, &member('a', true), 200)
        .unwrap();
    crate::remove_v16_schema(&store.conn);
    store
        .conn
        .execute_batch("DROP TABLE stack_results; DROP TABLE stack_plans; PRAGMA user_version=14;")
        .unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(
        store.container_plan(&p, &container.plan.id).unwrap(),
        container
    );
    store
        .plan_stack(&p, ContainerAction::Restart, "registered", &stack(), 210)
        .unwrap();
}

#[test]
fn partly_done_releases_locks_as_failed_never_succeeded_and_needs_proof_of_sent_members() {
    let (_dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "partly", 200);
    store
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 205)
        .unwrap();
    let partly = receipt(
        &job,
        &[
            ExecutionState::Verified,
            ExecutionState::PreconditionChanged,
        ],
    );
    assert_eq!(partly.state, ExecutionState::PartlyDone);
    let after = restarted(&job);
    // Without the sent member's own proof nothing is released.
    assert_eq!(
        store.record_stack_result(&job, &partly, None, 206).unwrap(),
        JobState::NeedsIntervention
    );
    assert!(
        store
            .record_stack_result(&job, &partly, Some(&[]), 206)
            .is_err()
    );
    assert_eq!(locks(&store, &job.id).len(), 5);
    assert_eq!(
        store
            .record_stack_result(&job, &partly, Some(&after[..1]), 207)
            .unwrap(),
        JobState::Failed
    );
    assert_eq!(
        store.stack_job(&p, &job.id).unwrap().state,
        JobState::Failed
    );
    assert!(locks(&store, &job.id).is_empty());
    let (stored, _) = store.stack_result(&p, &job.id).unwrap().unwrap();
    assert_eq!(stored.state, ExecutionState::PartlyDone);
    // An unknown sent member never reaches the terminal outcome.
    let other = queued(&mut store, &p, ContainerAction::Restart, "unknown", 210);
    store
        .claim_stack(&p, &other.id, &stack(), &"1".repeat(64), 215)
        .unwrap();
    let unknown = receipt(
        &other,
        &[
            ExecutionState::OutcomeUnknown,
            ExecutionState::PreconditionChanged,
        ],
    );
    assert_eq!(
        store
            .record_stack_result(&other, &unknown, Some(&restarted(&other)), 216)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(locks(&store, &other.id).len(), 5);
}

/// Pages through held candidates the way core's recovery rotation does.
fn rotation_pages(store: &Store, pages: usize) -> Vec<Vec<StackJob>> {
    let mut cursor = String::new();
    (0..pages)
        .map(|_| {
            let page: Vec<StackJob> = store
                .stack_candidates_after(&cursor)
                .unwrap()
                .into_iter()
                .map(|(_, job)| job)
                .collect();
            let held: Vec<&StackJob> = page
                .iter()
                .filter(|job| job.state != JobState::Queued)
                .collect();
            cursor = match held.last() {
                Some(last) if held.len() >= crate::RECOVERY_BATCH => last.id.clone(),
                _ => String::new(),
            };
            page
        })
        .collect()
}

#[test]
fn every_held_stack_job_is_listed_however_many_older_ones_are_held() {
    let (_dir, mut store, p) = setup();
    let batch = crate::RECOVERY_BATCH;
    let total = 2 * batch + 2;
    let job_for = |store: &mut Store, i: usize, claim: bool| {
        // Each stack has its own project, catalog name and members, so every
        // job can hold its locks at once.
        let members = (0..2)
            .map(|m| ContainerSnapshot {
                resource: format!("container:{:064x}", 4 * i + m + 1),
                ..member('a', true)
            })
            .collect();
        let snapshot = StackSnapshot::new(&format!("p{i}"), members).unwrap();
        let at = 200 + i as i64;
        let proposal = store
            .plan_stack(
                &p,
                ContainerAction::Restart,
                &format!("cat{i}"),
                &snapshot,
                at,
            )
            .unwrap();
        let approval = store
            .approve_stack(&p, &proposal.plan.id, &proposal.digest, at)
            .unwrap();
        let job = store
            .queue_stack(
                &p,
                &format!("k{i}"),
                &proposal,
                &approval.token,
                &snapshot,
                at,
            )
            .unwrap();
        if claim {
            store
                .claim_stack(&p, &job.id, &snapshot, &"1".repeat(64), at)
                .unwrap();
        }
        job.id
    };
    let held: std::collections::BTreeSet<String> =
        (0..total).map(|i| job_for(&mut store, i, true)).collect();
    let queued = job_for(&mut store, total, false);
    // One listing holds a batch of held jobs; the rest wait for later pages.
    let first = store.stack_candidates().unwrap();
    assert_eq!(
        first
            .iter()
            .filter(|(_, job)| job.state != JobState::Queued)
            .count(),
        batch
    );
    // Three pages reach every held job, the fourth starts over, and the
    // queued job is offered on every page.
    let pages = rotation_pages(&store, 4);
    let mut seen = std::collections::BTreeSet::new();
    for page in &pages {
        assert_eq!(page[0].id, queued);
        seen.extend(
            page.iter()
                .filter(|job| job.state != JobState::Queued)
                .map(|job| job.id.clone()),
        );
    }
    assert_eq!(seen, held);
    assert_eq!(pages[3].len(), pages[0].len());
    assert_eq!(pages[3][1].id, pages[0][1].id);
}

/// A stack job's closure receipt, as a closing lookup answers it.
fn closure_of(job: &StackJob) -> StackReceipt {
    let base = receipt(job, &[ExecutionState::PreconditionChanged; 2]);
    StackReceipt {
        error: Some(ErrorCode::Expired),
        members: base
            .members
            .iter()
            .map(|m| StackMemberReceipt {
                error: Some(ErrorCode::Expired),
                ..m.clone()
            })
            .collect(),
        ..base
    }
}
fn disagreements(s: &Store, job: &str) -> i64 {
    s.conn
        .query_row(
            "SELECT count(*) FROM events WHERE job=? AND kind=?",
            params![job, json(&EventKind::ExecutorRecordDisagrees).unwrap()],
            |r| r.get(0),
        )
        .unwrap()
}

/// As for a container job: a closure never overwrites a stack result core
/// already holds (the executor ran the stack, then lost its store).
#[test]
fn a_stack_closure_never_overwrites_a_result_core_already_holds() {
    let (_dir, mut s, p) = setup();
    let job = queued(&mut s, &p, ContainerAction::Restart, "ran", 200);
    let job = s
        .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 203)
        .unwrap();
    let held = locks(&s, &job.id);
    let ran = receipt(
        &job,
        &[
            ExecutionState::EffectAccepted,
            ExecutionState::OutcomeUnknown,
        ],
    );
    s.record_stack_result(&job, &ran, None, 204).unwrap();
    for _ in 0..2 {
        assert_eq!(
            s.record_stack_closure(&job, &closure_of(&job), 205)
                .unwrap(),
            JobState::NeedsIntervention
        );
    }
    assert_eq!(s.stack_result(&p, &job.id).unwrap().unwrap().0, ran);
    assert_eq!(locks(&s, &job.id), held);
    assert_eq!(disagreements(&s, &job.id), 1);
}

/// A stack held across a store epoch change is not released by a closure;
/// without the change it is.
#[test]
fn a_stack_closure_after_a_store_epoch_change_is_refused() {
    for changed in [false, true] {
        let (_dir, mut s, p) = setup();
        s.observe_executor_epoch(&"1".repeat(64), 200).unwrap();
        let job = queued(&mut s, &p, ContainerAction::Restart, "lost", 200);
        let job = s
            .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 203)
            .unwrap();
        if changed {
            assert_eq!(s.observe_executor_epoch(&"2".repeat(64), 204).unwrap(), 1);
        }
        let state = s
            .record_stack_closure(&job, &closure_of(&job), 205)
            .unwrap();
        if changed {
            assert_eq!(state, JobState::NeedsIntervention);
            assert_eq!(locks(&s, &job.id).len(), 5);
            assert_eq!(disagreements(&s, &job.id), 1);
        } else {
            assert_eq!(state, JobState::PreconditionChanged);
            assert!(locks(&s, &job.id).is_empty());
        }
    }
}

#[test]
fn an_administrator_cancels_a_queued_stack_job_and_its_claim_is_refused() {
    let (_dir, mut store, p) = setup();
    let job = queued(&mut store, &p, ContainerAction::Restart, "one", 210);
    crate::job_admin::tests::cancel_on_behalf(
        &mut store,
        &p,
        &job.id,
        limeos_domain::JobFamily::Stack,
        215,
    );
    assert_eq!(
        store
            .claim_stack(&p, &job.id, &stack(), &"1".repeat(64), 216)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert!(locks(&store, &job.id).is_empty());
    assert!(
        store
            .stack_candidates()
            .unwrap()
            .iter()
            .all(|(_, j)| j.id != job.id)
    );
}
