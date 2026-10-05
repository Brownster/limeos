use super::*;
use limeos_domain::{
    ContainerSnapshot, ExecutionState, JobState, Principal, RestartJob, RestartRequest,
};
use limeos_executor_protocol::{Receipt, Request};
use std::time::{Duration, Instant};

impl Core {
    async fn executor(&self, request: Request, deadline: Duration) -> Result<Receipt> {
        tokio::time::timeout(deadline, async {
            let mut stream = UnixStream::connect(self.container_socket.as_ref())
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            limeos_contracts::write_frame(&mut stream, &request).await?;
            let response: Receipt = limeos_contracts::read_frame(&mut stream).await?;
            if response.version != VERSION {
                return Err(Error(ErrorCode::Unavailable));
            }
            if let Some(error) = response.error {
                return Err(Error(error));
            }
            if !response.ready {
                return Err(Error(ErrorCode::Unavailable));
            }
            Ok(response)
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    pub(super) async fn inspect(&self, resource: &str) -> Result<ContainerSnapshot> {
        if !limeos_domain::opaque_id(resource.strip_prefix("container:").unwrap_or_default()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(
                Request::Inspect {
                    version: VERSION,
                    resource: resource.into(),
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .inspection
            .ok_or(Error(ErrorCode::Unavailable))?;
        value.validate()?;
        if value.resource != resource {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(value)
    }
    pub(super) async fn authorize_resource(
        &self,
        principal: Principal,
        resource: String,
    ) -> Result<()> {
        self.db
            .call(move |s| {
                limeos_policy::authorize(
                    &principal,
                    &s.grants(&principal)?,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                )
            })
            .await
    }
    pub(super) async fn propose(
        &self,
        token: String,
        csrf: String,
        resource: String,
    ) -> Result<limeos_domain::PlannedRestart> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        self.authorize_resource(principal.clone(), resource.clone())
            .await?;
        let snapshot = self.inspect(&resource).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_restart(&principal, &snapshot, now())
            })
            .await
    }
    pub(super) async fn submit(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<RestartJob> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        // Reject changed/unowned plans before selected inspection.
        let p = principal.clone();
        let proposal = input.proposal.clone();
        let key = input.idempotency_key.clone();
        let approval = input.approval.clone();
        let replay = self
            .db
            .call(move |s| s.replay_restart(&p, &key, &proposal, &approval))
            .await?;
        if let Some(job) = replay {
            return Ok(job);
        }
        let snapshot = self.inspect(&input.proposal.plan.expected.resource).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.queue_restart(
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
    pub(super) async fn task_principal(
        &self,
        token: &str,
        uid: u32,
        task: &str,
        resource: &str,
    ) -> Result<Principal> {
        if !limeos_domain::opaque_id(token) {
            return Err(Error(ErrorCode::Expired));
        }
        let digest = limeos_identity::digest(token);
        let task = task.to_owned();
        let resource = resource.to_owned();
        self.db
            .call(move |s| {
                s.check_task(
                    &digest,
                    uid,
                    &task,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                    now(),
                )
            })
            .await
    }
    pub(super) async fn submit_task(
        &self,
        token: String,
        uid: u32,
        task: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<RestartJob> {
        self.task_principal(&token, uid, &task, &input.proposal.plan.expected.resource)
            .await?;
        let digest = limeos_identity::digest(&token);
        let checked_task = task.clone();
        let supplied = input.clone();
        let checked_digest = digest.clone();
        let replay = self
            .db
            .call(move |s| {
                let p = s.check_task(
                    &checked_digest,
                    uid,
                    &checked_task,
                    &supplied.proposal.plan.expected.scope(),
                    now(),
                )?;
                s.replay_restart(
                    &p,
                    &supplied.idempotency_key,
                    &supplied.proposal,
                    &supplied.approval,
                )
            })
            .await?;
        if let Some(job) = replay {
            return Ok(job);
        }
        let snapshot = self.inspect(&input.proposal.plan.expected.resource).await?;
        self.db
            .call(move |s| {
                let p = s.check_task(&digest, uid, &task, &snapshot.scope(), now())?;
                s.queue_restart(
                    &p,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &snapshot,
                    now(),
                )
            })
            .await
    }
    async fn mark(&self, job: &RestartJob, state: JobState) -> Result<()> {
        let id = job.id.clone();
        let from = job.state;
        if from == state {
            return Ok(());
        }
        self.db
            .call(move |s| s.transition(&id, from, state, now()))
            .await
    }
    pub(super) async fn dispatch_one(
        &self,
        principal: Principal,
        mut job: RestartJob,
    ) -> Result<()> {
        let digest = limeos_identity::digest(
            &serde_json::to_string(&job.plan).map_err(|_| Error(ErrorCode::InvalidInput))?,
        );
        let receipt = if job.state == JobState::Queued {
            // Revocation and expiration are checked immediately before durable
            // dispatch intent. A resource lock conflict leaves this job queued.
            if self
                .authorize_resource(principal.clone(), job.plan.expected.resource.clone())
                .await
                .is_err()
                || job.plan.grant_revision != principal.grant_revision
                || job.plan.validate(now()).is_err()
            {
                return self.mark(&job, JobState::PreconditionChanged).await;
            }
            let current = self.inspect(&job.plan.expected.resource).await?;
            if job.plan.check_current(&current, now()).is_err() {
                return self.mark(&job, JobState::PreconditionChanged).await;
            }
            let p = principal;
            let id = job.id.clone();
            job = self
                .db
                .call(move |s| s.claim_restart(&p, &id, &current, now()))
                .await?;
            // Core death from here onward requires receipt reconciliation; it
            // cannot put the job back into the dispatch queue.
            match self
                .executor(
                    Request::Restart {
                        version: VERSION,
                        request: RestartRequest {
                            action: job.id.clone(),
                            plan_digest: digest.clone(),
                            plan: job.plan.clone(),
                        },
                    },
                    Duration::from_secs(38),
                )
                .await
            {
                Ok(value) => value.restart,
                Err(_) => {
                    self.mark(&job, JobState::NeedsIntervention).await?;
                    return Ok(());
                }
            }
        } else {
            // Recovery is GET-like receipt lookup, never an effect request.
            self.executor(
                Request::RestartReceipt {
                    version: VERSION,
                    action: job.id.clone(),
                    digest: digest.clone(),
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .restart
        };
        let Some(mut receipt) = receipt else {
            self.mark(&job, JobState::NeedsIntervention).await?;
            return Ok(());
        };
        // Reject mismatched IPC evidence before it can unlock a core resource.
        if receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
        {
            self.mark(&job, JobState::NeedsIntervention).await?;
            return Err(Error(ErrorCode::Conflict));
        }
        let mut after = None;
        if matches!(
            receipt.state,
            ExecutionState::EffectAccepted | ExecutionState::Verified
        ) {
            if matches!(job.state, JobState::Running | JobState::OutcomeUnknown) {
                self.mark(&job, JobState::Verifying).await?;
                job.state = JobState::Verifying;
            }
            if let Ok(current) = self.inspect(&job.plan.expected.resource).await {
                if limeos_domain::verify_restart(&receipt.before, &current).is_ok() {
                    if receipt.state == ExecutionState::EffectAccepted {
                        if let Ok(value) = self
                            .executor(
                                Request::VerifyRestart {
                                    version: VERSION,
                                    action: job.id.clone(),
                                    digest,
                                    after: current.clone(),
                                },
                                limeos_contracts::RPC_DEADLINE,
                            )
                            .await
                        {
                            if let Some(verified) = value.restart {
                                receipt = verified;
                            }
                        }
                    }
                    after = Some(current);
                }
            }
        }
        self.db
            .call(move |s| s.record_restart_result(&job, &receipt, after.as_ref(), now()))
            .await?;
        Ok(())
    }
    pub(super) async fn dispatcher(&self) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Bound recovery traffic and memory. Uncertain receipts are never retried
        // as mutations, even after their original plan has expired.
        let mut recovery = std::collections::HashMap::<String, Instant>::new();
        loop {
            tick.tick().await;
            let Ok(jobs) = self.db.call(|s| s.restart_candidates()).await else {
                continue;
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
                if let Err(e) = self.dispatch_one(principal, job).await {
                    tracing::warn!(code=?e.0,"container job remains pending or requires reconciliation");
                }
            }
        }
    }
}
