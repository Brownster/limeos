use super::*;

/// The operations whose jobs the container executor's receipts settle.
const EXECUTOR_OPERATIONS: &str = "('container_restart','container_start','container_stop','stack_restart','stack_start','stack_stop')";

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
        self.hold_for_operator(id, EventKind::ExecutorReceiptMissing, now)
    }

    /// The container executor's receipt-store epoch, from an answer core just
    /// received (`record_epoch`). Returns the jobs marked. An unchanged epoch
    /// writes nothing.
    pub fn observe_executor_epoch(&mut self, epoch: &str, now: i64) -> Result<usize> {
        if !limeos_domain::opaque_id(epoch) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let seen: Option<String> = self
            .conn
            .query_row("SELECT executor_epoch FROM meta WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(durable)?;
        if seen.as_deref() == Some(epoch) {
            return Ok(0);
        }
        self.write(|tx| record_epoch(tx, epoch, now))
    }

    /// A claimed container or stack job whose execute request certainly never
    /// ran: it never left core (the connection to the executor could not be
    /// established, so no byte of it was written), or the executor refused it
    /// before recording anything for the action. Core sends one execute per
    /// job, so nothing for this action can run later: the job moves from
    /// `running` to `precondition_changed` and releases its locks, as a
    /// refusal does. Only a job still `running` (just claimed) qualifies; a
    /// repeat is `Conflict`.
    pub fn record_unsent_execute(&mut self, id: &str, now: i64) -> Result<()> {
        self.write(|tx| {
            let (principal, intent): (String, String) = tx
                .query_row("SELECT principal,intent FROM jobs WHERE id=?", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
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
            if tx
                .execute(
                    "UPDATE jobs SET state='precondition_changed' WHERE id=? AND state='running'",
                    [id],
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
                    state: JobState::PreconditionChanged,
                },
                now,
            )
        })
    }

    /// Whether a `never_recorded` closure (`body`, the receipt as recorded)
    /// for this job contradicts what core holds: a result already recorded
    /// in `results` other than this very closure, or an
    /// `executor_store_changed` event, since the store it was dispatched to
    /// may have lost the job's receipt. Pure lookup.
    pub(crate) fn closure_disputed(&self, id: &str, results: &str, body: &str) -> Result<bool> {
        let state: String = self
            .conn
            .query_row("SELECT state FROM jobs WHERE id=?", [id], |r| r.get(0))
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))?;
        let recorded: Option<String> = self
            .conn
            .query_row(
                &format!("SELECT receipt FROM {results} WHERE job=?"),
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(durable)?;
        if state == JobState::PreconditionChanged.as_str() && recorded.as_deref() == Some(body) {
            // The same closure, recorded before: a repeat.
            return Ok(false);
        }
        if recorded.is_some() {
            return Ok(true);
        }
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE job=? AND kind=?)",
                params![id, json(&EventKind::ExecutorStoreChanged)?],
                |r| r.get(0),
            )
            .map_err(durable)
    }

    /// A `never_recorded` closure contradicted what core holds
    /// (`closure_disputed`): nothing is released. The job moves to (or stays
    /// in) `needs_intervention` with every lock, and one
    /// `executor_record_disagrees` event records why; a repeat writes nothing.
    pub fn record_receipt_disagrees(&mut self, id: &str, now: i64) -> Result<()> {
        self.hold_for_operator(id, EventKind::ExecutorRecordDisagrees, now)
    }

    /// Hold an uncertain container or stack job for an operator, recording
    /// `reason` once.
    fn hold_for_operator(&mut self, id: &str, reason: EventKind, now: i64) -> Result<()> {
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
            let kind = json(&reason)?;
            let recorded: i64 = tx
                .query_row(
                    "SELECT count(*) FROM events WHERE job=? AND kind=?",
                    params![id, kind],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if recorded == 0 {
                event(tx, Some(&principal), Some(id), reason, now)?;
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

/// Record `epoch`, the container executor's receipt-store epoch, in `tx`.
/// One that differs from the recorded epoch means the store core talks to is
/// not the one it dispatched its held jobs under: every held container and
/// stack job gets one `executor_store_changed` event (so a `never_recorded`
/// closure for it is refused, `record_container_closure`), and the new epoch
/// is recorded, in the same transaction. The first epoch ever recorded is a
/// change like any other: a job held then was dispatched under no known epoch
/// (before the upgrade that added it), so nothing ties it to this store.
///
/// This is the one place the recorded epoch changes, and a claim records the
/// epoch it dispatches under in its own transaction, before it takes the job
/// (`claim_container`): a job held when an epoch is recorded is marked by it,
/// and a job claimed after is dispatched under it and is not. So a job
/// without the mark was dispatched under the epoch recorded now, and since
/// core records the epoch of every answer before acting on it, a closure is
/// accepted only when its answer's epoch is the one the job was sent under.
/// Returns the jobs marked; an unchanged epoch writes nothing.
pub(crate) fn record_epoch(tx: &Transaction<'_>, epoch: &str, now: i64) -> Result<usize> {
    if !limeos_domain::opaque_id(epoch) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let seen: Option<String> = tx
        .query_row("SELECT executor_epoch FROM meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    if seen.as_deref() == Some(epoch) {
        return Ok(0);
    }
    // A change, or the first epoch ever recorded (`None`: a fresh install,
    // where nothing is held, or the first answer after the upgrade that added
    // the epoch, where every held job was dispatched under none).
    let kind = json(&EventKind::ExecutorStoreChanged)?;
    let marked = tx
        .execute(
            &format!(
                "INSERT INTO events(principal,job,kind,created) SELECT j.principal,j.id,?1,?2 FROM jobs j WHERE j.state IN ('running','verifying','outcome_unknown','needs_intervention') AND json_extract(j.intent,'$.operation') IN {EXECUTOR_OPERATIONS} AND NOT EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND e.kind=?1)"
            ),
            params![kind, now],
        )
        .map_err(durable)?;
    retention::account(tx, marked as i64)?;
    if tx
        .execute("UPDATE meta SET executor_epoch=? WHERE id=1", [epoch])
        .map_err(durable)?
        != 1
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(marked)
}
