//! Bounded history: the one retention mechanism for every table that grows with
//! use. Each bounded table declares a row budget, a batch size and a prune rule
//! in [`RULES`]. The writer thread runs a pass at most once a minute
//! ([`Store::maintain`]). A table at or under its budget costs one read and
//! writes nothing. A table over budget gets one batch of oldest-first deletions
//! in its own short IMMEDIATE transaction; a batch that finds nothing eligible
//! rolls back, so an idle or unchanged store is never written by retention.
//! Startup also runs the rules for audited rows (bulk events, then authority
//! events), inside its own transaction and before the writer thread exists,
//! when the recovery events for interrupted jobs would not fit in the reserve
//! ([`make_room`]); it refuses, rolling back the whole startup (any schema
//! upgrade too), only when neither rule can free anything more.
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
//! Authority events ([`AUTHORITY_KINDS`]) are a class of their own: the events
//! rule never removes them, whatever the pressure from bulk history. An
//! authority event of a job that still has its history ([`RETAINED_HISTORY`])
//! is pinned: kept, so a retained job never shows a gap, and outside the
//! budget. Of the others (prunable), their own rule keeps the newest
//! [`AUTHORITY_BUDGET`] and removes older ones oldest first. So the newest
//! prunable authority event is never removed while an older one exists.
//!
//! Only resolution requests, withdrawals and supersessions carry a job, so only they can be
//! pinned, and only for as long as their job's history: while the job is
//! unsettled or has an open request, and then until the events rule prunes
//! the settled job (its `job_queued` first), after which they are prunable
//! and count against the budget. Until then they are bounded like the rest
//! of the job's pinned history: each is written through the ordinary write
//! gate, which refuses at the audit cap.
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
//! own and the startup check reads no settled job's history. The job row,
//! with its final state, serves as the snapshot when its history is gone,
//! until job-row retention ([`jobs`]) removes it, with what history it still
//! has and every row that names it, 30 days after it settled.
//!
//! Job rows (`jobs`, the last rule; see that module for the rule and why):
//! a settled container, stack or health-probe job with no open resolution
//! request and no authority event, past the newest `FLOOR` of its family and
//! unchanged for `HORIZON`, goes with its events, resources, plan, result and
//! completed resolution. Every other job stays.
//!
//! # Known limit: automation results (solve before enabling a producer)
//!
//! Nothing here bounds automation results. `automation_observation_results`
//! holds up to `MAX_RESULTS` (65,536) rows that no trigger lets anyone
//! delete, each result pins its observation job's events (queued,
//! transition, settlement and notification proofs, about four), and job-row
//! retention never removes an observation job. Together they can fill the 64 MiB audit cap
//! on their own: one observation schedule at the 30 s minimum gets there in
//! about 23 days, after which every write but revocation is refused. No
//! producer reserves observations yet; this must be solved (result
//! retention that the completion check accepts, and job-row retention)
//! before one is enabled.
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
/// Prunable authority events kept: 1 MiB of the cap, decades of
/// administration at any plausible rate (a few enrolments, imports and grant
/// revisions, and a resolution request per intervention). Pinned ones are
/// kept besides.
const AUTHORITY_BUDGET: i64 = 4096;
const AUTHORITY_BATCH: i64 = 256;
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

