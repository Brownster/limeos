//! Job-row retention: settled jobs, with every row that names them, once
//! nothing needs them.
//!
//! Without it the per-job tables (`jobs`, `job_resources`, `container_plans`,
//! `container_results`, `stack_plans`, `stack_results`, `job_resolutions`)
//! only grew: about 4.7 KiB a job, 46 to 49 KiB a day at household rates in
//! the soak, past core.sqlite's 80 MiB bound within three years, and every
//! page of them read at every startup (`quick_check`, the foreign-key check,
//! the startup scans of `jobs`).
//!
//! # The rule
//!
//! A job is removed only when all of these hold (`CANDIDATES`):
//!
//! 1. It is settled: `succeeded`, `failed`, `canceled`,
//!    `precondition_changed` or `resolved`. Every other state is unfinished
//!    work or an unresolved in-doubt action (`running`, `verifying`,
//!    `outcome_unknown`, `needs_intervention` hold its resource locks and are
//!    what core's recovery and the operator's resolution read; `planned`,
//!    `waiting_approval` and `queued` may still be dispatched). No resource
//!    lock names it either (the release trigger removes them when a job
//!    settles; checked all the same).
//! 2. Its operation is one this rule knows ([`FAMILIES`]): a container or
//!    stack action, or a health probe. The others stay, whatever their state,
//!    because a startup check or a delete trigger reads their rows:
//!    `automation_observe_health` (results, occurrences and poll origins no
//!    trigger lets anyone delete, and the completion, automation and polling
//!    checks), `storage_mount` (its plan's delete trigger and its proof
//!    check), `storage_prepare_targets` and `backup_prepare_private` (their
//!    startup checks), and any operation added later, which fails safe. Every
//!    other table that can name a job is checked too ([`PROTECTING`]): any
//!    row there keeps the job.
//! 3. No resolution request is open for it (`confirmation IS NULL`): the
//!    resolution retry loop may still complete or supersede it.
//! 4. No authority event names it (`job_resolution_requested`, `_withdrawn`,
//!    `_superseded`, `job_canceled_by_administrator`): those are the only
//!    record of what an operator declared or which one cancelled the job
//!    and are kept against their own budget (`AUTHORITY_BUDGET`), pinned
//!    while the job's history lasts. The job stays until the events rule has
//!    taken its history and the authority rule those events; until then it
//!    is a snapshot like any other job whose history is gone.
//! 5. It settled more than [`HORIZON`] ago (`jobs.settled`, set by the
//!    statement that settled it and never changed after), and every other
//!    time stored with it is older too: its plan's expiry (`deadline`), every
//!    event it still has, its result's recording and its resolution's
//!    completion. The settlement time is what holds the promise: a job's
//!    events cannot, since the events rule removes a settled job's events
//!    whatever their age (oldest first, under pressure), and a cancellation,
//!    a probe's outcome or an execute never sent leaves no other dated row,
//!    so a job queued long ago and settled now would otherwise go at once.
//! 6. It is not among the [`FLOOR`] newest settled jobs of its family
//!    (container actions, stack actions, health probes, by settlement time,
//!    then insertion), so a quiet household's job history is never emptied.
//!    The floor is per family across every user, not per administrator: each
//!    listing (`container_jobs`, `stack_jobs`) shows one administrator's 32
//!    newest, and the floor keeps four such pages of the family as a whole,
//!    so an administrator whose jobs all settled before 128 newer ones of
//!    other administrators can find their listing empty once the horizon
//!    has passed.
//!
//! The rule reads the writer's wall clock. A clock set back keeps jobs
//! longer, which is safe. A clock set far forward makes every settled job
//! past the floor look past the horizon, and the next passes remove them
//! early (the floor still stays). Nothing guards against it: the store has
//! no trusted time to compare with (no other retention rule ages rows by
//! time, so none has such a guard), and refusing to prune while the clock is
//! far beyond the newest stored time would hold only until the first event
//! written under the forward clock, a minute at most.
//!
//! What goes with it, in one transaction: the job row, its events (its own
//! settled history: the events rule would remove them anyway, oldest first,
//! once over its budget; rule 4 leaves no authority event among them), its
//! resource set, its plan (`container_plans` or `stack_plans`, which name it
//! once consumed), its result and its completed resolution. Nothing that
//! names a job outlives it, so nothing dangles: a pruned job is simply
//! unknown. Every per-job read (`container_job`, `stack_job` and their
//! events, the resolution views) answers `NotFound` for it, as for an ID
//! never issued; the listings show the jobs that remain; a replayed approval
//! finds its plan gone (`NotFound`) and cannot queue the plan again. Job IDs
//! are random (`limeos_identity::opaque`), so none is ever reused.
//!
//! The executor's receipts are no reference: they are keyed by the job's ID
//! (its action), but core asks the executor about a job only while it holds
//! it (the recovery candidates are queued and held jobs) or has a resolution
//! request open for it, both excluded above, and it never looks a job up
//! from a receipt. The executor keeps a receipt at least
//! `RECEIPT_RETAIN_AFTER_EXPIRY` (an hour) past its plan's expiry; the
//! horizon is 720 times that.
//!
//! # Batches
//!
//! The writer thread runs one batch a pass (at most once a minute) while
//! some family has a settled job past its floor ([`past_floor`]; jobs of
//! other operations never count), oldest settlement first. A batch removes
//! at most [`BATCH`] jobs and at most [`ROWS`] rows in all (events, resources,
//! plans, results, resolutions and job rows), the scale of the events rule's
//! 1,024-row batch, so no batch stalls the writer longer than an events
//! batch does. A job with more history than fits in a batch on its own (a
//! long health-probe history) loses its oldest [`EVENT_BATCH`] events a
//! batch, its `job_queued` event first, so readers see its history as gone
//! from the first batch, exactly as the events rule leaves it; the job row
//! and the rest go in the batch where they fit. Each batch commits on its
//! own: a crash leaves the store as the last committed batch did, which the
//! startup checks accept.
//!
//! Selection is bounded too, whatever the store holds: the count reads at
//! most `FLOOR + BATCH` entries of `jobs_retention` per family, the floor
//! `FLOOR`, and a batch's candidates at most [`SCAN`] per family, from where
//! that family's last batch stopped ([`Scan`]), so a job a reference keeps
//! is read once per [`RESCAN`] (an hour), not every minute. Each window job
//! costs a dozen key lookups and a walk of its own events (`events_job`), as
//! the events rule's own check does. So a pass costs at most about
//! `3 * (FLOOR + BATCH + FLOOR + SCAN)` index entries (2,688), `3 * SCAN`
//! (768) jobs' protection checks and the deletion of `ROWS` rows: the scale
//! of an events batch. Nothing in it grows with jobs of other operations,
//! with kept jobs beyond the window, or with the table.
//!
//! The schema holds the same line (`migration-v19.sql`): `jobs_delete`
//! refuses to delete an unsettled job, one of another operation or one with
//! an open request; a resource set and a completed resolution can be deleted
//! only once their job row is gone. The batch defers foreign keys to its
//! commit, deletes the job rows first and then their dependents, and its
//! commit fails if anything still names a deleted job.
use super::*;

