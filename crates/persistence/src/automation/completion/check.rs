use super::*;

pub(super) fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error(ErrorCode::StateNotDurable))
    }
}
// Per-row startup proofs on `events`. Each names its index, so no lookup
// scans the table: a job's events through `events_job`, a notification's
// through `events_notification`, any other job-less kind through
// `events_kind`. The index narrows; the exact `kind` comparison decides.
// Narrowing notification proofs by kind alone still read every proof of that
// kind once per notification, quadratic in the notifications ever published.
pub(crate) const JOB_PROOF: &str = "SELECT count(*),coalesce(sum(principal IS NOT ? OR created<>?),0) FROM events INDEXED BY events_job WHERE job=? AND kind=?";
pub(crate) const JOBLESS_PROOF: &str = "SELECT count(*),coalesce(sum(principal IS NOT ? OR created<>?),0) FROM events INDEXED BY events_kind WHERE json_extract(kind,'$.kind')=? AND job IS NULL AND kind=?";
pub(crate) const CREATED_PROOFS: &str = "SELECT kind,principal,created FROM events INDEXED BY events_job WHERE job=? AND json_extract(kind,'$.kind')='automation_notification_created'";
pub(crate) const NOTIFICATION_PROOF: &str = "SELECT count(*),coalesce(sum(principal IS NOT ? OR created<>?),0) FROM events INDEXED BY events_notification WHERE json_extract(kind,'$.notification')=? AND job IS NULL AND kind=?";
pub(crate) const DELIVERY_TIMES: &str = "SELECT (SELECT min(created) FROM events INDEXED BY events_notification WHERE json_extract(kind,'$.notification')=?1 AND principal=?2 AND job IS NULL AND kind=?3),(SELECT max(created) FROM events INDEXED BY events_notification WHERE json_extract(kind,'$.notification')=?1 AND principal=?2 AND job IS NULL AND kind=?4),(SELECT min(created) FROM events INDEXED BY events_notification WHERE json_extract(kind,'$.notification')=?1 AND principal=?2 AND job IS NULL AND kind=?5)";
pub(crate) const QUEUED_PROOF: &str =
    "SELECT created,cursor FROM events INDEXED BY events_job WHERE job=? AND kind=?";
pub(crate) const SETTLEMENT_COUNT: &str = "SELECT count(*) FROM events INDEXED BY events_job WHERE job=? AND json_extract(kind,'$.kind') IN ('job_transition','automation_observation_settled')";
pub(crate) const NOTIFICATION_COUNT: &str = "SELECT count(*) FROM events INDEXED BY events_notification WHERE json_extract(kind,'$.notification')=? AND kind=?";

