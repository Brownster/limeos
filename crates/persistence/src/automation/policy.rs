//! Owner-scoped configured policy on the sole Store transaction boundary.
use super::*;

const MAX_VERSIONS: i64 = 1024;

pub(super) fn at(conn: &Connection, owner: &str, revision: i64) -> Result<HealthPolicyView> {
    if revision == 0 {
        return Ok(HealthPolicyView {
            revision: 0,
            updated: None,
            definition: HealthPolicyDefinition::default(),
        });
    }
    let row:Option<(String,String,i64)>=conn.query_row(
        "SELECT body,digest,updated FROM automation_health_policies WHERE owner=? AND revision=? AND length(CAST(body AS BLOB))<=65536",
        params![owner,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(durable)?;
    let (body, digest, updated) = row.ok_or(Error(ErrorCode::StateNotDurable))?;
    let definition: HealthPolicyDefinition = parse(&body)?;
    let view = HealthPolicyView {
        revision,
        updated: Some(updated),
        definition,
    };
    view.validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if limeos_identity::digest(&body) != digest || json(&view.definition)? != body {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(view)
}
pub(super) fn current(conn: &Connection, owner: &str) -> Result<HealthPolicyView> {
    let revision = conn
        .query_row(
            "SELECT coalesce(max(revision),0) FROM automation_health_policies WHERE owner=?",
            [owner],
            |r| r.get(0),
        )
        .map_err(durable)?;
    at(conn, owner, revision)
}
pub(super) fn revision_before(conn: &Connection, owner: &str, cursor: i64) -> Result<i64> {
    conn.query_row("SELECT coalesce(max(json_extract(kind,'$.revision')),0) FROM events WHERE principal=? AND job IS NULL AND cursor<? AND json_extract(kind,'$.kind')='automation_health_policy_changed'",params![owner,cursor],|r|r.get(0)).map_err(durable)
}
impl Store {
    pub fn observe_health_policy(
        &mut self,
        session: &str,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        reservation_live(Some(deadline))?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(durable)?;
        let _: i64 = tx
            .query_row("SELECT generation FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        reservation_live(Some(deadline))?;
        let now = clock();
        time(now)?;
        let actor = identity_store::authenticate(&tx, session, None, now)?;
        // Mount paths and silence metadata are management configuration, not
        // concrete read scopes. Do not disclose them under host-read alone.
        authorize(&tx, &actor, true)?;
        let value = current(&tx, &actor.id)?;
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        Ok(value)
    }
    pub fn observe_health_policy_replace(
        &mut self,
        session: &str,
        csrf: &str,
        input: &ReplaceHealthPolicy,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        reservation_live(Some(deadline))?;
        input.validate()?;
        self.write(|tx| {
            reservation_live(Some(deadline))?;
            let now = clock();
            time(now)?;
            let actor = identity_store::authenticate(tx, session, Some(csrf), now)?;
            authorize(tx, &actor, true)?;
            let previous = current(tx, &actor.id)?;
            if previous.updated.is_some_and(|at| now < at) {
                return Err(Error(ErrorCode::Conflict));
            }
            let same = previous.revision > 0 && previous.definition == input.definition;
            if same
                && (input.revision == previous.revision
                    || input.revision.checked_add(1) == Some(previous.revision))
            {
                reservation_live(Some(deadline))?;
                return Ok(previous);
            }
            if input.revision != previous.revision {
                return Err(Error(ErrorCode::Conflict));
            }
            let revision = previous
                .revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or(Error(ErrorCode::Overloaded))?;
            let count: i64 = tx
                .query_row("SELECT count(*) FROM automation_health_policies", [], |r| {
                    r.get(0)
                })
                .map_err(durable)?;
            if count >= MAX_VERSIONS {
                return Err(Error(ErrorCode::Overloaded));
            }
            audit_capacity(tx, 1)?;
            let body = json(&input.definition)?;
            let digest = limeos_identity::digest(&body);
            tx.execute(
                "INSERT INTO automation_health_policies VALUES(?,?,?,?,?)",
                params![actor.id, revision, body, digest, now],
            )
            .map_err(durable)?;
            event(
                tx,
                Some(&actor.id),
                None,
                EventKind::AutomationHealthPolicyChanged { revision, digest },
                now,
            )?;
            reservation_live(Some(deadline))?;
            Ok(HealthPolicyView {
                revision,
                updated: Some(now),
                definition: input.definition.clone(),
            })
        })
    }
}
pub(super) fn check(conn: &Connection) -> Result<()> {
    let count: i64 = conn
        .query_row("SELECT count(*) FROM automation_health_policies", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    if count > MAX_VERSIONS {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let invalid:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_health_policies GROUP BY owner HAVING min(revision)<>1 OR max(revision)<>count(*)) OR EXISTS(SELECT 1 FROM automation_health_policies a JOIN automation_health_policies b ON b.owner=a.owner AND b.revision=a.revision+1 WHERE b.updated<a.updated) OR EXISTS(SELECT 1 FROM events e LEFT JOIN automation_health_policies p ON p.owner=e.principal AND p.revision=json_extract(e.kind,'$.revision') AND p.digest=json_extract(e.kind,'$.digest') AND p.updated=e.created WHERE json_extract(e.kind,'$.kind')='automation_health_policy_changed' AND (p.owner IS NULL OR e.job IS NOT NULL))",[],|r|r.get(0)).map_err(durable)?;
    if invalid {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut stmt=conn.prepare("SELECT owner,revision,digest,updated FROM automation_health_policies ORDER BY owner,revision").map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let owner: String = row.get(0).map_err(durable)?;
        let revision: i64 = row.get(1).map_err(durable)?;
        let digest: String = row.get(2).map_err(durable)?;
        let updated: i64 = row.get(3).map_err(durable)?;
        principal(conn, &owner)?;
        at(conn, &owner, revision)?;
        let own:i64=conn.query_row("SELECT count(*) FROM events WHERE principal=? AND job IS NULL AND created=? AND kind=?",params![owner,updated,json(&EventKind::AutomationHealthPolicyChanged { revision,digest:digest.clone() })?],|r|r.get(0)).map_err(durable)?;
        if own != 1 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let event_cursor: i64 = conn
            .query_row(
                "SELECT cursor FROM events WHERE principal=? AND job IS NULL AND kind=?",
                params![
                    owner,
                    json(&EventKind::AutomationHealthPolicyChanged { revision, digest })?
                ],
                |r| r.get(0),
            )
            .map_err(durable)?;
        if revision_before(conn, &owner, event_cursor)? != revision - 1 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    check_publications(conn)
}
fn check_publications(conn: &Connection) -> Result<()> {
    let mut stmt=conn.prepare("SELECT cursor,principal,kind,created FROM events WHERE json_extract(kind,'$.kind')='automation_notification_published'").map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let cursor: i64 = row.get(0).map_err(durable)?;
        let owner: String = row.get(1).map_err(durable)?;
        let kind: String = row.get(2).map_err(durable)?;
        let EventKind::AutomationNotificationPublished { notification } = parse(&kind)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        let revision = revision_before(conn, &owner, cursor)?;
        let (count,valid):(i64,i64)=conn.query_row("SELECT count(*),coalesce(sum(principal IS ? AND job IS NULL AND created=? AND cursor<? AND json_extract(kind,'$.revision')=?),0) FROM events WHERE json_extract(kind,'$.kind')='automation_notification_policy_checked' AND json_extract(kind,'$.notification')=?",params![owner,row.get::<_,i64>(3).map_err(durable)?,cursor,revision,notification],|r|Ok((r.get(0)?,r.get(1)?))).map_err(durable)?;
        if (count, valid) != if revision == 0 { (0, 0) } else { (1, 1) } {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    let orphan:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM events e WHERE json_extract(e.kind,'$.kind')='automation_notification_policy_checked' AND NOT EXISTS(SELECT 1 FROM events p WHERE p.principal=e.principal AND p.job IS NULL AND p.created=e.created AND p.cursor>e.cursor AND json_extract(p.kind,'$.kind')='automation_notification_published' AND json_extract(p.kind,'$.notification')=json_extract(e.kind,'$.notification')))",[],|r|r.get(0)).map_err(durable)?;
    if orphan {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
