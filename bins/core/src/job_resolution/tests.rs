use super::*;
use crate::operations::Rotation;
use limeos_contracts::{Enrollment, Login};
use limeos_domain::{ContainerAction, ContainerSnapshot, ResolveKind, ResolveReceipt};
use limeos_executor_protocol::Receipt;
use std::sync::Mutex;

const PASSWORD: &str = "correct-long-password";
const LONG: &str = "Checked the engine by hand and accept the present state.";

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Free the row and answer.
    Ok,
    /// The executor is unreachable or fails before doing anything.
    Down,
    /// Free the row, then lose the answer.
    Lost,
    /// A definite error code.
    Refuse(ErrorCode),
    /// The store holds no row for the action, closes it, and answers with
    /// this absence.
    Absent(limeos_domain::ReceiptAbsence),
    /// As `Absent`, from an executor that did not close the action.
    Unclosed(limeos_domain::ReceiptAbsence),
}
struct Executor {
    /// `None`: the Engine no longer has the container.
    current: Option<ContainerSnapshot>,
    mode: Mode,
    freed: bool,
    inspects: usize,
    resolves: usize,
    /// Anything that would start, stop or restart. Must stay zero.
    effects: usize,
    last: Option<Request>,
}
struct Fixture {
    _dir: tempfile::TempDir,
    core: Core,
    session: limeos_api::IssuedSession,
    job: String,
    exec: Arc<Mutex<Executor>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn engine_state(started: &str, running: bool) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "b".repeat(64)),
        image: format!("sha256:{}", "c".repeat(64)),
        started_at: started.into(),
        running,
    }
}
const T0: &str = "2026-10-05T07:00:00Z";
const T1: &str = "2026-10-05T07:01:00Z";

fn answer(exec: &mut Executor, request: Request) -> Option<Receipt> {
    let mut response = Receipt::empty();
    let refuse = |response: &mut Receipt, code| {
        response.ready = false;
        response.error = Some(code);
    };
    match request {
        Request::Inspect { .. } => {
            exec.inspects += 1;
            match &exec.current {
                Some(current) => response.inspection = Some(current.clone()),
                None => refuse(&mut response, ErrorCode::NotFound),
            }
        }
        Request::ResolveAction {
            kind,
            ref action,
            ref evidence_digest,
            declared,
            ref resolved_by,
            ..
        } => {
            exec.resolves += 1;
            let confirmation = ResolveReceipt {
                kind,
                action: action.clone(),
                evidence_digest: evidence_digest.clone(),
                declared,
                resolved_by: resolved_by.clone(),
            };
            exec.last = Some(request);
            if exec.freed {
                response.resolution = Some(confirmation);
                return Some(response);
            }
            match exec.mode {
                Mode::Ok => {
                    exec.freed = true;
                    response.resolution = Some(confirmation);
                }
                Mode::Down => refuse(&mut response, ErrorCode::Unavailable),
                Mode::Lost => {
                    exec.freed = true;
                    return None;
                }
                Mode::Refuse(code) => refuse(&mut response, code),
                Mode::Absent(absence) => {
                    response.absence = Some(absence);
                    response.closed = true;
                }
                Mode::Unclosed(absence) => response.absence = Some(absence),
            }
        }
        _ => exec.effects += 1,
    }
    Some(response)
}
fn core(db: Database, socket: PathBuf) -> Core {
    Core {
        automation_status: Default::default(),
        reader_health: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new(socket),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        claims_in_doubt: Default::default(),
        stacks: Default::default(),
        compose_catalog: None,
    }
}
async fn login(core: &Core) -> limeos_api::IssuedSession {
    core.login(
        Login {
            username: "alice".into(),
            password: PASSWORD.into(),
        },
        limeos_api::PasswordAttempt::default(),
    )
    .await
    .result
    .unwrap()
}
/// An administrator, a restart job claimed and then left `needs_intervention`,
/// and an executor that still sees the container as planned.
async fn fixture() -> Fixture {
    fixture_at(None).await
}
/// As `fixture`, with the job planned, approved, queued and claimed at
/// `planned` instead of now, so its plan may already have expired.
async fn fixture_at(planned: Option<i64>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let path = dir.path().join("executor.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let exec = Arc::new(Mutex::new(Executor {
        current: Some(engine_state(T0, true)),
        mode: Mode::Ok,
        freed: false,
        inspects: 0,
        resolves: 0,
        effects: 0,
        last: None,
    }));
    let served = exec.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
            let response = answer(&mut served.lock().unwrap(), request);
            if let Some(response) = response {
                let _ = limeos_contracts::write_frame(&mut stream, &response).await;
            }
        }
    });
    let core = core(db, path);
    let token = "d".repeat(64);
    let digest = limeos_identity::digest(&token);
    core.db
        .call(move |s| s.issue_bootstrap(&digest, now()))
        .await
        .unwrap();
    core.enroll(Enrollment {
        token,
        username: "alice".into(),
        password: PASSWORD.into(),
    })
    .await
    .unwrap();
    let session = login(&core).await;
    let (token, csrf) = (session.token.clone(), session.view.csrf_token.clone());
    let job = match planned {
        None => {
            let resource = engine_state(T0, true).resource.parse().unwrap();
            let proposal = core
                .plan_restart(
                    token.clone(),
                    csrf.clone(),
                    limeos_contracts::RestartInput { resource },
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
            let job = core
                .queue_container(
                    token.clone(),
                    csrf,
                    limeos_contracts::QueueRestartInput {
                        proposal,
                        approval: approval.token,
                        idempotency_key: "stuck".into(),
                    },
                )
                .await
                .unwrap();
            let (p, id) = (session.view.principal.clone(), job.id.clone());
            core.db
                .call(move |s| {
                    s.claim_container(&p, &id, &engine_state(T0, true), &"1".repeat(64), now())?;
                    s.transition(&id, JobState::Running, JobState::NeedsIntervention, now())
                })
                .await
                .unwrap();
            job
        }
        Some(t0) => {
            let p = session.view.principal.clone();
            core.db
                .call(move |s| {
                    let snapshot = engine_state(T0, true);
                    let plan = s.plan_container(&p, ContainerAction::Restart, &snapshot, t0)?;
                    let approval = s.approve_container(&p, &plan.plan.id, &plan.digest, t0)?;
                    let queued =
                        s.queue_container(&p, "stuck", &plan, &approval.token, &snapshot, t0)?;
                    let job = s.claim_container(&p, &queued.id, &snapshot, &"1".repeat(64), t0)?;
                    s.transition(
                        &job.id,
                        JobState::Running,
                        JobState::NeedsIntervention,
                        now(),
                    )?;
                    Ok(job)
                })
                .await
                .unwrap()
        }
    };
    let job = job.id;
    // Planning inspected once; the tests count from here.
    exec.lock().unwrap().inspects = 0;
    Fixture {
        _dir: dir,
        core,
        session,
        job,
        exec,
        task,
    }
}
impl Fixture {
    async fn view(&self) -> ResolutionView {
        self.core
            .human_resolution_view(self.session.token.clone(), self.job.clone())
            .await
            .unwrap()
    }
    fn input(&self, declared: DeclaredOutcome, digest: &str, password: &str) -> ResolveJobInput {
        ResolveJobInput {
            declared,
            reason: LONG.into(),
            evidence_digest: digest.into(),
            password: password.into(),
        }
    }
    async fn resolve_with(
        &self,
        declared: DeclaredOutcome,
        digest: &str,
    ) -> Result<ResolveJobResponse> {
        self.core
            .human_resolve(
                self.session.token.clone(),
                self.session.view.csrf_token.clone(),
                self.job.clone(),
                self.input(declared, digest, PASSWORD),
            )
            .await
    }
    /// Show, then decide on exactly what was shown.
    async fn resolve(&self, declared: DeclaredOutcome) -> Result<ResolveJobResponse> {
        let digest = self.view().await.evidence_digest.unwrap();
        self.resolve_with(declared, &digest).await
    }
    async fn state(&self) -> JobState {
        self.core
            .restart_progress(self.session.token.clone(), self.job.clone(), 0)
            .await
            .unwrap()
            .job
            .state
    }
    fn set(&self, mode: Mode) {
        self.exec.lock().unwrap().mode = mode;
    }
    fn counts(&self) -> (usize, usize, usize) {
        let e = self.exec.lock().unwrap();
        (e.inspects, e.resolves, e.effects)
    }
    async fn retry(&self, retry: &mut Rotation) {
        self.core.complete_pending_resolutions(retry, 0).await;
    }
}

