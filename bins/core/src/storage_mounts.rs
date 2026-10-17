use super::*;
use limeos_domain::{
    JobState, MountExecutionState, Principal, StorageMountEffectPlan, StorageMountInspection,
    StorageMountJob, StorageMountSelection,
};
use limeos_executor_protocol::{Receipt, Request};
use std::time::Duration;

fn rpc_failure(operation: &'static str, phase: &'static str, error: Error) -> Error {
    use std::io::Write as _;
    let _ = writeln!(
        std::io::stderr().lock(),
        "{{\"level\":\"WARN\",\"event\":\"storage_mount_rpc_refused\",\"operation\":\"{operation}\",\"phase\":\"{phase}\",\"code\":\"{:?}\"}}",
        error.0
    );
    error
}

impl Core {
    pub(super) async fn human_mount_catalog(
        &self,
        token: String,
    ) -> Result<limeos_domain::StorageMountCatalog> {
        self.storage_principal(token.clone(), None).await?;
        let value = self
            .mount_executor(
                Request::StorageMountCatalog { version: VERSION },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?;
        let catalog = value
            .storage_mount_catalog
            .ok_or(Error(ErrorCode::Unavailable))?;
        catalog.validate()?;
        // Revocation during protected configuration collection hides the result.
        self.storage_principal(token, None).await?;
        Ok(catalog)
    }
    async fn mount_executor(&self, request: Request, deadline: Duration) -> Result<Receipt> {
        let operation = match &request {
            Request::StorageMountCatalog { .. } => "catalog",
            Request::InspectStorageMount { .. } => "inspect",
            Request::ExecuteStorageMount { .. } => "execute",
            Request::StorageMountReceipt { .. } => "receipt",
            Request::ReconcileStorageMount { .. } => "reconcile",
            _ => "other",
        };
        tokio::time::timeout(deadline, async {
            let mut stream = UnixStream::connect(self.storage_mount_socket.as_ref())
                .await
                .map_err(|_| rpc_failure(operation, "connect", Error(ErrorCode::Unavailable)))?;
            if stream
                .peer_cred()
                .map_err(|_| {
                    rpc_failure(operation, "peer_credentials", Error(ErrorCode::Unavailable))
                })?
                .uid()
                != 0
            {
                return Err(rpc_failure(
                    operation,
                    "peer_identity",
                    Error(ErrorCode::Forbidden),
                ));
            }
            limeos_contracts::write_frame(&mut stream, &request)
                .await
                .map_err(|error| rpc_failure(operation, "write_frame", error))?;
            let receipt: Receipt = limeos_contracts::read_frame(&mut stream)
                .await
                .map_err(|error| rpc_failure(operation, "read_frame", error))?;
            if receipt.version != VERSION {
                return Err(rpc_failure(
                    operation,
                    "reply_version",
                    Error(ErrorCode::Unavailable),
                ));
            }
            if let Some(code) = receipt.error {
                return Err(rpc_failure(operation, "root_reply", Error(code)));
            }
            if !receipt.ready {
                return Err(rpc_failure(
                    operation,
                    "reply_ready",
                    Error(ErrorCode::Unavailable),
                ));
            }
            Ok(receipt)
        })
        .await
        .map_err(|_| rpc_failure(operation, "deadline", Error(ErrorCode::Unavailable)))?
    }
    async fn inspect_mount(
        &self,
        selection: StorageMountSelection,
    ) -> Result<StorageMountInspection> {
        selection.validate()?;
        let _permit = self
            .storage_readers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        let expected = selection.target_id.clone();
        let value = self
            .mount_executor(
                Request::InspectStorageMount {
                    version: VERSION,
                    selection,
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?;
        let inspection = value.storage_mount_inspection.ok_or_else(|| {
            rpc_failure("inspect", "reply_inspection", Error(ErrorCode::Unavailable))
        })?;
        inspection
            .validate(now())
            .map_err(|error| rpc_failure("inspect", "validate_inspection", error))?;
        if inspection.target_id != expected {
            return Err(rpc_failure(
                "inspect",
                "reply_target",
                Error(ErrorCode::Conflict),
            ));
        }
        Ok(inspection)
    }
    pub(super) async fn human_plan_mount(
        &self,
        token: String,
        csrf: String,
        selection: StorageMountSelection,
    ) -> Result<limeos_domain::PlannedStorageMount> {
        self.storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let inspection = self.inspect_mount(selection).await?;
        let token = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |store| {
                let principal = store.authenticate(&token, Some(&csrf), now())?;
                store.plan_storage_mount(&principal, &inspection, now())
            })
            .await
            .map_err(|error| rpc_failure("plan", "store", error))
    }
    pub(super) async fn human_approve_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let principal = self
            .storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let plan_id = id.clone();
        let proposal = self
            .db
            .call(move |store| store.storage_mount_plan(&principal, &plan_id))
            .await?;
        if !limeos_identity::constant_eq(&proposal.digest, &digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        let inspection = self
            .inspect_mount(StorageMountSelection {
                target_id: proposal.plan.effect.device.id,
            })
            .await?;
        let token = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |store| {
                let principal = store.authenticate(&token, Some(&csrf), now())?;
                store.approve_storage_mount(&principal, &id, &digest, &inspection, now())
            })
            .await
    }
    pub(super) async fn human_queue_mount(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageMountInput,
    ) -> Result<StorageMountJob> {
        let principal = self
            .storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let supplied = input.clone();
        if let Some(job) = self
            .db
            .call(move |store| {
                store.replay_storage_mount(
                    &principal,
                    &supplied.idempotency_key,
                    &supplied.proposal,
                    &supplied.approval,
                )
            })
            .await?
        {
            return Ok(job);
        }
        let inspection = self
            .inspect_mount(StorageMountSelection {
                target_id: input.proposal.plan.effect.device.id.clone(),
            })
            .await?;
        let token = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |store| {
                let principal = store.authenticate(&token, Some(&csrf), now())?;
                store.queue_storage_mount(
                    &principal,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &inspection,
                    now(),
                )
            })
            .await
    }
    async fn mark_mount(&self, job: &StorageMountJob, to: JobState) -> Result<()> {
        if job.state == to {
            return Ok(());
        }
        let id = job.id.clone();
        let from = job.state;
        self.db
            .call(move |store| store.transition(&id, from, to, now()))
            .await
    }
    async fn independently_mounted(
        &self,
        plan: StorageMountEffectPlan,
    ) -> Result<limeos_domain::StorageMountedSnapshot> {
        let value = self
            .mount_executor(
                Request::InspectMountedStorage {
                    version: VERSION,
                    plan: Box::new(plan),
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?;
        value.mounted_storage.ok_or(Error(ErrorCode::Unavailable))
    }
    pub(super) async fn dispatch_mount(
        &self,
        principal: Principal,
        mut job: StorageMountJob,
    ) -> Result<()> {
        let receipt = if job.state == JobState::Queued {
            let checked = principal.clone();
            if self
                .db
                .call(move |store| store.authorize_storage(&checked))
                .await
                .is_err()
                || job.plan.grant_revision != principal.grant_revision
                || job.plan.validate(now()).is_err()
            {
                return self.mark_mount(&job, JobState::PreconditionChanged).await;
            }
            let id = job.id.clone();
            if !self
                .db
                .call(move |store| store.storage_mount_resources_available(&id))
                .await?
            {
                return Ok(());
            }
            let current = self
                .inspect_mount(StorageMountSelection {
                    target_id: job.plan.effect.device.id.clone(),
                })
                .await?;
            if job.plan.effect.check_current(&current, now()).is_err() {
                return self.mark_mount(&job, JobState::PreconditionChanged).await;
            }
            let id = job.id.clone();
            job = self
                .db
                .call(move |store| store.claim_storage_mount(&principal, &id, &current, now()))
                .await?;
            // Durable Running and the complete claim set precede any effect RPC.
            match self
                .mount_executor(
                    Request::ExecuteStorageMount {
                        version: VERSION,
                        plan: Box::new(job.plan.effect.clone()),
                    },
                    Duration::from_secs(
                        u64::from(limeos_domain::STORAGE_MOUNT.timeout_seconds) + 5,
                    ),
                )
                .await
            {
                Ok(value) => value.storage_mount,
                Err(_) => {
                    self.mark_mount(&job, JobState::NeedsIntervention).await?;
                    return Ok(());
                }
            }
        } else {
            let digest = limeos_identity::digest(
                &serde_json::to_string(&job.plan.effect)
                    .map_err(|_| Error(ErrorCode::StateNotDurable))?,
            );
            // Recovery reads the original receipt. It never executes or renews a plan.
            self.mount_executor(
                Request::StorageMountReceipt {
                    version: VERSION,
                    action: job.id.clone(),
                    digest,
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .storage_mount
        };
        let Some(mut receipt) = receipt else {
            return self.mark_mount(&job, JobState::NeedsIntervention).await;
        };
        // A crash may leave a durable accepted/reaped receipt before the root
        // verifier persisted Verified. This protected refinement cannot launch
        // another helper. Prepared, pending and unknown effects remain barriers.
        if receipt.state == MountExecutionState::EffectAccepted
            && receipt.plan == job.plan.effect
            && receipt.validate().is_ok()
        {
            let digest = limeos_identity::digest(
                &serde_json::to_string(&job.plan.effect)
                    .map_err(|_| Error(ErrorCode::StateNotDurable))?,
            );
            if limeos_identity::constant_eq(&receipt.digest, &digest) {
                if let Ok(value) = self
                    .mount_executor(
                        Request::ReconcileStorageMount {
                            version: VERSION,
                            action: job.id.clone(),
                            digest,
                        },
                        limeos_contracts::RPC_DEADLINE,
                    )
                    .await
                {
                    if let Some(refined) = value.storage_mount {
                        receipt = refined;
                    }
                }
            }
        }
        let after = if receipt.state == MountExecutionState::Verified {
            self.independently_mounted(job.plan.effect.clone())
                .await
                .ok()
        } else {
            None
        };
        let unmounted = if receipt.state == MountExecutionState::PreconditionChanged {
            self.inspect_mount(StorageMountSelection {
                target_id: job.plan.effect.device.id.clone(),
            })
            .await
            .ok()
        } else {
            None
        };
        let saved = job.clone();
        let result = self
            .db
            .call(move |store| {
                store.record_storage_mount_result(
                    &saved,
                    &receipt,
                    after.as_ref(),
                    unmounted.as_ref(),
                    now(),
                )
            })
            .await;
        if let Err(error) = result {
            self.mark_mount(&job, JobState::NeedsIntervention).await?;
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
