use super::*;
use limeos_domain::{
    ContainerAction, ContainerSnapshot, ExecutionState, JobState, StackJob, StackMemberReceipt,
    StackReceipt, StackSnapshot, aggregate_state,
    deployment::discovery::{
        CatalogFacts, Declaration, DiscoveryCohort, EngineFacts, FileStatus, Membership,
        ProjectIdentity,
    },
    verify_stack_member,
};
use limeos_executor_protocol::{Receipt, Request};
use std::sync::Mutex;

#[derive(Default)]
struct Engine {
    members: Vec<ContainerSnapshot>,
    receipt: Option<StackReceipt>,
    inspects: usize,
    executes: usize,
    receipts: usize,
    verifies: usize,
    resolves: usize,
    /// Fail (response lost, effect applied) at this target index.
    fail_at: Option<usize>,
    /// Write Prepared receipts, apply the first effect and then drop the call.
    crash_after_prepare: bool,
    /// ExecuteStack fails before any row is written.
    execute_error: Option<ErrorCode>,
    /// Receipt lookups fail.
    lookup_unavailable: bool,
    /// ResolveAction finds no row, as after retention pruned it.
    resolve_missing: bool,
    /// ResolveAction finds a terminal receipt with this recorded outcome and
    /// refuses any declaration it does not allow.
    resolve_recorded: Option<ExecutionState>,
    /// The executor's prune watermark: a lookup without a row answers
    /// `NeverRecorded` only for a plan expiring after it.
    watermark: i64,
    /// The failing target's effect is lost: nothing proves it later.
    fail_without_effect: bool,
    /// What a closure recorded for the action, once a lookup or a
    /// resolution has closed it (the real executor's `closed` column).
    closed: Option<limeos_domain::ReceiptAbsence>,
    /// Receipt lookups that did not ask to close: core never sends one.
    plain_reads: usize,
    /// The receipt store's epoch, sent with every receipt answer once set.
    epoch: Option<String>,
    /// After answering `ReceiptEpoch`, stop listening: the next connection
    /// is refused, so the execute request is never written.
    vanish_after_epoch: bool,
}
fn snap(c: char, running: bool) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running,
    }
}
fn ordinary() -> Vec<ContainerSnapshot> {
    vec![snap('a', true), snap('b', true), snap('c', false)]
}
fn cohort(members: &[ContainerSnapshot], declared: &[&str]) -> DiscoveryCohort {
    crowded_cohort(members, declared, 0)
}
/// The same Engine, but read over the bound: `truncated` more containers exist.
fn truncated_cohort(members: &[ContainerSnapshot], truncated: u16) -> DiscoveryCohort {
    let base = cohort(members, &["web", "db"]);
    let engine = base.engine().unwrap();
    DiscoveryCohort::new(
        base.catalog().cloned(),
        Some(EngineFacts {
            truncated,
            ..engine.clone()
        }),
    )
    .unwrap()
}
/// The registered project's members plus `unrelated` other containers spread over
/// two other projects and standalone containers, all in the same Engine.
fn crowded_cohort(
    members: &[ContainerSnapshot],
    declared: &[&str],
    unrelated: usize,
) -> DiscoveryCohort {
    let project = ProjectIdentity {
        name: "media".into(),
        working_directory: "/opt/Media".into(),
        config_files: vec!["/opt/Media/compose.yaml".into()],
    };
    let service = |c: &ContainerSnapshot| {
        if c.resource.contains("cccc") {
            "db"
        } else {
            "web"
        }
    };
    let mut engine: Vec<_> = members
        .iter()
        .map(|c| Membership::Compose {
            container: c.clone(),
            project: project.clone(),
            service: service(c).into(),
            one_off: false,
        })
        .collect();
    // A running one-off is part of the project but never a lifecycle target.
    engine.push(Membership::Compose {
        container: snap('d', true),
        project: project.clone(),
        service: "web".into(),
        one_off: true,
    });
    for i in 0..unrelated {
        let container = ContainerSnapshot {
            resource: format!("container:{:064x}", 0x1000 + i),
            image: format!("sha256:{}", "c".repeat(64)),
            started_at: "2026-10-05T07:00:00Z".into(),
            running: i % 3 != 0,
        };
        engine.push(match i % 4 {
            0 | 1 => Membership::Compose {
                container,
                project: ProjectIdentity {
                    name: format!("other-{}", i % 2),
                    working_directory: format!("/opt/Other{}", i % 2),
                    config_files: vec![format!("/opt/Other{}/compose.yaml", i % 2)],
                },
                service: format!("svc-{i}"),
                one_off: false,
            },
            _ => Membership::Standalone { container },
        });
    }
    DiscoveryCohort::new(
        Some(CatalogFacts {
            engine_id: "engine".into(),
            sampled_at: now(),
            stacks: vec![Declaration {
                stack: "registered".into(),
                project: "media".into(),
                working_directory: "/opt/Media".into(),
                config_files: vec!["/opt/Media/compose.yaml".into()],
                services: declared.iter().map(|s| s.to_string()).collect(),
                files: FileStatus::NotInspected,
            }],
        }),
        Some(EngineFacts {
            engine_id: "engine".into(),
            sampled_at: now(),
            members: engine,
            truncated: 0,
            omitted: Vec::new(),
            hidden_projects: Vec::new(),
            unattributed: 0,
        }),
    )
    .unwrap()
}
struct Fixture {
    _dir: tempfile::TempDir,
    core: Core,
    token: String,
    csrf: String,
    engine: Arc<Mutex<Engine>>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn principal(&self) -> limeos_domain::Principal {
        self.core
            .session(self.token.clone(), None)
            .await
            .unwrap()
            .principal
    }
    async fn plan(&self, operation: ContainerAction) -> Result<limeos_domain::PlannedStackAction> {
        self.core
            .plan_stack(
                self.token.clone(),
                self.csrf.clone(),
                limeos_contracts::StackInput {
                    project: "media".into(),
                    operation,
                },
            )
            .await
    }
    async fn approved(
        &self,
        operation: ContainerAction,
        key: &str,
    ) -> limeos_contracts::QueueStackInput {
        let proposal = self.plan(operation).await.unwrap();
        let approval = self
            .core
            .approve_stack(
                self.token.clone(),
                self.csrf.clone(),
                proposal.plan.id.clone(),
                proposal.digest.clone(),
            )
            .await
            .unwrap();
        limeos_contracts::QueueStackInput {
            proposal,
            approval: approval.token,
            idempotency_key: key.into(),
        }
    }
    async fn queued(&self, operation: ContainerAction, key: &str) -> StackJob {
        let input = self.approved(operation, key).await;
        self.core
            .queue_stack(self.token.clone(), self.csrf.clone(), input)
            .await
            .unwrap()
    }
    async fn state(&self, job: &StackJob) -> JobState {
        self.core
            .stack_job_progress(self.token.clone(), job.id.clone(), 0)
            .await
            .unwrap()
            .job
            .state
    }
    async fn current(&self, job: &StackJob) -> StackJob {
        let progress = self
            .core
            .stack_job_progress(self.token.clone(), job.id.clone(), 0)
            .await
            .unwrap();
        progress.job
    }
}
fn apply(operation: ContainerAction, member: &mut ContainerSnapshot) {
    match operation {
        ContainerAction::Stop => member.running = false,
        _ => {
            member.running = true;
            member.started_at = "2026-10-05T07:05:00Z".into();
        }
    }
}
/// Record a closure as the real executor does: a `never_recorded` one keeps
/// a `precondition_changed` receipt that later lookups and executes return.
fn close(
    state: &mut Engine,
    action: &str,
    digest: &str,
    plan: &limeos_domain::StackPlan,
    absence: limeos_domain::ReceiptAbsence,
) {
    state.closed = Some(absence);
    if absence == limeos_domain::ReceiptAbsence::NeverRecorded {
        let members: Vec<_> = plan
            .targets()
            .into_iter()
            .map(|t| StackMemberReceipt {
                resource: t.resource.clone(),
                state: ExecutionState::PreconditionChanged,
                error: Some(ErrorCode::Expired),
            })
            .collect();
        state.receipt = Some(StackReceipt {
            action: action.into(),
            plan_digest: digest.into(),
            operation: plan.operation,
            before: plan.expected.clone(),
            state: aggregate_state(&members),
            members,
            error: Some(ErrorCode::Expired),
        });
    }
}
fn reply(state: &mut Engine, request: Request) -> Option<Receipt> {
    let mut response = Receipt::empty();
    if matches!(
        request,
        Request::ExecuteStack { .. }
            | Request::StackReceipt { .. }
            | Request::VerifyStack { .. }
            | Request::ResolveAction { .. }
    ) {
        response.epoch = state.epoch.clone();
    }
    match request {
        Request::ReceiptEpoch { .. } => response.epoch = state.epoch.clone(),
        Request::InspectStack { .. } => {
            state.inspects += 1;
            let mut members = state.members.clone();
            members.sort_by(|a, b| a.resource.cmp(&b.resource));
            response.stack_members = Some(StackSnapshot::new("media", members).unwrap());
        }
        Request::ExecuteStack { request, .. } => {
            state.executes += 1;
            // A closed action never reaches the Engine.
            match state.closed {
                Some(limeos_domain::ReceiptAbsence::NeverRecorded) => {
                    response.stack = state.receipt.clone();
                    return Some(response);
                }
                Some(limeos_domain::ReceiptAbsence::NoLongerHeld) => {
                    response.ready = false;
                    response.error = Some(ErrorCode::Expired);
                    return Some(response);
                }
                None => {}
            }
            if let Some(code) = state.execute_error {
                response.ready = false;
                response.error = Some(code);
                return Some(response);
            }
            let operation = request.plan.operation;
            let mut members = Vec::new();
            for (index, target) in request.plan.targets().into_iter().enumerate() {
                let sent = state.crash_after_prepare && index > 0
                    || state.fail_at.is_some_and(|f| index > f);
                let (outcome, error) = if sent {
                    (
                        ExecutionState::PreconditionChanged,
                        Some(ErrorCode::Conflict),
                    )
                } else if state.crash_after_prepare {
                    (ExecutionState::Prepared, None)
                } else if state.fail_at == Some(index) {
                    (ExecutionState::OutcomeUnknown, Some(ErrorCode::Unavailable))
                } else {
                    (ExecutionState::EffectAccepted, None)
                };
                if !(sent || state.fail_without_effect && state.fail_at == Some(index)) {
                    let live = state
                        .members
                        .iter_mut()
                        .find(|m| m.resource == target.resource)
                        .unwrap();
                    apply(operation, live);
                }
                members.push(StackMemberReceipt {
                    resource: target.resource.clone(),
                    state: outcome,
                    error,
                });
            }
            let receipt = StackReceipt {
                action: request.action,
                plan_digest: request.plan_digest,
                operation,
                before: request.plan.expected,
                state: aggregate_state(&members),
                members,
                error: None,
            };
            state.receipt = Some(receipt.clone());
            if state.crash_after_prepare {
                return None;
            }
            response.stack = Some(receipt);
        }
        Request::StackReceipt {
            action,
            digest,
            plan,
            close_if_absent,
            ..
        } => {
            state.receipts += 1;
            if !close_if_absent {
                state.plain_reads += 1;
            }
            // Core sends the stored plan with the digest it names.
            assert_eq!(
                limeos_identity::digest(&serde_json::to_string(&plan).unwrap()),
                digest
            );
            if state.lookup_unavailable {
                response.ready = false;
                response.error = Some(ErrorCode::Unavailable);
                return Some(response);
            }
            // The real executor turns unresolved Prepared rows into uncertainty.
            if let Some(receipt) = state.receipt.as_mut() {
                for member in &mut receipt.members {
                    if member.state == ExecutionState::Prepared {
                        member.state = ExecutionState::OutcomeUnknown;
                        member.error = Some(ErrorCode::StateNotDurable);
                    }
                }
                receipt.state = aggregate_state(&receipt.members);
            }
            if state.receipt.is_none() && state.closed.is_none() {
                let absence =
                    limeos_domain::ReceiptAbsence::judge(plan.expires_at, state.watermark);
                if !close_if_absent {
                    response.absence = Some(absence);
                    return Some(response);
                }
                close(state, &action, &digest, &plan, absence);
            }
            response.closed = state.closed.is_some();
            match state.closed {
                Some(absence @ limeos_domain::ReceiptAbsence::NoLongerHeld) => {
                    response.absence = Some(absence)
                }
                _ => response.stack = state.receipt.clone(),
            }
        }
        Request::VerifyStack { after, .. } => {
            state.verifies += 1;
            let live = state.members.clone();
            let receipt = state.receipt.as_mut().unwrap();
            for member in &mut receipt.members {
                if !matches!(
                    member.state,
                    ExecutionState::EffectAccepted | ExecutionState::OutcomeUnknown
                ) {
                    continue;
                }
                let before = receipt
                    .before
                    .members
                    .iter()
                    .find(|m| m.resource == member.resource)
                    .unwrap();
                let proof = after.iter().find(|a| a.resource == member.resource);
                if proof.is_some_and(|p| {
                    live.contains(p) && verify_stack_member(receipt.operation, before, p).is_ok()
                }) {
                    member.state = ExecutionState::Verified;
                    member.error = None;
                }
            }
            receipt.state = aggregate_state(&receipt.members);
            response.stack = Some(receipt.clone());
        }
        Request::Inspect { resource, .. } => {
            state.inspects += 1;
            match state.members.iter().find(|m| m.resource == resource) {
                Some(member) => response.inspection = Some(member.clone()),
                None => {
                    response.ready = false;
                    response.error = Some(ErrorCode::NotFound);
                }
            }
        }
        Request::ResolveAction {
            kind,
            action,
            evidence_digest,
            declared,
            resolved_by,
            plan,
            ..
        } => {
            state.resolves += 1;
            if state.resolve_missing {
                // The executor closes the action before it answers.
                let absence = state.closed.unwrap_or(limeos_domain::ReceiptAbsence::judge(
                    plan.expires_at(),
                    state.watermark,
                ));
                state.closed.get_or_insert(absence);
                response.absence = Some(absence);
                response.closed = true;
                return Some(response);
            }
            if let Some(recorded) = state.resolve_recorded {
                let allowed = limeos_domain::declarations_for_recorded(recorded).unwrap();
                if !allowed.contains(&declared) {
                    response.resolution_refusal = Some(limeos_domain::ResolveRefusal {
                        recorded,
                        declared,
                        allowed: allowed.to_vec(),
                    });
                    return Some(response);
                }
            }
            response.resolution = Some(limeos_domain::ResolveReceipt {
                kind,
                action,
                evidence_digest,
                declared,
                resolved_by,
            });
        }
        _ => panic!("unexpected operation"),
    }
    Some(response)
}
async fn fixture(role: limeos_domain::Role, grants: Option<Vec<Scope>>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = limeos_identity::digest(&format!("csrf:{token}"));
    let (digest, csrf_digest) = (
        limeos_identity::digest(&token),
        limeos_identity::digest(&csrf),
    );
    db.call(move |s| {
        s.issue_bootstrap("bootstrap", now())?;
        s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
        let record = s.login_record("admin")?.unwrap();
        if let Some(grants) = grants {
            s.revise_grants(&record.principal.id, role, &grants, now())?;
        }
        let record = s.login_record("admin")?.unwrap();
        s.create_session(&record, None, &digest, &csrf_digest, now())?;
        Ok(())
    })
    .await
    .unwrap();
    let path = dir.path().join("executor.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let engine = Arc::new(Mutex::new(Engine {
        members: ordinary(),
        epoch: Some("1".repeat(64)),
        ..Engine::default()
    }));
    let served = engine.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
            let vanish = matches!(request, Request::ReceiptEpoch { .. })
                && served.lock().unwrap().vanish_after_epoch;
            let response = reply(&mut served.lock().unwrap(), request);
            if let Some(response) = response {
                let _ = limeos_contracts::write_frame(&mut stream, &response).await;
            }
            if vanish {
                // The listener is dropped: later connections are refused.
                break;
            }
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
    core.stacks
        .update(&cohort(&ordinary(), &["web", "db"]))
        .unwrap();
    Fixture {
        _dir: dir,
        core,
        token,
        csrf,
        engine,
        task,
    }
}
async fn admin() -> Fixture {
    fixture(limeos_domain::Role::Administrator, None).await
}

