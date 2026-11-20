//! Whole-stack lifecycle plans, jobs and results. The job table and its schema-7
//! resource set are reused: the primary resource is `stack:<project>` and every
//! member's `container:<id>` is added in the queue transaction, so stack jobs
//! and single-container jobs exclude each other in both directions.
use super::*;
use limeos_domain::{
    ContainerAction, ContainerSnapshot, ExecutionState, PlanApproval, PlannedStackAction, StackJob,
    StackPlan, StackReceipt, StackSnapshot,
};

const PLAN_LIMIT: i64 = 8192;
const STACK_OPERATIONS: &str = "('stack_restart','stack_start','stack_stop')";

impl Store {
    pub fn stack_jobs(&self, principal: &Principal) -> Result<Vec<StackJob>> {
        self.grants(principal)?;
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT id FROM jobs WHERE principal=? AND json_extract(intent,'$.operation') IN {STACK_OPERATIONS} ORDER BY rowid DESC LIMIT 32"))
            .map_err(durable)?;
        let ids = stmt
            .query_map([&principal.id], |r| r.get::<_, String>(0))
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        let mut jobs = Vec::new();
        for id in ids {
            match self.stack_job(principal, &id) {
                Ok(job) => jobs.push(job),
                Err(Error(ErrorCode::Forbidden | ErrorCode::Expired)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(jobs)
    }
    /// Internal worker discovery is bounded. A caller cannot supply an actor.
    pub fn stack_candidates(&self) -> Result<Vec<(Principal, StackJob)>> {
        let mut stmt = self.conn.prepare(&format!("SELECT j.id,j.intent,j.state,u.id,u.role,u.grant_revision FROM jobs j JOIN users u ON u.id=j.principal WHERE j.state IN ('queued','running','verifying','outcome_unknown','needs_intervention') AND json_extract(j.intent,'$.operation') IN {STACK_OPERATIONS} ORDER BY CASE j.state WHEN 'queued' THEN 0 ELSE 1 END,j.rowid LIMIT 64")).map_err(durable)?;
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
        .map(|r| {
            let (id, intent, state, owner, role, revision) = r.map_err(durable)?;
            let plan = parse::<Intent>(&intent)?.stack_plan()?;
            Ok((
                Principal {
                    id: owner,
                    role: parse(&role)?,
                    grant_revision: revision,
                },
                StackJob {
                    id,
                    plan,
                    state: parse(&json(&state)?)?,
                },
            ))
        })
        .collect()
    }

    /// The composition root supplies a receipt from authenticated executor IPC
    /// and fresh member inspections. Commit proof and state together.
    pub fn record_stack_result(
        &mut self,
        job: &StackJob,
        receipt: &StackReceipt,
        afters: Option<&[ContainerSnapshot]>,
        now: i64,
    ) -> Result<JobState> {
        receipt.validate()?;
        if receipt.action != job.id
            || receipt.plan_digest != limeos_identity::digest(&json(&job.plan)?)
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
            || afters.is_some_and(|a| a.len() > limeos_domain::MAX_STACK_MEMBERS)
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let next = match (receipt.state, afters) {
            (ExecutionState::Verified, Some(afters)) => {
                limeos_domain::verify_stack(receipt.operation, &receipt.before, afters)?;
                JobState::Succeeded
            }
            // Releases the locks without claiming success: the sent members
            // are proven and the rest were never sent.
            (ExecutionState::PartlyDone, Some(afters)) => {
                limeos_domain::verify_partly_done(receipt, afters)?;
                JobState::Failed
            }
            (ExecutionState::PreconditionChanged, _) => JobState::PreconditionChanged,
            _ => JobState::NeedsIntervention,
        };
        self.write(|tx| {
            let (state, intent): (String, String) = tx
                .query_row(
                    "SELECT state,intent FROM jobs WHERE id=? AND principal=?",
                    params![job.id, job.plan.principal],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if parse::<Intent>(&intent)? != Intent::stack(job.plan.clone()) {
                return Err(Error(ErrorCode::Conflict));
            }
            let body = json(receipt)?;
            let verification = afters.map(|a| json(&a.to_vec())).transpose()?;
            let existing: Option<(String, Option<String>)> = tx
                .query_row(
                    "SELECT receipt,verification FROM stack_results WHERE job=?",
                    [&job.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?;
            if state == next.as_str()
                && existing.as_ref() == Some(&(body.clone(), verification.clone()))
            {
                return Ok(());
            }
            if !matches!(
                state.as_str(),
                "running" | "verifying" | "outcome_unknown" | "needs_intervention"
            ) {
                return Err(Error(ErrorCode::Conflict));
            }
            tx.execute("INSERT INTO stack_results VALUES(?,?,?,?) ON CONFLICT(job) DO UPDATE SET receipt=excluded.receipt,verification=excluded.verification,recorded=excluded.recorded", params![job.id, body, verification, now]).map_err(durable)?;
            if state == next.as_str() {
                return Ok(());
            }
            tx.execute(
                "UPDATE jobs SET state=? WHERE id=?",
                params![next.as_str(), job.id],
            )
            .map_err(durable)?;
            event(
                tx,
                Some(&job.plan.principal),
                Some(&job.id),
                EventKind::JobTransition { state: next },
                now,
            )
        })?;
        Ok(next)
    }
    /// Nothing was sent: the executor holds no row for this action and
    /// answered that none can ever have been pruned
    /// (`limeos_domain::ReceiptAbsence::NeverRecorded`), every Engine call
    /// is preceded by a durable Prepared row, and the plan had expired when
    /// core asked (`limeos_domain::execution_closed`), so no delayed request
    /// can still write one. Releases the locks.
    pub fn record_stack_unsent(&mut self, job: &StackJob, now: i64) -> Result<()> {
        self.write(|tx| {
            let (state, intent): (String, String) = tx
                .query_row(
                    "SELECT state,intent FROM jobs WHERE id=? AND principal=?",
                    params![job.id, job.plan.principal],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if parse::<Intent>(&intent)? != Intent::stack(job.plan.clone()) {
                return Err(Error(ErrorCode::Conflict));
            }
            if state == JobState::PreconditionChanged.as_str() {
                return Ok(());
            }
            let results: i64 = tx
                .query_row(
                    "SELECT count(*) FROM stack_results WHERE job=?",
                    [&job.id],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if results != 0
                || !matches!(
                    state.as_str(),
                    "running" | "verifying" | "outcome_unknown" | "needs_intervention"
                )
            {
                return Err(Error(ErrorCode::Conflict));
            }
            tx.execute(
                "UPDATE jobs SET state='precondition_changed' WHERE id=?",
                [&job.id],
            )
            .map_err(durable)?;
            event(
                tx,
                Some(&job.plan.principal),
                Some(&job.id),
                EventKind::JobTransition {
                    state: JobState::PreconditionChanged,
                },
                now,
            )
        })
    }
    /// The last recorded per-member evidence for a job, if any.
    #[allow(clippy::type_complexity)]
    pub fn stack_result(
        &self,
        principal: &Principal,
        id: &str,
    ) -> Result<Option<(StackReceipt, Option<Vec<ContainerSnapshot>>)>> {
        self.stack_job(principal, id)?;
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT receipt,verification FROM stack_results WHERE job=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        row.map(|(receipt, verification)| {
            Ok((
                parse::<StackReceipt>(&receipt)?,
                verification.as_deref().map(parse).transpose()?,
            ))
        })
        .transpose()
    }

    pub fn plan_stack(
        &mut self,
        principal: &Principal,
        operation: ContainerAction,
        catalog_stack: &str,
        expected: &StackSnapshot,
        now: i64,
    ) -> Result<PlannedStackAction> {
        expected.validate_for(operation)?;
        self.authorize_stack(principal, catalog_stack, expected)?;
        let plan = StackPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::STACK_VERSION,
            operation,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            catalog_stack: catalog_stack.into(),
            expected: expected.clone(),
            created_at: now,
            expires_at: now
                .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                .ok_or(Error(ErrorCode::InvalidInput))?,
        };
        plan.validate(now)?;
        let body = json(&plan)?;
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            // Unqueued proposals expire. Durable jobs retain their original plans.
            tx.execute("DELETE FROM stack_plans WHERE job IS NULL AND expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM stack_plans WHERE job IS NULL", [], |r| r.get(0)).map_err(durable)?;
            if count >= PLAN_LIMIT { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO stack_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id, principal.id, body, digest, principal.grant_revision, plan.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedStackAction { plan, digest })
    }

    /// A stack grant never reaches a member the principal could not manage,
    /// and the registered catalog stack is authorized as well as the project.
    pub fn authorize_stack(
        &self,
        principal: &Principal,
        catalog_stack: &str,
        expected: &StackSnapshot,
    ) -> Result<()> {
        let grants = self.grants(principal)?;
        expected
            .scopes_for(catalog_stack)
            .iter()
            .try_for_each(|scope| limeos_policy::authorize(principal, &grants, scope))
    }

    pub fn stack_plan(&self, principal: &Principal, id: &str) -> Result<PlannedStackAction> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT body,digest FROM stack_plans WHERE id=? AND principal=? AND canceled=0",
                params![id, principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        let plan: StackPlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        self.authorize_stack(principal, &plan.catalog_stack, &plan.expected)?;
        Ok(PlannedStackAction { plan, digest })
    }

    /// The composition root supplies a principal resolved from a human session.
    pub fn approve_stack(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<PlanApproval> {
        let proposal = self.stack_plan(principal, id)?;
        proposal.plan.validate(now)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let token_digest = limeos_identity::digest(&token);
        self.write(|tx| {
            if tx.execute("UPDATE stack_plans SET approval_digest=? WHERE id=? AND canceled=0 AND job IS NULL", params![token_digest, id]).map_err(durable)? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), None, EventKind::PlanApproved { plan: id.into() }, now)
        })?;
        Ok(PlanApproval {
            token,
            expires_at: proposal.plan.expires_at,
        })
    }

    pub fn queue_stack(
        &mut self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStackAction,
        approval: &str,
        current: &StackSnapshot,
        now: i64,
    ) -> Result<StackJob> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let stored = self.stack_plan(principal, &proposal.plan.id)?;
        // The full normalized body also rejects edits that keep an earlier digest.
        if stored != *proposal {
            return Err(Error(ErrorCode::Conflict));
        }
        let encoded = json(&Intent::stack(stored.plan.clone()))?;
        let intent_digest = limeos_identity::digest(&encoded);
        let approval_digest = limeos_identity::digest(approval);
        let generation = self.generation;
        let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let original = self.write(|tx| {
            let (accepted, consumed): (Option<String>, Option<String>) = tx
                .query_row(
                    "SELECT approval_digest,job FROM stack_plans WHERE id=? AND canceled=0",
                    [&stored.plan.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if !accepted.is_some_and(|d| limeos_identity::constant_eq(&d, &approval_digest)) {
                return Err(Error(ErrorCode::Forbidden));
            }
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT id,digest FROM jobs WHERE principal=? AND idempotency=?",
                    params![principal.id, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?;
            if let Some((job, digest)) = existing {
                return if digest == intent_digest && consumed.as_ref() == Some(&job) {
                    Ok(job)
                } else {
                    Err(Error(ErrorCode::Conflict))
                };
            }
            if consumed.is_some() {
                return Err(Error(ErrorCode::Conflict));
            }
            stored.plan.check_current(current, now)?;
            tx.execute(
                "INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)",
                params![
                    id,
                    principal.id,
                    key,
                    encoded,
                    intent_digest,
                    stored.plan.expected.resource,
                    generation,
                    principal.grant_revision,
                    stored.plan.expires_at
                ],
            )
            .map_err(durable)?;
            // The primary project resource is inserted by trigger; add the
            // catalog stack (if different) and every member.
            for resource in stored.plan.resources().iter().skip(1) {
                tx.execute(
                    "INSERT INTO job_resources VALUES(?,?)",
                    params![id, resource],
                )
                .map_err(durable)?;
            }
            if tx
                .execute(
                    "UPDATE stack_plans SET job=? WHERE id=? AND job IS NULL",
                    params![id, stored.plan.id],
                )
                .map_err(durable)?
                != 1
            {
                return Err(Error(ErrorCode::Conflict));
            }
            event(
                tx,
                Some(&principal.id),
                Some(&id),
                EventKind::JobQueued,
                now,
            )?;
            Ok(id)
        })?;
        self.stack_job(principal, &original)
    }

    /// A lost queue response must remain recoverable even when Engine is down.
    pub fn replay_stack(
        &self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedStackAction,
        approval: &str,
    ) -> Result<Option<StackJob>> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if self.stack_plan(principal, &proposal.plan.id)? != *proposal {
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
        let consumed: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM stack_plans WHERE id=? AND job=? AND approval_digest=?)", params![proposal.plan.id, id, limeos_identity::digest(approval)], |r| r.get(0)).map_err(durable)?;
        if parse::<Intent>(&intent)? != Intent::stack(proposal.plan.clone()) || !consumed {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Some(self.stack_job(principal, &id)?))
    }

    pub fn stack_job(&self, principal: &Principal, id: &str) -> Result<StackJob> {
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
        let plan = parse::<Intent>(&intent)?.stack_plan()?;
        self.authorize_stack(principal, &plan.catalog_stack, &plan.expected)?;
        Ok(StackJob {
            id: id.into(),
            plan,
            state: parse(&json(&state)?)?,
        })
    }

    /// Durable dispatch intent precedes IPC. The executor must list again.
    pub fn claim_stack(
        &mut self,
        principal: &Principal,
        id: &str,
        current: &StackSnapshot,
        now: i64,
    ) -> Result<StackJob> {
        let mut job = self.stack_job(principal, id)?;
        if job.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        job.plan.check_current(current, now)?;
        let generation = self.generation;
        self.write(|tx| {
            // Schema-7 triggers acquire the stack and every member atomically;
            // any held member aborts this update and the job stays queued.
            if tx.execute("UPDATE jobs SET state='running',generation=? WHERE id=? AND state='queued' AND deadline>? AND revision=(SELECT grant_revision FROM users WHERE id=jobs.principal)", params![generation, id, now]).map_err(|_| Error(ErrorCode::Conflict))? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), Some(id), EventKind::JobTransition { state: JobState::Running }, now)
        })?;
        job.state = JobState::Running;
        Ok(job)
    }

    pub fn cancel_stack_plan(&mut self, principal: &Principal, id: &str, now: i64) -> Result<()> {
        self.stack_plan(principal, id)?;
        self.write(|tx| {
            if tx.execute("UPDATE stack_plans SET canceled=1,approval_digest=NULL WHERE id=? AND job IS NULL AND canceled=0", [id]).map_err(durable)? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), None, EventKind::PlanCanceled { plan: id.into() }, now)
        })
    }

    pub fn cancel_stack_job(&mut self, principal: &Principal, id: &str, now: i64) -> Result<()> {
        let job = self.stack_job(principal, id)?;
        match job.state {
            JobState::Canceled => Ok(()),
            JobState::Queued => self.transition(id, JobState::Queued, JobState::Canceled, now),
            // Once IPC might have dispatched, cancellation cannot prove no effect.
            _ => Err(Error(ErrorCode::Conflict)),
        }
    }

    pub fn stack_events(
        &self,
        principal: &Principal,
        id: &str,
        after: i64,
    ) -> Result<Vec<limeos_domain::Event>> {
        self.stack_job(principal, id)?;
        if after < 0 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        // A job whose history retention has begun to remove reads as having
        // none, never as a partial history.
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT cursor,kind,created FROM events WHERE job=? AND principal=? AND cursor>? AND {} ORDER BY cursor LIMIT 128", super::retention::RETAINED_HISTORY))
            .map_err(durable)?;
        let rows = stmt
            .query_map(params![id, principal.id, after], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(durable)?;
        rows.map(|row| {
            let (cursor, kind, created) = row.map_err(durable)?;
            Ok(limeos_domain::Event {
                cursor,
                principal: Some(principal.id.clone()),
                job: Some(id.into()),
                event: parse(&kind)?,
                created,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests;