/// How long a settled job is kept after it settled: 30 days. The same
/// span as `metrics.sqlite`'s 31-day history, so a job stays beside the
/// observations around it for as long as those exist. At household rates
/// (about 10 jobs a day) that is about 300 jobs, 1.4 MiB; a day of scripted
/// proposals at the rate the soak's storm uses (about 1,100 jobs) adds about
/// 5 MiB for 30 days, beside the events table's 40 MiB at its budget.
pub(super) const HORIZON: i64 = 30 * 24 * 3600;
/// Settled jobs of each family kept whatever their age: four listing pages
/// of 32 (of the family as a whole, across every user). No batch runs while
/// no family has more settled jobs than this.
pub(super) const FLOOR: i64 = 128;
/// Most jobs one batch removes.
pub(super) const BATCH: i64 = 64;
/// Most rows one batch removes, every table together.
const ROWS: i64 = 2048;

const SETTLED_STATES: &str = "('succeeded','failed','canceled','precondition_changed','resolved')";
/// The operations this rule removes jobs of, by family: the floor keeps the
/// newest of each. Mirrored by the `jobs_delete` trigger.
pub(super) const FAMILIES: [&str; 3] = [
    "('container_restart','container_start','container_stop')",
    "('stack_restart','stack_start','stack_stop')",
    "('health_probe')",
];
/// Tables whose rows name a job and go with it, each by its `job` column,
/// in the order deleted (after the job row, foreign keys deferred).
pub(super) const DEPENDENTS: [&str; 7] = [
    "events",
    "container_results",
    "stack_results",
    "container_plans",
    "stack_plans",
    "job_resolutions",
    "job_resources",
];
/// Tables whose rows name a job (or its resource set) and keep it: any row
/// here and the job is not removed.
pub(super) const PROTECTING: [&str; 9] = [
    "resource_locks",
    "automation_occurrences",
    "automation_observation_results",
    "automation_poll_origins",
    "storage_mount_plans",
    "storage_mount_results",
    "storage_target_plans",
    "storage_target_results",
    "backup_preparation_plans",
];

