use super::*;

/// Missing claims are corruption, not permission to reacquire or forget work.
/// Run before startup recovery changes generation or writes audit events.
pub(super) fn check(conn: &Connection) -> Result<()> {
    // A version number and healthy pages do not prove that dispatch/release
    // guards survived. Compare the compiled migration's triggers before use.
    let reference = Connection::open_in_memory().map_err(durable)?;
    reference
        .execute_batch(concat!(
            include_str!("schema.sql"),
            include_str!("migration-v2.sql"),
            include_str!("migration-v3.sql"),
            include_str!("migration-v4.sql"),
            include_str!("migration-v5.sql"),
            include_str!("migration-v6.sql"),
            include_str!("migration-v7.sql"),
            include_str!("migration-v8.sql")
        ))
        .map_err(durable)?;
    let mut stmt = reference
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger'")
        .map_err(durable)?;
    let triggers = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(durable)?;
    for trigger in triggers {
        let (name, expected) = trigger.map_err(durable)?;
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(durable)?;
        if actual.as_ref() != Some(&expected) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    for name in ["storage_target_plans", "storage_target_results"] {
        let expected: String = reference
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?",
                [name],
                |r| r.get(0),
            )
            .map_err(durable)?;
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(durable)?;
        if actual.as_ref() != Some(&expected) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
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
    Ok(())
}

#[cfg(test)]
mod tests;
