use super::*;
use limeos_domain::{
    BACKUP_PREPARATION_RESOURCE, BackupPreparationBinding, BackupPreparationJob,
    BackupPreparationJobPlan, PlanApproval, PlannedBackupPreparation,
};
const PLAN_BYTES: usize = 16 * 1024;

impl Store {
    /// Trusted coordinator facts only; this API performs no archive or path I/O.
    pub fn plan_backup_preparation(
        &mut self,
        principal: &Principal,
        binding: &BackupPreparationBinding,
        now: i64,
    ) -> Result<PlannedBackupPreparation> {
        self.authorize_storage(principal)?;
        binding.validate()?;
        let plan = BackupPreparationJobPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::BACKUP_PREPARATION_VERSION,
            action: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            created_at: now,
            expires_at: now
                .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                .ok_or(Error(ErrorCode::InvalidInput))?,
            binding: binding.clone(),
        };
        plan.validate(now)?;
        let body = json(&plan)?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            tx.execute("DELETE FROM backup_preparation_plans WHERE job IS NULL AND expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM backup_preparation_plans WHERE job IS NULL", [], |r| r.get(0)).map_err(durable)?;
            if count >= 1024 { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO backup_preparation_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id, principal.id, body, digest, principal.grant_revision, plan.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedBackupPreparation { plan, digest })
    }
    /// Current authorization governs historical reads; saved revisions stay historical.
    pub fn backup_preparation_plan(
        &self,
        principal: &Principal,
        id: &str,
    ) -> Result<PlannedBackupPreparation> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(Option<String>, String)> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(body AS BLOB))<=16384 THEN body ELSE NULL END,digest FROM backup_preparation_plans WHERE id=? AND principal=? AND canceled=0",
            params![id,principal.id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        let body = body.ok_or(Error(ErrorCode::StateNotDurable))?;
        let plan = checked_plan(&body, &digest)?;
        if plan.id != id || plan.principal != principal.id {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(PlannedBackupPreparation { plan, digest })
    }
    pub fn approve_backup_preparation(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        current: &BackupPreparationBinding,
        now: i64,
    ) -> Result<PlanApproval> {
        let proposal = self.backup_preparation_plan(principal, id)?;
        if proposal.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        proposal.plan.check_current(current, now)?;
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            if tx.execute("UPDATE backup_preparation_plans SET approval_digest=? WHERE id=? AND principal=? AND canceled=0 AND job IS NULL", params![limeos_identity::digest(&token), id, principal.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanApproved { plan:id.into() },now)
        })?;
        Ok(PlanApproval {
            token,
            expires_at: proposal.plan.expires_at,
        })
    }
    /// Exact consumed replay is a pure database read, including after expiry.
    pub fn replay_backup_preparation(
        &self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedBackupPreparation,
        approval: &str,
    ) -> Result<Option<BackupPreparationJob>> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        proposal.plan.validate(proposal.plan.created_at)?;
        if self.backup_preparation_plan(principal, &proposal.plan.id)? != *proposal {
            return Err(Error(ErrorCode::Conflict));
        }
        let row: Option<(String, Option<String>)> = self.conn.query_row(
            "SELECT id,CASE WHEN length(CAST(intent AS BLOB))<=17000 THEN intent ELSE NULL END FROM jobs WHERE principal=? AND idempotency=?",
            params![principal.id,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
        let Some((id, intent)) = row else {
            return Ok(None);
        };
        let intent = intent.ok_or(Error(ErrorCode::StateNotDurable))?;
        let consumed: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM backup_preparation_plans WHERE id=? AND job=? AND approval_digest=? AND consumed_approval_digest=approval_digest)",params![proposal.plan.id,id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
        if parse::<Intent>(&intent)?
            != (Intent::BackupPreparePrivate {
                plan: Box::new(proposal.plan.clone()),
            })
            || !consumed
        {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Some(self.backup_preparation_job(principal, &id)?))
    }
    pub fn queue_backup_preparation(
        &mut self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedBackupPreparation,
        approval: &str,
        current: &BackupPreparationBinding,
        now: i64,
    ) -> Result<BackupPreparationJob> {
        if let Some(job) = self.replay_backup_preparation(principal, key, proposal, approval)? {
            return Ok(job);
        }
        if proposal.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        proposal.plan.check_current(current, now)?;
        let body = json(&Intent::BackupPreparePrivate {
            plan: Box::new(proposal.plan.clone()),
        })?;
        let id = &proposal.plan.action;
        let generation = self.generation;
        self.write(|tx| {
            let approved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM backup_preparation_plans WHERE id=? AND principal=? AND approval_digest=? AND canceled=0 AND job IS NULL)",params![proposal.plan.id,principal.id,limeos_identity::digest(approval)],|r|r.get(0)).map_err(durable)?;
            if !approved { return Err(Error(ErrorCode::Forbidden)); }
            tx.execute("INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)", params![id,principal.id,key,body,limeos_identity::digest(&body),BACKUP_PREPARATION_RESOURCE,generation,principal.grant_revision,proposal.plan.expires_at]).map_err(durable)?;
            for resource in proposal.plan.binding.resources() {
                if resource != BACKUP_PREPARATION_RESOURCE {
                    tx.execute("INSERT INTO job_resources VALUES(?,?)",params![id,resource]).map_err(durable)?;
                }
            }
            if tx.execute("UPDATE backup_preparation_plans SET job=?,consumed_approval_digest=approval_digest WHERE id=? AND job IS NULL",params![id,proposal.plan.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),Some(id),EventKind::JobQueued,now)
        })?;
        self.backup_preparation_job(principal, id)
    }
    pub fn backup_preparation_job(
        &self,
        principal: &Principal,
        id: &str,
    ) -> Result<BackupPreparationJob> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(Option<String>,String)> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(intent AS BLOB))<=17000 THEN intent ELSE NULL END,state FROM jobs WHERE id=? AND principal=?",
            params![id,principal.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
        let (intent, state) = row.ok_or(Error(ErrorCode::NotFound))?;
        let Intent::BackupPreparePrivate { plan } =
            parse(&intent.ok_or(Error(ErrorCode::StateNotDurable))?)?
        else {
            return Err(Error(ErrorCode::InvalidInput));
        };
        let saved = self.backup_preparation_plan(principal, &plan.id)?;
        if saved.plan != *plan
            || plan.action != id
            || !matches!(state.as_str(), "queued" | "canceled")
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(BackupPreparationJob {
            id: id.into(),
            state: parse(&json(&state)?)?,
            plan: *plan,
        })
    }
    pub fn cancel_backup_preparation(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        self.backup_preparation_plan(principal, id)?;
        self.write(|tx| {
            if tx.execute("UPDATE backup_preparation_plans SET canceled=1,approval_digest=NULL WHERE id=? AND job IS NULL AND canceled=0",[id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal.id),None,EventKind::PlanCanceled { plan:id.into() },now)
        })
    }
    pub fn cancel_backup_preparation_job(
        &mut self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<()> {
        self.backup_preparation_job(principal, id)?;
        self.transition(id, JobState::Queued, JobState::Canceled, now)
    }
}