#[tokio::test]
async fn the_view_is_fresh_read_only_evidence_with_the_declarations_it_supports() {
    let f = fixture().await;
    let v = f.view().await;
    assert_eq!(v.state, JobState::NeedsIntervention);
    assert_eq!(v.kind, ResolveKind::Container);
    assert_eq!(v.targets.len(), 1);
    assert_eq!(v.targets[0].expected, engine_state(T0, true));
    assert!(v.targets[0].measured.matches_expected);
    assert_eq!(v.targets[0].measured.container, "b".repeat(64));
    assert_eq!(
        v.allowed,
        [
            DeclaredOutcome::NotDone,
            DeclaredOutcome::AcceptedDivergence
        ]
    );
    assert!(v.evidence_digest.is_some() && v.pending.is_none() && v.resolution.is_none());
    // Restarted since: only the intended state is present.
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    let moved = f.view().await;
    assert_eq!(
        moved.allowed,
        [
            DeclaredOutcome::VerifiedDone,
            DeclaredOutcome::AcceptedDivergence
        ]
    );
    assert_ne!(moved.evidence_digest, v.evidence_digest);
    // Gone from the Engine: evidence too, and it proves neither claim.
    f.exec.lock().unwrap().current = None;
    let gone = f.view().await;
    assert!(gone.targets[0].measured.is_absent());
    assert_eq!(gone.allowed, [DeclaredOutcome::AcceptedDivergence]);
    // Three reads, nothing else sent.
    assert_eq!(f.counts(), (3, 0, 0));
}

#[tokio::test]
async fn a_supported_declaration_runs_request_then_executor_then_completion() {
    let f = fixture().await;
    let response = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    assert_eq!(response.status, ResolveStatus::Resolved);
    let record = response.resolution.unwrap();
    assert_eq!(record.declared, DeclaredOutcome::NotDone);
    assert_eq!(record.resolved_by, f.session.view.principal.id);
    assert_eq!(record.reason, LONG);
    assert_eq!(f.state().await, JobState::Resolved);
    // One decision-time inspection (plus the view that showed it), one
    // ResolveAction addressed by job and plan digest, no effect of any kind.
    let (inspects, resolves, effects) = f.counts();
    assert_eq!((inspects, resolves, effects), (2, 1, 0));
    let Some(Request::ResolveAction {
        kind,
        action,
        digest,
        evidence_digest,
        declared,
        resolved_by,
        plan,
        ..
    }) = f.exec.lock().unwrap().last.take()
    else {
        panic!("no resolve sent")
    };
    // The stored plan travels with its digest, so the executor can check one
    // against the other and judge any absence by the plan's own expiry.
    let limeos_domain::ActionPlan::Container(plan) = plan else {
        panic!("not a container plan")
    };
    assert_eq!(
        limeos_identity::digest(&serde_json::to_string(&plan).unwrap()),
        digest
    );
    assert_eq!(
        (kind, action, declared, resolved_by),
        (
            ResolveKind::Container,
            f.job.clone(),
            DeclaredOutcome::NotDone,
            record.resolved_by.clone()
        )
    );
    assert_eq!(evidence_digest, record.evidence_digest);
    // The record is part of the view; no new evidence is gathered for it.
    let after = f.view().await;
    assert_eq!(after.resolution, Some(record));
    assert!(after.targets.is_empty() && after.evidence_digest.is_none());
    assert_eq!(f.counts().0, 2);
    // A second decision on a settled job is a conflict, not a second record.
    assert_eq!(
        f.resolve_with(DeclaredOutcome::AcceptedDivergence, "d")
            .await
            .unwrap_err(),
        Error(ErrorCode::Conflict)
    );
}

