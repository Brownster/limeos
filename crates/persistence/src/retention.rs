//! Bounded history: the one retention mechanism for every table that grows with
//! use. Each bounded table declares a row budget, a batch size and a prune rule
//! in [`RULES`]. The writer thread runs a pass at most once a minute
//! ([`Store::maintain`]). A table at or under its budget costs one read and
//! writes nothing. A table over budget gets one batch of oldest-first deletions
//! in its own short IMMEDIATE transaction; a batch that finds nothing eligible
//! rolls back, so an idle or unchanged store is never written by retention.
//! Startup also runs the events rule, before the writer thread exists, when
//! the recovery events for interrupted jobs would not fit in the reserve
//! ([`make_room`]); it refuses only when nothing more is prunable.
//!
//! # What is never pruned
//!
//! Derived from every trigger in `schema.sql` and the migrations and from every
//! startup check (`resources::check`, which runs the storage-mount, backup,
//! automation, completion, policy and polling checks). A new check that reads a
//! row must add that row here.
//!
//! Events (`loose`, [`SETTLED`]):
//! - Every event of a job that is not settled: `planned`, `waiting_approval`,
//!   `queued`, `running`, `verifying`, `outcome_unknown` or
//!   `needs_intervention`. Resource locks exist only in the last four states
//!   (migration 7), so this keeps the history of every held lock and every
//!   unresolved intervention; `needs_attention_jobs` reads their oldest event.
//! - Every event of a job with an open resolution request
//!   (`job_resolutions.confirmation IS NULL`), whatever the job's state.
//! - Every event of an `automation_observe_health` job, in any state. The
//!   automation check requires exactly one `job_queued` event per such job, the
//!   completion check its transition, settlement and notification proofs, and
//!   the polling check its poll proof.
//! - Every event of a `storage_mount` job: its check reads the `running`
//!   transition to decide whether a result must exist.
//! - Job-less events of the kinds in `PINNED_KINDS`: schedule changes (the
//!   proof of every pause cancellation), health policy changes (also the live
//!   policy revision) and notification publication, acknowledgement, prune and
//!   policy-check proofs. Any other job-less `automation_*` kind is pinned too
//!   unless listed in [`UNREAD_AUTOMATION_KINDS`], so a future kind fails safe.
//!
//! Every batch removes at most its limit of rows, oldest first by cursor, so a
//! job with a long history (a health probe cycling through `verifying` and
//! `outcome_unknown`) is removed over several passes. A job's `job_queued`
//! event is written with the job row and is its oldest event, and a job's
//! events all become eligible together (a settled job never becomes unsettled
//! again), so the first batch that touches a job removes `job_queued`. Its
//! absence is the durable "pruned" mark: a per-job reader (`container_events`,
//! `stack_events`) filters on [`RETAINED_HISTORY`] and so sees either a job's
//! whole history or none of it, never a gap, while later passes remove the
//! rest. Nothing else is needed for crash safety: each batch commits on its
//! own and the startup check reads no settled job's history. The job row
//! itself, with its final state, is never removed, and serves as the snapshot
//! when its history is gone.
//!
//! Occurrences (mirroring the `automation_occurrence_delete` trigger):
//! - An occurrence whose job is not `succeeded`, `failed`, `canceled` or
//!   `precondition_changed` (queued observation work).
//! - The occurrence at its schedule's current watermark (current revision and
//!   `due` not before `settled_through`).
//! - Each schedule's newest occurrence of any revision, which the schedule view
//!   reports as its last occurrence.
use super::*;
use std::time::{Duration, Instant};

