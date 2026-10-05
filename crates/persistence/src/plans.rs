use super::*;
use limeos_domain::{ContainerSnapshot, PlanApproval, PlannedRestart, RestartJob, RestartPlan};

const PLAN_LIMIT: i64 = 8192;

impl Store {
    pub fn plan_restart(
        &mut self,
        principal: &Principal,
        expected: &ContainerSnapshot,
        now: i64,
    ) -> Result<PlannedRestart> {
        expected.validate()?;
        self.authorize_restart(principal, expected)?;
        let plan = RestartPlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::RESTART_VERSION,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
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
            tx.execute("DELETE FROM restart_plans WHERE job IS NULL AND expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM restart_plans WHERE job IS NULL", [], |r| r.get(0)).map_err(durable)?;
            if count >= PLAN_LIMIT { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO restart_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id, principal.id, body, digest, principal.grant_revision, plan.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedRestart { plan, digest })
    }

    fn authorize_restart(&self, principal: &Principal, expected: &ContainerSnapshot) -> Result<()> {
        limeos_policy::authorize(principal, &self.grants(principal)?, &expected.scope())
    }

    pub fn restart_plan(&self, principal: &Principal, id: &str) -> Result<PlannedRestart> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT body,digest FROM restart_plans WHERE id=? AND principal=? AND canceled=0",
                params![id, principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        let plan: RestartPlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        self.authorize_restart(principal, &plan.expected)?;
        Ok(PlannedRestart { plan, digest })
    }

    /// The composition root supplies a principal resolved from a human session,
    /// never from a model's claimed actor or a task-token proposal.
    pub fn approve_restart(
        &mut self,
        principal: &Principal,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<PlanApproval> {
        let proposal = self.restart_plan(principal, id)?;
        proposal.plan.validate(now)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let token_digest = limeos_identity::digest(&token);
        self.write(|tx| {
            if tx.execute("UPDATE restart_plans SET approval_digest=? WHERE id=? AND canceled=0 AND job IS NULL", params![token_digest, id]).map_err(durable)? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), None, EventKind::PlanApproved { plan: id.into() }, now)
        })?;
        Ok(PlanApproval {
            token,
            expires_at: proposal.plan.expires_at,
        })
    }

    pub fn queue_restart(
        &mut self,
        principal: &Principal,
        key: &str,
        proposal: &PlannedRestart,
        approval: &str,
        current: &ContainerSnapshot,
        now: i64,
    ) -> Result<RestartJob> {
        if !limeos_domain::identifier(key) || !limeos_domain::opaque_id(approval) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let stored = self.restart_plan(principal, &proposal.plan.id)?;
        // Comparing the full normalized body also rejects caller-supplied edits
        // that retain an earlier digest or substitute another resource.
        if stored != *proposal {
            return Err(Error(ErrorCode::Conflict));
        }
        let encoded = json(&Intent::ContainerRestart {
            plan: stored.plan.clone(),
        })?;
        let intent_digest = limeos_identity::digest(&encoded);
        let approval_digest = limeos_identity::digest(approval);
        let generation = self.generation;
        let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let original = self.write(|tx| {
            let (accepted, consumed): (Option<String>, Option<String>) = tx
                .query_row(
                    "SELECT approval_digest,job FROM restart_plans WHERE id=? AND canceled=0",
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
            if tx
                .execute(
                    "UPDATE restart_plans SET job=? WHERE id=? AND job IS NULL",
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
        self.restart_job(principal, &original)
    }

    pub fn restart_job(&self, principal: &Principal, id: &str) -> Result<RestartJob> {
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
        let Intent::ContainerRestart { plan } = parse(&intent)? else {
            return Err(Error(ErrorCode::InvalidInput));
        };
        self.authorize_restart(principal, &plan.expected)?;
        Ok(RestartJob {
            id: id.into(),
            plan,
            state: parse(&json(&state)?)?,
        })
    }

    /// Durable dispatch intent precedes IPC. The executor must inspect again.
    pub fn claim_restart(
        &mut self,
        principal: &Principal,
        id: &str,
        current: &ContainerSnapshot,
        now: i64,
    ) -> Result<RestartJob> {
        let mut job = self.restart_job(principal, id)?;
        if job.plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        job.plan.check_current(current, now)?;
        let generation = self.generation;
        self.write(|tx| {
            // The unique partial resource index is the durable per-resource lock.
            if tx.execute("UPDATE jobs SET state='running',generation=? WHERE id=? AND state='queued' AND deadline>? AND revision=(SELECT grant_revision FROM users WHERE id=jobs.principal)", params![generation, id, now]).map_err(|_| Error(ErrorCode::Conflict))? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), Some(id), EventKind::JobTransition { state: JobState::Running }, now)
        })?;
        job.state = JobState::Running;
        Ok(job)
    }

    pub fn cancel_restart_plan(&mut self, principal: &Principal, id: &str, now: i64) -> Result<()> {
        self.restart_plan(principal, id)?;
        self.write(|tx| {
            if tx.execute("UPDATE restart_plans SET canceled=1,approval_digest=NULL WHERE id=? AND job IS NULL AND canceled=0", [id]).map_err(durable)? != 1 {
                return Err(Error(ErrorCode::Conflict));
            }
            event(tx, Some(&principal.id), None, EventKind::PlanCanceled { plan: id.into() }, now)
        })
    }

    pub fn cancel_restart_job(&mut self, principal: &Principal, id: &str, now: i64) -> Result<()> {
        let job = self.restart_job(principal, id)?;
        match job.state {
            JobState::Canceled => Ok(()),
            JobState::Queued => self.transition(id, JobState::Queued, JobState::Canceled, now),
            // Once IPC might have dispatched, cancellation cannot prove no effect.
            _ => Err(Error(ErrorCode::Conflict)),
        }
    }

    pub fn restart_events(
        &self,
        principal: &Principal,
        id: &str,
        after: i64,
    ) -> Result<Vec<limeos_domain::Event>> {
        self.restart_job(principal, id)?;
        if after < 0 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut stmt = self.conn.prepare("SELECT cursor,kind,created FROM events WHERE job=? AND principal=? AND cursor>? ORDER BY cursor LIMIT 128").map_err(durable)?;
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