#[tokio::test]
async fn unsupported_declarations_return_the_diff_and_record_nothing() {
    let f = fixture().await;
    let response = f.resolve(DeclaredOutcome::VerifiedDone).await.unwrap();
    assert_eq!(response.status, ResolveStatus::UnsupportedDeclaration);
    assert_eq!(response.blocking, ["b".repeat(64)]);
    let shown = response.view.unwrap();
    assert_eq!(shown.targets.len(), 1);
    assert!(!shown.targets[0].measured.intended_state_reached);
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    let response = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    assert_eq!(response.status, ResolveStatus::UnsupportedDeclaration);
    assert!(response.resolution.is_none());
    assert!(f.view().await.pending.is_none());
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    assert_eq!(f.counts().1, 0);
    // Divergence rests on the operator: always available, reason enforced.
    let digest = f.view().await.evidence_digest.unwrap();
    let mut short = f.input(DeclaredOutcome::AcceptedDivergence, &digest, PASSWORD);
    short.reason = "too short".into();
    assert_eq!(
        f.core
            .human_resolve(
                f.session.token.clone(),
                f.session.view.csrf_token.clone(),
                f.job.clone(),
                short
            )
            .await
            .unwrap_err(),
        Error(ErrorCode::InvalidInput)
    );
    assert_eq!(
        f.resolve(DeclaredOutcome::AcceptedDivergence)
            .await
            .unwrap()
            .status,
        ResolveStatus::Resolved
    );
}

#[tokio::test]
async fn stale_evidence_is_a_conflict_with_fresh_evidence_and_records_nothing() {
    let f = fixture().await;
    let shown = f.view().await.evidence_digest.unwrap();
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    let response = f
        .resolve_with(DeclaredOutcome::AcceptedDivergence, &shown)
        .await
        .unwrap();
    assert_eq!(response.status, ResolveStatus::StaleEvidence);
    let fresh = response.view.unwrap();
    assert_ne!(fresh.evidence_digest.as_deref(), Some(shown.as_str()));
    assert!(fresh.pending.is_none());
    assert_eq!(
        (f.counts().1, f.state().await),
        (0, JobState::NeedsIntervention)
    );
    // The digest is the whole binding: deciding on the new one works.
    let digest = fresh.evidence_digest.unwrap();
    assert_eq!(
        f.resolve_with(DeclaredOutcome::VerifiedDone, &digest)
            .await
            .unwrap()
            .status,
        ResolveStatus::Resolved
    );
}

#[tokio::test]
async fn an_unreachable_executor_leaves_the_request_pending_and_a_repeat_completes_it() {
    let f = fixture().await;
    f.set(Mode::Down);
    let pending = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    assert_eq!(pending.status, ResolveStatus::Pending);
    assert!(pending.message.contains("recorded"));
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    let held = f.view().await;
    assert_eq!(
        held.pending.as_ref().unwrap().declared,
        DeclaredOutcome::NotDone
    );
    // Nothing else can be decided while it is open, and the executor is not
    // asked to do anything different.
    let digest = held.evidence_digest.unwrap();
    assert_eq!(
        f.resolve_with(DeclaredOutcome::AcceptedDivergence, &digest)
            .await
            .unwrap_err(),
        Error(ErrorCode::Conflict)
    );
    // The identical request is idempotent and finishes it once the executor is up.
    f.set(Mode::Ok);
    let done = f
        .resolve_with(DeclaredOutcome::NotDone, &digest)
        .await
        .unwrap();
    assert_eq!(done.status, ResolveStatus::Resolved);
    assert_eq!(f.state().await, JobState::Resolved);
    assert_eq!(f.counts().2, 0);
}

#[tokio::test]
async fn a_lost_answer_after_the_executor_freed_is_not_withdrawn_and_completes_later() {
    let f = fixture().await;
    f.set(Mode::Lost);
    let response = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    assert_eq!(response.status, ResolveStatus::Pending);
    // The executor is free, core is held and the request is still recorded.
    assert!(f.exec.lock().unwrap().freed);
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    assert!(f.view().await.pending.is_some());
    // Startup (an empty retry map) re-sends the same request and completes.
    let mut retry = Rotation::default();
    f.retry(&mut retry).await;
    assert_eq!(f.state().await, JobState::Resolved);
    assert_eq!(f.counts().1, 2);
    let after = f.view().await;
    assert!(after.pending.is_none() && after.resolution.is_some());
}