/// Estimated bytes of one event. `meta.audit_bytes` is this times the number
/// of rows in `events`; [`account`] is its only incremental writer.
pub(super) const EVENT_BYTES: i64 = 256;
/// Settled history is pruned down to half of the hard cap.
const EVENT_BUDGET: i64 = AUDIT_LIMIT / 2;
/// Usage at or above this is reported as a warning: pruning cannot keep up, or
/// pinned history (open work, automation proofs) has grown large.
pub(super) const AUDIT_WARNING: i64 = AUDIT_LIMIT / 4 * 3;
/// Revocations and startup recovery may record their events this far past the
/// cap that stops every other write.
pub(super) const AUDIT_RESERVE: i64 = 1024 * 1024;
const EVENT_BATCH: i64 = 1024;
/// A quarter of `MAX_OCCURRENCES`; 128 schedules at the 30 s minimum add 256
/// rows a minute, which one batch removes.
const OCCURRENCE_BUDGET: i64 = 1024;
const OCCURRENCE_BATCH: i64 = 512;
pub(super) const INTERVAL: Duration = Duration::from_secs(60);

/// Job-less automation kinds that a startup check or live read depends on.
/// Pinned because they are not in [`UNREAD_AUTOMATION_KINDS`]; listed so the
/// tests prove it.
#[cfg(test)]
const PINNED_KINDS: [&str; 6] = [
    "automation_schedule_changed",
    "automation_health_policy_changed",
    "automation_notification_published",
    "automation_notification_acknowledged",
    "automation_notification_pruned",
    "automation_notification_policy_checked",
];
/// Job-less automation kinds nothing reads back.
const UNREAD_AUTOMATION_KINDS: [&str; 2] = [
    "automation_schedule_created",
    "automation_occurrence_settled",
];

/// A job-less event that no check reads: any kind outside the automation
/// family, or an automation kind listed as unread.
fn loose() -> String {
    let unread = UNREAD_AUTOMATION_KINDS
        .map(|kind| format!("'{kind}'"))
        .join(",");
    format!(
        "e.job IS NULL AND (json_extract(e.kind,'$.kind') NOT GLOB 'automation_*' OR json_extract(e.kind,'$.kind') IN ({unread}))"
    )
}
/// True while the job of the `events` row in scope still has its whole
/// history: retention removes a job's `job_queued` event, its oldest, in the
/// first batch that touches it. Per-job readers add this to their filter.
pub(super) const RETAINED_HISTORY: &str =
    r#"EXISTS(SELECT 1 FROM events q WHERE q.job=events.job AND q.kind='{"kind":"job_queued"}')"#;
/// A settled job whose history no check, lock or open request needs.
const SETTLED: &str = "j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') NOT IN ('automation_observe_health','storage_mount') AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NULL)";

