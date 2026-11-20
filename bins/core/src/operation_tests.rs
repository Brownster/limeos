use super::*;
use limeos_domain::{ContainerSnapshot, ExecutionState, JobState, RestartReceipt};
use limeos_executor_protocol::{Receipt, Request};
use std::sync::Mutex;

struct EngineFixture {
    current: ContainerSnapshot,
    receipt: Option<RestartReceipt>,
    posts: usize,
    reads: usize,
    hold: bool,
    /// The executor's prune watermark (see `ReceiptAbsence`).
    watermark: i64,
    /// A lookup asked to close an absent action records a closure. Off, it
    /// answers as a status read would: the bare absence.
    closes: bool,
    /// What the closure recorded, once there is one.
    closed: Option<limeos_domain::ReceiptAbsence>,
    /// Lookups that did not ask to close: core never sends one.
    plain_reads: usize,
    /// The receipt store's epoch, sent with every receipt answer.
    epoch: Option<String>,
}
#[tokio::test]
async fn logs_recheck_revocation_after_reading_and_viewer_scope_never_reaches_executor() {
    let (_dir, core, token, _csrf, state, entered, release, task) = fixture().await;
    let resource = state.lock().unwrap().current.resource.clone();
    state.lock().unwrap().hold = true;
    let reader = core.clone();
    let held_token = token.clone();
    let held_resource = resource.clone();
    let pending = tokio::spawn(async move {
        reader
            .human_logs(
                held_token,
                held_resource,
                limeos_domain::LogOptions { tail: 100 },
            )
            .await
    });
    entered.notified().await;
    let digest = limeos_identity::digest(&token);
    core.db
        .call(move |s| s.revoke_session(&digest, now()))
        .await
        .unwrap();
    release.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap_err().0,
        ErrorCode::Unauthenticated
    );
    assert_eq!(state.lock().unwrap().reads, 1);
    task.abort();

    let (_dir, core, token, _, state, _, _, task) = fixture().await;
    let principal = core.session(token.clone(), None).await.unwrap().principal;
    let resource = state.lock().unwrap().current.resource.clone();
    core.db
        .call(move |s| {
            s.revise_grants(
                &principal.id,
                limeos_domain::Role::Viewer,
                &[Scope {
                    operation: limeos_domain::Operation::HealthRead,
                    resource: "*".into(),
                }],
                now(),
            )
        })
        .await
        .unwrap();
    assert!(
        core.human_logs(token, resource, limeos_domain::LogOptions { tail: 100 })
            .await
            .is_err()
    );
    assert_eq!(state.lock().unwrap().reads, 0);
    task.abort();
}
#[tokio::test]
async fn start_and_stop_dispatch_use_the_same_approval_and_verified_job_path() {
    for operation in [
        limeos_domain::ContainerAction::Start,
        limeos_domain::ContainerAction::Stop,
    ] {
        let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
        if operation == limeos_domain::ContainerAction::Start {
            state.lock().unwrap().current.running = false;
        }
        let resource = state.lock().unwrap().current.resource.clone();
        let proposal = core
            .plan_container(
                token.clone(),
                csrf.clone(),
                limeos_contracts::ContainerInput {
                    resource,
                    operation,
                },
            )
            .await
            .unwrap();
        let approval = core
            .approve_container(
                token.clone(),
                csrf.clone(),
                proposal.plan.id.clone(),
                proposal.digest.clone(),
            )
            .await
            .unwrap();
        let input = limeos_contracts::QueueRestartInput {
            proposal,
            approval: approval.token,
            idempotency_key: "lifecycle".into(),
        };
        let job = core.submit(token.clone(), csrf, input).await.unwrap();
        let principal = core.session(token.clone(), None).await.unwrap().principal;
        core.dispatch_one(principal, job.clone()).await.unwrap();
        assert_eq!(
            core.restart_progress(token, job.id, 0)
                .await
                .unwrap()
                .job
                .state,
            JobState::Succeeded
        );
        assert_eq!(state.lock().unwrap().posts, 1);
        task.abort();
    }
}
async fn fixture() -> (
    tempfile::TempDir,
    Core,
    String,
    String,
    Arc<Mutex<EngineFixture>>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = limeos_identity::digest(&format!("csrf:{token}"));
    let digest = limeos_identity::digest(&token);
    let csrf_digest = limeos_identity::digest(&csrf);
    db.call(move |s| {
        s.issue_bootstrap("bootstrap", now())?;
        s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
        let record = s.login_record("admin")?.unwrap();
        s.create_session(&record, None, &digest, &csrf_digest, now())?;
        Ok(())
    })
    .await
    .unwrap();
    let path = dir.path().join("executor.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let state = Arc::new(Mutex::new(EngineFixture {
        current: ContainerSnapshot {
            resource: format!("container:{}", "b".repeat(64)),
            image: format!("sha256:{}", "c".repeat(64)),
            started_at: "2026-10-05T07:00:00Z".into(),
            running: true,
        },
        receipt: None,
        posts: 0,
        reads: 0,
        hold: false,
        watermark: 0,
        closes: true,
        closed: None,
        plain_reads: 0,
        epoch: Some("1".repeat(64)),
    }));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = state.clone();
    let observed = entered.clone();
    let unblocked = release.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
            if matches!(
                request,
                Request::Inspect { .. } | Request::ContainerLogs { .. }
            ) && fixture.lock().unwrap().hold
            {
                observed.notify_one();
                unblocked.notified().await;
            }
            let response = {
                let mut state = fixture.lock().unwrap();
                let mut response = Receipt::empty();
                if matches!(
                    request,
                    Request::Restart { .. }
                        | Request::ExecuteContainer { .. }
                        | Request::RestartReceipt { .. }
                        | Request::VerifyRestart { .. }
                ) {
                    response.epoch = state.epoch.clone();
                }
                match request {
                    Request::Inspect { .. } => response.inspection = Some(state.current.clone()),
                    Request::ContainerLogs { .. } => {
                        state.reads += 1;
                        response.logs = Some(limeos_domain::ContainerLogs {
                            resource: state.current.resource.clone(),
                            text: "fixture diagnostic".into(),
                            truncated: false,
                        });
                    }
                    Request::Restart { request, .. }
                    | Request::ExecuteContainer { request, .. }
                        if state.closed.is_some() =>
                    {
                        // A closed action never reaches the Engine.
                        assert_eq!(request.action, state.receipt.as_ref().unwrap().action);
                        response.restart = state.receipt.clone();
                    }
                    Request::Restart { request, .. }
                    | Request::ExecuteContainer { request, .. } => {
                        state.posts += 1;
                        state.receipt = Some(RestartReceipt {
                            operation: request.plan.operation,
                            action: request.action,
                            plan_digest: request.plan_digest,
                            before: state.current.clone(),
                            state: ExecutionState::EffectAccepted,
                            error: None,
                        });
                        if request.plan.operation == limeos_domain::ContainerAction::Stop {
                            state.current.running = false;
                        } else {
                            state.current.running = true;
                            state.current.started_at = "2026-10-05T07:01:00Z".into();
                        }
                        response.restart = state.receipt.clone();
                    }
                    Request::RestartReceipt {
                        action,
                        digest,
                        plan,
                        close_if_absent,
                        ..
                    } => {
                        // Core sends the stored plan with the digest it names.
                        assert_eq!(
                            limeos_identity::digest(&serde_json::to_string(&plan).unwrap()),
                            digest
                        );
                        if !close_if_absent {
                            state.plain_reads += 1;
                        }
                        if state.receipt.is_none() && state.closed.is_none() {
                            let absence = limeos_domain::ReceiptAbsence::judge(
                                plan.expires_at,
                                state.watermark,
                            );
                            if close_if_absent && state.closes {
                                state.closed = Some(absence);
                                if absence == limeos_domain::ReceiptAbsence::NeverRecorded {
                                    state.receipt = Some(RestartReceipt {
                                        operation: plan.operation,
                                        action,
                                        plan_digest: digest,
                                        before: plan.expected.clone(),
                                        state: ExecutionState::PreconditionChanged,
                                        error: Some(ErrorCode::Expired),
                                    });
                                }
                            } else {
                                response.absence = Some(absence);
                            }
                        }
                        response.closed = state.closed.is_some();
                        match state.closed {
                            Some(absence @ limeos_domain::ReceiptAbsence::NoLongerHeld) => {
                                response.absence = Some(absence)
                            }
                            _ => response.restart = state.receipt.clone(),
                        }
                    }
                    Request::VerifyRestart { .. } => {
                        state.receipt.as_mut().unwrap().state = ExecutionState::Verified;
                        response.restart = state.receipt.clone();
                    }
                    _ => panic!("unexpected operation"),
                }
                response
            };
            let _ = limeos_contracts::write_frame(&mut stream, &response).await;
        }
    });
    let core = Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new(path),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
    };
    (dir, core, token, csrf, state, entered, release, task)
}
async fn queued(
    core: &Core,
    token: &str,
    csrf: &str,
    resource: String,
) -> (
    limeos_domain::RestartJob,
    limeos_contracts::QueueRestartInput,
) {
    let proposal = core
        .plan_restart(
            token.into(),
            csrf.into(),
            limeos_contracts::RestartInput { resource },
        )
        .await
        .unwrap();
    let approval = core
        .approve_container(
            token.into(),
            csrf.into(),
            proposal.plan.id.clone(),
            proposal.digest.clone(),
        )
        .await
        .unwrap();
    let input = limeos_contracts::QueueRestartInput {
        proposal,
        approval: approval.token,
        idempotency_key: "fixture".into(),
    };
    (
        core.queue_container(token.into(), csrf.into(), input.clone())
            .await
            .unwrap(),
        input,
    )
}
#[tokio::test]
async fn dispatch_verifies_once_and_replay_returns_the_job_without_engine_access() {
    let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
    let resource = state.lock().unwrap().current.resource.clone();
    let (job, input) = queued(&core, &token, &csrf, resource).await;
    let principal = core.session(token.clone(), None).await.unwrap().principal;
    core.dispatch_one(principal, job.clone()).await.unwrap();
    assert_eq!(
        core.restart_progress(token.clone(), job.id.clone(), 0)
            .await
            .unwrap()
            .job
            .state,
        JobState::Succeeded
    );
    assert_eq!(state.lock().unwrap().posts, 1);
    task.abort();
    assert_eq!(
        core.queue_container(token, csrf, input).await.unwrap().id,
        job.id
    );
}
#[tokio::test]
async fn revoking_a_session_during_inspection_prevents_plan_persistence() {
    let (_dir, core, token, csrf, state, entered, release, task) = fixture().await;
    state.lock().unwrap().hold = true;
    let resource = state.lock().unwrap().current.resource.clone();
    let copy = core.clone();
    let supplied = token.clone();
    let operation = tokio::spawn(async move {
        copy.plan_restart(supplied, csrf, limeos_contracts::RestartInput { resource })
            .await
    });
    entered.notified().await;
    let digest = limeos_identity::digest(&token);
    core.db
        .call(move |s| s.revoke_session(&digest, now()))
        .await
        .unwrap();
    release.notify_one();
    assert_eq!(
        operation.await.unwrap().unwrap_err().0,
        ErrorCode::Unauthenticated
    );
    assert!(
        core.db
            .call(|s| s.container_candidates())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state.lock().unwrap().posts, 0);
    task.abort();
}
#[tokio::test]
async fn recovery_never_dispatches_again_and_closes_an_action_nothing_recorded() {
    for prepared in [false, true] {
        let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
        let snapshot = state.lock().unwrap().current.clone();
        let (job, _) = queued(&core, &token, &csrf, snapshot.resource.clone()).await;
        let principal = core.session(token, None).await.unwrap().principal;
        let p = principal.clone();
        let id = job.id.clone();
        let mut running = core
            .db
            .call(move |s| s.claim_container(&p, &id, &snapshot, now()))
            .await
            .unwrap();
        if prepared {
            state.lock().unwrap().receipt = Some(RestartReceipt {
                operation: limeos_domain::ContainerAction::Restart,
                action: running.id.clone(),
                plan_digest: limeos_identity::digest(
                    &serde_json::to_string(&running.plan).unwrap(),
                ),
                before: running.plan.expected.clone(),
                state: ExecutionState::Prepared,
                error: None,
            });
        }
        let id = running.id.clone();
        core.db
            .call(move |s| s.transition(&id, JobState::Running, JobState::NeedsIntervention, now()))
            .await
            .unwrap();
        running.state = JobState::NeedsIntervention;
        core.dispatch_one(principal, running).await.unwrap();
        assert_eq!(state.lock().unwrap().posts, 0);
        // A Prepared row may have been sent: held. No row at all: the lookup
        // closed the action, so nothing was or can be sent, and it is released.
        assert_eq!(
            core.db.call(move |s| s.job_state(&job.id)).await.unwrap(),
            if prepared {
                "needs_intervention"
            } else {
                "precondition_changed"
            }
        );
        task.abort();
    }
}
fn missing_events(progress: &limeos_contracts::JobProgress) -> usize {
    progress
        .events
        .iter()
        .filter(|e| e.event == limeos_domain::EventKind::ExecutorReceiptMissing)
        .count()
}
#[tokio::test]
async fn a_pruned_container_receipt_keeps_the_job_held_with_the_reason_recorded_once() {
    // An old plan, and a young one whose receipt was pruned while the clock ran
    // more than an hour ahead before stepping back: the watermark decides both.
    for age in [
        limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY + limeos_domain::PLAN_TTL_SECONDS + 60,
        0,
    ] {
        let (_dir, core, token, _csrf, state, _, _, task) = fixture().await;
        let snapshot = state.lock().unwrap().current.clone();
        let principal = core.session(token.clone(), None).await.unwrap().principal;
        let t0 = now() - age;
        let p = principal.clone();
        let job = core
            .db
            .call(move |s| {
                let plan =
                    s.plan_container(&p, limeos_domain::ContainerAction::Restart, &snapshot, t0)?;
                let approval = s.approve_container(&p, &plan.plan.id, &plan.digest, t0)?;
                let queued = s.queue_container(&p, "old", &plan, &approval.token, &snapshot, t0)?;
                s.claim_container(&p, &queued.id, &snapshot, t0)
            })
            .await
            .unwrap();
        // The executor executed and verified it, core never recorded the result,
        // and retention has since pruned the receipt: lookups find no row.
        assert!(state.lock().unwrap().receipt.is_none());
        state.lock().unwrap().watermark = job.plan.expires_at;
        for _ in 0..2 {
            let id = job.id.clone();
            let current = core
                .db
                .call(move |s| s.container_candidates())
                .await
                .unwrap()
                .into_iter()
                .map(|(_, job)| job)
                .find(|job| job.id == id)
                .unwrap();
            core.dispatch_one(principal.clone(), current).await.unwrap();
            let progress = core
                .restart_progress(token.clone(), job.id.clone(), 0)
                .await
                .unwrap();
            assert_eq!(progress.job.state, JobState::NeedsIntervention);
            assert_eq!(missing_events(&progress), 1);
        }
        assert_eq!(state.lock().unwrap().posts, 0);
        task.abort();
    }
}
async fn principal_of(core: &Core, token: &str) -> limeos_domain::Principal {
    core.session(token.into(), None).await.unwrap().principal
}
/// A claimed container job whose ExecuteContainer never reached the store:
/// no row, none ever pruned.
async fn claimed_unsent(
    core: &Core,
    token: &str,
    csrf: &str,
    state: &Arc<Mutex<EngineFixture>>,
) -> (limeos_domain::Principal, limeos_domain::RestartJob) {
    let snapshot = state.lock().unwrap().current.clone();
    let (job, _) = queued(core, token, csrf, snapshot.resource.clone()).await;
    let principal = core.session(token.into(), None).await.unwrap().principal;
    let (p, id) = (principal.clone(), job.id.clone());
    let running = core
        .db
        .call(move |s| s.claim_container(&p, &id, &snapshot, now()))
        .await
        .unwrap();
    (principal, running)
}
#[tokio::test]
async fn a_never_recorded_container_action_is_closed_and_released_at_once() {
    let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
    let (principal, running) = claimed_unsent(&core, &token, &csrf, &state).await;
    // The plan is still valid: nothing waits for it to expire.
    assert!(running.plan.validate(now()).is_ok());
    core.dispatch_one(principal, running.clone()).await.unwrap();
    let progress = core
        .restart_progress(token.clone(), running.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::PreconditionChanged);
    assert_eq!(missing_events(&progress), 0);
    // The closure receipt is the recorded result.
    let (admin, id) = (principal_of(&core, &token).await, running.id.clone());
    let members = core
        .db
        .call(move |s| s.resolution_members(&admin, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].state, ExecutionState::PreconditionChanged);
    assert_eq!(members[0].error, Some(ErrorCode::Expired));
    let snapshot = {
        let fixture = state.lock().unwrap();
        assert_eq!(fixture.posts, 0);
        assert_eq!(fixture.plain_reads, 0);
        fixture.current.clone()
    };
    // The lock is free: the next job on the same container is claimable.
    let p = principal_of(&core, &token).await;
    let next = core
        .db
        .call(move |s| {
            let plan = s.plan_container(
                &p,
                limeos_domain::ContainerAction::Restart,
                &snapshot,
                now(),
            )?;
            let approval = s.approve_container(&p, &plan.plan.id, &plan.digest, now())?;
            let queued = s.queue_container(&p, "next", &plan, &approval.token, &snapshot, now())?;
            s.claim_container(&p, &queued.id, &snapshot, now())
        })
        .await
        .unwrap();
    assert_eq!(next.state, JobState::Running);
    task.abort();
}
#[tokio::test]
async fn an_absence_the_executor_did_not_close_holds_the_job_without_the_missing_event() {
    let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
    // An executor that answers a bare absence, as to a status read: an execute
    // request may still be on its way, so nothing proves it was never sent.
    state.lock().unwrap().closes = false;
    let (principal, running) = claimed_unsent(&core, &token, &csrf, &state).await;
    core.dispatch_one(principal, running.clone()).await.unwrap();
    let progress = core
        .restart_progress(token.clone(), running.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::NeedsIntervention);
    assert_eq!(missing_events(&progress), 0);
    assert_eq!(state.lock().unwrap().posts, 0);
    task.abort();
}
#[test]
fn recovery_rotation_lists_every_held_job_and_pauses_each() {
    use super::operations::Rotation;
    let batch = limeos_persistence::RECOVERY_BATCH;
    // More held jobs than one listing returns, in ID order.
    let held: Vec<String> = (0..2 * batch + 5).map(|i| format!("{i:064x}")).collect();
    let list = |cursor: &str| -> Vec<&str> {
        held.iter()
            .map(String::as_str)
            .filter(|id| *id > cursor)
            .take(batch)
            .collect()
    };
    let mut rotation = Rotation::default();
    let mut seen = std::collections::BTreeSet::new();
    let mut polled = 0;
    // Three listings reach every job; the fourth starts over.
    for pass in 0..4 {
        let page = list(rotation.cursor());
        if pass == 3 {
            assert_eq!(page.first().copied(), held.first().map(String::as_str));
        }
        rotation.advance(page.iter().copied());
        for id in page {
            seen.insert(id.to_owned());
            if rotation.due(id) {
                polled += 1;
            }
        }
    }
    assert_eq!(seen.len(), held.len());
    // Each is looked up once per pause, however often it is listed.
    assert_eq!(polled, held.len());
}

