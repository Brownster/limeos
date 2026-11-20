//! Core's dispatcher, recovery and resolution paths, step for step, on the
//! simulated clock. `limeos-core` is a binary whose loops read the wall clock
//! (`main.rs`'s `now()`, `Instant` in `Rotation`), so the harness cannot call
//! them; it makes the same store and executor calls in the same order, with
//! the same branches. Each function names the one it mirrors:
//!
//! - `dispatcher_pass`: one iteration of `operations::Core::dispatcher`
//! - `dispatch_one`: `operations::Core::dispatch_one`
//! - `dispatch_stack` and helpers: `stack_operations.rs`
//! - `complete_pending_resolutions`, `finish_resolution`, `resolve_as`:
//!   `job_resolution.rs`
//!
//! A change to those paths that adds a write must be copied here, or the
//! soak no longer covers it.
use crate::sim::{Failure, Halt, JobFault, Sim, at};
use limeos_contracts::VERSION;
use limeos_domain::{
    ContainerSnapshot, DeclaredOutcome, Error, ErrorCode, ExecutionState, JobState, Principal,
    ReceiptAbsence, ResolutionEvidence, ResolveReceipt, RestartJob, RestartRequest, Scope,
    StackJob, StackReceipt, StackRequest, StackSnapshot, stack_project, stack_resource,
    verify_container, verify_stack_member,
};
use limeos_executor_protocol::Request;
use limeos_persistence::PendingResolution;

/// Errors core logs and moves on from in a dispatch: the executor is
/// unreachable or lost an answer, a lock is held, or a receipt is refused.
const DISPATCH_LEGIT: [ErrorCode; 4] = [
    ErrorCode::Conflict,
    ErrorCode::Unavailable,
    ErrorCode::NotFound,
    ErrorCode::Expired,
];

/// What finishing one recorded resolution request came to (core's `Finish`).
pub enum Finish {
    Done,
    Refused(ErrorCode),
    Disagrees,
    Pending(ErrorCode),
    Superseded,
}

fn step_name(step: &'static str, otherwise: &'static str) -> &'static str {
    if step.is_empty() { otherwise } else { step }
}

fn held(state: JobState) -> bool {
    matches!(
        state,
        JobState::Running
            | JobState::Verifying
            | JobState::OutcomeUnknown
            | JobState::NeedsIntervention
    )
}
fn awaiting(state: JobState) -> bool {
    matches!(
        state,
        JobState::NeedsIntervention | JobState::Running | JobState::Verifying
    )
}

impl Sim {
    /// One dispatcher tick. A simulated core crash inside it is returned for
    /// the driver to carry out.
    pub async fn dispatcher_pass(&mut self) -> Result<(), Halt> {
        self.stats.dispatcher_passes += 1;
        self.complete_pending_resolutions(5).await?;
        self.dispatch_stack_candidates().await?;
        // Storage target and mount jobs: never produced in the shipped scope,
        // but core lists them every tick.
        let targets = self.db().storage_target_candidates().map(|_| ());
        self.judge("list storage target jobs", targets, &[]);
        let mounts = self.db().storage_mount_candidates().map(|_| ());
        self.judge("list storage mount jobs", mounts, &[]);
        let cursor = self.rotation.cursor();
        let listed = self.db().container_candidates_after(&cursor);
        let Some(jobs) = self.judge("list container jobs", listed, &[]) else {
            return Ok(());
        };
        let now = self.now();
        self.count_seen(jobs.iter().map(|(_, job)| job.state));
        self.rotation.advance(
            jobs.iter()
                .filter(|(_, job)| job.state != JobState::Queued)
                .map(|(_, job)| job.id.as_str()),
            now,
        );
        for (principal, job) in jobs {
            if job.state != JobState::Queued && !self.rotation.due(&job.id, now) {
                continue;
            }
            match self.dispatch_one(principal, job).await {
                Ok(()) => {}
                Err(Halt::CoreCrash) => return Err(Halt::CoreCrash),
                Err(Halt::Error(step, e)) => {
                    let op = step_name(step, "dispatch container job");
                    self.judge::<()>(op, Err(e), &DISPATCH_LEGIT);
                }
            }
        }
        Ok(())
    }

