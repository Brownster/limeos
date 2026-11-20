use super::*;

/// One recovery candidate row: id, intent, state, owner id, role, revision.
pub(crate) type CandidateRow = (String, String, String, String, String, i64);

/// The worker's candidates among jobs whose operation is in `operations` (an
/// SQL list): every queued job, oldest first, then held jobs (dispatched, not
/// yet settled) whose ID sorts after `cursor`, in ID order; each part at most
/// `RECOVERY_BATCH`. Queued jobs are tried every pass and drain on their own,
/// since a queued plan expires. Held ones wait on the executor, possibly for
/// an operator, and could outnumber any fixed batch: a caller that pages
/// through them with the cursor reaches every one, however many there are and
/// whatever their age.
pub(crate) fn candidate_rows(
    conn: &Connection,
    operations: &str,
    cursor: &str,
) -> Result<Vec<CandidateRow>> {
    let select = "SELECT j.id,j.intent,j.state,u.id,u.role,u.grant_revision FROM jobs j JOIN users u ON u.id=j.principal";
    let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<CandidateRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    };
    let mut rows = Vec::new();
    for (sql, cursor) in [
        (
            format!(
                "{select} WHERE j.state='queued' AND json_extract(j.intent,'$.operation') IN {operations} ORDER BY j.rowid LIMIT {RECOVERY_BATCH}"
            ),
            None,
        ),
        (
            format!(
                "{select} WHERE j.state IN ('running','verifying','outcome_unknown','needs_intervention') AND json_extract(j.intent,'$.operation') IN {operations} AND j.id>?1 ORDER BY j.id LIMIT {RECOVERY_BATCH}"
            ),
            Some(cursor),
        ),
    ] {
        let mut stmt = conn.prepare(&sql).map_err(durable)?;
        let found = match cursor {
            Some(cursor) => stmt.query_map([cursor], row),
            None => stmt.query_map([], row),
        }
        .map_err(durable)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(durable)?;
        rows.extend(found);
    }
    Ok(rows)
}

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
        // Resolution needs a recorded request and executor confirmation.
        if to == JobState::Resolved {
            return Err(Error(ErrorCode::Forbidden));
        }
        let encoded: String = self
            .conn
            .query_row("SELECT intent FROM jobs WHERE id=?", [id], |r| r.get(0))
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))?;
        if matches!(
            parse::<Intent>(&encoded)?,
            Intent::AutomationObserveHealth { .. }
        ) {
            return Err(Error(ErrorCode::Forbidden));
        }
        if matches!(
            parse::<Intent>(&encoded)?,
            Intent::BackupPreparePrivate { .. }
        ) && (from != JobState::Queued || to != JobState::Canceled)
        {
            return Err(Error(ErrorCode::Forbidden));
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
    /// A successful executor lookup found no receipt for this uncertain
    /// container or stack job, and the executor answered that one may have
    /// been pruned (`limeos_domain::ReceiptAbsence::NoLongerHeld`).
    /// The job moves to (or stays in) `needs_intervention` with every lock, and
    /// one `executor_receipt_missing` event records why. Repeating it changes
    /// nothing, so the recovery poll cannot grow the audit log.
    pub fn record_receipt_missing(&mut self, id: &str, now: i64) -> Result<()> {
        self.write(|tx| {
            let (principal, state, intent): (String, String, String) = tx
                .query_row(
                    "SELECT principal,state,intent FROM jobs WHERE id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if !matches!(
                parse::<Intent>(&intent)?,
                Intent::ContainerRestart { .. }
                    | Intent::ContainerStart { .. }
                    | Intent::ContainerStop { .. }
                    | Intent::StackRestart { .. }
                    | Intent::StackStart { .. }
                    | Intent::StackStop { .. }
            ) {
                return Err(Error(ErrorCode::Forbidden));
            }
            if !matches!(
                state.as_str(),
                "running" | "verifying" | "outcome_unknown" | "needs_intervention"
            ) {
                return Err(Error(ErrorCode::Conflict));
            }
            if state != JobState::NeedsIntervention.as_str() {
                if tx
                    .execute(
                        "UPDATE jobs SET state='needs_intervention' WHERE id=? AND state=?",
                        params![id, state],
                    )
                    .map_err(durable)?
                    != 1
                {
                    return Err(Error(ErrorCode::Conflict));
                }
                event(
                    tx,
                    Some(&principal),
                    Some(id),
                    EventKind::JobTransition {
                        state: JobState::NeedsIntervention,
                    },
                    now,
                )?;
            }
            let kind = json(&EventKind::ExecutorReceiptMissing)?;
            let recorded: i64 = tx
                .query_row(
                    "SELECT count(*) FROM events WHERE job=? AND kind=?",
                    params![id, kind],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if recorded == 0 {
                event(
                    tx,
                    Some(&principal),
                    Some(id),
                    EventKind::ExecutorReceiptMissing,
                    now,
                )?;
            }
            Ok(())
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
