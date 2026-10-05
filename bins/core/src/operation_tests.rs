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
                    Request::RestartReceipt { .. } => response.restart = state.receipt.clone(),
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
        db,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new(path),
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
async fn recovery_without_an_accepted_receipt_never_dispatches_again() {
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
        assert_eq!(
            core.db.call(move |s| s.job_state(&job.id)).await.unwrap(),
            "needs_intervention"
        );
        task.abort();
    }
}
