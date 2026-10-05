use super::*;
use limeos_domain::{
    PlanApproval, PlannedStorageSetup, StorageInventory, StorageSetupInput, StorageSetupPlan,
};
const PLAN_LIMIT: i64 = 1024;
const PLAN_BYTES: usize = 64 * 1024;

pub fn storage_scope() -> Scope {
    Scope {
        operation: Operation::StorageManage,
        resource: "storage:configuration".into(),
    }
}
impl Store {
    pub fn authorize_storage(&self, principal: &Principal) -> Result<()> {
        limeos_policy::authorize(principal, &self.grants(principal)?, &storage_scope())
    }
    pub fn plan_storage(
        &mut self,
        principal: &Principal,
        input: &StorageSetupInput,
        inventory: &StorageInventory,
        now: i64,
    ) -> Result<PlannedStorageSetup> {
        self.authorize_storage(principal)?;
        input.validate()?;
        inventory.validate()?;
        if input.inventory_digest != limeos_identity::digest(&json(inventory)?) {
            return Err(Error(ErrorCode::Conflict));
        }
        let plan = StorageSetupPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::STORAGE_PLAN_VERSION,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            expected: inventory.clone(),
            desired: input.contract.clone(),
            readiness: input.contract.mount_wait_plan(input.timeout_seconds)?,
            managed_fstab: limeos_domain::storage_managed_fstab(
                &input.contract,
                input.timeout_seconds,
                inventory,
            )?,
            created_at: now,
            expires_at: now
                .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                .ok_or(Error(ErrorCode::InvalidInput))?,
        };
        plan.validate(now)?;
        let body = json(&plan)?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            tx.execute("DELETE FROM storage_plans WHERE expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM storage_plans", [], |r| r.get(0)).map_err(durable)?;
            if count >= PLAN_LIMIT { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO storage_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id,principal.id,body,digest,principal.grant_revision,plan.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedStorageSetup { plan, digest })
    }
    /// Read-only durable review; unavailable/changed hardware never prevents an
    /// operator from reading or withdrawing the stored proposal.
    pub fn storage_plan(
        &self,
        principal: &Principal,
        id: &str,
        now: i64,
    ) -> Result<PlannedStorageSetup> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.authorize_storage(principal)?;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT body,digest FROM storage_plans WHERE id=? AND principal=? AND canceled=0",
                params![id, principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        if body.len() > PLAN_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let plan: StorageSetupPlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        plan.validate(now)?;
        Ok(PlannedStorageSetup { plan, digest })
    }
    /// Human review only, deliberately no executable storage intent or queue.
    /// Mount execution will require a new version and fresh human approval.
    pub fn approve_storage(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        inventory: &StorageInventory,
        now: i64,
    ) -> Result<PlanApproval> {
        let proposal = self.storage_plan(principal, id, now)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        proposal.plan.check_current(inventory, now)?;
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let hashed = limeos_identity::digest(&token);
        self.write(|tx| {
            if tx.execute("UPDATE storage_plans SET approval_digest=? WHERE id=? AND principal=? AND canceled=0", params![hashed,id,principal.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx, Some(&principal.id), None, EventKind::PlanApproved { plan: id.into() }, now)
        })?;
        Ok(PlanApproval {
            token,
            expires_at: proposal.plan.expires_at,
        })
    }
    pub fn cancel_storage(&mut self, principal: &Principal, id: &str, now: i64) -> Result<()> {
        self.storage_plan(principal, id, now)?;
        self.write(|tx| {
            tx.execute("UPDATE storage_plans SET canceled=1,approval_digest=NULL WHERE id=? AND principal=?", params![id,principal.id]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCanceled { plan: id.into() }, now)
        })
    }
}
#[cfg(test)]
mod tests;