fn record_events(
    progress: &limeos_contracts::JobProgress,
    kind: &limeos_domain::EventKind,
) -> usize {
    progress.events.iter().filter(|e| &e.event == kind).count()
}

/// The executor ran the action and core recorded its uncertain outcome; then
/// the receipt store was lost and recreated (a new epoch, no row). The
/// recovery lookup's closure says it was never recorded, contradicting the
/// result core holds: the job stays held with its lock and its result, and
/// the disagreement is recorded once.
#[tokio::test]
async fn a_recreated_store_never_releases_a_container_job_core_holds_a_result_for() {
    use limeos_domain::EventKind;
    let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
    let (principal, running) = claimed_unsent(&core, &token, &csrf, &state).await;
    // The execute answer: the restart may have happened.
    let ran = RestartReceipt {
        operation: running.plan.operation,
        action: running.id.clone(),
        plan_digest: limeos_identity::digest(&serde_json::to_string(&running.plan).unwrap()),
        before: running.plan.expected.clone(),
        state: ExecutionState::OutcomeUnknown,
        error: Some(ErrorCode::Unavailable),
    };
    let (job, recorded) = (running.clone(), ran.clone());
    core.db
        .call(move |s| s.record_container_result(&job, &recorded, None, now()))
        .await
        .unwrap();
    // The store is lost and recreated: no row, and a new epoch.
    state.lock().unwrap().epoch = Some("2".repeat(64));
    for _ in 0..2 {
        let id = running.id.clone();
        let current = core
            .db
            .call(move |s| s.container_candidates())
            .await
            .unwrap()
            .into_iter()
            .map(|(_, job)| job)
            .find(|job| job.id == id)
            .unwrap();
        core.dispatch_one(principal.clone(), current).await.unwrap();
    }
    let progress = core
        .restart_progress(token.clone(), running.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::NeedsIntervention);
    // The result core recorded from the execute answer is kept.
    let (admin, id) = (principal_of(&core, &token).await, running.id.clone());
    let members = core
        .db
        .call(move |s| s.resolution_members(&admin, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(members[0].state, ran.state);
    assert_eq!(members[0].error, ran.error);
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorRecordDisagrees),
        1
    );
    // Core saw no earlier epoch here: the result it holds decides alone.
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorStoreChanged),
        0
    );
    assert_eq!(state.lock().unwrap().posts, 0);
    task.abort();
}

/// A job dispatched while core knew the store's epoch, its execute answer
/// lost before any row; the store is then replaced by an older copy (a new
/// epoch). Its closure is not accepted: the job stays held.
#[tokio::test]
async fn a_container_job_dispatched_under_an_earlier_store_epoch_is_held_on_a_closure() {
    use limeos_domain::EventKind;
    let (_dir, core, token, csrf, state, _, _, task) = fixture().await;
    core.db
        .call(|s| s.observe_executor_epoch(&"1".repeat(64), now()))
        .await
        .unwrap();
    let (principal, running) = claimed_unsent(&core, &token, &csrf, &state).await;
    state.lock().unwrap().epoch = Some("2".repeat(64));
    core.dispatch_one(principal, running.clone()).await.unwrap();
    let progress = core
        .restart_progress(token.clone(), running.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::NeedsIntervention);
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorStoreChanged),
        1
    );
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorRecordDisagrees),
        1
    );
    assert_eq!(state.lock().unwrap().posts, 0);
    task.abort();
}