#[tokio::test]
async fn each_action_plans_from_the_cohort_runs_once_and_succeeds_after_member_verification() {
    for (operation, expected) in [
        (ContainerAction::Restart, 2),
        (ContainerAction::Stop, 2),
        (ContainerAction::Start, 1),
    ] {
        let f = admin().await;
        let input = f.approved(operation, "key").await;
        // The one-off is not a member; stopped and running ordinary ones are.
        assert_eq!(input.proposal.plan.expected.members.len(), 3);
        assert_eq!(input.proposal.plan.targets().len(), expected);
        let job = f
            .core
            .queue_stack(f.token.clone(), f.csrf.clone(), input)
            .await
            .unwrap();
        f.core
            .dispatch_stack(f.principal().await, job.clone())
            .await
            .unwrap();
        let progress = f
            .core
            .stack_job_progress(f.token.clone(), job.id.clone(), 0)
            .await
            .unwrap();
        assert_eq!(progress.job.state, JobState::Succeeded);
        let receipt = progress.receipt.unwrap();
        assert_eq!(receipt.members.len(), expected);
        assert!(
            receipt
                .members
                .iter()
                .all(|m| m.state == ExecutionState::Verified)
        );
        assert_eq!(f.engine.lock().unwrap().executes, 1);
        assert_eq!(
            f.core.stack_job_list(f.token.clone()).await.unwrap().len(),
            1
        );
        f.task.abort();
    }
}

