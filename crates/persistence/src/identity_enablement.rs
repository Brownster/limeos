//! Fixed self-enablement only; password verification remains in Core.
use super::*;
use std::time::Instant;

const RESOURCE: &str = "automation:observe-health";

fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(())
}

fn record(conn: &Connection, session: &str, csrf: &str, now: i64) -> Result<LoginRecord> {
    let principal = identity_store::authenticate(conn, session, Some(csrf), now)?;
    let grants = identity_store::current_grants(conn, &principal)?;
    for (operation, resource) in [
        (Operation::IdentityManage, RESOURCE),
        (Operation::HealthRead, "host:system"),
    ] {
        limeos_policy::authorize(
            &principal,
            &grants,
            &Scope {
                operation,
                resource: resource.into(),
            },
        )?;
    }
    let password_hash = conn
        .query_row(
            "SELECT password_hash FROM users WHERE id=?",
            [&principal.id],
            |r| r.get(0),
        )
        .map_err(durable)?;
    Ok(LoginRecord {
        principal,
        password_hash,
    })
}

fn checked_record(
    conn: &Connection,
    session: &str,
    csrf: &str,
    expected: &LoginRecord,
    now: i64,
) -> Result<LoginRecord> {
    let current = record(conn, session, csrf, now)?;
    if current.principal != expected.principal || current.password_hash != expected.password_hash {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(current)
}

fn enabled(conn: &Connection, principal: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM grants WHERE principal=? AND operation=? AND resource=?)",
        params![principal, json(&Operation::AutomationManage)?, RESOURCE],
        |r| r.get(0),
    )
    .map_err(durable)
}

impl Store {
    /// Authenticated stored CAS expectation for Core's existing password worker.
    /// A caller cannot choose another subject or provision a different grant.
    pub fn observe_health_reauthentication(
        &self,
        session: &str,
        csrf: &str,
        now: i64,
    ) -> Result<LoginRecord> {
        record(&self.conn, session, csrf, now)
    }

    /// Core calls this only after its bounded verifier succeeds. `expected` is
    /// internal stored state, never a request DTO or a password-verification flag.
    pub fn enable_observe_health(
        &mut self,
        session: &str,
        csrf: &str,
        expected: &LoginRecord,
        upgraded: Option<&str>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<Principal> {
        live(deadline)?;
        // A true no-op works as a read snapshot, even at the write/audit cap,
        // and never opportunistically upgrades a password hash.
        let read = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(durable)?;
        // BEGIN DEFERRED alone does not acquire a read snapshot. Pin it before
        // sampling the trusted clock, so lock waits cannot reuse old expiry.
        let _: i64 = read
            .query_row("SELECT generation FROM meta WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(durable)?;
        live(deadline)?;
        let current = checked_record(&read, session, csrf, expected, clock())?;
        let present = enabled(&read, &current.principal.id)?;
        live(deadline)?;
        read.commit().map_err(durable)?;
        if present {
            return Ok(current.principal);
        }
        self.write(|tx| {
            live(deadline)?;
            // Core supplies its own clock function, never a request timestamp.
            // Sample only after Store::write acquired the IMMEDIATE transaction.
            let now = clock();
            let current = checked_record(tx, session, csrf, expected, now)?;
            if enabled(tx, &current.principal.id)? { live(deadline)?; return Ok(current.principal); }
            let count: i64 = tx.query_row("SELECT count(*) FROM grants WHERE principal=?", [&current.principal.id], |r|r.get(0)).map_err(durable)?;
            if count >= 32 { return Err(Error(ErrorCode::Overloaded)); }
            let audit: i64 = tx.query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r|r.get(0)).map_err(durable)?;
            if audit.checked_add(256).is_none_or(|v|v>AUDIT_LIMIT) { return Err(Error(ErrorCode::StateNotDurable)); }
            let revision=current.principal.grant_revision.checked_add(1).filter(|v|*v<=limeos_domain::automation::MAX_REVISION).ok_or(Error(ErrorCode::StateNotDurable))?;
            live(deadline)?;
            if tx.execute("UPDATE users SET password_hash=coalesce(?,password_hash),grant_revision=? WHERE id=? AND password_hash=? AND grant_revision=? AND role=?", params![upgraded,revision,current.principal.id,expected.password_hash,current.principal.grant_revision,json(&current.principal.role)?]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Expired)); }
            tx.execute("INSERT INTO grants VALUES(?,?,?)", params![current.principal.id,json(&Operation::AutomationManage)?,RESOURCE]).map_err(durable)?;
            event(tx,Some(&current.principal.id),None,EventKind::GrantsRevised,now)?;
            live(deadline)?;
            Ok(Principal { grant_revision: revision, ..current.principal })
        })
    }
}

#[cfg(test)]
mod tests;