/// A string field of an encoded event: `kind` is the key of `events_kind`,
/// `notification` of `events_notification`.
fn field(kind: &EventKind, name: &str) -> Option<String> {
    Some(
        serde_json::to_value(kind)
            .ok()?
            .get(name)?
            .as_str()?
            .to_owned(),
    )
}
pub(super) fn audit(
    conn: &Connection,
    owner: &str,
    job: Option<&str>,
    kind: EventKind,
    at: i64,
) -> Result<()> {
    let encoded = json(&kind)?;
    let counts: (i64, i64) = match job {
        Some(job) => conn.query_row(JOB_PROOF, params![owner, at, job, encoded], |r| {
            Ok((r.get(0)?, r.get(1)?))
        }),
        None => match field(&kind, "notification") {
            Some(notification) => conn.query_row(
                NOTIFICATION_PROOF,
                params![owner, at, notification, encoded],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ),
            None => conn.query_row(
                JOBLESS_PROOF,
                params![
                    owner,
                    at,
                    field(&kind, "kind").ok_or(Error(ErrorCode::InvalidInput))?,
                    encoded
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ),
        },
    }
    .map_err(durable)?;
    require(counts == (1, 0))
}
/// Events of exactly `kind`, about notification `id`, anywhere.
fn notification_count(conn: &Connection, id: &str, kind: &EventKind) -> Result<i64> {
    conn.query_row(NOTIFICATION_COUNT, params![id, json(kind)?], |r| r.get(0))
        .map_err(durable)
}
type DeliveryTimes = (Option<i64>, Option<i64>, Option<i64>);
fn created_proofs(conn: &Connection, owner: &str, job: &str, at: i64, expected: i64) -> Result<()> {
    let mut stmt = conn.prepare(CREATED_PROOFS).map_err(durable)?;
    let mut rows = stmt.query([job]).map_err(durable)?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(durable)? {
        count += 1;
        require(count <= MAX_ALERTS as i64)?;
        let kind: String = row.get(0).map_err(durable)?;
        let EventKind::AutomationNotificationCreated {
            notification: id,
            digest,
        } = parse(&kind)?
        else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        require(
            limeos_domain::opaque_id(&id)
                && limeos_domain::opaque_id(&digest)
                && row.get::<_, String>(1).map_err(durable)? == owner
                && row.get::<_, i64>(2).map_err(durable)? == at,
        )?;
        if let Some(value) = in_app::notification(conn, &id)? {
            require(
                value.owner == owner
                    && value.job == job
                    && notification_digest(
                        owner,
                        value.revision,
                        &value.resource,
                        job,
                        &value.value,
                    )? == digest,
            )?;
        } else {
            let (delivered, ack, pruned): DeliveryTimes = conn
                .query_row(
                    DELIVERY_TIMES,
                    params![
                        id,
                        owner,
                        json(&EventKind::AutomationNotificationPublished {
                            notification: id.clone()
                        })?,
                        json(&EventKind::AutomationNotificationAcknowledged {
                            notification: id.clone()
                        })?,
                        json(&EventKind::AutomationNotificationPruned {
                            notification: id.clone()
                        })?
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(durable)?;
            let delivered = delivered.ok_or(Error(ErrorCode::StateNotDurable))?;
            let pruned = pruned.ok_or(Error(ErrorCode::StateNotDurable))?;
            require(
                delivered >= at
                    && ack.is_none_or(|v| v >= delivered)
                    && pruned >= ack.unwrap_or(delivered)
                    && pruned - ack.unwrap_or(delivered) >= DELIVERED_RETENTION_SECONDS,
            )?;
            audit(
                conn,
                owner,
                None,
                EventKind::AutomationNotificationPublished {
                    notification: id.clone(),
                },
                delivered,
            )?;
            if let Some(ack) = ack {
                audit(
                    conn,
                    owner,
                    None,
                    EventKind::AutomationNotificationAcknowledged {
                        notification: id.clone(),
                    },
                    ack,
                )?;
            }
            audit(
                conn,
                owner,
                None,
                EventKind::AutomationNotificationPruned { notification: id },
                pruned,
            )?;
        }
    }
    require(count == expected)
}
/// Every settled observation job has its result (of the observation jobs
/// startup's walk of `jobs` listed, `?1`, `resources::JOB_WALK`, each read
/// through the primary key); every result has its stream, job and owner;
/// every notification was created once.
pub(crate) const OBSERVATION_RESULTS: &str = "SELECT EXISTS(SELECT 1 FROM json_each(?1) w CROSS JOIN jobs j ON j.id=w.value WHERE json_extract(j.intent,'$.operation')='automation_observe_health' AND j.state IN ('succeeded','failed','precondition_changed') AND NOT EXISTS(SELECT 1 FROM automation_observation_results r WHERE r.job=j.id)) OR EXISTS(SELECT 1 FROM automation_observation_results r LEFT JOIN automation_streams s ON s.owner=r.owner LEFT JOIN jobs j ON j.id=r.job LEFT JOIN users u ON u.id=r.owner WHERE s.owner IS NULL OR j.id IS NULL OR u.id IS NULL) OR EXISTS(SELECT 1 FROM events WHERE json_extract(kind,'$.kind')='automation_notification_created' GROUP BY json_extract(kind,'$.notification') HAVING count(*)<>1)";
pub(super) fn check(conn: &Connection, walked: &crate::resources::Walked) -> Result<()> {
    let counts:(i64,i64,i64,i64)=conn.query_row("SELECT (SELECT count(*) FROM automation_streams),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_alerts),(SELECT count(*) FROM automation_notifications)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(durable)?;
    require(
        counts.0 <= MAX_SCHEDULES as i64
            && counts.1 <= MAX_RESULTS
            && counts.2 <= MAX_ALERTS as i64
            && counts.3 <= MAX_NOTIFICATIONS as i64,
    )?;
    let invalid: bool = conn
        .query_row(
            OBSERVATION_RESULTS,
            [walked.ids("automation_observe_health")?],
            |r| r.get(0),
        )
        .map_err(durable)?;
    require(!invalid)?;
    let mut stmt = conn
        .prepare("SELECT owner,sequence,complete,owner_revision,recorded,alerts_digest,policy_revision FROM automation_streams")
        .map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let owner: String = row.get(0).map_err(durable)?;
        let sequence: i64 = row.get(1).map_err(durable)?;
        let complete: bool = row.get(2).map_err(durable)?;
        let revision: i64 = row.get(3).map_err(durable)?;
        let recorded: i64 = row.get(4).map_err(durable)?;
        let proof:(i64,i64,i64)=conn.query_row("SELECT count(*),coalesce(min(sequence),0),coalesce(max(sequence),0) FROM automation_observation_results WHERE owner=?",[&owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(durable)?;
        require(proof == (sequence, 1, sequence))?;
        let digest: String = row.get(5).map_err(durable)?;
        require(material_digest(conn, &owner)? == digest)?;
        let matches:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_observation_results WHERE owner=? AND sequence=? AND complete=? AND owner_revision=? AND recorded=? AND alerts_digest=? AND policy_revision=?)",params![owner,sequence,complete,revision,recorded,digest,row.get::<_,i64>(6).map_err(durable)?],|r|r.get(0)).map_err(durable)?;
        require(matches)?;
        let backwards:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_observation_results a JOIN automation_observation_results b ON b.owner=a.owner AND b.sequence=a.sequence+1 WHERE a.owner=? AND b.recorded<a.recorded)",[owner],|r|r.get(0)).map_err(durable)?;
        require(!backwards)?;
    }
    let mut stmt=conn.prepare("SELECT r.job,r.owner,r.sequence,r.owner_revision,r.outcome,r.reason,r.complete,r.recorded,j.principal,j.revision,j.state,j.deadline,r.notifications,r.policy_revision FROM automation_observation_results r JOIN jobs j ON j.id=r.job").map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let job: String = row.get(0).map_err(durable)?;
        let owner: String = row.get(1).map_err(durable)?;
        let sequence: i64 = row.get(2).map_err(durable)?;
        let revision: i64 = row.get(3).map_err(durable)?;
        let state: String = row.get(4).map_err(durable)?;
        let reason: String = row.get(5).map_err(durable)?;
        let complete: bool = row.get(6).map_err(durable)?;
        let at: i64 = row.get(7).map_err(durable)?;
        require(
            owner == row.get::<_, String>(8).map_err(durable)?
                && revision == row.get::<_, i64>(9).map_err(durable)?
                && state == row.get::<_, String>(10).map_err(durable)?,
        )?;
        let deadline: i64 = row.get(11).map_err(durable)?;
        if reason == "observed" {
            require(at < deadline)?;
        } else if reason == "expired" {
            require(at >= deadline && !complete)?;
        }
        let (queued, queued_cursor): (i64, i64) = conn
            .query_row(
                QUEUED_PROOF,
                params![job, json(&EventKind::JobQueued)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(durable)?;
        require(at >= queued)?;
        let policy_revision = row.get::<_, i64>(13).map_err(durable)?;
        require(
            policy_revision == super::super::policy::revision_before(conn, &owner, queued_cursor)?,
        )?;
        super::super::policy::at(conn, &owner, policy_revision)?;
        audit(
            conn,
            &owner,
            Some(&job),
            EventKind::JobTransition {
                state: parse(&json(&state)?)?,
            },
            at,
        )?;
        audit(
            conn,
            &owner,
            Some(&job),
            EventKind::AutomationObservationSettled { sequence, complete },
            at,
        )?;
        let transitions: i64 = conn
            .query_row(SETTLEMENT_COUNT, [&job], |r| r.get(0))
            .map_err(durable)?;
        require(transitions == 2)?;
        created_proofs(conn, &owner, &job, at, row.get(12).map_err(durable)?)?;
    }
    let mut stmt = conn
        .prepare("SELECT owner,key,resource,body,updated FROM automation_alerts")
        .map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let owner: String = row.get(0).map_err(durable)?;
        let key: String = row.get(1).map_err(durable)?;
        let resource: String = row.get(2).map_err(durable)?;
        let body: String = row.get(3).map_err(durable)?;
        let at: i64 = row.get(4).map_err(durable)?;
        let state: AlertState = parse(&body)?;
        state.validate()?;
        require(body.len() <= ROW_BYTES && state.key == key && resource_valid(&resource))?;
        let proof:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_observation_results WHERE owner=? AND sequence=? AND outcome='succeeded' AND recorded=?)",params![owner,state.last_sequence,at],|r|r.get(0)).map_err(durable)?;
        require(proof)?;
        if let IncidentState::Open {
            generation,
            announced: true,
            ..
        } = state.incident
        {
            let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_notifications WHERE owner=? AND key=? AND resource=? AND generation=? AND phase='incident')",params![owner,key,resource,generation],|r|r.get(0)).map_err(durable)?;
            require(exists)?;
        }
    }
    let mut stmt = conn
        .prepare("SELECT id FROM automation_notifications")
        .map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let id: String = row.get(0).map_err(durable)?;
        let row = in_app::notification(conn, &id)?.ok_or(Error(ErrorCode::StateNotDurable))?;
        let n = &row.value;
        audit(
            conn,
            &row.owner,
            Some(&row.job),
            EventKind::AutomationNotificationCreated {
                notification: id.clone(),
                digest: notification_digest(&row.owner, row.revision, &row.resource, &row.job, n)?,
            },
            n.event.at,
        )?;
        let pruned = notification_count(
            conn,
            &id,
            &EventKind::AutomationNotificationPruned {
                notification: id.clone(),
            },
        )?;
        require(pruned == 0)?;
        let origin:Option<(String,i64,i64,i64,String)>=conn.query_row("SELECT owner,owner_revision,sequence,recorded,outcome FROM automation_observation_results WHERE job=?",[&row.job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(durable)?;
        let (owner, revision, sequence, at, outcome) =
            origin.ok_or(Error(ErrorCode::StateNotDurable))?;
        require(
            owner == row.owner
                && revision == row.revision
                && n.event.generation <= sequence
                && n.event.at == at
                && outcome == "succeeded",
        )?;
        let publications = notification_count(
            conn,
            &id,
            &EventKind::AutomationNotificationPublished {
                notification: id.clone(),
            },
        )?;
        match n.delivery {
            DeliveryState::Pending => require(
                publications == 0
                    && n.revision == 1
                    && n.attempts == 0
                    && n.acknowledged_at.is_none(),
            )?,
            DeliveryState::Delivered { at } => {
                require(
                    n.attempts == 1
                        && n.revision == if n.acknowledged_at.is_some() { 4 } else { 3 },
                )?;
                audit(
                    conn,
                    &owner,
                    None,
                    EventKind::AutomationNotificationPublished {
                        notification: id.clone(),
                    },
                    at,
                )?;
                require(n.acknowledged_at.is_none_or(|ack| ack >= at))?;
            }
            _ => return Err(Error(ErrorCode::StateNotDurable)),
        }
        let acknowledgments = notification_count(
            conn,
            &id,
            &EventKind::AutomationNotificationAcknowledged {
                notification: id.clone(),
            },
        )?;
        if let Some(at) = n.acknowledged_at {
            audit(
                conn,
                &owner,
                None,
                EventKind::AutomationNotificationAcknowledged { notification: id },
                at,
            )?;
        } else {
            require(acknowledgments == 0)?;
        }
        if n.event.event == AlertEventKind::Recovery {
            require(n.event.generation < sequence)?;
            let opening_id:Option<String>=conn.query_row("SELECT id FROM automation_notifications WHERE owner=? AND key=? AND generation=? AND phase='incident'",params![owner,n.event.key,n.event.generation],|r|r.get(0)).optional().map_err(durable)?;
            let opening =
                in_app::notification(conn, &opening_id.ok_or(Error(ErrorCode::StateNotDurable))?)?
                    .ok_or(Error(ErrorCode::StateNotDurable))?;
            require(opening.resource == row.resource)?;
            plan_in_app_delivery(n, Some(&opening.value), n.event.at)?;
            if let DeliveryState::Delivered { at } = n.delivery {
                require(
                    matches!(opening.value.delivery,DeliveryState::Delivered {at:opened} if opened<=at),
                )?;
            }
        }
    }
    Ok(())
}