#[tokio::test]
async fn a_lost_queue_response_replays_the_job_without_any_executor_contact() {
    let f = admin().await;
    let input = f.approved(ContainerAction::Restart, "retry").await;
    let first = f
        .core
        .queue_stack(f.token.clone(), f.csrf.clone(), input.clone())
        .await
        .unwrap();
    f.task.abort();
    let contacts = f.engine.lock().unwrap().inspects;
    let again = f
        .core
        .queue_stack(f.token.clone(), f.csrf.clone(), input)
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(f.engine.lock().unwrap().inspects, contacts);
    assert_eq!(
        f.core.stack_job_list(f.token.clone()).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn partial_failure_is_recorded_per_member_and_recovery_never_resends() {
    let f = admin().await;
    f.engine.lock().unwrap().fail_at = Some(0);
    let job = f.queued(ContainerAction::Restart, "partial").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    // The lost first response is proven by fresh inspection and the second
    // target was never sent: terminal and lock-releasing, but not a success.
    assert_eq!(progress.job.state, JobState::Failed);
    let receipt = progress.receipt.unwrap();
    assert_eq!(receipt.state, ExecutionState::PartlyDone);
    assert_eq!(receipt.members[0].state, ExecutionState::Verified);
    assert_eq!(
        receipt.members[1].state,
        ExecutionState::PreconditionChanged
    );
    assert_eq!(f.engine.lock().unwrap().executes, 1);
    assert!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .is_empty()
    );
    f.task.abort();
}

#[tokio::test]
async fn crash_after_prepared_recovers_by_lookup_and_fresh_verification_only() {
    let f = admin().await;
    f.engine.lock().unwrap().crash_after_prepare = true;
    let job = f.queued(ContainerAction::Restart, "crash").await;
    // The executor dies after Prepared: core loses the call and is uncertain.
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert_eq!(f.engine.lock().unwrap().executes, 1);
    f.engine.lock().unwrap().crash_after_prepare = false;
    // Recovery: lookup turns Prepared into uncertainty; the first target is
    // proven by inspection, the never-sent one is not. Nothing is resent.
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    let receipt = {
        let engine = f.engine.lock().unwrap();
        assert_eq!(engine.executes, 1);
        assert_eq!(engine.receipts, 2);
        assert_eq!(engine.verifies, 1);
        engine.receipt.clone().unwrap()
    };
    assert_eq!(receipt.members[0].state, ExecutionState::Verified);
    assert_eq!(
        receipt.members[1].state,
        ExecutionState::PreconditionChanged
    );
    // The sent member is proven and the other was never sent: terminal, not success.
    assert_eq!(receipt.state, ExecutionState::PartlyDone);
    assert_eq!(f.state(&job).await, JobState::Failed);
    f.task.abort();
}

#[tokio::test]
async fn member_drift_between_plan_and_queue_or_claim_is_refused_without_effect() {
    // Drift before queueing.
    let f = admin().await;
    let input = f.approved(ContainerAction::Restart, "early").await;
    f.engine.lock().unwrap().members.push(snap('e', true));
    assert_eq!(
        f.core
            .queue_stack(f.token.clone(), f.csrf.clone(), input)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert!(
        f.core
            .stack_job_list(f.token.clone())
            .await
            .unwrap()
            .is_empty()
    );
    f.task.abort();

    for change in 0..3 {
        // Drift after queueing, before the claim.
        let f = admin().await;
        let job = f.queued(ContainerAction::Restart, "late").await;
        {
            let mut engine = f.engine.lock().unwrap();
            match change {
                0 => engine.members[1].started_at = "2026-10-05T08:00:00Z".into(),
                1 => engine.members[2].running = true,
                _ => {
                    engine.members.pop();
                }
            }
        }
        f.core
            .dispatch_stack(f.principal().await, job.clone())
            .await
            .unwrap();
        assert_eq!(
            f.state(&job).await,
            JobState::PreconditionChanged,
            "{change}"
        );
        assert_eq!(f.engine.lock().unwrap().executes, 0);
        f.task.abort();
    }
}

#[tokio::test]
async fn revoked_grants_and_ineligible_stacks_stop_work_before_the_executor() {
    // Missing member grant: nothing reaches the executor.
    let stack_only = vec![Scope {
        operation: limeos_domain::Operation::ContainerManage,
        resource: "stack:media".into(),
    }];
    let f = fixture(limeos_domain::Role::Administrator, Some(stack_only)).await;
    assert_eq!(
        f.plan(ContainerAction::Restart).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert_eq!(f.engine.lock().unwrap().inspects, 0);
    f.task.abort();
    // A viewer role never reaches the operation.
    let all: Vec<_> = ordinary()
        .iter()
        .map(|m| m.resource.clone())
        .chain(["stack:media".to_owned(), "stack:registered".to_owned()])
        .map(|resource| Scope {
            operation: limeos_domain::Operation::ContainerManage,
            resource,
        })
        .collect();
    let f = fixture(limeos_domain::Role::Viewer, Some(all.clone())).await;
    assert_eq!(
        f.plan(ContainerAction::Restart).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    f.task.abort();

    // Grants revoked after queueing: the claim never happens.
    let f = fixture(limeos_domain::Role::Administrator, Some(all)).await;
    let job = f.queued(ContainerAction::Restart, "revoked").await;
    let principal = f.principal().await;
    f.core
        .db
        .call(move |s| s.revise_grants(&principal.id, limeos_domain::Role::Viewer, &[], now()))
        .await
        .unwrap();
    // The dispatcher resolves the owner's current principal. The revision
    // moved and the scopes are gone: the job is changed with no executor call.
    let current = f
        .core
        .db
        .call(move |s| Ok(s.login_record("admin")?.unwrap().principal))
        .await
        .unwrap();
    f.core.dispatch_stack(current, job.clone()).await.unwrap();
    assert_eq!(f.engine.lock().unwrap().executes, 0);
    let id = job.id.clone();
    assert_eq!(
        f.core.db.call(move |s| s.job_state(&id)).await.unwrap(),
        "precondition_changed"
    );
    f.task.abort();

    // An undeclared running service makes the stack ineligible, decided from
    // the cohort alone. A declared service without a container is not a member.
    let mut f = admin().await;
    f.core.stacks = Default::default();
    f.core
        .stacks
        .update(&cohort(&ordinary(), &["web"]))
        .unwrap();
    assert_eq!(
        f.plan(ContainerAction::Restart).await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(f.engine.lock().unwrap().inspects, 0);
    f.core.stacks = Default::default();
    f.core
        .stacks
        .update(&cohort(&ordinary(), &["web", "db", "extra"]))
        .unwrap();
    assert_eq!(
        f.plan(ContainerAction::Restart)
            .await
            .unwrap()
            .plan
            .expected
            .members
            .len(),
        3
    );
    f.task.abort();
    let f = admin().await;
    assert_eq!(
        f.core
            .plan_stack(
                f.token.clone(),
                f.csrf.clone(),
                limeos_contracts::StackInput {
                    project: "other".into(),
                    operation: ContainerAction::Stop,
                },
            )
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(f.engine.lock().unwrap().inspects, 0);
    f.task.abort();
}

#[tokio::test]
async fn an_active_container_job_on_a_member_holds_the_stack_job_in_the_queue() {
    let f = admin().await;
    let job = f.queued(ContainerAction::Restart, "stack").await;
    // A single-container job claims one member first.
    let member = snap('a', true);
    let principal = f.principal().await;
    let (p, m) = (principal.clone(), member.clone());
    let single = f
        .core
        .db
        .call(move |s| {
            let plan = s.plan_container(&p, ContainerAction::Restart, &m, now())?;
            let approval = s.approve_container(&p, &plan.plan.id, &plan.digest, now())?;
            let queued = s.queue_container(&p, "single", &plan, &approval.token, &m, now())?;
            s.claim_container(&p, &queued.id, &m, &"1".repeat(64), now())
        })
        .await
        .unwrap();
    assert!(
        f.core
            .dispatch_stack(principal.clone(), job.clone())
            .await
            .is_err()
    );
    assert_eq!(f.state(&job).await, JobState::Queued);
    assert_eq!(f.engine.lock().unwrap().executes, 0);
    // Releasing the member lets the stack proceed on the next tick.
    let released = single.clone();
    let (p, m) = (principal.clone(), member.clone());
    f.core
        .db
        .call(move |s| {
            let after = ContainerSnapshot {
                started_at: "2026-10-05T07:05:00Z".into(),
                ..m.clone()
            };
            s.record_container_result(
                &released,
                &limeos_domain::ContainerReceipt {
                    action: released.id.clone(),
                    plan_digest: limeos_identity::digest(
                        &serde_json::to_string(&released.plan).unwrap(),
                    ),
                    operation: ContainerAction::Restart,
                    before: m,
                    state: ExecutionState::Verified,
                    error: None,
                },
                Some(&after),
                now(),
            )?;
            assert_eq!(p.id, released.plan.principal);
            Ok(())
        })
        .await
        .unwrap();
    // The member moved on, so the stack's plan is stale and is refused whole.
    f.engine.lock().unwrap().members[0].started_at = "2026-10-05T07:05:00Z".into();
    f.core.dispatch_stack(principal, job.clone()).await.unwrap();
    assert_eq!(f.state(&job).await, JobState::PreconditionChanged);
    assert_eq!(f.engine.lock().unwrap().executes, 0);
    f.task.abort();
}

#[tokio::test]
async fn cancel_is_allowed_only_before_dispatch_and_progress_is_owner_scoped() {
    let mut f = admin().await;
    let queued = f.queued(ContainerAction::Restart, "cancel").await;
    f.core
        .cancel_stack(f.token.clone(), f.csrf.clone(), queued.id.clone(), false)
        .await
        .unwrap();
    assert_eq!(f.state(&queued).await, JobState::Canceled);
    let running = f.queued(ContainerAction::Restart, "run").await;
    f.core
        .dispatch_stack(f.principal().await, running.clone())
        .await
        .unwrap();
    assert_eq!(
        f.core
            .cancel_stack(f.token.clone(), f.csrf.clone(), running.id.clone(), false)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // Plans can be canceled until queued; a wrong CSRF token changes nothing.
    // A real sample is newer than the retained one; this fixture starts anew.
    let members = f.engine.lock().unwrap().members.clone();
    f.core.stacks = Default::default();
    f.core
        .stacks
        .update(&cohort(&members, &["web", "db"]))
        .unwrap();
    let proposal = f.plan(ContainerAction::Stop).await.unwrap();
    assert!(
        f.core
            .cancel_stack(
                f.token.clone(),
                "wrong".into(),
                proposal.plan.id.clone(),
                true
            )
            .await
            .is_err()
    );
    f.core
        .cancel_stack(f.token.clone(), f.csrf.clone(), proposal.plan.id, true)
        .await
        .unwrap();
    assert!(
        f.core
            .stack_job_progress("b".repeat(64), running.id, 0)
            .await
            .is_err()
    );
    f.task.abort();
}

#[tokio::test]
async fn a_registered_stack_with_only_one_off_containers_has_nothing_to_act_on() {
    let mut f = admin().await;
    f.core.stacks = Default::default();
    f.core.stacks.update(&cohort(&[], &["web", "db"])).unwrap();
    assert_eq!(
        f.plan(ContainerAction::Stop).await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(f.engine.lock().unwrap().inspects, 0);
    f.task.abort();
}

#[tokio::test]
async fn an_unproven_sent_member_keeps_the_stack_held_even_with_unsent_peers() {
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.fail_at = Some(1);
        engine.fail_without_effect = true;
    }
    let job = f.queued(ContainerAction::Restart, "unproven").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::NeedsIntervention);
    let receipt = progress.receipt.unwrap();
    assert_eq!(receipt.members[0].state, ExecutionState::Verified);
    assert_eq!(receipt.members[1].state, ExecutionState::OutcomeUnknown);
    assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
    assert_eq!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .len(),
        1
    );
    f.task.abort();
}

#[tokio::test]
async fn a_held_stack_resolves_on_per_member_evidence_and_frees_every_member() {
    use limeos_contracts::ResolveStatus;
    use limeos_domain::DeclaredOutcome;
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.fail_at = Some(1);
        engine.fail_without_effect = true;
    }
    let job = f.queued(ContainerAction::Restart, "resolve").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    let admin = f.principal().await;
    let view = f
        .core
        .resolution_view(admin.clone(), job.id.clone())
        .await
        .unwrap();
    assert_eq!(view.kind, limeos_domain::ResolveKind::Stack);
    // One member restarted, the other never was: neither claim holds for all.
    assert_eq!(view.targets.len(), 2);
    assert_eq!(
        view.members
            .as_ref()
            .unwrap()
            .iter()
            .map(|m| m.state)
            .collect::<Vec<_>>(),
        [ExecutionState::Verified, ExecutionState::OutcomeUnknown]
    );
    assert_eq!(view.allowed, [DeclaredOutcome::AcceptedDivergence]);
    let digest = view.evidence_digest.unwrap();
    let refused = f
        .core
        .resolve_as(
            admin.clone(),
            job.id.clone(),
            DeclaredOutcome::NotDone,
            "no".into(),
            digest.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        refused.status,
        limeos_contracts::ResolveStatus::UnsupportedDeclaration
    );
    assert_eq!(refused.blocking.len(), 1);
    let done = f
        .core
        .resolve_as(
            admin,
            job.id.clone(),
            DeclaredOutcome::AcceptedDivergence,
            "Restarted a by hand; b was never touched.".into(),
            digest,
        )
        .await
        .unwrap();
    assert_eq!(done.status, ResolveStatus::Resolved);
    assert_eq!(f.state(&job).await, JobState::Resolved);
    assert!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .is_empty()
    );
    let engine = f.engine.lock().unwrap();
    assert_eq!((engine.executes, engine.resolves), (1, 1));
    f.task.abort();
}

/// Whether a single-container job can claim the stack's first member now.
async fn member_claimable(f: &Fixture, key: &str) -> bool {
    let member = f.engine.lock().unwrap().members[0].clone();
    let principal = f.principal().await;
    let key = key.to_owned();
    f.core
        .db
        .call(move |s| {
            let plan = s.plan_container(&principal, ContainerAction::Restart, &member, now())?;
            let approval = s.approve_container(&principal, &plan.plan.id, &plan.digest, now())?;
            let queued =
                s.queue_container(&principal, &key, &plan, &approval.token, &member, now())?;
            s.claim_container(&principal, &queued.id, &member, &"1".repeat(64), now())
        })
        .await
        .is_ok()
}

#[tokio::test]
async fn an_executor_failure_before_any_row_closes_the_action_and_releases_the_stack_at_once() {
    for code in [
        ErrorCode::Expired,
        ErrorCode::Unavailable,
        ErrorCode::Overloaded,
        ErrorCode::Conflict,
        ErrorCode::StateNotDurable,
    ] {
        let f = admin().await;
        f.engine.lock().unwrap().execute_error = Some(code);
        let job = f.queued(ContainerAction::Restart, "prewrite").await;
        // The plan is still valid when core asks.
        assert!(job.plan.validate(now()).is_ok());
        f.core
            .dispatch_stack(f.principal().await, job.clone())
            .await
            .unwrap();
        // The lookup found no row and closed the action: a delayed request
        // for it can no longer run, so the job is released now, without
        // waiting for its plan to expire, as a refusal.
        let progress = f
            .core
            .stack_job_progress(f.token.clone(), job.id.clone(), 0)
            .await
            .unwrap();
        assert_eq!(
            progress.job.state,
            JobState::PreconditionChanged,
            "{code:?}"
        );
        assert_eq!(missing_events(&progress), 0);
        let receipt = progress.receipt.unwrap();
        assert!(
            receipt
                .members
                .iter()
                .all(|m| m.state == ExecutionState::PreconditionChanged)
        );
        {
            let engine = f.engine.lock().unwrap();
            assert_eq!((engine.executes, engine.receipts), (1, 1));
            assert_eq!(engine.plain_reads, 0);
            assert_eq!(
                engine.closed,
                Some(limeos_domain::ReceiptAbsence::NeverRecorded)
            );
        }
        assert!(
            f.core
                .db
                .call(|s| s.stack_candidates())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(member_claimable(&f, "after").await, "{code:?}");
        f.task.abort();
    }
}

#[tokio::test]
async fn a_never_recorded_stack_is_released_at_once_whatever_its_plan_expiry() {
    use limeos_domain::PLAN_TTL_SECONDS;
    // Seconds since the plan expired when recovery asks; negative is unexpired.
    for since_expiry in [-60, 0, 2, 10] {
        let f = admin().await;
        let job = claimed_at(&f, "window", now() - PLAN_TTL_SECONDS - since_expiry).await;
        let current = f.current(&job).await;
        f.core
            .dispatch_stack(f.principal().await, current)
            .await
            .unwrap();
        let progress = f
            .core
            .stack_job_progress(f.token.clone(), job.id.clone(), 0)
            .await
            .unwrap();
        assert_eq!(
            progress.job.state,
            JobState::PreconditionChanged,
            "{since_expiry}"
        );
        assert_eq!(missing_events(&progress), 0);
        assert!(member_claimable(&f, "after").await, "{since_expiry}");
        let engine = f.engine.lock().unwrap();
        assert_eq!((engine.executes, engine.receipts), (0, 1));
        assert_eq!(engine.plain_reads, 0);
        drop(engine);
        f.task.abort();
    }
}

#[tokio::test]
async fn an_absence_the_executor_did_not_close_never_releases_the_stack() {
    // An executor answering a bare absence, as to a status read, proves
    // nothing about an execute request still on its way.
    let f = admin().await;
    let job = claimed_at(&f, "unclosed", now()).await;
    let mut response = Receipt::empty();
    response.absence = Some(limeos_domain::ReceiptAbsence::NeverRecorded);
    // Answer this one lookup by hand, as an executor that ignored the intent.
    f.task.abort();
    let path = f.core.container_socket.as_ref().clone();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
        assert!(matches!(
            request,
            Request::StackReceipt {
                close_if_absent: true,
                ..
            }
        ));
        limeos_contracts::write_frame(&mut stream, &response)
            .await
            .unwrap();
    });
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    served.await.unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::Running);
    assert_eq!(missing_events(&progress), 0);
    assert!(!member_claimable(&f, "after").await);
}

#[tokio::test]
async fn when_the_lookup_cannot_close_the_action_the_stack_stays_held_until_it_can() {
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.execute_error = Some(ErrorCode::Unavailable);
        engine.lookup_unavailable = true;
    }
    let job = f.queued(ContainerAction::Restart, "held").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert_eq!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .len(),
        1
    );
    // Recovery with the lookup still failing keeps it held and sends nothing.
    let current = f.current(&job).await;
    assert!(
        f.core
            .dispatch_stack(f.principal().await, current.clone())
            .await
            .is_err()
    );
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    // A later lookup that succeeds closes the action, which proves nothing
    // was sent and nothing can be: released at once.
    f.engine.lock().unwrap().lookup_unavailable = false;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::PreconditionChanged);
    assert_eq!(f.engine.lock().unwrap().executes, 1);
    f.task.abort();
}