    fn count_seen(&mut self, states: impl Iterator<Item = JobState>) {
        for state in states {
            if state == JobState::Queued {
                self.seen_queued += 1;
            } else {
                self.seen_held += 1;
            }
        }
    }
    fn fault(&mut self, job: &str, wanted: impl Fn(JobFault) -> bool) -> Option<JobFault> {
        let fault = *self.armed.get(job)?;
        if !wanted(fault) {
            return None;
        }
        self.armed.remove(job);
        Some(fault)
    }
    fn transport_fault(&mut self, job: &str) -> Option<JobFault> {
        self.fault(job, |f| {
            matches!(
                f,
                JobFault::AnswerLost
                    | JobFault::LostBeforeStore
                    | JobFault::ExecutorKilledInEngineCall
                    | JobFault::EngineFailure { .. }
            )
        })
    }
    fn crash_at(&mut self, job: &str, point: JobFault) -> Result<(), Halt> {
        if self.fault(job, |f| f == point).is_some() {
            self.count_fault(point);
            return Err(Halt::CoreCrash);
        }
        Ok(())
    }
    fn executor_down_after_epoch(&mut self, job: &str) {
        if self
            .fault(job, |f| f == JobFault::ExecutorDownAfterEpoch)
            .is_some()
        {
            self.count_fault(JobFault::ExecutorDownAfterEpoch);
            self.exec.stop();
            self.executor_back_at = Some(self.now() + 20);
        }
    }