/// Low-volume, high-value events, kept apart from the bulk: who issued a
/// bootstrap token, who enrolled or imported an administrator, who revised
/// grants, and what was declared in a resolution request and that it was
/// withdrawn or superseded (such a request's row is deleted, so its events
/// are the only record). Not `task_issued`: once assistant tasks issue tokens it is one per
/// task, for authority that expires within the hour and with the core
/// generation. Not session events, the bulk of the table.
pub(super) const AUTHORITY_KINDS: [&str; 7] = [
    "bootstrap_issued",
    "administrator_enrolled",
    "legacy_identity_imported",
    "grants_revised",
    "job_resolution_requested",
    "job_resolution_withdrawn",
    "job_resolution_superseded",
];
fn quoted(kinds: &[&str]) -> String {
    kinds
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// A job-less event that no check reads and that is not an authority event:
/// any other kind outside the automation family, or an automation kind listed
/// as unread.
fn loose() -> String {
    let unread = quoted(&UNREAD_AUTOMATION_KINDS);
    let authority = quoted(&AUTHORITY_KINDS);
    format!(
        "e.job IS NULL AND (json_extract(e.kind,'$.kind') NOT GLOB 'automation_*' OR json_extract(e.kind,'$.kind') IN ({unread})) AND json_extract(e.kind,'$.kind') NOT IN ({authority})"
    )
}
/// True while the job of the `events` row in scope still has its whole
/// history: retention removes a job's `job_queued` event, its oldest, in the
/// first batch that touches it. Per-job readers add this to their filter.
pub(super) const RETAINED_HISTORY: &str =
    r#"EXISTS(SELECT 1 FROM events q WHERE q.job=events.job AND q.kind='{"kind":"job_queued"}')"#;
/// A settled job whose history no check, lock or open request needs.
const SETTLED: &str = "j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') NOT IN ('automation_observe_health','storage_mount') AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NULL)";

/// What one batch removed: rows of its rule (events, occurrences or jobs),
/// and of them or besides, audited events (which [`account`] has already
/// taken off the counter).
#[derive(Debug, Default)]
pub(super) struct Removed {
    pub(super) rows: i64,
    pub(super) events: i64,
    /// The jobs a job-row batch removed.
    pub(super) jobs: Vec<String>,
}
impl Removed {
    fn events(rows: i64) -> Self {
        Self {
            rows,
            events: rows,
            jobs: Vec::new(),
        }
    }
    fn any(&self) -> bool {
        self.rows > 0 || self.events > 0
    }
}

struct Rule {
    table: &'static str,
    /// Rows kept before pruning starts.
    budget: i64,
    /// Most rows one batch (one transaction) removes.
    batch: i64,
    rows: fn(&Connection) -> Result<i64>,
    /// One batch of at most the limit given, at the time given (seconds since
    /// the Unix epoch), resuming the scan given (only job-row retention reads
    /// either).
    prune: fn(&Transaction<'_>, i64, i64, &mut jobs::Scan) -> Result<Removed>,
    /// Whether the rows it removes count against the audit cap, so startup
    /// may run it to make room for recovery ([`make_room`]).
    audited: bool,
}
const RULES: [Rule; 4] = [
    Rule {
        table: "events",
        budget: EVENT_BUDGET / EVENT_BYTES,
        batch: EVENT_BATCH,
        rows: audited_events,
        prune: prune_events,
        audited: true,
    },
    Rule {
        table: "authority events",
        budget: AUTHORITY_BUDGET,
        batch: AUTHORITY_BATCH,
        rows: authority_events,
        prune: prune_authority,
        audited: true,
    },
    Rule {
        table: "automation_occurrences",
        budget: OCCURRENCE_BUDGET,
        batch: OCCURRENCE_BATCH,
        rows: occurrences,
        prune: prune_occurrences,
        audited: false,
    },
    // Not audited although it removes events: it frees only what is past its
    // horizon, so it cannot be counted on to make room at startup.
    Rule {
        table: "jobs",
        budget: 0,
        batch: jobs::BATCH,
        rows: jobs::past_floor,
        prune: jobs::prune,
        audited: false,
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

/// Startup, when recovery does not fit: the batches of every audited rule
/// in [`RULES`] order (bulk events first, then authority events down to their
/// own budget), inside the startup transaction so a refusal leaves nothing
/// changed (a schema upgrade stays atomic with startup). It needs room for
/// one event per interrupted job, so it removes about that many rows, rounded
/// up to a batch, and moves to the next rule when one has nothing more to
/// give. Refused only when no rule can free enough (pinned history).
pub(super) fn make_room(tx: &Transaction<'_>, interrupted: i64) -> Result<()> {
    for rule in RULES.iter().filter(|rule| rule.audited) {
        loop {
            let rows = audited_events(tx)?;
            let bytes = rows
                .checked_mul(EVENT_BYTES)
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            if recovery_fits(interrupted, bytes) {
                return Ok(());
            }
            let excess = (rule.rows)(tx)?.saturating_sub(rule.budget);
            if excess <= 0
                || (rule.prune)(
                    tx,
                    excess.min(rule.batch),
                    crate::writer_clock::unix_now(),
                    &mut jobs::Scan::default(),
                )?
                .rows
                    == 0
            {
                break;
            }
        }
    }
    Err(Error(ErrorCode::StateNotDurable))
}

/// One batch of `rule` if its table is over budget, in its own transaction,
/// at `now` (seconds since the Unix epoch), resuming `scan`. A batch that
/// removes nothing rolls back.
fn run(conn: &mut Connection, rule: &Rule, now: i64, scan: &mut jobs::Scan) -> Result<Removed> {
    let excess = (rule.rows)(conn)?.saturating_sub(rule.budget);
    if excess <= 0 {
        return Ok(Removed::default());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(durable)?;
    let removed = (rule.prune)(&tx, excess.min(rule.batch), now, scan)?;
    if removed.any() {
        tx.commit().map_err(durable)?;
    }
    Ok(removed)
}

fn audited_events(conn: &Connection) -> Result<i64> {
    let bytes: i64 = conn
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(durable)?;
    Ok(bytes / EVENT_BYTES)
}

/// Prunable authority events: the rule's budget counts only these.
fn authority_events(conn: &Connection) -> Result<i64> {
    conn.query_row(
        &format!("SELECT count(*) FROM events WHERE {}", prunable_authority()),
        [],
        |r| r.get(0),
    )
    .map_err(durable)
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
        "SELECT e.cursor FROM jobs j CROSS JOIN events e ON e.job=j.id WHERE {SETTLED} AND json_extract(e.kind,'$.kind') NOT IN ({}) ORDER BY e.cursor LIMIT ?",
        quoted(&AUTHORITY_KINDS)
    )
}
/// An authority event that is not pinned: job-less, or of a job whose
/// history is already being pruned. Read through `events_kind`.
fn prunable_authority() -> String {
    format!(
        "json_extract(kind,'$.kind') IN ({}) AND (job IS NULL OR NOT {RETAINED_HISTORY})",
        quoted(&AUTHORITY_KINDS)
    )
}
/// The oldest prunable authority events.
fn authority_candidates() -> String {
    format!(
        "SELECT cursor FROM events WHERE {} ORDER BY cursor LIMIT ?",
        prunable_authority()
    )
}

/// Removes the oldest `limit` eligible events, and never more: a job whose
/// history is longer is finished by later passes (see [`RETAINED_HISTORY`]).
/// The cost of a pass follows the settled jobs and the prunable rows, not
/// the pinned history in front of them.
fn prune_events(tx: &Transaction<'_>, limit: i64, _: i64, _: &mut jobs::Scan) -> Result<Removed> {
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
        return Ok(Removed::default());
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
    Ok(Removed::events(removed))
}

/// Removes the oldest `limit` prunable authority events; only ever called
/// with `limit` at most the prunable count above [`AUTHORITY_BUDGET`], so the
/// newest [`AUTHORITY_BUDGET`] prunable ones always stay.
fn prune_authority(
    tx: &Transaction<'_>,
    limit: i64,
    _: i64,
    _: &mut jobs::Scan,
) -> Result<Removed> {
    let cursors: Vec<i64> = tx
        .prepare(&authority_candidates())
        .map_err(durable)?
        .query_map([limit], |r| r.get(0))
        .map_err(durable)?
        .collect::<std::result::Result<_, _>>()
        .map_err(durable)?;
    if cursors.is_empty() {
        return Ok(Removed::default());
    }
    let removed = tx
        .execute(
            "DELETE FROM events WHERE cursor IN (SELECT value FROM json_each(?))",
            [json(&cursors)?],
        )
        .map_err(durable)?;
    let removed = i64::try_from(removed).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    account(tx, -removed)?;
    Ok(Removed::events(removed))
}

/// Removes the oldest `limit` occurrences the delete trigger permits, keeping
/// each schedule's newest one.
fn prune_occurrences(
    tx: &Transaction<'_>,
    limit: i64,
    _: i64,
    _: &mut jobs::Scan,
) -> Result<Removed> {
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
    Ok(Removed {
        rows: i64::try_from(removed).map_err(|_| Error(ErrorCode::StateNotDurable))?,
        ..Removed::default()
    })
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
    /// A retention pass if a minute has passed since the last one, or `None`
    /// when none was due. Every rule runs, each on its own: a failed batch
    /// is reported in the pass and retried on the next one, and never fails
    /// the caller's work or stops another rule. `wall` is the time in
    /// seconds since the Unix epoch, which job-row retention ages jobs by.
    pub(super) fn maintain(&mut self, now: Instant, wall: i64) -> Option<Pass> {
        if self
            .retained_at
            .is_some_and(|at| now.saturating_duration_since(at) < INTERVAL)
        {
            return None;
        }
        self.retained_at = Some(now);
        let batches = self.prune_each(wall);
        let failed = batches
            .iter()
            .filter_map(|(rule, result)| result.as_ref().err().map(|error| (*rule, error.0)))
            .collect();
        Some(Pass {
            usage: self.audit_usage().ok(),
            failed,
            #[cfg(feature = "test-support")]
            removed: batches
                .iter()
                .filter_map(|(rule, result)| result.as_ref().ok().map(|r| (*rule, r.rows)))
                .collect(),
            #[cfg(feature = "test-support")]
            job_events: batches
                .iter()
                .filter(|(rule, _)| *rule == "jobs")
                .filter_map(|(_, result)| result.as_ref().ok().map(|r| r.events))
                .sum(),
            #[cfg(feature = "test-support")]
            pruned_jobs: batches
                .into_iter()
                .filter_map(|(_, result)| result.ok().map(|r| r.jobs))
                .flatten()
                .collect(),
        })
    }
    /// One batch for every table over its budget, each in its own transaction,
    /// whatever the others did. What each removed or its error, per rule.
    fn prune_each(&mut self, wall: i64) -> Vec<(&'static str, Result<Removed>)> {
        RULES
            .iter()
            .map(|rule| {
                #[cfg(test)]
                if self.failing_rule == Some(rule.table) {
                    return (rule.table, Err(Error(ErrorCode::StateNotDurable)));
                }
                (
                    rule.table,
                    run(&mut self.conn, rule, wall, &mut self.job_scan),
                )
            })
            .collect()
    }
    /// [`Self::prune_each`] on the system clock, with the rows removed per
    /// table (omitting tables with none), or the first error once every rule
    /// has run.
    #[cfg(test)]
    pub(super) fn prune(&mut self) -> Result<Vec<(&'static str, i64)>> {
        self.prune_at(crate::writer_clock::unix_now())
    }
    /// [`Self::prune`] at `wall` (seconds since the Unix epoch). A job-row
    /// batch that removed only part of one job's history counts its events.
    #[cfg(test)]
    pub(super) fn prune_at(&mut self, wall: i64) -> Result<Vec<(&'static str, i64)>> {
        let mut removed = Vec::new();
        let mut failed = None;
        for (table, result) in self.prune_each(wall) {
            match result {
                Ok(r) if !r.any() => {}
                Ok(r) if r.rows == 0 => removed.push((table, r.events)),
                Ok(r) => removed.push((table, r.rows)),
                Err(error) => {
                    failed.get_or_insert(error);
                }
            }
        }
        failed.map_or(Ok(removed), Err)
    }
}

/// What a retention pass observed: the audit usage after it (`None` if that
/// read failed) and the rules whose batch failed.
pub(super) struct Pass {
    pub(super) usage: Option<AuditUsage>,
    pub(super) failed: Vec<(&'static str, ErrorCode)>,
    /// The rows each rule's batch (one transaction) removed.
    #[cfg(feature = "test-support")]
    pub(super) removed: Vec<(&'static str, i64)>,
    /// The events job-row retention removed with the jobs (or of one job's
    /// long history), besides the events rules' own.
    #[cfg(feature = "test-support")]
    pub(super) job_events: i64,
    /// The jobs job-row retention removed.
    #[cfg(feature = "test-support")]
    pub(super) pruned_jobs: Vec<String>,
}

/// The soak harness's seam (`bins/core/src/soak`), compiled only with the
/// `test-support` feature, which only dev-dependencies enable: a `Database`
/// whose real writer thread runs on simulated time ([`open`], [`SimClock`]),
/// so its own retention schedule is what the soak exercises; the writer's
/// retention entry point, [`Store::maintain`] with its once-a-minute gate, on
/// the caller's clock, for the tests that build a store's state directly;
/// and the rules' limits, so the harness asserts against these constants
/// rather than copies of them.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    use super::{AuditUsage, ErrorCode, Instant, RULES, Store};
    pub use crate::writer_clock::SimClock;
    /// `Database::open`, with the writer thread on `clock`: its startup pass,
    /// and one whenever `clock` has advanced a minute past the last, exactly
    /// as the system clock drives it in production. Every pass is reported
    /// to `clock` ([`SimClock::take_passes`]).
    pub fn open(
        path: &std::path::Path,
        clock: &std::sync::Arc<SimClock>,
    ) -> crate::Result<crate::Database> {
        crate::assert_test_support_allowed("the simulated writer clock");
        crate::Database::open_with(path, crate::writer_clock::Clock::Simulated(clock.clone()))
    }
    /// Bytes `meta.audit_bytes` counts per row of `events`.
    pub const EVENT_BYTES: i64 = super::EVENT_BYTES;
    /// How far past the audit cap revocations and startup recovery may write.
    pub const AUDIT_RESERVE: i64 = super::AUDIT_RESERVE;
    /// How long a settled job is kept after it settled, in seconds, and
    /// how many of the newest settled jobs of each family are kept whatever
    /// their age (`retention/jobs.rs`).
    pub const JOB_HORIZON: i64 = super::jobs::HORIZON;
    pub const JOB_FLOOR: i64 = super::jobs::FLOOR;
    /// Every rule: its name, the rows it keeps and the most one batch removes.
    pub fn rules() -> Vec<(&'static str, i64, i64)> {
        RULES
            .iter()
            .map(|rule| (rule.table, rule.budget, rule.batch))
            .collect()
    }
    /// What one pass did: the audit usage after it, the rules whose batch
    /// failed, the rows each rule's batch removed, and the events and jobs
    /// job-row retention removed.
    #[derive(Clone, Debug)]
    pub struct Pass {
        pub usage: Option<AuditUsage>,
        pub failed: Vec<(&'static str, ErrorCode)>,
        pub removed: Vec<(&'static str, i64)>,
        pub job_events: i64,
        pub pruned_jobs: Vec<String>,
    }
    thread_local! {
        static OPEN_HOOK: std::cell::Cell<Option<fn(&rusqlite::Connection)>> =
            const { std::cell::Cell::new(None) };
    }
    /// Every `Store::open` on this thread from now on hands its connection
    /// to `hook` as soon as it exists, before any statement of startup runs:
    /// the soak installs a statement trace there, so it measures what the
    /// open itself executes.
    pub fn on_open(hook: Option<fn(&rusqlite::Connection)>) {
        crate::assert_test_support_allowed("the open hook");
        OPEN_HOOK.with(|h| h.set(hook));
    }
    pub(crate) fn opened(conn: &rusqlite::Connection) {
        if let Some(hook) = OPEN_HOOK.with(std::cell::Cell::get) {
            hook(conn);
        }
    }
    /// The writer thread's `maintain(now, wall)`: `None` when the last pass
    /// was less than a minute before `now`; `wall` is the same moment in
    /// seconds since the Unix epoch.
    pub fn maintain(store: &mut Store, now: Instant, wall: i64) -> Option<Pass> {
        store.maintain(now, wall).map(|pass| Pass {
            usage: pass.usage,
            failed: pass.failed,
            removed: pass.removed,
            job_events: pass.job_events,
            pruned_jobs: pass.pruned_jobs,
        })
    }
}

/// The latest failed retention batch, for the daemon's log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionFailure {
    /// The rule (table) whose batch failed.
    pub rule: &'static str,
    pub code: ErrorCode,
    /// Failed batches since the store opened, across all rules.
    pub count: u64,
}

mod jobs;
pub(crate) use jobs::Scan as JobScan;
#[cfg(test)]
mod tests;