#[tokio::test]
async fn a_row_found_after_an_executor_failure_never_releases_the_stack() {
    let f = admin().await;
    f.engine.lock().unwrap().crash_after_prepare = true;
    let job = f.queued(ContainerAction::Restart, "row").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert_eq!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .len(),
        1
    );
    f.task.abort();
}

#[tokio::test]
async fn the_catalog_stack_name_is_locked_and_authorized_beside_the_compose_project() {
    // The registered stack ("registered") and the Compose project ("media") differ.
    let f = admin().await;
    let input = f.approved(ContainerAction::Restart, "catalog").await;
    assert_eq!(input.proposal.plan.catalog_stack, "registered");
    assert_eq!(input.proposal.plan.expected.resource, "stack:media");
    let resources = input.proposal.plan.resources();
    assert_eq!(resources[..2], ["stack:media", "stack:registered"]);
    f.task.abort();
    let scopes = [
        "stack:media".to_owned(),
        "container:".to_owned() + &"a".repeat(64),
        "container:".to_owned() + &"b".repeat(64),
        "container:".to_owned() + &"c".repeat(64),
    ]
    .map(|resource| Scope {
        operation: limeos_domain::Operation::ContainerManage,
        resource,
    });
    let f = fixture(limeos_domain::Role::Administrator, Some(scopes.to_vec())).await;
    assert_eq!(
        f.plan(ContainerAction::Restart).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    f.task.abort();
}

#[tokio::test]
async fn a_stack_among_forty_unrelated_containers_plans_approves_queues_and_runs() {
    // 3 members + 1 one-off of the registered project, 36 more containers in two
    // other projects and standalone: 40 in total, well over the 16-member stack bound.
    let mut f = admin().await;
    f.core.stacks = Default::default();
    let members = f.engine.lock().unwrap().members.clone();
    f.core
        .stacks
        .update(&crowded_cohort(&members, &["web", "db"], 36))
        .unwrap();
    let cohort = f.core.stacks.cohort().unwrap();
    assert_eq!(cohort.engine().unwrap().members.len(), 40);
    let inventory = cohort.project(now()).unwrap();
    assert_eq!(
        inventory.engine.state,
        limeos_domain::deployment::discovery::SourceState::Fresh
    );
    assert_eq!(inventory.other_projects.len(), 2);
    let input = f.approved(ContainerAction::Restart, "crowded").await;
    assert_eq!(input.proposal.plan.expected.members.len(), 3);
    assert_eq!(input.proposal.plan.targets().len(), 2);
    let job = f
        .core
        .queue_stack(f.token.clone(), f.csrf.clone(), input)
        .await
        .unwrap();
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id, 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::Succeeded);
    assert_eq!(f.engine.lock().unwrap().executes, 1);
    f.task.abort();
}