#[tokio::test]
async fn only_a_definite_refusal_withdraws_the_request() {
    for (code, withdrawn) in [
        (ErrorCode::Conflict, true),
        (ErrorCode::NotFound, true),
        (ErrorCode::Forbidden, true),
        (ErrorCode::Unavailable, false),
        (ErrorCode::Overloaded, false),
        (ErrorCode::InvalidInput, false),
        (ErrorCode::StateNotDurable, false),
    ] {
        let f = fixture().await;
        f.set(Mode::Refuse(code));
        let result = f.resolve(DeclaredOutcome::NotDone).await;
        let view = f.view().await;
        assert_eq!(view.pending.is_none(), withdrawn, "{code:?}");
        assert_eq!(f.state().await, JobState::NeedsIntervention, "{code:?}");
        match (code, result) {
            (ErrorCode::Conflict, Ok(r)) => {
                assert_eq!(r.status, ResolveStatus::StaleEvidence);
                assert!(r.view.is_some());
            }
            (ErrorCode::NotFound | ErrorCode::Forbidden, Err(e)) => {
                assert_eq!(e, Error(ErrorCode::Conflict))
            }
            (_, Ok(r)) => assert_eq!(r.status, ResolveStatus::Pending, "{code:?}"),
            (_, Err(e)) => panic!("{code:?} {e:?}"),
        }
        if withdrawn {
            // The administrator can look again and decide differently.
            f.set(Mode::Ok);
            assert_eq!(
                f.resolve(DeclaredOutcome::AcceptedDivergence)
                    .await
                    .unwrap()
                    .status,
                ResolveStatus::Resolved
            );
        }
    }
}

#[tokio::test]
async fn the_retry_loop_completes_withdraws_throttles_and_waits_for_young_requests() {
    let f = fixture().await;
    f.set(Mode::Down);
    f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    let sent = f.counts().1;
    // A request younger than the minimum age is left to its own handler.
    let mut retry = Rotation::default();
    f.core.complete_pending_resolutions(&mut retry, 3600).await;
    assert_eq!(f.counts().1, sent);
    // Still down: stays pending, and the next tick does not hammer.
    f.retry(&mut retry).await;
    f.retry(&mut retry).await;
    assert_eq!(f.counts().1, sent + 1);
    assert!(f.view().await.pending.is_some());
    // A definite refusal on retry withdraws, acting as the recorded administrator.
    f.set(Mode::Refuse(ErrorCode::Conflict));
    retry = Rotation::default();
    f.retry(&mut retry).await;
    assert!(f.view().await.pending.is_none());
    // Back up with a new decision: completed by the loop with no HTTP help.
    f.set(Mode::Down);
    f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    f.set(Mode::Ok);
    retry = Rotation::default();
    f.retry(&mut retry).await;
    assert_eq!(f.state().await, JobState::Resolved);
    f.retry(&mut retry).await;
    assert!(f.view().await.pending.is_none());
}

#[tokio::test]
async fn without_an_executor_receipt_a_resolution_completes_once_the_action_is_closed() {
    use limeos_domain::ReceiptAbsence;
    for absence in [ReceiptAbsence::NeverRecorded, ReceiptAbsence::NoLongerHeld] {
        // The plan is still valid, but the executor closed the action before it
        // answered: no execute request sent for it can run after core frees its
        // locks, whatever either clock says. It completes now on core's fresh
        // measurement of exactly the approved evidence.
        let f = fixture().await;
        f.set(Mode::Absent(absence));
        let done = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
        assert_eq!(done.status, ResolveStatus::Resolved, "{absence:?}");
        assert_eq!(f.state().await, JobState::Resolved, "{absence:?}");
        assert_eq!(f.counts().2, 0);
        // An absence the executor did not close proves nothing about a request
        // still on its way: recorded, held, and retried.
        let f = fixture().await;
        f.set(Mode::Unclosed(absence));
        let pending = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
        assert_eq!(pending.status, ResolveStatus::Pending, "{absence:?}");
        assert_eq!(f.state().await, JobState::NeedsIntervention, "{absence:?}");
        f.retry(&mut Rotation::default()).await;
        assert_eq!(f.state().await, JobState::NeedsIntervention, "{absence:?}");
        assert!(f.view().await.pending.is_some(), "{absence:?}");
        // Once the executor closes it, the retry loop completes it.
        f.set(Mode::Absent(absence));
        f.retry(&mut Rotation::default()).await;
        assert_eq!(f.state().await, JobState::Resolved, "{absence:?}");
    }
}

/// A never-recorded closure proves nothing was ever sent, so "verified done"
/// contradicts it as it contradicts a precondition_changed receipt: core
/// withdraws the request even from an executor that answered the absence,
/// and the job stays held. A no_longer_held closure leaves the declaration
/// to the operator.
#[tokio::test]
async fn a_never_recorded_closure_is_not_resolved_as_verified_done() {
    use limeos_domain::ReceiptAbsence;
    let f = fixture().await;
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    f.set(Mode::Absent(ReceiptAbsence::NeverRecorded));
    let refused = f.resolve(DeclaredOutcome::VerifiedDone).await.unwrap();
    assert_eq!(refused.status, ResolveStatus::UnsupportedDeclaration);
    assert!(refused.message.contains("precondition_changed"));
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    assert!(matches!(
        f.exec.lock().unwrap().last,
        Some(Request::ResolveAction {
            disputed: false,
            ..
        })
    ));
    let view = f.view().await;
    assert!(view.pending.is_none() && view.resolution.is_none());
    f.set(Mode::Absent(ReceiptAbsence::NoLongerHeld));
    let done = f.resolve(DeclaredOutcome::VerifiedDone).await.unwrap();
    assert_eq!(done.status, ResolveStatus::Resolved);
    assert_eq!(f.state().await, JobState::Resolved);
    assert_eq!(f.counts().2, 0);
}