/// Settled jobs examined at most per family in one batch: the candidate
/// window, from where the family's last batch stopped.
const SCAN: i64 = 4 * BATCH;
/// How often each family's scan starts again from its oldest settled job
/// (seconds), so a job a reference kept is looked at again once that
/// reference is gone: an hour, well inside the soak's one-day allowance.
const RESCAN: i64 = 3600;

/// A job's family, as an expression over `jobs.intent`: 0, 1 or 2 by
/// [`FAMILIES`], NULL for every other operation. Exactly the expression of
/// the `jobs_retention` index (`migration-v19.sql`), which the planner
/// matches by it.
pub(super) fn family() -> String {
    let cases: String = FAMILIES
        .iter()
        .enumerate()
        .map(|(n, operations)| {
            format!(" WHEN json_extract(intent,'$.operation') IN {operations} THEN {n}")
        })
        .collect();
    format!("CASE{cases} END")
}

/// Where each family's scan resumes: the settlement time and rowid of the
/// last job a batch looked at and left (a reference kept it) or removed,
/// and when every family starts again from its oldest. Held by the writer's
/// store, in memory: a restart starts again from the oldest.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    after: [Option<(i64, i64)>; FAMILIES.len()],
    restart: Option<i64>,
}

/// The rule's row count: settled jobs past their family's floor, at most
/// [`BATCH`] counted per family, never another operation's jobs. Each family
/// is read through `jobs_retention`, newest first, at most `FLOOR + BATCH`
/// entries. Its budget is 0: a batch runs only while some family has a job
/// past its floor.
pub(super) fn past_floor(conn: &Connection) -> Result<i64> {
    let mut statement = conn.prepare_cached(&past_floor_query()).map_err(durable)?;
    let mut total = 0;
    for n in 0..FAMILIES.len() {
        let past: i64 = statement
            .query_row(params![n as i64, BATCH, FLOOR], |r| r.get(0))
            .map_err(durable)?;
        total += past;
    }
    Ok(total)
}

/// The count [`past_floor`] reads, per family (`?1`): `LIMIT ?2 OFFSET ?3`.
pub(super) fn past_floor_query() -> String {
    format!(
        "SELECT count(*) FROM (SELECT 1 FROM jobs WHERE {}=?1 AND settled IS NOT NULL ORDER BY settled DESC,rowid DESC LIMIT ?2 OFFSET ?3)",
        family()
    )
}

