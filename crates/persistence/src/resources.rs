use super::*;

/// Missing claims are corruption, not permission to reacquire or forget work.
/// Run before startup recovery changes generation or writes audit events.
#[cfg(test)]
pub(super) fn check(conn: &Connection) -> Result<()> {
    super::migrations::check_schema(conn, super::migrations::MIGRATIONS)?;
    check_data(conn)
}
/// The operations whose jobs an owner's startup check examines one by one:
/// storage target preparation (here), storage mounts, backup preparation
/// and automation observation. Job-row retention never removes their jobs
/// (`retention/jobs.rs`).
const CHECKED_OPERATIONS: [&str; 4] = [
    "storage_prepare_targets",
    "storage_mount",
    "backup_prepare_private",
    "automation_observe_health",
];

/// Startup's one walk of `jobs`: besides the whole-file checks
/// (`quick_check`, the foreign-key check), the only statement of the open
/// that reads every job row. It returns every row, as the table holds it:
///
/// - for the jobs of each of [`CHECKED_OPERATIONS`], listed so that the
///   owners' checks read only those jobs, each through its primary key,
///   instead of each walking `jobs` again to find them (eight walks of every
///   job, settled or not, until 24 November);
/// - for every job, the invariant its resource set keeps while the job
///   exists (the `jobs_primary_resource`, `job_resources_*` and
///   `jobs_identity_immutable` triggers): its primary resource is in the
///   set, and the set has at most 128 resources (the last column);
/// - and for the indexes of `jobs` that locks, recovery and dispatch read
///   through, which `accounting` holds to these rows: the id, idempotency
///   key, state, deadline and resource of every job.
const JOB_WALK: &str = "SELECT j.id,j.rowid,j.principal,j.idempotency,j.state,j.deadline,j.resource,json_extract(j.intent,'$.operation'),NOT EXISTS(SELECT 1 FROM job_resources r WHERE r.job=j.id AND r.resource=j.resource) OR (SELECT count(*) FROM job_resources r WHERE r.job=j.id)>128 FROM jobs j";

/// The listed jobs (`?1`, a JSON array of `[id, rowid, operation]`) that
/// the owners' access path finds: through the primary key's index, as every
/// owner's statement reads them, at the rowid the walk read each at, with
/// the operation it read. [`Walked::read`] refuses the store unless this
/// counts every listed job.
///
/// The walk reads the table; the owners read through the index
/// (`sqlite_autoindex_jobs_1`), and `quick_check` only compares the two by
/// their counts (`accounting`). A job row the index lacks (or files under
/// another id, with the count kept) would otherwise be listed by the walk
/// and then read by no owner, so checked by none: the eight walks this
/// replaced read the table and refused it. `accounting` now holds the whole
/// index to the table; this stays, exact where that is a hash, for the rows
/// the owners read. An
/// index entry pointing at another row is refused the same way (its rowid
/// differs from the walk's), and one pointing at no row fails the statement
/// (SQLite reports the store corrupt). The `+` keeps the rowid out of the
/// lookup, so it is the index entry's rowid that is compared, not used.
const LISTED_BY_KEY: &str = "SELECT count(*) FROM json_each(?1) w CROSS JOIN jobs j ON j.id=json_extract(w.value,'$[0]') WHERE +j.rowid=json_extract(w.value,'$[1]') AND json_extract(j.intent,'$.operation')=json_extract(w.value,'$[2]')";

/// The lock invariants: every resource of a job that holds locks
/// (`running`, `verifying`, `outcome_unknown`, `needs_intervention`) is
/// locked by it, and no other job holds a lock; a job is `resolved` exactly
/// when its resolution is completed; and every foreign key holds. Each part
/// starts from the jobs in those states (`jobs_state`), from the locks or
/// from the resolutions, never from every job: the walk of every job is
/// [`JOB_WALK`]'s. Every index they read through has been held to its
/// table's content by then (`accounting`): `jobs_state`, the primary keys of
/// `jobs` and of the unsettled jobs' `job_resources`, and the lock tables'.
const LOCK_INVARIANTS: &str = "SELECT EXISTS(
    SELECT 1 FROM job_resources r JOIN jobs j ON j.id=r.job
    LEFT JOIN resource_locks l ON l.job=r.job AND l.resource=r.resource
    WHERE j.state IN ('running','verifying','outcome_unknown','needs_intervention')
        AND l.resource IS NULL
) OR EXISTS(
    SELECT 1 FROM resource_locks l JOIN jobs j ON j.id=l.job
    WHERE j.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')
) OR EXISTS(
    SELECT 1 FROM jobs j WHERE j.state='resolved' AND NOT EXISTS(
        SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NOT NULL)
) OR EXISTS(
    SELECT 1 FROM job_resolutions r JOIN jobs j ON j.id=r.job
    WHERE (r.confirmation IS NOT NULL) <> (j.state='resolved')
) OR EXISTS(SELECT 1 FROM pragma_foreign_key_check)";

