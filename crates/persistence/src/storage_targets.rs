use super::*;
use limeos_domain::{
    PlannedStorageTargets, PreparationState, StorageContract, StorageTargetJob, StorageTargetPlan,
    StorageTargetSnapshot, TargetPreparationPlan, TargetPreparationReceipt,
};
const PLAN_BYTES: usize = 24 * 1024;

impl Store {
    pub fn plan_storage_targets(
        &mut self,
        principal: &Principal,
        contract: &StorageContract,
        snapshot: &StorageTargetSnapshot,
        now: i64,
    ) -> Result<PlannedStorageTargets> {
        self.authorize_storage(principal)?;
        let plan = StorageTargetPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::STORAGE_TARGET_VERSION,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            preparation: TargetPreparationPlan {
                version: 1,
                operation: limeos_domain::STORAGE_TARGET_OPERATION.into(),
                action: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
                expected: snapshot.inventory.clone(),
                contract: contract.clone(),
                targets: snapshot.targets.clone(),
                created_at: now,
                expires_at: now
                    .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                    .ok_or(Error(ErrorCode::InvalidInput))?,
            },
        };
        plan.validate(now)?;
        limeos_domain::storage_managed_fstab(contract, 10, &snapshot.inventory)?;
        let body = json(&plan)?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            tx.execute("DELETE FROM storage_target_plans WHERE job IS NULL AND expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM storage_target_plans WHERE job IS NULL", [], |r| r.get(0)).map_err(durable)?;
            if count >= 1024 { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO storage_target_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id,principal.id,body,digest,principal.grant_revision,plan.preparation.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedStorageTargets { plan, digest })
    }
    /// Reading a consumed plan supports lost-response replay after expiry.
    /// Reads never inspect hardware, refresh approval or advance a job.
    pub fn storage_target_plan(
        &self,
        principal: &Principal,
        id: &str,
    ) -> Result<PlannedStorageTargets> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(String,String)> = self.conn.query_row(
            "SELECT body,digest FROM storage_target_plans WHERE id=? AND principal=? AND canceled=0",
            params![id,principal.id], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let plan: StorageTargetPlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        plan.validate(plan.preparation.created_at)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(PlannedStorageTargets { plan, digest })
    }
    pub fn approve_storage_targets(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        current: &StorageTargetSnapshot,
        now: i64,
    ) -> Result<limeos_domain::PlanApproval> {
        let proposal = self.storage_target_plan(principal, id)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        proposal.plan.preparation.check_current(current, now)?;
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            if tx.execute("UPDATE storage_target_plans SET approval_digest=? WHERE id=? AND principal=? AND canceled=0 AND job IS NULL",params![limeos_identity::digest(&token),id,principal.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanApproved { plan:id.into() },now)
        })?;
        Ok(limeos_domain::PlanApproval {
            token,
            expires_at: proposal.plan.preparation.expires_at,
        })
    }
    pub fn replay_storage_targets(
        &self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStorageTargets,
        approval: &str,
    ) -> Result<Option<StorageTargetJob>> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if self.storage_target_plan(principal, &proposal.plan.id)? != *proposal {
            return Err(Error(ErrorCode::Conflict));
        }
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id,intent FROM jobs WHERE principal=? AND idempotency=?",
                params![principal.id, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let Some((id, intent)) = row else {
            return Ok(None);
        };
        let consumed: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM storage_target_plans WHERE id=? AND job=? AND approval_digest=?)",params![proposal.plan.id,id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
        if parse::<Intent>(&intent)?
            != (Intent::StoragePrepareTargets {
                plan: Box::new(proposal.plan.clone()),
            })
            || !consumed
        {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Some(self.storage_target_job(principal, &id)?))
    }
    pub fn queue_storage_targets(
        &mut self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStorageTargets,
        approval: &str,
        current: &StorageTargetSnapshot,
        now: i64,
    ) -> Result<StorageTargetJob> {
        if let Some(job) = self.replay_storage_targets(principal, key, proposal, approval)? {
            return Ok(job);
        }
        proposal.plan.preparation.check_current(current, now)?;
        let body = json(&Intent::StoragePrepareTargets {
            plan: Box::new(proposal.plan.clone()),
        })?;
        let id = &proposal.plan.preparation.action;
        let generation = self.generation;
        self.write(|tx| {
            let consumed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM storage_target_plans WHERE id=? AND principal=? AND approval_digest=? AND canceled=0 AND job IS NULL)",params![proposal.plan.id,principal.id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
            if !consumed { return Err(Error(ErrorCode::Forbidden)); }
            tx.execute("INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)",params![id,principal.id,key,body,limeos_identity::digest(&body),"storage:configuration",generation,principal.grant_revision,proposal.plan.preparation.expires_at]).map_err(durable)?;
            for resource in proposal.plan.preparation.resources() {
                if resource != "storage:configuration" {
                    tx.execute("INSERT INTO job_resources VALUES(?,?)",params![id,resource]).map_err(durable)?;
                }
            }
            if tx.execute("UPDATE storage_target_plans SET job=? WHERE id=? AND job IS NULL",params![id,proposal.plan.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),Some(id),EventKind::JobQueued,now)
        })?;
        self.storage_target_job(principal, id)
    }
    pub fn storage_target_job(&self, principal: &Principal, id: &str) -> Result<StorageTargetJob> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT intent,state FROM jobs WHERE id=? AND principal=?",
                params![id, principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let (intent, state) = row.ok_or(Error(ErrorCode::NotFound))?;
        let Intent::StoragePrepareTargets { plan } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::InvalidInput));
        };
        Ok(StorageTargetJob {
            id: id.into(),
            state: parse(&json(&state)?)?,
            plan: *plan,
        })
    }
    pub fn claim_storage_targets(
        &mut self,
        principal: &Principal,
        id: &str,
        current: &StorageTargetSnapshot,
        now: i64,
    ) -> Result<StorageTargetJob> {
        let mut job = self.storage_target_job(principal, id)?;
        if job.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        job.plan.preparation.check_current(current, now)?;
        let generation = self.generation;
        self.write(|tx| {
            if tx.execute("UPDATE jobs SET state='running',generation=? WHERE id=? AND state='queued' AND deadline>? AND revision=(SELECT grant_revision FROM users WHERE id=jobs.principal)",params![generation,id,now]).map_err(|_| Error(ErrorCode::Conflict))? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),Some(id),EventKind::JobTransition { state:JobState::Running },now)
        })?;
        job.state = JobState::Running;
        Ok(job)
    }
    pub fn cancel_storage_targets(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        self.storage_target_plan(principal, id)?;
        self.write(|tx| {
            if tx.execute("UPDATE storage_target_plans SET canceled=1,approval_digest=NULL WHERE id=? AND job IS NULL AND canceled=0",[id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanCanceled { plan:id.into() },now)
        })
    }
    pub fn cancel_storage_target_job(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        let job = self.storage_target_job(principal, id)?;
        match job.state {
            JobState::Canceled => Ok(()),
            JobState::Queued => self.transition(id, JobState::Queued, JobState::Canceled, now),
            _ => Err(Error(ErrorCode::Conflict)),
        }
    }
    // Scheduling hint only. The claim transaction remains the authority for locks.
    pub fn storage_target_resources_available(&self, id: &str) -> Result<bool> {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs j WHERE j.id=? AND j.state='queued' AND NOT EXISTS(SELECT 1 FROM job_resources r JOIN resource_locks l ON l.resource=r.resource WHERE r.job=j.id))", [id], |r| r.get(0)).map_err(durable)
    }
    pub fn storage_target_candidates(&self) -> Result<Vec<(Principal, StorageTargetJob)>> {
        let mut stmt = self.conn.prepare("SELECT j.id,j.intent,j.state,u.id,u.role,u.grant_revision FROM jobs j JOIN users u ON u.id=j.principal WHERE j.state IN ('queued','running','verifying','outcome_unknown','needs_intervention') AND json_extract(j.intent,'$.operation')='storage_prepare_targets' ORDER BY CASE j.state WHEN 'queued' THEN 1 ELSE 0 END,j.rowid LIMIT 32").map_err(durable)?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(durable)?
        .map(|row| {
            let (id, intent, state, owner, role, revision) = row.map_err(durable)?;
            let Intent::StoragePrepareTargets { plan } = parse::<Intent>(&intent)? else {
                return Err(Error(ErrorCode::StateNotDurable));
            };
            Ok((
                Principal {
                    id: owner,
                    role: parse(&role)?,
                    grant_revision: revision,
                },
                StorageTargetJob {
                    id,
                    plan: *plan,
                    state: parse(&json(&state)?)?,
                },
            ))
        })
        .collect()
    }
    pub fn record_storage_target_result(
        &mut self,
        job: &StorageTargetJob,
        receipt: &TargetPreparationReceipt,
        after: Option<&StorageTargetSnapshot>,
        now: i64,
    ) -> Result<JobState> {
        if receipt.plan != job.plan.preparation
            || receipt.plan.action != job.id
            || receipt.digest != limeos_identity::digest(&json(&job.plan.preparation)?)
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let next = match receipt.state {
            PreparationState::Verified if after.is_some() => {
                limeos_domain::verify_target_preparation(
                    &job.plan.preparation,
                    receipt,
                    after.unwrap(),
                )?;
                JobState::Succeeded
            }
            PreparationState::PreconditionChanged => JobState::PreconditionChanged,
            _ => JobState::NeedsIntervention,
        };
        self.write(|tx| {
            let row: Option<(String,String)> = tx.query_row("SELECT intent,state FROM jobs WHERE id=? AND principal=?",params![job.id,job.plan.principal],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
            let (intent,state) = row.ok_or(Error(ErrorCode::NotFound))?;
            if parse::<Intent>(&intent)? != (Intent::StoragePrepareTargets { plan:Box::new(job.plan.clone()) }) { return Err(Error(ErrorCode::Conflict)); }
            let body = json(receipt)?;
            let verification = after.map(json).transpose()?;
            let old: Option<(String,Option<String>)> = tx.query_row("SELECT receipt,verification FROM storage_target_results WHERE job=?",[&job.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
            if state == next.as_str() && old.as_ref() == Some(&(body.clone(),verification.clone())) { return Ok(()); }
            if !matches!(state.as_str(),"running"|"verifying"|"outcome_unknown"|"needs_intervention") { return Err(Error(ErrorCode::Conflict)); }
            tx.execute("INSERT INTO storage_target_results VALUES(?,?,?,?) ON CONFLICT(job) DO UPDATE SET receipt=excluded.receipt,verification=excluded.verification,recorded=excluded.recorded",params![job.id,body,verification,now]).map_err(durable)?;
            if state != next.as_str() {
                tx.execute("UPDATE jobs SET state=? WHERE id=?",params![next.as_str(),job.id]).map_err(durable)?;
                event(tx,Some(&job.plan.principal),Some(&job.id),EventKind::JobTransition { state:next },now)?;
            }
            Ok(())
        })?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