struct Rule {
    table: &'static str,
    /// Rows kept before pruning starts.
    budget: i64,
    /// Most rows one batch (one transaction) removes.
    batch: i64,
    rows: fn(&Connection) -> Result<i64>,
    prune: fn(&Transaction<'_>, i64) -> Result<i64>,
}
const RULES: [Rule; 2] = [
    Rule {
        table: "events",
        budget: EVENT_BUDGET / EVENT_BYTES,
        batch: EVENT_BATCH,
        rows: audited_events,
        prune: prune_events,
    },
    Rule {
        table: "automation_occurrences",
        budget: OCCURRENCE_BUDGET,
        batch: OCCURRENCE_BATCH,
        rows: occurrences,
        prune: prune_occurrences,
    },
];

/// Adds `events` (negative when pruning) to the audit counter.
pub(super) fn account(tx: &Transaction<'_>, events: i64) -> Result<()> {
    let bytes = events
        .checked_mul(EVENT_BYTES)
        .ok_or(Error(ErrorCode::StateNotDurable))?;
    tx.execute(
        "UPDATE meta SET audit_bytes=audit_bytes+? WHERE id=1",
        [bytes],
    )
    .map_err(durable)?;
    Ok(())
}

/// Startup: recompute the counter from the table when they disagree (an older
/// build, a hand edit or a lost update) instead of refusing to start.
pub(super) fn reconcile(tx: &Transaction<'_>) -> Result<i64> {
    let (bytes, rows): (i64, i64) = tx
        .query_row(
            "SELECT audit_bytes,(SELECT count(*) FROM events) FROM meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(durable)?;
    let actual = rows
        .checked_mul(EVENT_BYTES)
        .ok_or(Error(ErrorCode::StateNotDurable))?;
    if bytes != actual {
        tx.execute("UPDATE meta SET audit_bytes=? WHERE id=1", [actual])
            .map_err(durable)?;
    }
    Ok(actual)
}

/// Whether startup recovery's one event per interrupted job fits within
/// [`AUDIT_RESERVE`] past the cap.
pub(super) fn recovery_fits(interrupted: i64, audit_bytes: i64) -> bool {
    interrupted == 0
        || interrupted.saturating_mul(EVENT_BYTES) <= AUDIT_LIMIT + AUDIT_RESERVE - audit_bytes
}

/// Startup, when recovery does not fit: the events rule's ordinary batches,
/// each its own transaction, until it fits. Refused only when a batch finds
/// nothing more to prune (the history is pinned).
pub(super) fn make_room(conn: &mut Connection, interrupted: i64) -> Result<()> {
    loop {
        let bytes = audited_events(conn)?
            .checked_mul(EVENT_BYTES)
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        if recovery_fits(interrupted, bytes) {
            return Ok(());
        }
        if run(conn, &RULES[0])? == 0 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
}

/// One batch of `rule` if its table is over budget, in its own transaction.
/// A batch that removes nothing rolls back.
fn run(conn: &mut Connection, rule: &Rule) -> Result<i64> {
    let excess = (rule.rows)(conn)?.saturating_sub(rule.budget);
    if excess <= 0 {
        return Ok(0);
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(durable)?;
    let rows = (rule.prune)(&tx, excess.min(rule.batch))?;
    if rows > 0 {
        tx.commit().map_err(durable)?;
    }
    Ok(rows)
}

fn audited_events(conn: &Connection) -> Result<i64> {
    let bytes: i64 = conn
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(durable)?;
    Ok(bytes / EVENT_BYTES)
}

fn occurrences(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
        r.get(0)
    })
    .map_err(durable)
}

/// The oldest prunable job-less events. Walks only job-less rows, in cursor
/// order through `events_job`; the pinned ones among them are automation
/// proofs, which stay few.
fn loose_candidates() -> String {
    format!(
        "SELECT e.cursor FROM events e INDEXED BY events_job WHERE {} ORDER BY e.cursor LIMIT ?",
        loose()
    )
}
/// The oldest events of settled jobs. Jobs are filtered first, by state
/// through `jobs_state` and then by operation and open request once per job,
/// so the events of open and pinned jobs, which accumulate at the start of the
/// table, are never visited.
fn settled_candidates() -> String {
    format!(
        "SELECT e.cursor FROM jobs j CROSS JOIN events e ON e.job=j.id WHERE {SETTLED} ORDER BY e.cursor LIMIT ?"
    )
}

/// Removes the oldest `limit` eligible events, and never more: a job whose
/// history is longer is finished by later passes (see [`RETAINED_HISTORY`]).
/// The cost of a pass follows the settled jobs and the prunable rows, not
/// the pinned history in front of them.
fn prune_events(tx: &Transaction<'_>, limit: i64) -> Result<i64> {
    let select = |sql: String| -> Result<Vec<i64>> {
        let mut stmt = tx.prepare(&sql).map_err(durable)?;
        stmt.query_map([limit], |r| r.get(0))
            .map_err(durable)?
            .collect::<std::result::Result<_, _>>()
            .map_err(durable)
    };
    // Each list holds its own oldest `limit`, so together they hold the
    // oldest `limit` overall.
    let mut cursors: Vec<(i64, bool)> = select(loose_candidates())?
        .into_iter()
        .map(|cursor| (cursor, true))
        .chain(
            select(settled_candidates())?
                .into_iter()
                .map(|cursor| (cursor, false)),
        )
        .collect();
    cursors.sort_unstable();
    cursors.truncate(usize::try_from(limit).map_err(|_| Error(ErrorCode::InvalidInput))?);
    if cursors.is_empty() {
        return Ok(0);
    }
    let (loose, settled): (Vec<_>, Vec<_>) = cursors.into_iter().partition(|(_, loose)| *loose);
    let delete = |rows: Vec<(i64, bool)>| -> Result<usize> {
        let rows: Vec<i64> = rows.into_iter().map(|(cursor, _)| cursor).collect();
        tx.execute(
            "DELETE FROM events WHERE cursor IN (SELECT value FROM json_each(?))",
            [json(&rows)?],
        )
        .map_err(durable)
    };
    let mut removed = delete(loose)?;
    #[cfg(test)]
    tests::pause_mid_batch();
    removed += delete(settled)?;
    let removed = i64::try_from(removed).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    account(tx, -removed)?;
    Ok(removed)
}

/// Removes the oldest `limit` occurrences the delete trigger permits, keeping
/// each schedule's newest one.
fn prune_occurrences(tx: &Transaction<'_>, limit: i64) -> Result<i64> {
    let mut stmt = tx
        .prepare("SELECT o.schedule,o.revision,o.due FROM automation_occurrences o JOIN automation_schedules s ON s.id=o.schedule LEFT JOIN jobs j ON j.id=o.job WHERE (o.job IS NULL OR j.state IN ('succeeded','failed','canceled','precondition_changed')) AND (o.revision<s.revision OR (o.revision=s.revision AND o.due<s.settled_through)) AND EXISTS(SELECT 1 FROM automation_occurrences n WHERE n.schedule=o.schedule AND (n.revision,n.due)>(o.revision,o.due)) ORDER BY o.created,o.schedule,o.revision,o.due LIMIT ?")
        .map_err(durable)?;
    let keys = stmt
        .query_map([limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(durable)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(durable)?;
    let mut removed = 0;
    for (schedule, revision, due) in keys {
        removed += tx
            .execute(
                "DELETE FROM automation_occurrences WHERE schedule=? AND revision=? AND due=?",
                params![schedule, revision, due],
            )
            .map_err(durable)?;
    }
    i64::try_from(removed).map_err(|_| Error(ErrorCode::StateNotDurable))
}

/// How full the audit table is, against the hard cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditUsage {
    pub bytes: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditLevel {
    Normal,
    /// At or above 75% of the cap. Retention keeps settled history at half the
    /// cap, so this means pinned history is large or writes outpace pruning.
    Warning,
    /// At the cap: every write except revocation is refused until pruning
    /// frees space.
    Full,
}
impl AuditUsage {
    pub const LIMIT: i64 = AUDIT_LIMIT;
    pub fn level(self) -> AuditLevel {
        if self.bytes >= AUDIT_LIMIT {
            AuditLevel::Full
        } else if self.bytes >= AUDIT_WARNING {
            AuditLevel::Warning
        } else {
            AuditLevel::Normal
        }
    }
}

impl Store {
    pub fn audit_usage(&self) -> Result<AuditUsage> {
        let bytes = self
            .conn
            .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        Ok(AuditUsage { bytes })
    }
    /// A retention pass if a minute has passed since the last one. Returns the
    /// audit usage it observed, or `None` when no pass was due. A failed batch
    /// is retried on the next pass; it never fails the caller's work.
    pub(super) fn maintain(&mut self, now: Instant) -> Option<AuditUsage> {
        if self
            .retained_at
            .is_some_and(|at| now.saturating_duration_since(at) < INTERVAL)
        {
            return None;
        }
        self.retained_at = Some(now);
        let _ = self.prune();
        self.audit_usage().ok()
    }
    /// One batch for every table over its budget, each in its own transaction.
    /// Returns the rows removed per table, omitting tables with none.
    pub(super) fn prune(&mut self) -> Result<Vec<(&'static str, i64)>> {
        let mut removed = Vec::new();
        for rule in &RULES {
            let rows = run(&mut self.conn, rule)?;
            if rows > 0 {
                removed.push((rule.table, rows));
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