#[tokio::test]
async fn a_truncated_inventory_plans_a_complete_stack_but_never_one_missing_a_member() {
    // Every member is shown and other containers were left out: plannable,
    // because the fresh per-project listing agrees with the plan.
    let mut f = admin().await;
    f.core.stacks = Default::default();
    f.core
        .stacks
        .update(&truncated_cohort(&ordinary(), 5))
        .unwrap();
    let view = f.core.stacks.cohort().unwrap().project(now()).unwrap();
    assert_eq!(view.truncated_containers, Some(5));
    let planned = f.plan(ContainerAction::Restart).await.unwrap();
    assert_eq!(planned.plan.expected.members, ordinary());
    f.task.abort();
    // The stopped member was truncated out of the inventory: the fresh
    // per-project listing still names it, so the incomplete plan is refused.
    let mut f = admin().await;
    f.core.stacks = Default::default();
    let shown: Vec<_> = ordinary().into_iter().filter(|m| m.running).collect();
    f.core.stacks.update(&truncated_cohort(&shown, 1)).unwrap();
    assert_eq!(
        f.plan(ContainerAction::Restart).await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert!(f.engine.lock().unwrap().inspects > 0);
    assert_eq!(f.engine.lock().unwrap().executes, 0);
    f.task.abort();
}

#[test]
fn the_engine_bound_is_exactly_sixty_four_containers() {
    let members = ordinary();
    // 3 + 1 one-off + 60 = 64 is the whole-Engine bound; one more is refused.
    let at_bound = crowded_cohort(&members, &["web"], 60);
    assert_eq!(at_bound.engine().unwrap().members.len(), 64);
}

/// A stack job claimed long ago: its plan expired more than the receipt
/// retention before now, so the executor may since have pruned its receipt.
async fn claimed_before_retention(f: &Fixture, key: &str) -> StackJob {
    let t0 =
        now() - limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY - limeos_domain::PLAN_TTL_SECONDS - 60;
    claimed_at(f, key, t0).await
}
/// A stack job planned, approved, queued and claimed at `t0`.
async fn claimed_at(f: &Fixture, key: &str, t0: i64) -> StackJob {
    let principal = f.principal().await;
    let members = StackSnapshot::new("media", ordinary()).unwrap();
    let key = key.to_owned();
    f.core
        .db
        .call(move |s| {
            let plan = s.plan_stack(
                &principal,
                ContainerAction::Restart,
                "registered",
                &members,
                t0,
            )?;
            let approval = s.approve_stack(&principal, &plan.plan.id, &plan.digest, t0)?;
            let queued = s.queue_stack(&principal, &key, &plan, &approval.token, &members, t0)?;
            // Dispatched under the store's epoch, the fixture's.
            s.claim_stack(&principal, &queued.id, &members, &"1".repeat(64), t0)
        })
        .await
        .unwrap()
}
fn missing_events(progress: &limeos_contracts::StackJobProgress) -> usize {
    progress
        .events
        .iter()
        .filter(|e| e.event == limeos_domain::EventKind::ExecutorReceiptMissing)
        .count()
}

#[tokio::test]
async fn a_pruned_receipt_of_an_executed_stack_never_releases_it_as_unsent() {
    let f = admin().await;
    // A young plan by core's clock: the executor pruned its receipt while the
    // shared clock ran more than an hour ahead, and the clock has since stepped
    // back. Its watermark is a durable fact and still covers the plan.
    let job = claimed_at(&f, "pruned", now()).await;
    f.engine.lock().unwrap().watermark = job.plan.expires_at;
    // The executor verified every member, but core crashed before recording it
    // and its restart left the job for reconciliation.
    let id = job.id.clone();
    f.core
        .db
        .call(move |s| s.transition(&id, JobState::Running, JobState::NeedsIntervention, now()))
        .await
        .unwrap();
    for member in &mut f.engine.lock().unwrap().members {
        apply(ContainerAction::Restart, member);
    }
    // Then retention pruned the executor's receipt: every lookup finds no row.
    assert!(f.engine.lock().unwrap().receipt.is_none());
    for _ in 0..2 {
        let current = f.current(&job).await;
        f.core
            .dispatch_stack(f.principal().await, current)
            .await
            .unwrap();
        let progress = f
            .core
            .stack_job_progress(f.token.clone(), job.id.clone(), 0)
            .await
            .unwrap();
        assert_eq!(progress.job.state, JobState::NeedsIntervention);
        // The reason is recorded once, however often recovery polls.
        assert_eq!(missing_events(&progress), 1);
    }
    // Every lock is still held: no single-container job can claim a member.
    assert_eq!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .len(),
        1
    );
    let member = f.engine.lock().unwrap().members[0].clone();
    let principal = f.principal().await;
    let claimed = f
        .core
        .db
        .call(move |s| {
            let plan = s.plan_container(&principal, ContainerAction::Restart, &member, now())?;
            let approval = s.approve_container(&principal, &plan.plan.id, &plan.digest, now())?;
            let queued =
                s.queue_container(&principal, "after", &plan, &approval.token, &member, now())?;
            s.claim_container(&principal, &queued.id, &member, &"1".repeat(64), now())
        })
        .await;
    assert!(claimed.is_err());
    let engine = f.engine.lock().unwrap();
    assert_eq!((engine.executes, engine.receipts), (0, 2));
    drop(engine);
    f.task.abort();
}

