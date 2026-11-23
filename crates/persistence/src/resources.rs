use super::*;

/// Missing claims are corruption, not permission to reacquire or forget work.
/// Run before startup recovery changes generation or writes audit events.
#[cfg(test)]
pub(super) fn check(conn: &Connection) -> Result<()> {
    check_migrated(conn, super::migrations::MIGRATIONS)
}
/// `check`, against the schema `migrations` build.
pub(super) fn check_migrated(
    conn: &Connection,
    migrations: &[super::migrations::Migration],
) -> Result<()> {
    // A version number and healthy pages do not prove that dispatch/release
    // guards survived. Compare the compiled migrations' triggers, and the
    // tables and indexes they name, before use.
    super::migrations::check_schema(conn, migrations)?;
    let invalid: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM jobs j WHERE NOT EXISTS(
                    SELECT 1 FROM job_resources r WHERE r.job=j.id AND r.resource=j.resource
                ) OR (SELECT count(*) FROM job_resources r WHERE r.job=j.id)>128
            ) OR EXISTS(
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
            ) OR EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
            [],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if invalid {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    // The required set must also match the closed operation, including queued
    // work. A missing dependency is not permission to dispatch after restart.
    let mut stmt = conn.prepare("SELECT id,principal,intent,digest,resource,revision,deadline FROM jobs WHERE json_extract(intent,'$.operation')='storage_prepare_targets'").map_err(durable)?;
    for row in stmt
        .query_map([], |r| {
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
    super::storage_mounts::check(conn)?;
    super::backup_preparation::check(conn)?;
    super::automation::check(conn)
}

#[cfg(test)]
mod tests;
