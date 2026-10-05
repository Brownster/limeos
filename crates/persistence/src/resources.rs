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
            include_str!("migration-v7.sql")
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
    Ok(())
}

#[cfg(test)]
mod tests;
