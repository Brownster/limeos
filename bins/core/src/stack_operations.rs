//! Whole-stack start, stop and restart: the same human plan, approval, queue,
//! claim, executor-receipt and verification path as a single container, with
//! the member list of a registered stack as the bound precondition.
use super::*;
use limeos_domain::{
    ContainerAction, ContainerSnapshot, ExecutionState, JobState, PlanApproval, PlannedStackAction,
    Principal, ReceiptAbsence, StackJob, StackSnapshot,
    deployment::discovery::{RuntimeStatus, SourceState},
    execution_closed, stack_definition, stack_project, stack_resource, verify_stack_member,
};
use limeos_executor_protocol::Request;
use std::time::{Duration, Instant};

impl Core {
    /// The protected inventory cohort decides eligibility and supplies the
    /// planned member list. A stack that is not registered, has any issue, or
    /// whose sources are not both fresh and matching is never planned.
    pub(super) fn cohort_stack(&self, project: &str) -> Result<(String, StackSnapshot)> {
        if !stack_project(project) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let inventory = self.stacks.cohort()?.project(now())?;
        if inventory.catalog.state != SourceState::Fresh
            || inventory.engine.state != SourceState::Fresh
            || inventory.engine_matches_catalog != Some(true)
        {
            return Err(Error(ErrorCode::Unavailable));
        }
        let stack = inventory
            .stacks
            .iter()
            .find(|stack| stack.project == project)
            .ok_or(Error(ErrorCode::NotFound))?;
        if !stack.issues.is_empty()
            || matches!(
                stack.runtime,
                RuntimeStatus::Unknown | RuntimeStatus::Conflict
            )
        {
            return Err(Error(ErrorCode::Conflict));
        }
        // One-off containers are never members of a lifecycle action.
        let members: Vec<_> = stack
            .services
            .iter()
            .flat_map(|service| &service.members)
            .filter(|member| !member.one_off)
            .map(|member| member.container.clone())
            .collect();
        // A registered stack without service containers has nothing to act on.
        if members.is_empty() {
            return Err(Error(ErrorCode::Conflict));
        }
        // The catalog's stack name and the Compose project may differ; deploy
        // jobs lock `stack:<stack>`, so lifecycle jobs lock and authorize it too.
        Ok((stack.stack.clone(), StackSnapshot::new(project, members)?))
    }
    /// A fresh Engine listing through the container executor.
    pub(super) async fn inspect_stack(&self, project: &str) -> Result<StackSnapshot> {
        if !stack_project(project) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(
                Request::InspectStack {
                    version: VERSION,
                    project: project.into(),
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .stack_members
            .ok_or(Error(ErrorCode::Unavailable))?;
        value.validate()?;
        if value.resource != stack_resource(project) {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(value)
    }
    pub(super) async fn propose_stack(
        &self,
        token: String,
        csrf: String,
        project: String,
        operation: ContainerAction,
    ) -> Result<PlannedStackAction> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        let (catalog_stack, planned) = self.cohort_stack(&project)?;
        planned.validate_for(operation)?;
        // Refuse before any executor contact if the grants do not reach the
        // stack and every member.
        let (p, s, c) = (principal.clone(), planned.clone(), catalog_stack.clone());
        self.db
            .call(move |store| store.authorize_stack(&p, &c, &s))
            .await?;
        // The cache can lag the Engine. Fail now rather than at queue time.
        if self.inspect_stack(&project).await? != planned {
            return Err(Error(ErrorCode::Conflict));
        }
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_stack(&principal, operation, &catalog_stack, &planned, now())
            })
            .await
    }
    pub(super) async fn approve_stack_plan(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<PlanApproval> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.approve_stack(&principal, &id, &plan_digest, now())
            })
            .await
    }
    pub(super) async fn submit_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStackInput,
    ) -> Result<StackJob> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        // Reject changed/unowned plans before selected inspection.
        let (p, proposal) = (principal.clone(), input.proposal.clone());
        let (key, approval) = (input.idempotency_key.clone(), input.approval.clone());
        let replay = self
            .db
            .call(move |s| s.replay_stack(&p, &key, &proposal, &approval))
            .await?;
        if let Some(job) = replay {
            return Ok(job);
        }
        let project = input.proposal.plan.expected.project()?.to_owned();
        let snapshot = self.inspect_stack(&project).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.queue_stack(
                    &principal,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &snapshot,
                    now(),
                )
            })
            .await
    }
    pub(super) async fn stack_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> Result<limeos_contracts::StackJobProgress> {
        let principal = self.session(token, None).await?.principal;
        self.db
            .call(move |s| {
                Ok(limeos_contracts::StackJobProgress {
                    job: s.stack_job(&principal, &id)?,
                    events: s.stack_events(&principal, &id, after)?,
                    receipt: s.stack_result(&principal, &id)?.map(|(receipt, _)| receipt),
                })
            })
            .await
    }
    pub(super) async fn stack_job_list(&self, token: String) -> Result<Vec<StackJob>> {
        let principal = self.session(token, None).await?.principal;
        self.db.call(move |s| s.stack_jobs(&principal)).await
    }
    pub(super) async fn cancel_stack(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> Result<()> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                if plan {
                    s.cancel_stack_plan(&principal, &id, now())
                } else {
                    s.cancel_stack_job(&principal, &id, now())
                }
            })
            .await
    }
    async fn mark_stack(&self, job: &StackJob, state: JobState) -> Result<()> {
        let (id, from) = (job.id.clone(), job.state);
        if from == state {
            return Ok(());
        }
        self.db
            .call(move |s| s.transition(&id, from, state, now()))
            .await
    }

    /// A lookup that succeeded and found no row; `asked_at` is core's clock
    /// read before that lookup was sent. Only the executor's `NeverRecorded`
    /// proves no receipt was ever written, since no receipt for this plan was
    /// ever pruned. That proves nothing was sent only once no execute request
    /// can still be accepted: one delivered earlier may not yet have reached the
    /// receipt store, and would then prepare and run while the plan is valid.
    /// The executor refuses a plan from its expiry, before preparing and again
    /// before the Engine call, so once `limeos_domain::execution_closed` holds
    /// the locks are released as `precondition_changed`; until then the job
    /// keeps its state and its locks, and the recovery poll asks again.
    /// `NoLongerHeld`, or no answer at all, means the executor may have
    /// executed, verified and pruned the action: the job stays held in
    /// `needs_intervention` for an operator, with the reason.
    async fn settle_missing_receipt(
        &self,
        job: &StackJob,
        absence: Option<ReceiptAbsence>,
        asked_at: i64,
    ) -> Result<()> {
        let job = job.clone();
        if absence == Some(ReceiptAbsence::NeverRecorded) {
            if !execution_closed(job.plan.expires_at, asked_at) {
                tracing::info!(
                    "executor holds no receipt for a stack job whose plan is still valid; held until it expires"
                );
                return Ok(());
            }
            return self
                .db
                .call(move |s| s.record_stack_unsent(&job, now()))
                .await;
        }
        tracing::warn!(
            "executor no longer holds a receipt for an uncertain stack job; held for an operator"
        );
        self.db
            .call(move |s| s.record_receipt_missing(&job.id, now()))
            .await
    }

    /// After a failed ExecuteStack call, one receipt lookup decides. A lookup
    /// that succeeds and finds no row is settled by `settle_missing_receipt`.
    /// A failed lookup, or any row, keeps the stack held for reconciliation.
    async fn release_if_unsent(&self, job: &StackJob, digest: &str) -> Result<()> {
        // Read before the lookup: the executor's answer is no older than this.
        let asked_at = now();
        let lookup = self
            .executor(
                Request::StackReceipt {
                    version: VERSION,
                    action: job.id.clone(),
                    digest: digest.to_owned(),
                    expires_at: job.plan.expires_at,
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await;
        match lookup {
            Ok(value) if value.stack.is_none() => {
                self.settle_missing_receipt(job, value.absence, asked_at)
                    .await
            }
            _ => self.mark_stack(job, JobState::NeedsIntervention).await,
        }
    }

    pub(super) async fn dispatch_stack(
        &self,
        principal: Principal,
        mut job: StackJob,
    ) -> Result<()> {
        let digest = limeos_identity::digest(
            &serde_json::to_string(&job.plan).map_err(|_| Error(ErrorCode::InvalidInput))?,
        );
        let project = job.plan.expected.project()?.to_owned();
        let recovering = job.state != JobState::Queued;
        let mut absence = None;
        // Read before the recovery lookup below; an earlier read only errs
        // towards holding the job (`settle_missing_receipt`).
        let asked_at = now();
        let receipt = if job.state == JobState::Queued {
            // Revocation, expiry and member drift are all decided before the
            // durable dispatch intent. A held lock leaves this job queued.
            let (p, id) = (principal.clone(), job.id.clone());
            // Only a definite refusal is revocation. A transient store error
            // leaves the job queued for the next tick.
            let revoked = match self.db.call(move |s| s.stack_job(&p, &id)).await {
                Ok(_) => false,
                Err(Error(ErrorCode::Forbidden | ErrorCode::Expired | ErrorCode::NotFound)) => true,
                Err(e) => return Err(e),
            };
            if revoked
                || job.plan.grant_revision != principal.grant_revision
                || job.plan.validate(now()).is_err()
            {
                return self.mark_stack(&job, JobState::PreconditionChanged).await;
            }
            let current = self.inspect_stack(&project).await?;
            if job.plan.check_current(&current, now()).is_err() {
                return self.mark_stack(&job, JobState::PreconditionChanged).await;
            }
            let (p, id) = (principal, job.id.clone());
            job = self
                .db
                .call(move |s| s.claim_stack(&p, &id, &current, now()))
                .await?;
            // Core death from here onward requires receipt reconciliation; it
            // cannot put the job back into the dispatch queue.
            let budget = u64::from(stack_definition(job.plan.operation).timeout_seconds)
                * job.plan.targets().len() as u64
                + 8;
            match self
                .executor(
                    Request::ExecuteStack {
                        version: VERSION,
                        request: limeos_domain::StackRequest {
                            action: job.id.clone(),
                            plan_digest: digest.clone(),
                            plan: job.plan.clone(),
                        },
                    },
                    Duration::from_secs(budget),
                )
                .await
            {
                Ok(value) => value.stack,
                Err(_) => {
                    // The executor may have failed before writing any row.
                    return self.release_if_unsent(&job, &digest).await;
                }
            }
        } else {
            // Recovery is GET-like receipt lookup, never an effect request.
            let value = self
                .executor(
                    Request::StackReceipt {
                        version: VERSION,
                        action: job.id.clone(),
                        digest: digest.clone(),
                        expires_at: job.plan.expires_at,
                    },
                    limeos_contracts::RPC_DEADLINE,
                )
                .await?;
            absence = value.absence;
            value.stack
        };
        let Some(mut receipt) = receipt else {
            if recovering {
                // The lookup succeeded and the executor holds no row.
                return self.settle_missing_receipt(&job, absence, asked_at).await;
            }
            self.mark_stack(&job, JobState::NeedsIntervention).await?;
            return Ok(());
        };
        // Reject mismatched IPC evidence before it can unlock a core resource.
        if receipt.validate().is_err()
            || receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
        {
            self.mark_stack(&job, JobState::NeedsIntervention).await?;
            return Err(Error(ErrorCode::Conflict));
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
            if matches!(job.state, JobState::Running | JobState::OutcomeUnknown) {
                self.mark_stack(&job, JobState::Verifying).await?;
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
                // Only members whose own post-state is proven are offered; the
                // executor still inspects each again before trusting them.
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
                    if let Ok(value) = self
                        .executor(
                            Request::VerifyStack {
                                version: VERSION,
                                action: job.id.clone(),
                                digest: digest.clone(),
                                after: proofs,
                            },
                            limeos_contracts::RPC_DEADLINE,
                        )
                        .await
                    {
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
                }
                afters = Some(seen);
            }
        }
        self.db
            .call(move |s| s.record_stack_result(&job, &receipt, afters.as_deref(), now()))
            .await?;
        Ok(())
    }

    /// Bounded retry of receipt reconciliation, never of an effect.
    pub(super) async fn dispatch_stack_candidates(
        &self,
        recovery: &mut std::collections::HashMap<String, Instant>,
    ) {
        let Ok(jobs) = self.db.call(|s| s.stack_candidates()).await else {
            return;
        };
        recovery.retain(|id, _| jobs.iter().any(|(_, job)| &job.id == id));
        for (principal, job) in jobs {
            if job.state != JobState::Queued {
                if recovery
                    .get(&job.id)
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
                {
                    continue;
                }
                recovery.insert(job.id.clone(), Instant::now());
            }
            if let Err(e) = self.dispatch_stack(principal, job).await {
                tracing::warn!(code=?e.0,"stack job remains pending or requires reconciliation");
            }
        }
    }
}