/// A family's floor: the settlement time and rowid of its `FLOOR`th newest
/// settled job (by settlement), or `None` while it has no more than `FLOOR`.
/// `FLOOR` entries of `jobs_retention`.
fn floor(tx: &Transaction<'_>, n: usize) -> Result<Option<(i64, i64)>> {
    tx.prepare_cached(&floor_query())
        .map_err(durable)?
        .query_row(params![n as i64, FLOOR - 1], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(durable)
}

/// The floor of family `?1`: `OFFSET ?2`.
pub(super) fn floor_query() -> String {
    format!(
        "SELECT settled,rowid FROM jobs WHERE {}=?1 AND settled IS NOT NULL ORDER BY settled DESC,rowid DESC LIMIT 1 OFFSET ?2",
        family()
    )
}

/// One family's candidate window: its settled jobs past the horizon
/// (`settled<?2`), after where its scan resumes and before its floor (`?5`,
/// `?6`), oldest settlement first, at most `?7` of them, each with its
/// settlement time, rowid, ID and whether rules 1 to 5 let it go. In two
/// parts, since the planner bounds an index range by `settled` alone, not by
/// a row value with the rowid: the rest of the resumed settlement time
/// (`tie`: `settled=?3 AND rowid>?4`), then every later one (`settled>?3`).
/// Many jobs share a settlement time only after the upgrade (every job
/// already settled took the upgrade's), and each part is a range that
/// starts exactly where the scan resumed. Read through `jobs_retention`
/// (the family, the settlement time, then the rowid), each job's events
/// through `events_job` and everything else by its primary or unique key:
/// its cost is the window's jobs, never the family's or the table's.
pub(super) fn window(tie: bool) -> String {
    let protecting = PROTECTING
        .iter()
        .map(|table| format!(" AND NOT EXISTS(SELECT 1 FROM {table} x WHERE x.job=j.id)"))
        .collect::<String>();
    format!(
        "SELECT j.settled,j.rowid,j.id,j.state IN {SETTLED_STATES} AND j.deadline<?2 AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND (r.confirmation IS NULL OR r.completed>=?2)) AND NOT EXISTS(SELECT 1 FROM events e INDEXED BY events_job WHERE e.job=j.id AND (e.created>=?2 OR json_extract(e.kind,'$.kind') IN ({}))) AND NOT EXISTS(SELECT 1 FROM container_results x WHERE x.job=j.id AND x.recorded>=?2) AND NOT EXISTS(SELECT 1 FROM stack_results x WHERE x.job=j.id AND x.recorded>=?2){protecting} FROM jobs j WHERE {}=?1 AND j.settled<?2 AND {} AND (j.settled,j.rowid)<(?5,?6) ORDER BY j.settled,j.rowid LIMIT ?7",
        quoted(&AUTHORITY_KINDS),
        family(),
        if tie {
            "j.settled=?3 AND j.rowid>?4"
        } else {
            "j.settled>?3 AND ?4 IS NOT NULL"
        }
    )
}

/// One batch at `now`: at most `limit` (and [`BATCH`]) jobs, at most [`ROWS`]
/// rows, or the oldest events of one job too long for a batch.
///
/// Each family with more than `FLOOR` settled jobs has its floor read and
/// its window (at most [`SCAN`] jobs from where its scan resumes); the
/// windows are taken together, oldest settlement first. A job rules 1 to 5
/// let go is removed while the batch has room; a job a reference keeps is
/// passed over, and its family's scan resumes after it, so a later batch
/// does not read it again until the scan starts over, every [`RESCAN`]
/// seconds (or when the clock has gone back past the last start). A job past
/// the horizon but inside the floor, or not yet past the horizon, sorts after
/// every job a window can hold, so nothing is passed over that a later batch
/// should remove. A batch that rolls back after passing over jobs only
/// delays them to the next start.
pub(super) fn prune(
    tx: &Transaction<'_>,
    limit: i64,
    now: i64,
    scan: &mut Scan,
) -> Result<Removed> {
    let cutoff = now.saturating_sub(HORIZON);
    if scan
        .restart
        .is_none_or(|at| now >= at || now < at.saturating_sub(RESCAN))
    {
        *scan = Scan {
            restart: Some(now.saturating_add(RESCAN)),
            ..Scan::default()
        };
    }
    let mut found: Vec<((i64, i64), String, bool, usize)> = Vec::new();
    for n in 0..FAMILIES.len() {
        let Some(floor) = floor(tx, n)? else {
            continue;
        };
        let after = scan.after[n].unwrap_or((i64::MIN, i64::MIN));
        let mut taken = 0;
        for tie in [true, false] {
            let rows = tx
                .prepare_cached(&window(tie))
                .map_err(durable)?
                .query_map(
                    params![
                        n as i64,
                        cutoff,
                        after.0,
                        after.1,
                        floor.0,
                        floor.1,
                        SCAN - taken
                    ],
                    |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?, r.get(3)?, n)),
                )
                .map_err(durable)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(durable)?;
            taken += rows.len() as i64;
            found.extend(rows);
        }
    }
    found.sort_by_key(|(key, ..)| *key);
    let mut cost = tx
        .prepare_cached("SELECT (SELECT count(*) FROM events INDEXED BY events_job WHERE job=?1),(SELECT count(*) FROM job_resources WHERE job=?1)")
        .map_err(durable)?;
    let mut removed = Removed::default();
    let mut rows = 0;
    let mut whole = Vec::new();
    for (key, id, removable, n) in found {
        if removable {
            let (events, resources): (i64, i64) = cost
                .query_row([&id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(durable)?;
            // The job row, its plan, result and resolution, at most one each.
            let cost = events + resources + 4;
            if (whole.len() as i64) < limit.min(BATCH) && rows + cost <= ROWS {
                rows += cost;
                whole.push(id);
            } else {
                if whole.is_empty() {
                    // Too long for any batch: its oldest events, `job_queued`
                    // first, and the rest on later passes.
                    let gone = tx
                        .execute(
                            "DELETE FROM events WHERE cursor IN (SELECT cursor FROM events INDEXED BY events_job WHERE job=? ORDER BY cursor LIMIT ?)",
                            params![id, EVENT_BATCH],
                        )
                        .map_err(durable)?;
                    let gone =
                        i64::try_from(gone).map_err(|_| Error(ErrorCode::StateNotDurable))?;
                    account(tx, -gone)?;
                    removed.events = gone;
                }
                // Not passed over: the next batch starts with it.
                break;
            }
        }
        scan.after[n] = Some(key);
    }
    if whole.is_empty() {
        return Ok(removed);
    }
    // Checked at commit: the job rows go first, and the commit fails if any
    // row still names one of them.
    tx.pragma_update(None, "defer_foreign_keys", true)
        .map_err(durable)?;
    let ids = json(&whole)?;
    let gone = tx
        .execute(
            "DELETE FROM jobs WHERE id IN (SELECT value FROM json_each(?))",
            [&ids],
        )
        .map_err(durable)?;
    #[cfg(test)]
    super::tests::pause_mid_batch();
    for table in DEPENDENTS {
        let deleted = tx
            .execute(
                &format!("DELETE FROM {table} WHERE job IN (SELECT value FROM json_each(?))"),
                [&ids],
            )
            .map_err(durable)?;
        if table == "events" {
            let deleted = i64::try_from(deleted).map_err(|_| Error(ErrorCode::StateNotDurable))?;
            account(tx, -deleted)?;
            removed.events = deleted;
        }
    }
    removed.rows = i64::try_from(gone).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    removed.jobs = whole;
    Ok(removed)
}

#[cfg(test)]
mod tests;