#[tokio::test]
async fn an_operator_resolves_a_stack_whose_executor_receipt_was_pruned() {
    use limeos_domain::DeclaredOutcome;
    let f = admin().await;
    let job = claimed_before_retention(&f, "resolve-pruned").await;
    for member in &mut f.engine.lock().unwrap().members {
        apply(ContainerAction::Restart, member);
    }
    f.engine.lock().unwrap().resolve_missing = true;
    f.engine.lock().unwrap().watermark = job.plan.expires_at;
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    let admin = f.principal().await;
    let view = f
        .core
        .resolution_view(admin.clone(), job.id.clone())
        .await
        .unwrap();
    assert!(view.allowed.contains(&DeclaredOutcome::VerifiedDone));
    // The executor answers that it holds no row; it has nothing to free, so the
    // decision completes on core's fresh evidence.
    let done = f
        .core
        .resolve_as(
            admin,
            job.id.clone(),
            DeclaredOutcome::VerifiedDone,
            "Both members restarted; the receipt was pruned.".into(),
            view.evidence_digest.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done.status, limeos_contracts::ResolveStatus::Resolved);
    assert_eq!(f.state(&job).await, JobState::Resolved);
    assert!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.engine.lock().unwrap().resolves, 1);
    f.task.abort();
}

#[tokio::test]
async fn a_declaration_the_executors_recorded_outcome_contradicts_is_refused_with_the_reason() {
    use limeos_domain::DeclaredOutcome;
    let f = admin().await;
    // The executor finished and verified the stack; core crashed before
    // recording it, so the job is held while the receipt is terminal.
    let job = claimed_at(&f, "terminal", now()).await;
    let id = job.id.clone();
    f.core
        .db
        .call(move |s| s.transition(&id, JobState::Running, JobState::NeedsIntervention, now()))
        .await
        .unwrap();
    f.engine.lock().unwrap().resolve_recorded = Some(ExecutionState::Verified);
    let operator = f.principal().await;
    let view = f
        .core
        .resolution_view(operator.clone(), job.id.clone())
        .await
        .unwrap();
    let refused = f
        .core
        .resolve_as(
            operator.clone(),
            job.id.clone(),
            DeclaredOutcome::NotDone,
            "Nothing happened.".into(),
            view.evidence_digest.clone().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        refused.status,
        limeos_contracts::ResolveStatus::UnsupportedDeclaration
    );
    assert!(refused.message.contains("verified"), "{}", refused.message);
    assert!(refused.message.contains("accepted_divergence"));
    // Withdrawn and still held: the operator can decide again.
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert!(refused.view.unwrap().pending.is_none());
    let done = f
        .core
        .resolve_as(
            operator,
            job.id.clone(),
            DeclaredOutcome::AcceptedDivergence,
            "The executor verified it; core lost the result.".into(),
            view.evidence_digest.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done.status, limeos_contracts::ResolveStatus::Resolved);
    f.task.abort();
}