#[tokio::test]
async fn a_closed_absence_withdraws_the_request_when_the_evidence_has_changed() {
    use limeos_domain::ReceiptAbsence;
    let f = fixture().await;
    let view = f.view().await;
    let evidence = limeos_domain::ResolutionEvidence {
        targets: view.targets.iter().map(|t| t.measured.clone()).collect(),
    };
    let (p, id, digest) = (
        f.session.view.principal.clone(),
        f.job.clone(),
        view.evidence_digest.unwrap(),
    );
    f.core
        .db
        .call(move |s| {
            s.request_resolution(
                &p,
                &id,
                DeclaredOutcome::NotDone,
                LONG,
                &evidence,
                &digest,
                now(),
            )
        })
        .await
        .unwrap();
    f.set(Mode::Absent(ReceiptAbsence::NeverRecorded));
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    f.retry(&mut Rotation::default()).await;
    let after = f.view().await;
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    assert!(after.pending.is_none() && after.resolution.is_none());
    assert_eq!(f.counts().2, 0);
}

/// Core crashed after the executor recorded `resolved` and before completing
/// the resolution. Its retry completes from the stored confirmation; once the
/// executor has pruned the resolved row it closes the action and answers
/// `no_longer_held`, and core completes on its fresh evidence, unchanged.
#[tokio::test]
async fn a_crash_after_the_executor_resolved_completes_on_retry_even_once_pruned() {
    use limeos_domain::{PLAN_TTL_SECONDS, RECEIPT_RETAIN_AFTER_EXPIRY, ReceiptAbsence};
    for (pruned, changed) in [(false, false), (true, false), (true, true)] {
        let f = fixture_at(Some(
            now() - PLAN_TTL_SECONDS - RECEIPT_RETAIN_AFTER_EXPIRY - 60,
        ))
        .await;
        let view = f.view().await;
        let evidence = limeos_domain::ResolutionEvidence {
            targets: view.targets.iter().map(|t| t.measured.clone()).collect(),
        };
        let (p, id, digest) = (
            f.session.view.principal.clone(),
            f.job.clone(),
            view.evidence_digest.unwrap(),
        );
        // Step 1 is durable; the executor then resolved its row (step 2) and
        // core died before step 3.
        f.core
            .db
            .call(move |s| {
                s.request_resolution(
                    &p,
                    &id,
                    DeclaredOutcome::NotDone,
                    LONG,
                    &evidence,
                    &digest,
                    now(),
                )
            })
            .await
            .unwrap();
        if pruned {
            f.set(Mode::Absent(ReceiptAbsence::NoLongerHeld));
        } else {
            f.exec.lock().unwrap().freed = true;
        }
        if changed {
            f.exec.lock().unwrap().current = Some(engine_state(T1, true));
        }
        f.retry(&mut Rotation::default()).await;
        let after = f.view().await;
        if changed {
            // The state moved since the decision: withdrawn, decide again.
            assert_eq!(f.state().await, JobState::NeedsIntervention);
            assert!(after.pending.is_none() && after.resolution.is_none());
        } else {
            assert_eq!(f.state().await, JobState::Resolved, "{pruned}");
            let record = after.resolution.unwrap();
            assert_eq!(record.declared, DeclaredOutcome::NotDone);
            assert!(after.pending.is_none());
        }
        assert_eq!(f.counts().2, 0);
    }
}

#[tokio::test]
async fn the_password_check_is_reported_apart_from_what_follows_it() {
    use limeos_api::{Backend, PasswordCheck};
    let f = fixture().await;
    let checked = |job: String, password: &str| {
        let core = f.core.clone();
        let (token, csrf) = (f.session.token.clone(), f.session.view.csrf_token.clone());
        let input = f.input(DeclaredOutcome::NotDone, &"d".repeat(64), password);
        async move {
            core.resolve_job(
                token,
                csrf,
                job,
                input,
                limeos_api::PasswordAttempt::default(),
            )
            .await
        }
    };
    // Correct password, no such job: refused after the match.
    let outcome = checked("0".repeat(64), PASSWORD).await;
    assert_eq!(outcome.check, PasswordCheck::Accepted);
    assert_eq!(outcome.result.unwrap_err().0, ErrorCode::NotFound);
    let outcome = checked(f.job.clone(), "wrong-password").await;
    assert_eq!(outcome.check, PasswordCheck::Rejected);
    assert_eq!(outcome.result.unwrap_err().0, ErrorCode::Unauthenticated);
    let outcome = f
        .core
        .login(
            Login {
                username: "alice".into(),
                password: "wrong-password".into(),
            },
            limeos_api::PasswordAttempt::default(),
        )
        .await;
    assert_eq!(outcome.check, PasswordCheck::Rejected);
    let outcome = f
        .core
        .login(
            Login {
                username: "alice".into(),
                password: PASSWORD.into(),
            },
            limeos_api::PasswordAttempt::default(),
        )
        .await;
    assert_eq!(outcome.check, PasswordCheck::Accepted);
    assert!(outcome.result.is_ok());
}

/// The fixture's core with a real dummy hash and `unknown` comparisons a
/// minute for names with no account.
fn login_core(f: &Fixture, unknown: u32) -> Core {
    let mut core = f.core.clone();
    core.dummy_hash = limeos_identity::hash("dummy-long-password").unwrap().into();
    core.password_workers = Arc::new(PasswordWorkers::with_limits(
        2,
        8,
        std::time::Duration::from_secs(2),
        unknown,
    ));
    core
}
async fn try_login(
    core: &Core,
    username: &str,
    password: &str,
) -> limeos_api::PasswordChecked<limeos_api::IssuedSession> {
    use limeos_api::Backend;
    core.login(
        Login {
            username: username.into(),
            password: password.into(),
        },
        limeos_api::PasswordAttempt::default(),
    )
    .await
}

