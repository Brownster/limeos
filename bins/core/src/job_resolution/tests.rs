use super::*;
use limeos_contracts::{Enrollment, Login};
use limeos_domain::{ContainerSnapshot, ResolveKind, ResolveReceipt};
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
            }
        }
        _ => exec.effects += 1,
    }
    Some(response)
}
fn core(db: Database, socket: PathBuf) -> Core {
    Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new(socket),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
    }
}
async fn login(core: &Core) -> limeos_api::IssuedSession {
    core.login(Login {
        username: "alice".into(),
        password: PASSWORD.into(),
    })
    .await
    .result
    .unwrap()
}
/// An administrator, a restart job claimed and then left `needs_intervention`,
/// and an executor that still sees the container as planned.
async fn fixture() -> Fixture {
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
    let resource = engine_state(T0, true).resource;
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
            s.claim_container(&p, &id, &engine_state(T0, true), now())?;
            s.transition(&id, JobState::Running, JobState::NeedsIntervention, now())
        })
        .await
        .unwrap();
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
    async fn retry(&self, retry: &mut HashMap<String, Instant>) {
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
        evidence_digest,
        declared,
        resolved_by,
        ..
    }) = f.exec.lock().unwrap().last.take()
    else {
        panic!("no resolve sent")
    };
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
    let mut retry = HashMap::new();
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
    let mut retry = HashMap::new();
    f.core.complete_pending_resolutions(&mut retry, 3600).await;
    assert_eq!(f.counts().1, sent);
    // Still down: stays pending, and the next tick does not hammer.
    f.retry(&mut retry).await;
    f.retry(&mut retry).await;
    assert_eq!(f.counts().1, sent + 1);
    assert!(f.view().await.pending.is_some());
    // A definite refusal on retry withdraws, acting as the recorded administrator.
    f.set(Mode::Refuse(ErrorCode::Conflict));
    retry.clear();
    f.retry(&mut retry).await;
    assert!(f.view().await.pending.is_none());
    // Back up with a new decision: completed by the loop with no HTTP help.
    f.set(Mode::Down);
    f.resolve(DeclaredOutcome::NotDone).await.unwrap();
    f.set(Mode::Ok);
    retry.clear();
    f.retry(&mut retry).await;
    assert_eq!(f.state().await, JobState::Resolved);
    f.retry(&mut retry).await;
    assert!(retry.is_empty());
}

#[tokio::test]
async fn the_password_check_is_reported_apart_from_what_follows_it() {
    use limeos_api::{Backend, PasswordCheck};
    let f = fixture().await;
    let checked = |job: String, password: &str| {
        let core = f.core.clone();
        let (token, csrf) = (f.session.token.clone(), f.session.view.csrf_token.clone());
        let input = f.input(DeclaredOutcome::NotDone, &"d".repeat(64), password);
        async move { core.resolve_job(token, csrf, job, input).await }
    };
    // Correct password, no such job: refused after the match.
    let outcome = checked("0".repeat(64), PASSWORD).await;
    assert_eq!(outcome.check, PasswordCheck::Accepted);
    assert_eq!(outcome.result.unwrap_err().0, ErrorCode::NotFound);
    let outcome = checked(f.job.clone(), "wrong-password").await;
    assert_eq!(outcome.check, PasswordCheck::Rejected);
    assert_eq!(outcome.result.unwrap_err().0, ErrorCode::Unauthenticated);
    // A missing account is compared against the dummy hash: never accepted.
    for username in ["alice", "nobody"] {
        let outcome = f
            .core
            .login(Login {
                username: username.into(),
                password: "wrong-password".into(),
            })
            .await;
        assert_eq!(outcome.check, PasswordCheck::Rejected, "{username}");
    }
    let outcome = f
        .core
        .login(Login {
            username: "alice".into(),
            password: PASSWORD.into(),
        })
        .await;
    assert_eq!(outcome.check, PasswordCheck::Accepted);
    assert!(outcome.result.is_ok());
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