#[tokio::test]
async fn a_retried_resolution_without_an_executor_receipt_measures_again_before_it_unlocks() {
    use limeos_domain::{DeclaredOutcome, ResolutionEvidence};
    let f = admin().await;
    let job = claimed_before_retention(&f, "retry-pruned").await;
    for member in &mut f.engine.lock().unwrap().members {
        apply(ContainerAction::Restart, member);
    }
    {
        let mut engine = f.engine.lock().unwrap();
        engine.resolve_missing = true;
        engine.watermark = job.plan.expires_at;
    }
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    // The administrator decides "verified done" and core records it, then
    // crashes before the executor step and completion.
    let operator = f.principal().await;
    let view = f
        .core
        .resolution_view(operator.clone(), job.id.clone())
        .await
        .unwrap();
    let evidence = ResolutionEvidence {
        targets: view.targets.iter().map(|t| t.measured.clone()).collect(),
    };
    let (p, id, digest) = (
        operator.clone(),
        job.id.clone(),
        view.evidence_digest.unwrap(),
    );
    f.core
        .db
        .call(move |s| {
            s.request_resolution(
                &p,
                &id,
                DeclaredOutcome::VerifiedDone,
                "Both members restarted.",
                &evidence,
                &digest,
                now() - 3600,
            )
        })
        .await
        .unwrap();
    // Hours later a member has stopped; the retry must not unlock on the
    // evidence approved back then.
    f.engine.lock().unwrap().members[0].running = false;
    f.core
        .complete_pending_resolutions(&mut Default::default(), 0)
        .await;
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert!(
        f.core
            .db
            .call(|s| s.pending_resolutions())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .len(),
        1
    );
    // Withdrawn, so the administrator decides again on fresh evidence.
    let view = f
        .core
        .resolution_view(operator.clone(), job.id.clone())
        .await
        .unwrap();
    let done = f
        .core
        .resolve_as(
            operator,
            job.id.clone(),
            DeclaredOutcome::AcceptedDivergence,
            "One member stopped again after its restart.".into(),
            view.evidence_digest.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done.status, limeos_contracts::ResolveStatus::Resolved);
    f.task.abort();
}

#[tokio::test]
async fn an_old_never_sent_stack_is_released_without_waiting_for_any_window() {
    // Claimed long ago; the executor never wrote a row and never pruned one
    // for a plan this late, so nothing was sent, however old the plan is.
    let f = admin().await;
    let job = claimed_before_retention(&f, "never-sent").await;
    f.engine.lock().unwrap().watermark = job.plan.expires_at - 1;
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::PreconditionChanged);
    assert_eq!(missing_events(&progress), 0);
    assert!(
        f.core
            .db
            .call(|s| s.stack_candidates())
            .await
            .unwrap()
            .is_empty()
    );
    let engine = f.engine.lock().unwrap();
    assert_eq!((engine.executes, engine.receipts), (0, 1));
    drop(engine);
    f.task.abort();
}

#[tokio::test]
async fn a_never_recorded_plan_is_resolved_without_waiting_for_its_expiry() {
    use limeos_domain::DeclaredOutcome;
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.execute_error = Some(ErrorCode::Unavailable);
        engine.lookup_unavailable = true;
        engine.resolve_missing = true;
    }
    let job = f.queued(ContainerAction::Restart, "young").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
    assert!(job.plan.validate(now()).is_ok());
    let operator = f.principal().await;
    let view = f
        .core
        .resolution_view(operator.clone(), job.id.clone())
        .await
        .unwrap();
    // The executor closes the action before it answers, so no delayed
    // ExecuteStack can run afterwards: the decision completes now, on core's
    // fresh measurement of the approved evidence.
    let done = f
        .core
        .resolve_as(
            operator,
            job.id.clone(),
            DeclaredOutcome::NotDone,
            "Nothing happened.".into(),
            view.evidence_digest.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done.status, limeos_contracts::ResolveStatus::Resolved);
    assert_eq!(f.state(&job).await, JobState::Resolved);
    assert!(member_claimable(&f, "after").await);
    let engine = f.engine.lock().unwrap();
    assert_eq!(
        engine.closed,
        Some(limeos_domain::ReceiptAbsence::NeverRecorded)
    );
    assert_eq!(engine.executes, 1);
    drop(engine);
    f.task.abort();
}

#[tokio::test]
async fn a_closed_no_longer_held_stack_resolves_on_matching_evidence_and_withdraws_otherwise() {
    use limeos_domain::{DeclaredOutcome, ResolutionEvidence};
    for changed in [false, true] {
        // A store upgraded from schema 6 starts its watermark at the maximum, so
        // even a just-approved stack it never saw is answered `no_longer_held`.
        let f = admin().await;
        {
            let mut engine = f.engine.lock().unwrap();
            engine.execute_error = Some(ErrorCode::Unavailable);
            engine.resolve_missing = true;
            engine.watermark = i64::MAX;
        }
        let job = f.queued(ContainerAction::Restart, "upgraded").await;
        f.core
            .dispatch_stack(f.principal().await, job.clone())
            .await
            .unwrap();
        // The lookup closed the action as `no_longer_held`: held for an operator.
        assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
        assert!(!member_claimable(&f, "held").await);
        let operator = f.principal().await;
        let view = f
            .core
            .resolution_view(operator.clone(), job.id.clone())
            .await
            .unwrap();
        let evidence = ResolutionEvidence {
            targets: view.targets.iter().map(|t| t.measured.clone()).collect(),
        };
        let (p, id, digest) = (
            operator.clone(),
            job.id.clone(),
            view.evidence_digest.unwrap(),
        );
        f.core
            .db
            .call(move |s| {
                s.request_resolution(
                    &p,
                    &id,
                    DeclaredOutcome::NotDone,
                    "Nothing happened.",
                    &evidence,
                    &digest,
                    now(),
                )
            })
            .await
            .unwrap();
        if changed {
            f.engine.lock().unwrap().members[0].running = false;
        }
        // The plan is still valid; nothing waits for it to expire.
        assert!(job.plan.validate(now()).is_ok());
        f.core
            .complete_pending_resolutions(&mut Default::default(), 0)
            .await;
        assert!(
            f.core
                .db
                .call(|s| s.pending_resolutions())
                .await
                .unwrap()
                .is_empty()
        );
        if changed {
            // Withdrawn: the operator decides again on fresh evidence.
            assert_eq!(f.state(&job).await, JobState::NeedsIntervention);
            assert!(!member_claimable(&f, "after").await);
        } else {
            assert_eq!(f.state(&job).await, JobState::Resolved);
            assert!(member_claimable(&f, "after").await);
        }
        f.task.abort();
    }
}

