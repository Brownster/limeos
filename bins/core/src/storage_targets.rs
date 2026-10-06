use super::*;
use limeos_domain::{
    JobState, Principal, StorageContract, StorageTargetJob, StorageTargetSnapshot,
};
use limeos_executor_protocol::{Receipt, Request};
use std::time::Duration;

impl Core {
    async fn target_executor(&self, request: Request, deadline: Duration) -> Result<Receipt> {
        tokio::time::timeout(deadline, async {
            let mut stream = UnixStream::connect(self.storage_target_socket.as_ref())
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            if stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Unavailable))?
                .uid()
                != 0
            {
                return Err(Error(ErrorCode::Forbidden));
            }
            limeos_contracts::write_frame(&mut stream, &request).await?;
            let value: Receipt = limeos_contracts::read_frame(&mut stream).await?;
            if let Some(code) = value.error {
                return Err(Error(code));
            }
            if value.version != VERSION || !value.ready {
                return Err(Error(ErrorCode::Unavailable));
            }
            Ok(value)
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn target_snapshot(&self, contract: &StorageContract) -> Result<StorageTargetSnapshot> {
        contract.validate()?;
        let value = self
            .target_executor(
                Request::InspectStorageTargets {
                    version: VERSION,
                    contract: contract.clone(),
                },
                Duration::from_secs(14),
            )
            .await?;
        let snapshot = value.storage_targets.ok_or(Error(ErrorCode::Unavailable))?;
        snapshot.inventory.validate()?;
        Ok(snapshot)
    }
    pub(super) async fn human_plan_targets(
        &self,
        token: String,
        csrf: String,
        contract: StorageContract,
    ) -> Result<limeos_domain::PlannedStorageTargets> {
        self.storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let current = self.target_snapshot(&contract).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let p = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_storage_targets(&p, &contract, &current, now())
            })
            .await
    }
    pub(super) async fn human_approve_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let p = self
            .storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let plan_id = id.clone();
        let proposal = self
            .db
            .call(move |s| s.storage_target_plan(&p, &plan_id))
            .await?;
        if !limeos_identity::constant_eq(&proposal.digest, &digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        let current = self
            .target_snapshot(&proposal.plan.preparation.contract)
            .await?;
        let token = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let p = s.authenticate(&token, Some(&csrf), now())?;
                s.approve_storage_targets(&p, &id, &digest, &current, now())
            })
            .await
    }
    pub(super) async fn human_queue_targets(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageTargetsInput,
    ) -> Result<StorageTargetJob> {
        let p = self
            .storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let supplied = input.clone();
        if let Some(job) = self
            .db
            .call(move |s| {
                s.replay_storage_targets(
                    &p,
                    &supplied.idempotency_key,
                    &supplied.proposal,
                    &supplied.approval,
                )
            })
            .await?
        {
            return Ok(job);
        }
        let current = self
            .target_snapshot(&input.proposal.plan.preparation.contract)
            .await?;
        let token = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let p = s.authenticate(&token, Some(&csrf), now())?;
                s.queue_storage_targets(
                    &p,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &current,
                    now(),
                )
            })
            .await
    }
    async fn mark_target(&self, job: &StorageTargetJob, to: JobState) -> Result<()> {
        if job.state == to {
            return Ok(());
        }
        let id = job.id.clone();
        let from = job.state;
        self.db
            .call(move |s| s.transition(&id, from, to, now()))
            .await
    }
    pub(super) async fn dispatch_target(
        &self,
        p: Principal,
        mut job: StorageTargetJob,
    ) -> Result<()> {
        let receipt = if job.state == JobState::Queued {
            let checked = p.clone();
            if self
                .db
                .call(move |s| s.authorize_storage(&checked))
                .await
                .is_err()
                || job.plan.grant_revision != p.grant_revision
                || job.plan.validate(now()).is_err()
            {
                return self.mark_target(&job, JobState::PreconditionChanged).await;
            }
            let id = job.id.clone();
            if !self
                .db
                .call(move |s| s.storage_target_resources_available(&id))
                .await?
            {
                return Ok(());
            }
            let current = self.target_snapshot(&job.plan.preparation.contract).await?;
            if job.plan.preparation.check_current(&current, now()).is_err() {
                return self.mark_target(&job, JobState::PreconditionChanged).await;
            }
            let id = job.id.clone();
            job = self
                .db
                .call(move |s| s.claim_storage_targets(&p, &id, &current, now()))
                .await?;
            // Once durable dispatch exists, timeout/death cannot restore Queued.
            match self
                .target_executor(
                    Request::PrepareStorageTargets {
                        version: VERSION,
                        plan: Box::new(job.plan.preparation.clone()),
                    },
                    Duration::from_secs(
                        u64::from(limeos_domain::STORAGE_PREPARE_TARGETS.timeout_seconds) + 5,
                    ),
                )
                .await
            {
                Ok(value) => value.target_preparation,
                Err(_) => {
                    self.mark_target(&job, JobState::NeedsIntervention).await?;
                    return Ok(());
                }
            }
        } else {
            let digest = limeos_identity::digest(
                &serde_json::to_string(&job.plan.preparation)
                    .map_err(|_| Error(ErrorCode::StateNotDurable))?,
            );
            // Recovery only reads a receipt. Root reconciliation is explicit.
            self.target_executor(
                Request::StorageTargetReceipt {
                    version: VERSION,
                    action: job.id.clone(),
                    digest,
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .target_preparation
        };
        let Some(receipt) = receipt else {
            return self.mark_target(&job, JobState::NeedsIntervention).await;
        };
        let after = if receipt.state == limeos_domain::PreparationState::Verified {
            self.target_snapshot(&job.plan.preparation.contract)
                .await
                .ok()
        } else {
            None
        };
        self.db
            .call(move |s| s.record_storage_target_result(&job, &receipt, after.as_ref(), now()))
            .await?;
        Ok(())
    }
}
#[cfg(test)]
mod tests;