    /// Core's `receipt_epoch`.
    async fn receipt_epoch(&mut self) -> Result<String, Error> {
        self.executor(Request::ReceiptEpoch { version: VERSION })
            .await?
            .epoch
            .ok_or(Error(ErrorCode::Unavailable))
    }
    /// Core's `inspect`.
    pub async fn inspect(&mut self, resource: &str) -> Result<ContainerSnapshot, Error> {
        if !limeos_domain::opaque_id(resource.strip_prefix("container:").unwrap_or_default()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(Request::Inspect {
                version: VERSION,
                resource: resource.into(),
            })
            .await?
            .inspection
            .ok_or(Error(ErrorCode::Unavailable))?;
        value.validate()?;
        if value.resource != resource {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(value)
    }
    /// Core's `inspect_stack`.
    pub async fn inspect_stack(&mut self, project: &str) -> Result<StackSnapshot, Error> {
        if !stack_project(project) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(Request::InspectStack {
                version: VERSION,
                project: project.into(),
            })
            .await?
            .stack_members
            .ok_or(Error(ErrorCode::Unavailable))?;
        value.validate()?;
        if value.resource != stack_resource(project) {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(value)
    }
    fn mark(&mut self, id: &str, from: JobState, to: JobState) -> Result<(), Error> {
        if from == to {
            return Ok(());
        }
        let now = self.now();
        self.db().transition(id, from, to, now)
    }

    /// `operations::Core::dispatch_one`.
    pub async fn dispatch_one(
        &mut self,
        principal: Principal,
        mut job: RestartJob,
    ) -> Result<(), Halt> {
        let digest = limeos_identity::digest(&serde_json::to_string(&job.plan).unwrap());
        let recovering = job.state != JobState::Queued;
        let (mut absence, mut closed) = (None, false);
        let now = self.now();
        let receipt = if job.state == JobState::Queued {
            let scope = Scope {
                operation: limeos_domain::Operation::ContainerManage,
                resource: job.plan.expected.resource.clone(),
            };
            let authorized = self
                .db()
                .grants(&principal)
                .and_then(|grants| limeos_policy::authorize(&principal, &grants, &scope));
            if authorized.is_err()
                || job.plan.grant_revision != principal.grant_revision
                || job.plan.validate(now).is_err()
            {
                return Ok(self.mark(&job.id, job.state, JobState::PreconditionChanged)?);
            }
            let current = self
                .inspect(&job.plan.expected.resource)
                .await
                .map_err(at("dispatch container job: inspect"))?;
            if job.plan.check_current(&current, now).is_err() {
                return Ok(self.mark(&job.id, job.state, JobState::PreconditionChanged)?);
            }
            let epoch = self
                .receipt_epoch()
                .await
                .map_err(at("dispatch container job: receipt epoch"))?;
            self.executor_down_after_epoch(&job.id);
            job = self
                .db()
                .claim_container(&principal, &job.id, &current, &epoch, now)
                .map_err(at("dispatch container job: claim (a lock is held)"))?;
            self.crash_at(&job.id, JobFault::CoreCrashAfterClaim)?;
            let transport = self.transport_fault(&job.id);
            let request = Request::ExecuteContainer {
                version: VERSION,
                request: RestartRequest {
                    action: job.id.clone(),
                    plan_digest: digest.clone(),
                    plan: job.plan.clone(),
                },
            };
            match self.exchange(request, transport).await {
                Ok(value) => value.restart,
                Err(Failure { error, sent })
                    if !sent || error.0 == ErrorCode::BeyondReceiptHistory =>
                {
                    let id = job.id.clone();
                    return Ok(self.db().record_unsent_execute(&id, now)?);
                }
                Err(_) => {
                    self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
                    return Ok(());
                }
            }
        } else {
            let lost = self.fault(&job.id, |f| f == JobFault::LookupLost);
            let request = Request::RestartReceipt {
                version: VERSION,
                action: job.id.clone(),
                digest: digest.clone(),
                plan: job.plan.clone(),
                close_if_absent: true,
            };
            let value = self
                .exchange(request, lost)
                .await
                .map_err(|f| Halt::Error("dispatch container job: recovery lookup", f.error))?;
            (absence, closed) = (value.absence, value.closed);
            value.restart
        };
        let Some(mut receipt) = receipt else {
            if recovering {
                if absence == Some(ReceiptAbsence::NeverRecorded) && !closed {
                    return Ok(self.mark(&job.id, job.state, JobState::NeedsIntervention)?);
                }
                let id = job.id.clone();
                return Ok(self.db().record_receipt_missing(&id, now)?);
            }
            self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
            return Ok(());
        };
        if receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
        {
            self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
            return Err(Error(ErrorCode::Conflict).into());
        }
        if closed {
            let result = self.db().record_container_closure(&job, &receipt, now);
            return Ok(result.map(|_| ())?);
        }
        let mut after = None;
        if matches!(
            receipt.state,
            ExecutionState::EffectAccepted | ExecutionState::Verified
        ) {
            self.crash_at(&job.id, JobFault::CoreCrashAfterExecute)?;
            if matches!(job.state, JobState::Running | JobState::OutcomeUnknown) {
                self.mark(&job.id, job.state, JobState::Verifying)?;
                job.state = JobState::Verifying;
            }
            if let Ok(current) = self.inspect(&job.plan.expected.resource).await {
                if verify_container(receipt.operation, &receipt.before, &current).is_ok() {
                    if receipt.state == ExecutionState::EffectAccepted {
                        let request = Request::VerifyRestart {
                            version: VERSION,
                            action: job.id.clone(),
                            digest,
                            after: current.clone(),
                        };
                        if let Ok(value) = self.executor(request).await {
                            if let Some(verified) = value.restart {
                                receipt = verified;
                            }
                        }
                        self.crash_at(&job.id, JobFault::CoreCrashAfterVerify)?;
                    }
                    after = Some(current);
                }
            }
        }
        self.db()
            .record_container_result(&job, &receipt, after.as_ref(), now)?;
        Ok(())
    }

    /// `stack_operations::Core::dispatch_stack_candidates`.
    async fn dispatch_stack_candidates(&mut self) -> Result<(), Halt> {
        let cursor = self.stack_rotation.cursor();
        let listed = self.db().stack_candidates_after(&cursor);
        let Some(jobs) = self.judge("list stack jobs", listed, &[]) else {
            return Ok(());
        };
        let now = self.now();
        self.count_seen(jobs.iter().map(|(_, job)| job.state));
        self.stack_rotation.advance(
            jobs.iter()
                .filter(|(_, job)| job.state != JobState::Queued)
                .map(|(_, job)| job.id.as_str()),
            now,
        );
        for (principal, job) in jobs {
            if job.state != JobState::Queued && !self.stack_rotation.due(&job.id, now) {
                continue;
            }
            match self.dispatch_stack(principal, job).await {
                Ok(()) => {}
                Err(Halt::CoreCrash) => return Err(Halt::CoreCrash),
                Err(Halt::Error(step, e)) => {
                    let op = step_name(step, "dispatch stack job");
                    self.judge::<()>(op, Err(e), &DISPATCH_LEGIT);
                }
            }
        }
        Ok(())
    }

    /// `stack_operations::Core::settle_missing_receipt`.
    fn settle_missing_receipt(
        &mut self,
        job: &StackJob,
        absence: Option<ReceiptAbsence>,
        closed: bool,
    ) -> Result<(), Error> {
        if absence == Some(ReceiptAbsence::NeverRecorded) && !closed {
            return Ok(());
        }
        let now = self.now();
        self.db().record_receipt_missing(&job.id, now)
    }
    /// `stack_operations::Core::record_closure`.
    fn record_closure(
        &mut self,
        job: &StackJob,
        digest: &str,
        receipt: StackReceipt,
    ) -> Result<(), Error> {
        if receipt.validate().is_err()
            || receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
            || receipt.state != ExecutionState::PreconditionChanged
        {
            self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
            return Err(Error(ErrorCode::Conflict));
        }
        let now = self.now();
        self.db()
            .record_stack_closure(job, &receipt, now)
            .map(|_| ())
    }
    /// `stack_operations::Core::release_if_unsent`.
    async fn release_if_unsent(&mut self, job: &StackJob, digest: &str) -> Result<(), Error> {
        let lookup = self
            .executor(Request::StackReceipt {
                version: VERSION,
                action: job.id.clone(),
                digest: digest.to_owned(),
                plan: job.plan.clone(),
                close_if_absent: true,
            })
            .await;
        match lookup {
            Ok(value) => match value.stack {
                None => self.settle_missing_receipt(job, value.absence, value.closed),
                Some(receipt) if value.closed => self.record_closure(job, digest, receipt),
                Some(_) => self.mark(&job.id, job.state, JobState::NeedsIntervention),
            },
            Err(_) => self.mark(&job.id, job.state, JobState::NeedsIntervention),
        }
    }

    /// `stack_operations::Core::dispatch_stack`.
    async fn dispatch_stack(
        &mut self,
        principal: Principal,
        mut job: StackJob,
    ) -> Result<(), Halt> {
        let digest = limeos_identity::digest(&serde_json::to_string(&job.plan).unwrap());
        let project = job.plan.expected.project()?.to_owned();
        let recovering = job.state != JobState::Queued;
        let (mut absence, mut closed) = (None, false);
        let now = self.now();
        let receipt = if job.state == JobState::Queued {
            let revoked = match self.db().stack_job(&principal, &job.id) {
                Ok(_) => false,
                Err(Error(ErrorCode::Forbidden | ErrorCode::Expired | ErrorCode::NotFound)) => true,
                Err(e) => return Err(e.into()),
            };
            if revoked
                || job.plan.grant_revision != principal.grant_revision
                || job.plan.validate(now).is_err()
            {
                return Ok(self.mark(&job.id, job.state, JobState::PreconditionChanged)?);
            }
            let current = self
                .inspect_stack(&project)
                .await
                .map_err(at("dispatch stack job: inspect"))?;
            if job.plan.check_current(&current, now).is_err() {
                return Ok(self.mark(&job.id, job.state, JobState::PreconditionChanged)?);
            }
            let epoch = self
                .receipt_epoch()
                .await
                .map_err(at("dispatch stack job: receipt epoch"))?;
            self.executor_down_after_epoch(&job.id);
            job = self
                .db()
                .claim_stack(&principal, &job.id, &current, &epoch, now)
                .map_err(at("dispatch stack job: claim (a lock is held)"))?;
            self.crash_at(&job.id, JobFault::CoreCrashAfterClaim)?;
            let transport = self.transport_fault(&job.id);
            let request = Request::ExecuteStack {
                version: VERSION,
                request: StackRequest {
                    action: job.id.clone(),
                    plan_digest: digest.clone(),
                    plan: job.plan.clone(),
                },
            };
            match self.exchange(request, transport).await {
                Ok(value) => value.stack,
                Err(Failure { error, sent })
                    if !sent || error.0 == ErrorCode::BeyondReceiptHistory =>
                {
                    let id = job.id.clone();
                    return Ok(self.db().record_unsent_execute(&id, now)?);
                }
                Err(_) => return Ok(self.release_if_unsent(&job, &digest).await?),
            }
        } else {
            let lost = self.fault(&job.id, |f| f == JobFault::LookupLost);
            let request = Request::StackReceipt {
                version: VERSION,
                action: job.id.clone(),
                digest: digest.clone(),
                plan: job.plan.clone(),
                close_if_absent: true,
            };
            let value = self
                .exchange(request, lost)
                .await
                .map_err(|f| Halt::Error("dispatch stack job: recovery lookup", f.error))?;
            (absence, closed) = (value.absence, value.closed);
            value.stack
        };
        let Some(mut receipt) = receipt else {
            if recovering {
                return Ok(self.settle_missing_receipt(&job, absence, closed)?);
            }
            self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
            return Ok(());
        };
        if closed {
            return Ok(self.record_closure(&job, &digest, receipt)?);
        }
        if receipt.validate().is_err()
            || receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
        {
            self.mark(&job.id, job.state, JobState::NeedsIntervention)?;
            return Err(Error(ErrorCode::Conflict).into());
        }
        let mut afters = None;
        let proof_possible = receipt.members.iter().any(|m| {
            matches!(
                m.state,
                ExecutionState::EffectAccepted
                    | ExecutionState::OutcomeUnknown
                    | ExecutionState::Verified
            )
        });
        if proof_possible {
            self.crash_at(&job.id, JobFault::CoreCrashAfterExecute)?;
            if matches!(job.state, JobState::Running | JobState::OutcomeUnknown) {
                self.mark(&job.id, job.state, JobState::Verifying)?;
                job.state = JobState::Verifying;
            }
            if let Ok(current) = self.inspect_stack(&project).await {
                let targets: Vec<&ContainerSnapshot> = job.plan.targets();
                let seen: Vec<ContainerSnapshot> = current
                    .members
                    .iter()
                    .filter(|m| targets.iter().any(|t| t.resource == m.resource))
                    .cloned()
                    .collect();
                let proofs: Vec<ContainerSnapshot> = targets
                    .iter()
                    .filter_map(|before| {
                        seen.iter()
                            .find(|now| now.resource == before.resource)
                            .filter(|now| {
                                verify_stack_member(job.plan.operation, before, now).is_ok()
                            })
                            .cloned()
                    })
                    .collect();
                let unresolved = receipt.members.iter().any(|m| {
                    matches!(
                        m.state,
                        ExecutionState::EffectAccepted | ExecutionState::OutcomeUnknown
                    )
                });
                if unresolved && !proofs.is_empty() {
                    let request = Request::VerifyStack {
                        version: VERSION,
                        action: job.id.clone(),
                        digest: digest.clone(),
                        after: proofs,
                    };
                    if let Ok(value) = self.executor(request).await {
                        if let Some(verified) = value.stack {
                            if verified.validate().is_ok()
                                && verified.action == receipt.action
                                && verified.plan_digest == receipt.plan_digest
                                && verified.before == receipt.before
                                && verified.operation == receipt.operation
                            {
                                receipt = verified;
                            }
                        }
                    }
                    self.crash_at(&job.id, JobFault::CoreCrashAfterVerify)?;
                }
                afters = Some(seen);
            }
        }
        self.db()
            .record_stack_result(&job, &receipt, afters.as_deref(), now)?;
        Ok(())
    }

    /// `job_resolution::Core::measure`.
    async fn measure(
        &mut self,
        operation: limeos_domain::ContainerAction,
        targets: &[ContainerSnapshot],
    ) -> Result<ResolutionEvidence, Error> {
        let mut current = Vec::with_capacity(targets.len());
        for target in targets {
            current.push(match self.inspect(&target.resource).await {
                Ok(snapshot) => Some(snapshot),
                Err(Error(ErrorCode::NotFound)) => None,
                Err(e) => return Err(e),
            });
        }
        ResolutionEvidence::observe(operation, &targets.iter().collect::<Vec<_>>(), &current)
    }

    /// `job_resolution::Core::resolve_as`, for the operator's decision on
    /// one job: fresh evidence, the declaration the evidence supports (on a
    /// second attempt, an accepted divergence), then the crash-safe
    /// request, executor, completion order.
    pub async fn resolve_as(
        &mut self,
        admin: Principal,
        job: String,
    ) -> Result<Option<Finish>, Halt> {
        let subject = self.db().resolution_subject(&admin, &job)?;
        let open = self.db().pending_resolution(&job)?.is_some();
        let done = self.db().job_resolution(&admin, &job)?.is_some();
        if done || open || !awaiting(subject.state) {
            return Ok(None);
        }
        let evidence = self.measure(subject.operation, &subject.targets).await?;
        let attempt = {
            let count = self.attempts.entry(job.clone()).or_default();
            *count += 1;
            *count
        };
        let declared =
            if attempt == 1 && evidence.blocking(DeclaredOutcome::VerifiedDone).is_empty() {
                DeclaredOutcome::VerifiedDone
            } else if attempt == 1 && evidence.blocking(DeclaredOutcome::NotDone).is_empty() {
                DeclaredOutcome::NotDone
            } else {
                DeclaredOutcome::AcceptedDivergence
            };
        let reason = match declared {
            DeclaredOutcome::AcceptedDivergence => {
                "checked the containers by hand; accepting their state"
            }
            _ => "checked by the soak operator",
        };
        let digest = evidence.digest()?;
        let now = self.now();
        let pending = self
            .db()
            .request_resolution(&admin, &job, declared, reason, &evidence, &digest, now)?;
        if let Some(record) = self.ledger.get_mut(&job) {
            record.declared = Some(declared);
        }
        *self.stats.ops.entry("resolution requested").or_default() += 1;
        // An operator decision interrupted by a core restart: the retry loop
        // completes it after startup.
        if self.rng.per_mille(100) {
            *self
                .stats
                .faults
                .entry("core restart between resolution request and executor")
                .or_default() += 1;
            return Err(Halt::CoreCrash);
        }
        Ok(Some(self.finish_resolution(&pending, &admin).await))
    }

    /// `job_resolution::Core::finish_resolution`.
    pub async fn finish_resolution(
        &mut self,
        pending: &PendingResolution,
        actor: &Principal,
    ) -> Finish {
        let request = &pending.request;
        let confirmation = self
            .executor(Request::ResolveAction {
                version: VERSION,
                kind: pending.kind,
                action: request.job.clone(),
                digest: pending.plan_digest.clone(),
                evidence_digest: request.evidence_digest.clone(),
                declared: request.declared,
                resolved_by: request.resolved_by.clone(),
                plan: pending.plan.clone(),
                disputed: pending.disputed,
            })
            .await;
        match confirmation {
            Ok(receipt) => {
                if let Some(confirmation) = receipt.resolution {
                    return self.complete(&request.job, confirmation);
                }
                if receipt.resolution_refusal.is_some() {
                    return self.withdraw(actor, &request.job, Finish::Disagrees);
                }
                match receipt.absence {
                    Some(ReceiptAbsence::NeverRecorded) if receipt.closed && !pending.disputed => {
                        let recorded = ExecutionState::PreconditionChanged;
                        if let Some(allowed) = limeos_domain::declarations_for_recorded(recorded)
                            && !allowed.contains(&request.declared)
                        {
                            return self.withdraw(actor, &request.job, Finish::Disagrees);
                        }
                    }
                    Some(ReceiptAbsence::NoLongerHeld | ReceiptAbsence::NeverRecorded)
                        if receipt.closed => {}
                    _ => return Finish::Pending(ErrorCode::Unavailable),
                }
                match self.measure(pending.operation, &pending.targets).await {
                    Ok(now) if now.digest().ok().as_deref() == Some(&request.evidence_digest) => {}
                    Ok(_) => {
                        return self.withdraw(
                            actor,
                            &request.job,
                            Finish::Refused(ErrorCode::Conflict),
                        );
                    }
                    Err(e) => return Finish::Pending(e.0),
                }
                self.complete(
                    &request.job,
                    ResolveReceipt {
                        kind: pending.kind,
                        action: request.job.clone(),
                        evidence_digest: request.evidence_digest.clone(),
                        declared: request.declared,
                        resolved_by: request.resolved_by.clone(),
                    },
                )
            }
            Err(Error(
                code @ (ErrorCode::Conflict | ErrorCode::NotFound | ErrorCode::Forbidden),
            )) => self.withdraw(actor, &request.job, Finish::Refused(code)),
            Err(e) => Finish::Pending(e.0),
        }
    }
    fn complete(&mut self, job: &str, confirmation: ResolveReceipt) -> Finish {
        let now = self.now();
        match self.db().complete_resolution(job, &confirmation, now) {
            Ok(_) => Finish::Done,
            Err(Error(ErrorCode::Conflict)) => match self.db().supersede_resolution(job, now) {
                Ok(Some(_)) => {
                    *self.stats.ops.entry("authority: superseded").or_default() += 1;
                    Finish::Superseded
                }
                Ok(None) => Finish::Pending(ErrorCode::Conflict),
                Err(e) => Finish::Pending(e.0),
            },
            Err(Error(ErrorCode::NotFound)) => match self.db().job_state(job) {
                Ok(state) => match serde_json::from_value::<JobState>(state.into()) {
                    Ok(state) if !held(state) => Finish::Superseded,
                    _ => Finish::Pending(ErrorCode::NotFound),
                },
                Err(e) => Finish::Pending(e.0),
            },
            Err(e) => Finish::Pending(e.0),
        }
    }
    fn withdraw(&mut self, actor: &Principal, job: &str, then: Finish) -> Finish {
        let now = self.now();
        match self.db().withdraw_resolution(actor, job, now) {
            Ok(()) => {
                *self.stats.ops.entry("authority: withdrawn").or_default() += 1;
                then
            }
            Err(Error(ErrorCode::NotFound)) => then,
            Err(e) => Finish::Pending(e.0),
        }
    }
    /// Counts what finishing came to; a lifetime code among the pending
    /// ones is a violation like any other.
    pub fn note_finish(&mut self, finish: Finish) {
        let (op, result) = match finish {
            Finish::Done => ("resolution completed", Ok(())),
            Finish::Superseded => ("resolution superseded", Ok(())),
            Finish::Disagrees => ("resolution refused by the receipt", Ok(())),
            Finish::Refused(code) => ("resolution refused", Err(Error(code))),
            Finish::Pending(code) => ("resolution left pending", Err(Error(code))),
        };
        self.judge(op, result, &DISPATCH_LEGIT);
    }

    /// `job_resolution::Core::complete_pending_resolutions`.
    async fn complete_pending_resolutions(&mut self, min_age: i64) -> Result<(), Halt> {
        let now = self.now();
        let superseded = self.db().supersede_resolutions(now);
        if let Some(superseded) =
            self.judge("supersede settled resolution requests", superseded, &[])
        {
            *self.stats.ops.entry("authority: superseded").or_default() += superseded.len() as u64;
        }
        let cursor = self.resolution_rotation.cursor();
        let listed = self.db().pending_resolutions_after(&cursor);
        let Some(pending) = self.judge("list pending resolutions", listed, &[]) else {
            return Ok(());
        };
        self.seen_held += pending.len();
        self.resolution_rotation
            .advance(pending.iter().map(|p| p.request.job.as_str()), now);
        for pending in pending {
            if now - pending.request.requested_at < min_age
                || !self.resolution_rotation.due(&pending.request.job, now)
            {
                continue;
            }
            let id = pending.request.resolved_by.clone();
            let actor = self.db().resolution_principal(&id);
            let Some(actor) = self.judge("resolution administrator", actor, &[]) else {
                continue;
            };
            let finish = self.finish_resolution(&pending, &actor).await;
            self.note_finish(finish);
        }
        Ok(())
    }
}