#[tokio::test]
async fn a_hidden_overlapping_project_or_an_unattributed_container_refuses_planning() {
    let overlapping = ProjectIdentity {
        name: "other".into(),
        working_directory: "/opt/Media".into(),
        config_files: vec!["/opt/Media/other.yaml".into()],
    };
    for (hidden, unattributed) in [(vec![overlapping], 0), (Vec::new(), 1)] {
        let mut f = admin().await;
        f.core.stacks = Default::default();
        let base = truncated_cohort(&ordinary(), 1);
        let cohort = DiscoveryCohort::new(
            base.catalog().cloned(),
            Some(EngineFacts {
                hidden_projects: hidden,
                unattributed,
                ..base.engine().unwrap().clone()
            }),
        )
        .unwrap();
        f.core.stacks.update(&cohort).unwrap();
        assert_eq!(
            f.plan(ContainerAction::Restart).await.unwrap_err().0,
            ErrorCode::Conflict
        );
        // Refused from the cohort, before any executor contact.
        assert_eq!(f.engine.lock().unwrap().inspects, 0);
        f.task.abort();
    }
}

fn record_events(
    progress: &limeos_contracts::StackJobProgress,
    kind: &limeos_domain::EventKind,
) -> usize {
    progress.events.iter().filter(|e| &e.event == kind).count()
}

/// The executor ran the stack and core recorded its uncertain outcome; then
/// the receipt store was lost and recreated (a new epoch, no row). The
/// recovery lookup's closure says the action was never recorded, which
/// contradicts the result core holds: the job stays held with every lock,
/// the result is kept, and the disagreement is recorded once.
#[tokio::test]
async fn a_recreated_store_never_releases_a_stack_core_holds_a_result_for() {
    use limeos_domain::EventKind;
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.epoch = Some("1".repeat(64));
        engine.fail_at = Some(1);
        engine.fail_without_effect = true;
    }
    let job = f.queued(ContainerAction::Restart, "ran").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let held = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(held.job.state, JobState::NeedsIntervention);
    {
        let mut engine = f.engine.lock().unwrap();
        engine.receipt = None;
        engine.epoch = Some("2".repeat(64));
    }
    for _ in 0..2 {
        let current = f.current(&job).await;
        f.core
            .dispatch_stack(f.principal().await, current)
            .await
            .unwrap();
    }
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::NeedsIntervention);
    assert_eq!(progress.receipt, held.receipt);
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorRecordDisagrees),
        1
    );
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorStoreChanged),
        1
    );
    assert!(!member_claimable(&f, "after").await);
    f.task.abort();
}

/// A stack dispatched while core knew the store's epoch, its execute answer
/// lost before any row; the store is then replaced by an older copy (a new
/// epoch, no row). Its closure is not accepted: the stack stays held.
#[tokio::test]
async fn a_stack_dispatched_under_an_earlier_store_epoch_is_held_on_a_closure() {
    use limeos_domain::EventKind;
    let f = admin().await;
    f.engine.lock().unwrap().epoch = Some("1".repeat(64));
    let job = claimed_at(&f, "lost", now()).await;
    f.engine.lock().unwrap().epoch = Some("2".repeat(64));
    let current = f.current(&job).await;
    f.core
        .dispatch_stack(f.principal().await, current)
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
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
    assert!(!member_claimable(&f, "after").await);
    // Under one unchanged epoch, the same closure releases a stack at once.
    let next = admin().await;
    next.engine.lock().unwrap().epoch = Some("1".repeat(64));
    let job = claimed_at(&next, "fresh", now()).await;
    let current = next.current(&job).await;
    next.core
        .dispatch_stack(next.principal().await, current)
        .await
        .unwrap();
    assert_eq!(next.state(&job).await, JobState::PreconditionChanged);
    f.task.abort();
    next.task.abort();
}

/// The executor restarts (a new epoch) between two stack dispatches. The
/// second stack is claimed under the new epoch, asked for before the claim,
/// so the closure that follows its failed execute, under the same epoch,
/// releases it at once with no operator.
#[tokio::test]
async fn a_stack_dispatched_after_an_executor_restart_is_settled_by_its_closure() {
    use limeos_domain::EventKind;
    let mut f = admin().await;
    let first = f.queued(ContainerAction::Restart, "first").await;
    f.core
        .dispatch_stack(f.principal().await, first.clone())
        .await
        .unwrap();
    assert_eq!(f.state(&first).await, JobState::Succeeded);
    {
        // A routine restart: same store, reopened. The execute then fails
        // before any row is written.
        let mut engine = f.engine.lock().unwrap();
        engine.epoch = Some("2".repeat(64));
        engine.receipt = None;
        engine.execute_error = Some(ErrorCode::Unavailable);
    }
    // The inventory has seen the restarted members.
    let members = f.engine.lock().unwrap().members.clone();
    f.core.stacks = Default::default();
    f.core
        .stacks
        .update(&cohort(&members, &["web", "db"]))
        .unwrap();
    let second = f.queued(ContainerAction::Restart, "second").await;
    f.core
        .dispatch_stack(f.principal().await, second.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), second.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::PreconditionChanged);
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorStoreChanged),
        0
    );
    assert_eq!(
        record_events(&progress, &EventKind::ExecutorRecordDisagrees),
        0
    );
    assert!(member_claimable(&f, "after").await);
    f.task.abort();
}

/// The executor is down when the stack's execute request is sent: the
/// connection is refused, nothing was written, and the stack is released at
/// once, with no lookup and no operator.
#[tokio::test]
async fn a_refused_connection_releases_the_stack_without_an_operator() {
    let f = admin().await;
    f.engine.lock().unwrap().vanish_after_epoch = true;
    let job = f.queued(ContainerAction::Restart, "refused").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::PreconditionChanged);
    assert_eq!(missing_events(&progress), 0);
    assert!(progress.receipt.is_none());
    {
        let engine = f.engine.lock().unwrap();
        assert_eq!((engine.executes, engine.receipts), (0, 0));
        assert!(engine.closed.is_none());
    }
    assert!(member_claimable(&f, "after").await);
}

/// An ExecuteStack refused with `beyond_receipt_history` was refused before
/// the executor recorded anything, and core sends one execute per job: the
/// stack is settled as refused at once, with no closing lookup (which would
/// answer `no_longer_held`) and no operator.
#[tokio::test]
async fn a_stack_refused_beyond_receipt_history_is_settled_as_refused() {
    let f = admin().await;
    {
        let mut engine = f.engine.lock().unwrap();
        engine.execute_error = Some(ErrorCode::BeyondReceiptHistory);
        engine.watermark = i64::MAX;
    }
    let job = f.queued(ContainerAction::Restart, "beyond").await;
    f.core
        .dispatch_stack(f.principal().await, job.clone())
        .await
        .unwrap();
    let progress = f
        .core
        .stack_job_progress(f.token.clone(), job.id.clone(), 0)
        .await
        .unwrap();
    assert_eq!(progress.job.state, JobState::PreconditionChanged);
    assert_eq!(missing_events(&progress), 0);
    {
        let engine = f.engine.lock().unwrap();
        assert_eq!((engine.executes, engine.receipts), (1, 0));
        assert!(engine.closed.is_none());
    }
    assert!(member_claimable(&f, "after").await);
    f.task.abort();
}