/// What [`JOB_WALK`] listed: the ids of each checked operation's jobs, in
/// the order of [`CHECKED_OPERATIONS`]. Read in the startup transaction, so
/// they are every job of those operations the checks after it can see, and
/// each found by key at the row the walk read ([`LISTED_BY_KEY`]), so every
/// owner's statement, reading by key, reads every one of those rows.
pub(super) struct Walked {
    jobs: [Vec<String>; CHECKED_OPERATIONS.len()],
}
impl Walked {
    pub(super) fn read(conn: &Connection) -> Result<Self> {
        let mut jobs: [Vec<String>; CHECKED_OPERATIONS.len()] = Default::default();
        let mut keyed = Vec::new();
        let mut accounts = accounting::Accounts::new();
        let mut statement = conn.prepare(JOB_WALK).map_err(durable)?;
        let mut rows = statement.query([]).map_err(durable)?;
        while let Some(row) = rows.next().map_err(durable)? {
            let broken: bool = row.get(8).map_err(durable)?;
            if broken {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            accounts.row(row)?;
            let operation: Option<String> = row.get(7).map_err(durable)?;
            let Some(listed) = CHECKED_OPERATIONS
                .iter()
                .position(|o| Some(*o) == operation.as_deref())
            else {
                continue;
            };
            let id: String = row.get(0).map_err(durable)?;
            let rowid: i64 = row.get(1).map_err(durable)?;
            keyed.push(serde_json::json!([id, rowid, CHECKED_OPERATIONS[listed]]));
            jobs[listed].push(id);
        }
        drop(rows);
        accounts.verify(conn)?;
        let found: i64 = conn
            .query_row(LISTED_BY_KEY, [json(&keyed)?], |r| r.get(0))
            .map_err(durable)?;
        if usize::try_from(found).ok() != Some(keyed.len()) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(Self { jobs })
    }
    /// The ids of `operation`'s jobs as a JSON array, for a check's
    /// `json_each(?)`. `operation` is one of [`CHECKED_OPERATIONS`].
    pub(super) fn ids(&self, operation: &str) -> Result<String> {
        let listed = CHECKED_OPERATIONS
            .iter()
            .position(|o| *o == operation)
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        json(&self.jobs[listed])
    }
}

/// What `check` checks beyond the schema (which `Store::open` compares
/// itself, to name a mismatch): job, resource and lock invariants and each
/// owner's own checks.
pub(super) fn check_data(conn: &Connection) -> Result<()> {
    let walked = Walked::read(conn)?;
    let invalid: bool = conn
        .query_row(LOCK_INVARIANTS, [], |r| r.get(0))
        .map_err(durable)?;
    if invalid {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    // The required set must also match the closed operation, including queued
    // work. A missing dependency is not permission to dispatch after restart.
    let mut stmt = conn.prepare(STORAGE_TARGET_JOBS).map_err(durable)?;
    for row in stmt
        .query_map([walked.ids("storage_prepare_targets")?], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(durable)?
    {
        let (id, principal, intent, digest, resource, revision, deadline) = row.map_err(durable)?;
        let Intent::StoragePrepareTargets { plan } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        plan.validate(plan.preparation.created_at)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let mut required = conn
            .prepare("SELECT resource FROM job_resources WHERE job=? ORDER BY resource")
            .map_err(durable)?;
        let resources = required
            .query_map([&id], |r| r.get::<_, String>(0))
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        let approved: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM storage_target_plans WHERE id=? AND principal=? AND body=? AND digest=? AND revision=? AND expires=? AND job=? AND canceled=0 AND approval_digest IS NOT NULL)",params![plan.id,principal,json(&plan)?,limeos_identity::digest(&json(&plan)?),revision,deadline,id],|r|r.get(0)).map_err(durable)?;
        if plan.preparation.action != id
            || plan.principal != principal
            || plan.grant_revision != revision
            || plan.preparation.expires_at != deadline
            || resource != "storage:configuration"
            || digest != limeos_identity::digest(&intent)
            || resources != plan.preparation.resources()
            || !approved
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    super::storage_mounts::check(conn, &walked)?;
    super::backup_preparation::check(conn, &walked)?;
    super::automation::check(conn, &walked)
}

/// The storage target preparation jobs [`JOB_WALK`] listed (`?1`), each
/// read through the primary key.
const STORAGE_TARGET_JOBS: &str = "SELECT j.id,j.principal,j.intent,j.digest,j.resource,j.revision,j.deadline FROM json_each(?1) w CROSS JOIN jobs j ON j.id=w.value WHERE json_extract(j.intent,'$.operation')='storage_prepare_targets'";

/// Every startup statement that reads the jobs [`JOB_WALK`] listed, with
/// the operation whose ids it takes as `?1`, for the test that checks none
/// of them walks `jobs`.
#[cfg(test)]
pub(crate) fn listed_job_queries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("storage_prepare_targets", STORAGE_TARGET_JOBS),
        ("storage_mount", super::storage_mounts::MOUNT_JOBS),
        (
            "backup_prepare_private",
            super::backup_preparation::BACKUP_JOBS,
        ),
        (
            "backup_prepare_private",
            super::backup_preparation::UNLINKED_BACKUP_JOBS,
        ),
        (
            "automation_observe_health",
            super::automation::OBSERVATION_JOBS,
        ),
        (
            "automation_observe_health",
            super::automation::OBSERVATION_RESULTS,
        ),
    ]
}

mod accounting;
#[cfg(test)]
mod tests;