#[tokio::test]
async fn a_missing_account_goes_through_the_real_dummy_hash_and_is_never_accepted() {
    use limeos_api::PasswordCheck;
    let f = fixture().await;
    let core = login_core(&f, 3);
    // The dummy hash is a real argon2id hash, so this is a real comparison.
    assert!(core.dummy_hash.starts_with("$argon2id$"));
    for password in ["wrong-password", "dummy-long-password"] {
        let outcome = try_login(&core, "nobody", password).await;
        assert_eq!(outcome.check, PasswordCheck::Rejected, "{password}");
        assert_eq!(
            outcome.result.err(),
            Some(Error(ErrorCode::Unauthenticated))
        );
    }
}

#[tokio::test]
async fn a_comparison_is_marked_started_only_once_a_worker_is_admitted() {
    use limeos_api::PasswordAttempt;
    let f = fixture().await;
    let mut core = login_core(&f, 3);
    // A wait far longer than the cancellation below, whatever the runner load.
    core.password_workers = Arc::new(PasswordWorkers::with_limits(
        2,
        8,
        std::time::Duration::from_secs(30),
        3,
    ));
    let login = |attempt: PasswordAttempt| {
        let core = core.clone();
        async move {
            use limeos_api::Backend;
            let input = Login {
                username: "alice".into(),
                password: PASSWORD.into(),
            };
            core.login(input, attempt).await
        }
    };
    // Both workers busy: the request is cancelled while it waits.
    let busy = (
        core.password_workers.admit().await.unwrap(),
        core.password_workers.admit().await.unwrap(),
    );
    let attempt = PasswordAttempt::default();
    let waited = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        login(attempt.clone()),
    )
    .await;
    assert!(waited.is_err());
    assert!(!attempt.comparison_started());
    drop(busy);
    let attempt = PasswordAttempt::default();
    assert!(login(attempt.clone()).await.result.is_ok());
    assert!(attempt.comparison_started());
}

#[tokio::test]
async fn a_flood_of_unknown_names_spends_their_budget_but_never_a_real_account() {
    use limeos_api::PasswordCheck;
    let f = fixture().await;
    let core = login_core(&f, 3);
    for i in 0..3 {
        let outcome = try_login(&core, &format!("ghost{i}"), "wrong-password").await;
        assert_eq!(outcome.check, PasswordCheck::Rejected);
        assert_eq!(
            outcome.result.err(),
            Some(Error(ErrorCode::Unauthenticated))
        );
    }
    // Spent: refused without a comparison, so the per-name charge is refunded.
    for i in 3..10 {
        let outcome = try_login(&core, &format!("ghost{i}"), "wrong-password").await;
        assert_eq!(outcome.check, PasswordCheck::NotReached);
        assert_eq!(outcome.result.err(), Some(Error(ErrorCode::Overloaded)));
    }
    // A real account is compared as before, wrong or right.
    let outcome = try_login(&core, "alice", "wrong-password").await;
    assert_eq!(outcome.check, PasswordCheck::Rejected);
    let outcome = try_login(&core, "alice", PASSWORD).await;
    assert_eq!(outcome.check, PasswordCheck::Accepted);
    assert!(outcome.result.is_ok());
}

/// One HTTP login against the real router over loopback: status and body,
/// without the per-response `audit_id`.
async fn http_login(address: std::net::SocketAddr, username: &str) -> (String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = serde_json::json!({"username": username, "password": "wrong-password"}).to_string();
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST /api/v1/auth/login HTTP/1.1\r\nHost: localhost\r\nOrigin: https://localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let mut json: serde_json::Value = serde_json::from_str(body).unwrap();
    json.as_object_mut().unwrap().remove("audit_id");
    (head.lines().next().unwrap().to_owned(), json.to_string())
}

#[tokio::test]
async fn before_the_flood_budget_is_spent_a_missing_name_answers_like_a_wrong_password() {
    let f = fixture().await;
    let core = login_core(&f, 3);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(limeos_api::serve(
        listener,
        limeos_api::router(core, "https://localhost".into()),
    ));
    let missing = http_login(address, "nobody").await;
    let wrong = http_login(address, "alice").await;
    assert_eq!(missing, wrong);
    assert!(missing.0.contains(" 401 "));
    server.abort();
}