fn checked_plan(body: &str, digest: &str) -> Result<BackupPreparationJobPlan> {
    if body.len() > PLAN_BYTES || !limeos_domain::opaque_id(digest) {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let plan: BackupPreparationJobPlan = parse(body)?;
    plan.validate(plan.created_at)
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if json(&plan)? != body || limeos_identity::digest(body) != digest {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(plan)
}

/// Before generation/recovery: validate every saved plan and every backup job.
pub(super) fn check(conn: &Connection) -> Result<()> {
    let invalid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM backup_preparation_plans WHERE length(CAST(body AS BLOB)) NOT BETWEEN 1 AND 16384 OR length(CAST(id AS BLOB))!=64 OR length(CAST(principal AS BLOB)) NOT BETWEEN 1 AND 128 OR length(CAST(digest AS BLOB))!=64 OR (approval_digest IS NOT NULL AND length(CAST(approval_digest AS BLOB))!=64) OR (consumed_approval_digest IS NOT NULL AND length(CAST(consumed_approval_digest AS BLOB))!=64) OR (job IS NOT NULL AND length(CAST(job AS BLOB))!=64)) OR EXISTS(SELECT 1 FROM jobs WHERE json_extract(intent,'$.operation')='backup_prepare_private' AND (length(CAST(intent AS BLOB))>17000 OR state NOT IN ('queued','canceled'))) OR EXISTS(SELECT 1 FROM job_resources r JOIN jobs j ON j.id=r.job WHERE json_extract(j.intent,'$.operation')='backup_prepare_private' AND length(CAST(r.resource AS BLOB)) NOT BETWEEN 1 AND 768)",[],|r|r.get(0)).map_err(durable)?;
    if invalid {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut plans = conn.prepare("SELECT id,principal,body,digest,revision,expires,canceled,approval_digest,consumed_approval_digest,job FROM backup_preparation_plans").map_err(durable)?;
    let mut rows = plans.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let id: String = row.get(0).map_err(durable)?;
        let principal: String = row.get(1).map_err(durable)?;
        let body: String = row.get(2).map_err(durable)?;
        let digest: String = row.get(3).map_err(durable)?;
        let revision: i64 = row.get(4).map_err(durable)?;
        let expires: i64 = row.get(5).map_err(durable)?;
        let canceled: i64 = row.get(6).map_err(durable)?;
        let approval: Option<String> = row.get(7).map_err(durable)?;
        let consumed: Option<String> = row.get(8).map_err(durable)?;
        let job: Option<String> = row.get(9).map_err(durable)?;
        let plan = checked_plan(&body, &digest)?;
        if plan.id != id
            || plan.principal != principal
            || plan.grant_revision != revision
            || plan.expires_at != expires
            || !matches!(canceled, 0 | 1)
            || approval
                .as_ref()
                .is_some_and(|s| !limeos_domain::opaque_id(s))
            || consumed
                .as_ref()
                .is_some_and(|s| !limeos_domain::opaque_id(s))
            || (job.is_none() && consumed.is_some())
            || (canceled == 1 && approval.is_some())
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if let Some(job) = job {
            if job != plan.action || canceled != 0 || approval.is_none() || consumed != approval {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            let valid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND principal=? AND intent=? AND digest=? AND resource=? AND revision=? AND deadline=? AND state IN ('queued','canceled'))",params![job,principal,json(&Intent::BackupPreparePrivate{plan:Box::new(plan.clone())})?,limeos_identity::digest(&json(&Intent::BackupPreparePrivate{plan:Box::new(plan.clone())})?),BACKUP_PREPARATION_RESOURCE,revision,expires],|r|r.get(0)).map_err(durable)?;
            let mut required = conn
                .prepare("SELECT resource FROM job_resources WHERE job=? ORDER BY resource")
                .map_err(durable)?;
            let resources = required
                .query_map([&job], |r| r.get::<_, String>(0))
                .map_err(durable)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(durable)?;
            if !valid || resources != plan.binding.resources() {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
    }
    let unlinked: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='backup_prepare_private' AND NOT EXISTS(SELECT 1 FROM backup_preparation_plans p WHERE p.job=j.id))",[],|r|r.get(0)).map_err(durable)?;
    if unlinked {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
