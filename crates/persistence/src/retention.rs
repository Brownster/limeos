//! Bounded history: the one retention mechanism for every table that grows with
//! use. Each bounded table declares a row budget, a batch size and a prune rule
//! in [`RULES`]. The writer thread runs a pass at most once a minute
//! ([`Store::maintain`]). A table at or under its budget costs one read and
//! writes nothing. A table over budget gets one batch of oldest-first deletions
//! in its own short IMMEDIATE transaction; a batch that finds nothing eligible
//! rolls back, so an idle or unchanged store is never written by retention.
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
//! A job's events are removed together, so a per-job reader
//! (`container_events`, `stack_events`) sees either a job's whole history or
//! none of it, never a gap. The job row itself, with its final state, is never
//! removed, and serves as the snapshot when its history is gone.
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
/// Startup recovery may record its events this far past the cap that stops
/// every other write.
pub(super) const AUDIT_RESERVE: i64 = 1024 * 1024;
const EVENT_BATCH: i64 = 1024;
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
/// A settled job whose history no check, lock or open request needs.
const SETTLED: &str = "j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') NOT IN ('automation_observe_health','storage_mount') AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NULL)";

struct Rule {
    table: &'static str,
    /// Rows kept before pruning starts.
    budget: i64,
    /// Most rows one batch removes (whole jobs may add a few more).
    batch: i64,
    rows: fn(&Connection) -> Result<i64>,
    prune: fn(&Transaction<'_>, i64) -> Result<i64>,
}
const RULES: [Rule; 1] = [Rule {
    table: "events",
    budget: EVENT_BUDGET / EVENT_BYTES,
    batch: EVENT_BATCH,
    rows: audited_events,
    prune: prune_events,
}];

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

fn audited_events(conn: &Connection) -> Result<i64> {
    let bytes: i64 = conn
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(durable)?;
    Ok(bytes / EVENT_BYTES)
}

/// Removes the oldest `limit` eligible events, completing any job they touch.
fn prune_events(tx: &Transaction<'_>, limit: i64) -> Result<i64> {
    let loose = loose();
    let horizon: Option<i64> = tx
        .query_row(
            &format!("SELECT max(cursor) FROM (SELECT e.cursor FROM events e LEFT JOIN jobs j ON j.id=e.job WHERE ({loose}) OR (e.job IS NOT NULL AND {SETTLED}) ORDER BY e.cursor LIMIT ?)"),
            [limit],
            |r| r.get(0),
        )
        .map_err(durable)?;
    let Some(horizon) = horizon else {
        return Ok(0);
    };
    let mut removed = tx
        .execute(
            &format!("DELETE FROM events AS e WHERE e.cursor<=? AND {loose}"),
            [horizon],
        )
        .map_err(durable)?;
    #[cfg(test)]
    tests::pause_mid_batch();
    removed += tx
        .execute(
            &format!("DELETE FROM events WHERE job IN (SELECT e.job FROM events e JOIN jobs j ON j.id=e.job WHERE e.cursor<=? AND {SETTLED})"),
            [horizon],
        )
        .map_err(durable)?;
    let removed = i64::try_from(removed).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    account(tx, -removed)?;
    Ok(removed)
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
            let excess = (rule.rows)(&self.conn)?.saturating_sub(rule.budget);
            if excess <= 0 {
                continue;
            }
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(durable)?;
            let rows = (rule.prune)(&tx, excess.min(rule.batch))?;
            if rows > 0 {
                tx.commit().map_err(durable)?;
                removed.push((rule.table, rows));
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
