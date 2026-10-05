use super::*;

impl Store {
    pub fn queue_job(
        &mut self,
        principal: &Principal,
        key: &str,
        intent: &Intent,
        deadline: i64,
        now: i64,
    ) -> Result<String> {
        // Effectful intent must go through atomic plan-bound approval consumption.
        if !matches!(intent, Intent::HealthProbe { .. }) {
            return Err(Error(ErrorCode::Forbidden));
        }
        if !limeos_domain::identifier(key)
            || !limeos_domain::identifier(intent.resource())
            || deadline <= now
            || deadline > now + 3600
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let grants = self.grants(principal)?;
        limeos_policy::authorize(
            principal,
            &grants,
            &Scope {
                operation: Operation::HealthRead,
                resource: intent.resource().into(),
            },
        )?;
        let encoded = json(intent)?;
        let digest = limeos_identity::digest(&encoded);
        let generation = self.generation;
        let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT id,digest FROM jobs WHERE principal=? AND idempotency=?",
                    params![principal.id, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?;
            if let Some((old_id, old_digest)) = existing {
                return if old_digest == digest {
                    Ok(old_id)
                } else {
                    Err(Error(ErrorCode::Conflict))
                };
            }
            tx.execute(
                "INSERT INTO jobs VALUES (?,?,?,?,?,?,'queued',?,?,?)",
                params![
                    id,
                    principal.id,
                    key,
                    encoded,
                    digest,
                    intent.resource(),
                    generation,
                    principal.grant_revision,
                    deadline
                ],
            )
            .map_err(durable)?;
            event(
                tx,
                Some(&principal.id),
                Some(&id),
                EventKind::JobQueued,
                now,
            )?;
            Ok(id)
        })
    }
    pub fn transition(&mut self, id: &str, from: JobState, to: JobState, now: i64) -> Result<()> {
        if !from.allows(to) {
            return Err(Error(ErrorCode::Conflict));
        }
        if matches!(
            to,
            JobState::Succeeded | JobState::Failed | JobState::PreconditionChanged
        ) && from != JobState::Queued
        {
            let intent: String = self
                .conn
                .query_row("SELECT intent FROM jobs WHERE id=?", [id], |r| r.get(0))
                .map_err(durable)?;
            if !matches!(parse::<Intent>(&intent)?, Intent::HealthProbe { .. }) {
                return Err(Error(ErrorCode::Forbidden));
            }
        }
        if to == JobState::Running || (to == JobState::Canceled && from != JobState::Queued) {
            let encoded: String = self
                .conn
                .query_row("SELECT intent FROM jobs WHERE id=?", [id], |r| r.get(0))
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if !matches!(parse::<Intent>(&encoded)?, Intent::HealthProbe { .. }) {
                return Err(Error(ErrorCode::Forbidden));
            }
        }
        self.write(|tx| {
            let principal:String=tx.query_row("SELECT principal FROM jobs WHERE id=?",[id],|r|r.get(0)).optional().map_err(durable)?.ok_or(Error(ErrorCode::NotFound))?;
            let changed = if to == JobState::Running {
                tx.execute("UPDATE jobs SET state=? WHERE id=? AND state=? AND deadline>? AND revision=(SELECT grant_revision FROM users WHERE id=jobs.principal)",params![to.as_str(),id,from.as_str(),now]).map_err(|_|Error(ErrorCode::Conflict))?
            } else { tx.execute("UPDATE jobs SET state=? WHERE id=? AND state=?",params![to.as_str(),id,from.as_str()]).map_err(durable)? };
            if changed!=1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx,Some(&principal),Some(id),EventKind::JobTransition { state: to },now)
        })
    }
    pub fn job_state(&self, id: &str) -> Result<String> {
        self.conn
            .query_row("SELECT state FROM jobs WHERE id=?", [id], |r| r.get(0))
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))
    }
}
