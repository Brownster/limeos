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
//!    `_superseded`): those are the only record of what an operator declared
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
//!    (container actions, stack actions, health probes, by insertion order),
//!    so a quiet household's job history is never emptied: each listing
//!    (`container_jobs`, `stack_jobs`) shows an administrator's 32 newest,
//!    and the floor keeps four such pages of each family.
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
//! The writer thread runs one batch a pass (at most once a minute) when more
//! than [`FLOOR`] jobs are settled, oldest plan expiry first. A batch removes
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
/// of 32. Also the rule's budget: no batch runs while no more jobs than this
/// are settled in all.
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

/// Settled jobs of every operation: the rule's row count. Read through
/// `jobs_state`.
pub(super) fn settled(conn: &Connection) -> Result<i64> {
    conn.query_row(
        &format!("SELECT count(*) FROM jobs WHERE state IN {SETTLED_STATES}"),
        [],
        |r| r.get(0),
    )
    .map_err(durable)
}

/// The oldest removable jobs (rules 1 to 6), with their events, resources
/// and family. Parameters: the horizon's cutoff, each family's floor (the
/// rowid of its `FLOOR`th newest settled job, NULL while it has no more, or
/// the largest rowid while it is not read yet), the limit.
/// Settled jobs past the cutoff are found through `jobs_state`; each one's
/// events through `events_job`, everything else by its primary or unique key.
fn candidates() -> String {
    let families = FAMILIES
        .iter()
        .enumerate()
        .map(|(n, operations)| {
            format!(
                "(json_extract(j.intent,'$.operation') IN {operations} AND j.rowid<?{})",
                n + 2
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let family = format!(
        "CASE {} END",
        FAMILIES
            .iter()
            .enumerate()
            .map(|(n, operations)| format!(
                " WHEN json_extract(j.intent,'$.operation') IN {operations} THEN {n}"
            ))
            .collect::<String>()
    );
    let protecting = PROTECTING
        .iter()
        .map(|table| format!(" AND NOT EXISTS(SELECT 1 FROM {table} x WHERE x.job=j.id)"))
        .collect::<String>();
    let limit = FAMILIES.len() + 2;
    format!(
        "SELECT j.id,(SELECT count(*) FROM events e INDEXED BY events_job WHERE e.job=j.id),(SELECT count(*) FROM job_resources r WHERE r.job=j.id),{family} FROM jobs j WHERE j.state IN {SETTLED_STATES} AND j.settled<?1 AND j.deadline<?1 AND ({families}) AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND (r.confirmation IS NULL OR r.completed>=?1)) AND NOT EXISTS(SELECT 1 FROM events e INDEXED BY events_job WHERE e.job=j.id AND (e.created>=?1 OR json_extract(e.kind,'$.kind') IN ({}))) AND NOT EXISTS(SELECT 1 FROM container_results x WHERE x.job=j.id AND x.recorded>=?1) AND NOT EXISTS(SELECT 1 FROM stack_results x WHERE x.job=j.id AND x.recorded>=?1){protecting} ORDER BY j.deadline,j.rowid LIMIT ?{limit}",
        quoted(&AUTHORITY_KINDS)
    )
}

/// The rowid below which a family's settled jobs are past the floor: that of
/// its `FLOOR`th newest, or `None` while it has no more than `FLOOR`.
fn floor(tx: &Transaction<'_>, operations: &str) -> Result<Option<i64>> {
    tx.query_row(
        &format!(
            "SELECT rowid FROM jobs WHERE state IN {SETTLED_STATES} AND json_extract(intent,'$.operation') IN {operations} ORDER BY rowid DESC LIMIT 1 OFFSET ?"
        ),
        [FLOOR - 1],
        |r| r.get(0),
    )
    .optional()
    .map_err(durable)
}

/// One batch at `now`: at most `limit` (and [`BATCH`]) jobs, at most [`ROWS`]
/// rows, or the oldest events of one job too long for a batch.
///
/// A family's floor is read (a walk of `jobs` from its newest row until the
/// floor's worth of that family) only once that family has a job past the
/// horizon: the candidates are read with no floor for a family whose floor
/// is not known yet, then again with the floor of every family among them,
/// until no new family appears (at most once per family more). Every job
/// returned in the end is past its family's floor; a pass with nothing past
/// the horizon reads no floor at all.
pub(super) fn prune(tx: &Transaction<'_>, limit: i64, now: i64) -> Result<Removed> {
    let cutoff = now.saturating_sub(HORIZON);
    let sql = candidates();
    let mut statement = tx.prepare_cached(&sql).map_err(durable)?;
    // Per family: not read yet (`None`), or its floor (`Some(None)` while it
    // has no more than `FLOOR` settled jobs, so none of them goes).
    let mut floors: [Option<Option<i64>>; FAMILIES.len()] = [None; FAMILIES.len()];
    let found = loop {
        let mut values: Vec<rusqlite::types::Value> = vec![cutoff.into()];
        values.extend(floors.iter().map(|floor| match floor {
            None => i64::MAX.into(),
            Some(floor) => rusqlite::types::Value::from(*floor),
        }));
        values.push(limit.min(BATCH).into());
        let found: Vec<(String, i64, i64, usize)> = statement
            .query_map(rusqlite::params_from_iter(values), |r| {
                let family: i64 = r.get(3)?;
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    usize::try_from(family).unwrap_or(usize::MAX),
                ))
            })
            .map_err(durable)?
            .collect::<std::result::Result<_, _>>()
            .map_err(durable)?;
        let mut unread = false;
        for (_, _, _, family) in &found {
            if floors
                .get(*family)
                .ok_or(Error(ErrorCode::StateNotDurable))?
                .is_none()
            {
                floors[*family] = Some(floor(tx, FAMILIES[*family])?);
                unread = true;
            }
        }
        if !unread {
            break found;
        }
    };
    let mut removed = Removed::default();
    let mut rows = 0;
    let mut whole = Vec::new();
    for (id, events, resources, _) in found {
        // The job row, its plan, result and resolution, at most one each.
        let cost = events + resources + 4;
        if rows + cost <= ROWS {
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
                let gone = i64::try_from(gone).map_err(|_| Error(ErrorCode::StateNotDurable))?;
                account(tx, -gone)?;
                removed.events = gone;
            }
            break;
        }
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
