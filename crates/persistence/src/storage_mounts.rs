use super::*;
use limeos_domain::{
    MountExecutionState, PlannedStorageMount, StorageMountEffectPlan, StorageMountInspection,
    StorageMountJob, StorageMountPlan, StorageMountReceipt,
};
const PLAN_BYTES: usize = 24 * 1024;

impl Store {
    pub fn plan_storage_mount(
        &mut self,
        principal: &Principal,
        snapshot: &StorageMountInspection,
        now: i64,
    ) -> Result<PlannedStorageMount> {
        self.authorize_storage(principal)?;
        snapshot.validate(now)?;
        let plan = StorageMountPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::STORAGE_MOUNT_VERSION,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            effect: StorageMountEffectPlan {
                version: 1,
                operation: limeos_domain::STORAGE_MOUNT_OPERATION.into(),
                action: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
                device: snapshot.device.clone(),
                expected: snapshot.expected.clone(),
                target: snapshot.target.clone(),
                physical_devices: snapshot.physical_devices.clone(),
                consumer_digest: snapshot.consumer_digest.clone(),
                created_at: now,
                expires_at: now
                    .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                    .ok_or(Error(ErrorCode::InvalidInput))?,
            },
        };
        plan.validate(now)?;
        let body = json(&plan)?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            tx.execute("DELETE FROM storage_mount_plans WHERE job IS NULL AND expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM storage_mount_plans WHERE job IS NULL", [], |r| r.get(0)).map_err(durable)?;
            if count >= 1024 { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO storage_mount_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id,principal.id,body,digest,principal.grant_revision,plan.effect.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedStorageMount { plan, digest })
    }
    /// Reading a consumed plan supports lost-response replay after expiry.
    /// Reads never inspect hardware, refresh approval or advance a job.
    pub fn storage_mount_plan(
        &self,
        principal: &Principal,
        id: &str,
    ) -> Result<PlannedStorageMount> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(String,String)> = self.conn.query_row(
            "SELECT body,digest FROM storage_mount_plans WHERE id=? AND principal=? AND canceled=0",
            params![id,principal.id], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let plan: StorageMountPlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        plan.validate(plan.effect.created_at)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(PlannedStorageMount { plan, digest })
    }
    pub fn approve_storage_mount(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        current: &StorageMountInspection,
        now: i64,
    ) -> Result<limeos_domain::PlanApproval> {
        let proposal = self.storage_mount_plan(principal, id)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        proposal.plan.effect.check_current(current, now)?;
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            if tx.execute("UPDATE storage_mount_plans SET approval_digest=? WHERE id=? AND principal=? AND canceled=0 AND job IS NULL",params![limeos_identity::digest(&token),id,principal.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanApproved { plan:id.into() },now)
        })?;
        Ok(limeos_domain::PlanApproval {
            token,
            expires_at: proposal.plan.effect.expires_at,
        })
    }
    pub fn replay_storage_mount(
        &self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStorageMount,
        approval: &str,
    ) -> Result<Option<StorageMountJob>> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if self.storage_mount_plan(principal, &proposal.plan.id)? != *proposal {
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
        let consumed: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM storage_mount_plans WHERE id=? AND job=? AND approval_digest=?)",params![proposal.plan.id,id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
        if parse::<Intent>(&intent)?
            != (Intent::StorageMount {
                plan: Box::new(proposal.plan.clone()),
            })
            || !consumed
        {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Some(self.storage_mount_job(principal, &id)?))
    }
    pub fn queue_storage_mount(
        &mut self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStorageMount,
        approval: &str,
        current: &StorageMountInspection,
        now: i64,
    ) -> Result<StorageMountJob> {
        if let Some(job) = self.replay_storage_mount(principal, key, proposal, approval)? {
            return Ok(job);
        }
        proposal.plan.effect.check_current(current, now)?;
        let body = json(&Intent::StorageMount {
            plan: Box::new(proposal.plan.clone()),
        })?;
        let id = &proposal.plan.effect.action;
        let generation = self.generation;
        self.write(|tx| {
            let consumed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM storage_mount_plans WHERE id=? AND principal=? AND approval_digest=? AND canceled=0 AND job IS NULL)",params![proposal.plan.id,principal.id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
            if !consumed { return Err(Error(ErrorCode::Forbidden)); }
            tx.execute("INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)",params![id,principal.id,key,body,limeos_identity::digest(&body),"storage:configuration",generation,principal.grant_revision,proposal.plan.effect.expires_at]).map_err(durable)?;
            for resource in proposal.plan.effect.resources() {
                if resource != "storage:configuration" {
                    tx.execute("INSERT INTO job_resources VALUES(?,?)",params![id,resource]).map_err(durable)?;
                }
            }
            if tx.execute("UPDATE storage_mount_plans SET job=? WHERE id=? AND job IS NULL",params![id,proposal.plan.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),Some(id),EventKind::JobQueued,now)
        })?;
        self.storage_mount_job(principal, id)
    }
    pub fn storage_mount_job(&self, principal: &Principal, id: &str) -> Result<StorageMountJob> {
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
        let Intent::StorageMount { plan } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::InvalidInput));
        };
        plan.validate(plan.effect.created_at)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if plan.effect.action != id || plan.principal != principal.id {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(StorageMountJob {
            id: id.into(),
            state: parse(&json(&state)?)?,
            plan: *plan,
        })
    }
    pub fn claim_storage_mount(
        &mut self,
        principal: &Principal,
        id: &str,
        current: &StorageMountInspection,
        now: i64,
    ) -> Result<StorageMountJob> {
        let mut job = self.storage_mount_job(principal, id)?;
        if job.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        job.plan.effect.check_current(current, now)?;
        let generation = self.generation;
        self.write(|tx| {
            if tx.execute("UPDATE jobs SET state='running',generation=? WHERE id=? AND state='queued' AND deadline>? AND revision=(SELECT grant_revision FROM users WHERE id=jobs.principal)",params![generation,id,now]).map_err(|_| Error(ErrorCode::Conflict))? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),Some(id),EventKind::JobTransition { state:JobState::Running },now)
        })?;
        job.state = JobState::Running;
        Ok(job)
    }
    pub fn cancel_storage_mount(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        self.storage_mount_plan(principal, id)?;
        self.write(|tx| {
            if tx.execute("UPDATE storage_mount_plans SET canceled=1,approval_digest=NULL WHERE id=? AND job IS NULL AND canceled=0",[id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanCanceled { plan:id.into() },now)
        })
    }
    pub fn cancel_storage_mount_job(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        let job = self.storage_mount_job(principal, id)?;
        match job.state {
            JobState::Canceled => Ok(()),
            JobState::Queued => self.transition(id, JobState::Queued, JobState::Canceled, now),
            _ => Err(Error(ErrorCode::Conflict)),
        }
    }
    // Scheduling hint only. The claim transaction remains the authority for locks.
    pub fn storage_mount_resources_available(&self, id: &str) -> Result<bool> {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs j WHERE j.id=? AND j.state='queued' AND NOT EXISTS(SELECT 1 FROM job_resources r JOIN resource_locks l ON l.resource=r.resource WHERE r.job=j.id))", [id], |r| r.get(0)).map_err(durable)
    }
    pub fn storage_mount_candidates(&self) -> Result<Vec<(Principal, StorageMountJob)>> {
        let mut stmt = self.conn.prepare("SELECT j.id,j.intent,j.state,u.id,u.role,u.grant_revision FROM jobs j JOIN users u ON u.id=j.principal WHERE j.state IN ('queued','running','verifying','outcome_unknown','needs_intervention') AND json_extract(j.intent,'$.operation')='storage_mount' ORDER BY CASE j.state WHEN 'queued' THEN 1 ELSE 0 END,j.rowid LIMIT 32").map_err(durable)?;
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
            let Intent::StorageMount { plan } = parse::<Intent>(&intent)? else {
                return Err(Error(ErrorCode::StateNotDurable));
            };
            Ok((
                Principal {
                    id: owner,
                    role: parse(&role)?,
                    grant_revision: revision,
                },
                StorageMountJob {
                    id,
                    plan: *plan,
                    state: parse(&json(&state)?)?,
                },
            ))
        })
        .collect()
    }
    pub fn storage_mount_jobs(&self, principal: &Principal) -> Result<Vec<StorageMountJob>> {
        self.authorize_storage(principal)?;
        let mut statement = self.conn.prepare("SELECT id FROM jobs WHERE principal=? AND json_extract(intent,'$.operation')='storage_mount' ORDER BY rowid DESC LIMIT 32").map_err(durable)?;
        let ids = statement
            .query_map([&principal.id], |row| row.get::<_, String>(0))
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        ids.iter()
            .map(|id| self.storage_mount_job(principal, id))
            .collect()
    }
    pub fn record_storage_mount_result(
        &mut self,
        job: &StorageMountJob,
        receipt: &StorageMountReceipt,
        after: Option<&limeos_domain::StorageMountedSnapshot>,
        unmounted: Option<&StorageMountInspection>,
        now: i64,
    ) -> Result<JobState> {
        receipt.validate()?;
        if receipt.plan != job.plan.effect
            || receipt.plan.action != job.id
            || receipt.digest != limeos_identity::digest(&json(&job.plan.effect)?)
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let next = match receipt.state {
            MountExecutionState::Verified if after.is_some() => {
                limeos_domain::verify_storage_mount(&job.plan.effect, receipt, after.unwrap())?;
                JobState::Succeeded
            }
            MountExecutionState::PreconditionChanged if unmounted.is_some() => {
                job.plan
                    .effect
                    .check_reconciliation(unmounted.unwrap(), now)?;
                JobState::PreconditionChanged
            }
            _ => JobState::NeedsIntervention,
        };
        self.write(|tx| {
            let row: Option<(String,String)> = tx.query_row("SELECT intent,state FROM jobs WHERE id=? AND principal=?",params![job.id,job.plan.principal],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
            let (intent,state) = row.ok_or(Error(ErrorCode::NotFound))?;
            if parse::<Intent>(&intent)? != (Intent::StorageMount { plan:Box::new(job.plan.clone()) }) { return Err(Error(ErrorCode::Conflict)); }
            let body = json(receipt)?;
            let verification = if after.is_some() || unmounted.is_some() {
                Some(json(&(after, unmounted))?)
            } else {
                None
            };
            let old: Option<(String,Option<String>)> = tx.query_row("SELECT receipt,verification FROM storage_mount_results WHERE job=?",[&job.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
            if state == next.as_str() && old.as_ref() == Some(&(body.clone(),verification.clone())) { return Ok(()); }
            if !matches!(state.as_str(),"running"|"verifying"|"outcome_unknown"|"needs_intervention") { return Err(Error(ErrorCode::Conflict)); }
            tx.execute("INSERT INTO storage_mount_results VALUES(?,?,?,?) ON CONFLICT(job) DO UPDATE SET receipt=excluded.receipt,verification=excluded.verification,recorded=excluded.recorded",params![job.id,body,verification,now]).map_err(durable)?;
            if state != next.as_str() {
                tx.execute("UPDATE jobs SET state=? WHERE id=?",params![next.as_str(),job.id]).map_err(durable)?;
                event(tx,Some(&job.plan.principal),Some(&job.id),EventKind::JobTransition { state:next },now)?;
            }
            Ok(())
        })?;
        Ok(next)
    }
}

/// Check immutable proposal/job/resource/result bindings before startup recovery.
/// Historical timestamps validate past facts without renewing execution authority.
pub(super) fn check(conn: &Connection) -> Result<()> {
    let corrupt = |_| Error(ErrorCode::StateNotDurable);
    let mut statement = conn.prepare("SELECT id,principal,body,digest,revision,expires,canceled,approval_digest,job FROM storage_mount_plans").map_err(durable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(durable)?;
    for row in rows {
        let (id, principal, body, digest, revision, expires, canceled, approval, job) =
            row.map_err(durable)?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let plan: StorageMountPlan = parse(&body)?;
        plan.validate(plan.effect.created_at).map_err(corrupt)?;
        if plan.id != id
            || plan.principal != principal
            || plan.grant_revision != revision
            || plan.effect.expires_at != expires
            || json(&plan)? != body
            || limeos_identity::digest(&body) != digest
            || approval
                .as_ref()
                .is_some_and(|value| !limeos_domain::opaque_id(value))
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if let Some(job) = job {
            let intent: Option<String> = conn
                .query_row("SELECT intent FROM jobs WHERE id=?", [&job], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(durable)?;
            if canceled
                || approval.is_none()
                || plan.effect.action != job
                || intent.as_deref().map(parse::<Intent>).transpose()?
                    != Some(Intent::StorageMount {
                        plan: Box::new(plan),
                    })
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
    }
    let mut statement = conn.prepare("SELECT id,principal,intent,digest,resource,revision,deadline FROM jobs WHERE json_extract(intent,'$.operation')='storage_mount'").map_err(durable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(durable)?;
    for row in rows {
        let (id, principal, intent, digest, resource, revision, deadline) = row.map_err(durable)?;
        let Intent::StorageMount { plan } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        plan.validate(plan.effect.created_at).map_err(corrupt)?;
        let mut required = conn
            .prepare("SELECT resource FROM job_resources WHERE job=? ORDER BY resource")
            .map_err(durable)?;
        let resources = required
            .query_map([&id], |row| row.get::<_, String>(0))
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        let body = json(&plan)?;
        let approved: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM storage_mount_plans WHERE id=? AND principal=? AND body=? AND digest=? AND revision=? AND expires=? AND job=? AND canceled=0 AND approval_digest IS NOT NULL)",
            params![plan.id, principal, body, limeos_identity::digest(&body), revision, deadline, id], |row| row.get(0)).map_err(durable)?;
        if plan.effect.action != id
            || plan.principal != principal
            || plan.grant_revision != revision
            || plan.effect.expires_at != deadline
            || resource != "storage:configuration"
            || digest != limeos_identity::digest(&intent)
            || resources != plan.effect.resources()
            || !approved
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    // Audit from jobs as well as existing results: a deleted terminal receipt
    // must not disappear from an INNER JOIN and silently pass startup. A plan
    // rejected while still queued legitimately has no executor receipt.
    let missing_terminal_proof: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='storage_mount' AND (j.state='succeeded' OR (j.state='precondition_changed' AND EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND json_extract(e.kind,'$.kind')='job_transition' AND json_extract(e.kind,'$.state')='running'))) AND NOT EXISTS(SELECT 1 FROM storage_mount_results r WHERE r.job=j.id))",
        [], |row| row.get(0),
    ).map_err(durable)?;
    if missing_terminal_proof {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut statement = conn.prepare("SELECT r.receipt,r.verification,j.intent,j.state FROM storage_mount_results r JOIN jobs j ON j.id=r.job").map_err(durable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(durable)?;
    for row in rows {
        let (body, proof, intent, state) = row.map_err(durable)?;
        if body.len() > limeos_domain::STORAGE_MOUNT_RECEIPT_BYTES
            || proof.as_ref().is_some_and(|v| v.len() > 64 * 1024)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let receipt: StorageMountReceipt = parse(&body)?;
        receipt.validate().map_err(corrupt)?;
        let Intent::StorageMount { plan } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        if receipt.plan != plan.effect
            || receipt.digest != limeos_identity::digest(&json(&plan.effect)?)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let verification: (
            Option<limeos_domain::StorageMountedSnapshot>,
            Option<StorageMountInspection>,
        ) = proof.as_deref().map(parse).transpose()?.unwrap_or_default();
        match state.as_str() {
            "succeeded" => {
                if receipt.state != MountExecutionState::Verified || verification.1.is_some() {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                limeos_domain::verify_storage_mount(
                    &plan.effect,
                    &receipt,
                    verification
                        .0
                        .as_ref()
                        .ok_or(Error(ErrorCode::StateNotDurable))?,
                )
                .map_err(corrupt)?;
            }
            "precondition_changed" => {
                if receipt.state != MountExecutionState::PreconditionChanged
                    || verification.0.is_some()
                {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                let current = verification
                    .1
                    .as_ref()
                    .ok_or(Error(ErrorCode::StateNotDurable))?;
                plan.effect
                    .check_reconciliation(current, current.observed_at)
                    .map_err(corrupt)?;
            }
            "running" | "verifying" | "outcome_unknown" | "needs_intervention" => {}
            _ => return Err(Error(ErrorCode::StateNotDurable)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