#[tokio::test]
async fn authorization_matrix() {
    let f = fixture().await;
    let digest = f.view().await.evidence_digest.unwrap();
    let (token, csrf) = (f.session.token.clone(), f.session.view.csrf_token.clone());
    let decide = |token: &str, csrf: &str, password: &str| {
        f.core.human_resolve(
            token.into(),
            csrf.into(),
            f.job.clone(),
            f.input(DeclaredOutcome::NotDone, &digest, password),
        )
    };
    // Wrong password: nothing recorded, executor never asked.
    assert_eq!(
        decide(&token, &csrf, "wrong-password").await.unwrap_err(),
        Error(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        decide(&token, &csrf, "").await.unwrap_err(),
        Error(ErrorCode::InvalidInput)
    );
    // Wrong CSRF binding.
    assert_eq!(
        decide(&token, &"0".repeat(64), PASSWORD).await.unwrap_err(),
        Error(ErrorCode::Forbidden)
    );
    // Not a session at all.
    assert_eq!(
        decide(&"9".repeat(64), &csrf, PASSWORD).await.unwrap_err(),
        Error(ErrorCode::Unauthenticated)
    );
    assert!(f.view().await.pending.is_none());
    assert_eq!(f.counts().1, 0);
    // A stale session: revoked after login.
    let revoked = login(&f.core).await;
    let session = limeos_identity::digest(&revoked.token);
    f.core
        .db
        .call(move |s| s.revoke_session(&session, now()))
        .await
        .unwrap();
    assert_eq!(
        decide(&revoked.token, &revoked.view.csrf_token, PASSWORD)
            .await
            .unwrap_err(),
        Error(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        f.core
            .human_resolution_view(revoked.token, f.job.clone())
            .await
            .unwrap_err(),
        Error(ErrorCode::Unauthenticated)
    );
    assert_eq!(f.state().await, JobState::NeedsIntervention);
    // Demoted: every older session dies, and a fresh login as an operator is
    // refused both ways even with the right password.
    let id = f.session.view.principal.id.clone();
    f.core
        .db
        .call(move |s| {
            s.revise_grants(
                &id,
                limeos_domain::Role::Operator,
                &[Scope {
                    operation: limeos_domain::Operation::HealthRead,
                    resource: "*".into(),
                }],
                now(),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        decide(&token, &csrf, PASSWORD).await.unwrap_err(),
        Error(ErrorCode::Unauthenticated)
    );
    let operator = login(&f.core).await;
    assert_eq!(
        decide(&operator.token, &operator.view.csrf_token, PASSWORD)
            .await
            .unwrap_err(),
        Error(ErrorCode::Forbidden)
    );
    assert_eq!(
        f.core
            .human_resolution_view(operator.token, f.job.clone())
            .await
            .unwrap_err(),
        Error(ErrorCode::Forbidden)
    );
    assert_eq!(f.counts(), (2, 0, 0));
}

#[tokio::test]
async fn the_root_peer_path_names_an_administrator_and_marks_the_reason() {
    let f = fixture().await;
    let digest = f.view().await.evidence_digest.unwrap();
    let rpc = |admin: &str, reason: &str| {
        f.core.rpc_resolve(
            admin.into(),
            f.job.clone(),
            DeclaredOutcome::AcceptedDivergence,
            reason.into(),
            digest.clone(),
        )
    };
    assert_eq!(
        rpc("nobody", LONG).await.unwrap_err(),
        Error(ErrorCode::NotFound)
    );
    assert_eq!(
        rpc("bad name!", LONG).await.unwrap_err(),
        Error(ErrorCode::InvalidInput)
    );
    // The operator's own text must meet the rule before the channel is added.
    assert_eq!(
        rpc("alice", "short").await.unwrap_err(),
        Error(ErrorCode::InvalidInput)
    );
    assert_eq!(f.counts().1, 0);
    let read = f
        .core
        .rpc_resolution_view("alice".into(), f.job.clone())
        .await
        .unwrap();
    assert_eq!(read.evidence_digest, Some(digest.clone()));
    let done = rpc("alice", LONG).await.unwrap();
    assert_eq!(done.status, ResolveStatus::Resolved);
    let record = done.resolution.unwrap();
    assert_eq!(record.resolved_by, f.session.view.principal.id);
    assert_eq!(record.reason, format!("{LONG} [limeosctl, root peer]"));
    assert_eq!(f.state().await, JobState::Resolved);
}

#[tokio::test]
async fn a_non_administrator_cannot_be_named_over_the_root_socket() {
    let f = fixture().await;
    let id = f.session.view.principal.id.clone();
    f.core
        .db
        .call(move |s| s.revise_grants(&id, limeos_domain::Role::Viewer, &[], now()))
        .await
        .unwrap();
    assert_eq!(
        f.core
            .rpc_resolution_view("alice".into(), f.job.clone())
            .await
            .unwrap_err(),
        Error(ErrorCode::Forbidden)
    );
    assert_eq!(f.counts(), (0, 0, 0));
}

/// What recovery records when its closing lookup finds no receipt for a
/// container job: the closure receipt, which settles it.
fn closure_of(job: &limeos_domain::RestartJob) -> limeos_domain::RestartReceipt {
    limeos_domain::RestartReceipt {
        action: job.id.clone(),
        plan_digest: limeos_identity::digest(&serde_json::to_string(&job.plan).unwrap()),
        operation: job.plan.operation,
        before: job.plan.expected.clone(),
        state: limeos_domain::ExecutionState::PreconditionChanged,
        error: Some(ErrorCode::Expired),
    }
}
async fn settle_by_recovery(f: &Fixture) {
    let (p, id) = (f.session.view.principal.clone(), f.job.clone());
    f.core
        .db
        .call(move |s| {
            let job = s.container_job(&p, &id)?;
            s.record_container_result(&job, &closure_of(&job), None, now())
        })
        .await
        .unwrap();
}
async fn superseded_events(f: &Fixture) -> Vec<limeos_domain::EventKind> {
    f.core
        .restart_progress(f.session.token.clone(), f.job.clone(), 0)
        .await
        .unwrap()
        .events
        .into_iter()
        .map(|e| e.event)
        .filter(|e| matches!(e, limeos_domain::EventKind::JobResolutionSuperseded { .. }))
        .collect()
}

/// Recovery records a closure while an operator's request is open: the job
/// is `precondition_changed`, so the request can never complete. It ends
/// superseded, once, with an event naming that state, whether the retry
/// loop finds it or a completion in flight loses the race; it is never sent
/// to the executor again and never stays pending.
#[tokio::test]
async fn a_resolution_overtaken_by_recovery_ends_superseded_once() {
    use limeos_domain::{EventKind, ReceiptAbsence};
    let expected = vec![EventKind::JobResolutionSuperseded {
        state: JobState::PreconditionChanged,
    }];
    let f = fixture().await;
    f.set(Mode::Down);
    let pending = f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    assert_eq!(pending.status, ResolveStatus::Pending);
    settle_by_recovery(&f).await;
    f.set(Mode::Absent(ReceiptAbsence::NeverRecorded));
    let sent = f.counts().1;
    for _ in 0..2 {
        f.retry(&mut Rotation::default()).await;
    }
    assert_eq!(f.state().await, JobState::PreconditionChanged);
    let view = f.view().await;
    assert!(view.pending.is_none() && view.resolution.is_none());
    assert_eq!(superseded_events(&f).await, expected);
    assert_eq!(f.counts().1, sent);

    // The same race with the completion already on its way.
    let f = fixture().await;
    f.set(Mode::Down);
    f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    let id = f.job.clone();
    let pending = f
        .core
        .db
        .call(move |s| s.pending_resolution(&id))
        .await
        .unwrap()
        .unwrap();
    settle_by_recovery(&f).await;
    f.set(Mode::Absent(ReceiptAbsence::NeverRecorded));
    let actor = f.session.view.principal.clone();
    assert!(matches!(
        f.core.finish_resolution(&pending, &actor).await,
        Finish::Superseded(JobState::PreconditionChanged)
    ));
    assert!(f.view().await.pending.is_none());
    f.retry(&mut Rotation::default()).await;
    assert_eq!(superseded_events(&f).await, expected);
    assert_eq!(f.counts().2, 0);
}

/// A hundred requests overtaken by recovery, all older than a request on a
/// job that is still held: the retry loop closes them and completes the
/// newer request on its first pass instead of retrying the stuck ones.
#[tokio::test]
async fn a_hundred_superseded_requests_do_not_starve_a_newer_one() {
    let f = fixture().await;
    let p = f.session.view.principal.clone();
    f.core
        .db
        .call(move |s| {
            let other = ContainerSnapshot {
                resource: format!("container:{}", "e".repeat(64)),
                ..engine_state(T0, true)
            };
            let evidence = limeos_domain::ResolutionEvidence::observe(
                ContainerAction::Restart,
                &[&other],
                std::slice::from_ref(&Some(other.clone())),
            )?;
            for i in 0..100 {
                let plan = s.plan_container(&p, ContainerAction::Restart, &other, now())?;
                let approval = s.approve_container(&p, &plan.plan.id, &plan.digest, now())?;
                let queued = s.queue_container(
                    &p,
                    &format!("stuck{i}"),
                    &plan,
                    &approval.token,
                    &other,
                    now(),
                )?;
                let mut job = s.claim_container(&p, &queued.id, &other, &"1".repeat(64), now())?;
                s.transition(
                    &job.id,
                    JobState::Running,
                    JobState::NeedsIntervention,
                    now(),
                )?;
                job.state = JobState::NeedsIntervention;
                s.request_resolution(
                    &p,
                    &job.id,
                    DeclaredOutcome::NotDone,
                    LONG,
                    &evidence,
                    &evidence.digest()?,
                    now(),
                )?;
                s.record_container_result(&job, &closure_of(&job), None, now())?;
            }
            Ok(())
        })
        .await
        .unwrap();
    f.set(Mode::Down);
    assert_eq!(
        f.resolve(DeclaredOutcome::NotDone).await.unwrap().status,
        ResolveStatus::Pending
    );
    f.set(Mode::Ok);
    f.retry(&mut Rotation::default()).await;
    assert_eq!(f.state().await, JobState::Resolved);
}

/// The retry loop superseded (deleted) the request before a completion in
/// flight could record the executor's confirmation: nothing remains to
/// retry, so the operator is told the job already settled, with its state,
/// never that the request is pending.
#[tokio::test]
async fn a_completion_whose_request_was_already_superseded_reports_the_settled_job() {
    use limeos_domain::EventKind;
    let f = fixture().await;
    f.set(Mode::Down);
    f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    let id = f.job.clone();
    let pending = f
        .core
        .db
        .call(move |s| s.pending_resolution(&id))
        .await
        .unwrap()
        .unwrap();
    settle_by_recovery(&f).await;
    let superseded = f
        .core
        .db
        .call(|s| s.supersede_resolutions(now()))
        .await
        .unwrap();
    assert_eq!(superseded.len(), 1);
    f.set(Mode::Ok);
    let actor = f.session.view.principal.clone();
    assert!(matches!(
        f.core.finish_resolution(&pending, &actor).await,
        Finish::Superseded(JobState::PreconditionChanged)
    ));
    assert_eq!(
        superseded_events(&f).await,
        vec![EventKind::JobResolutionSuperseded {
            state: JobState::PreconditionChanged,
        }]
    );
    assert_eq!(f.state().await, JobState::PreconditionChanged);
    assert_eq!(f.counts().2, 0);
}

/// A job held across a change of the executor's receipt store is disputed:
/// a never-recorded closure is not evidence for it, so "verified done" is
/// allowed on the operator's own fresh measurement, and core tells the
/// executor so. Without the dispute the restriction stays (see
/// `a_never_recorded_closure_is_not_resolved_as_verified_done`).
#[tokio::test]
async fn a_disputed_job_accepts_every_declaration_its_evidence_supports() {
    let f = fixture().await;
    f.core
        .db
        .call(|s| s.observe_executor_epoch(&"2".repeat(64), now()))
        .await
        .unwrap();
    f.exec.lock().unwrap().current = Some(engine_state(T1, true));
    f.set(Mode::Absent(limeos_domain::ReceiptAbsence::NeverRecorded));
    let done = f.resolve(DeclaredOutcome::VerifiedDone).await.unwrap();
    assert_eq!(done.status, ResolveStatus::Resolved, "{}", done.message);
    assert_eq!(f.state().await, JobState::Resolved);
    assert!(matches!(
        f.exec.lock().unwrap().last,
        Some(Request::ResolveAction { disputed: true, .. })
    ));
    assert_eq!(f.counts().2, 0);
}
