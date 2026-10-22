use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InAppPublication {
    pub scanned: usize,
    pub published: usize,
    pub blocked: usize,
    pub next: Option<String>,
}
pub(super) struct NotificationRow {
    pub owner: String,
    pub revision: i64,
    pub resource: String,
    pub job: String,
    pub value: Notification,
}
pub(super) fn notification(conn: &Connection, id: &str) -> Result<Option<NotificationRow>> {
    let row: Option<(String,i64,String,String,String)> = conn.query_row("SELECT owner,owner_revision,resource,job,body FROM automation_notifications WHERE id=?", [id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(durable)?;
    row.map(|(owner, revision, resource, job, body)| {
        if body.len() > ROW_BYTES || !resource_valid(&resource) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let value: Notification = parse(&body)?;
        value
            .validate()
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if value.id != id
            || value.channel != DeliveryChannel::InApp
            || !matches!(
                value.delivery,
                DeliveryState::Pending | DeliveryState::Delivered { .. }
            )
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(NotificationRow {
            owner,
            revision,
            resource,
            job,
            value,
        })
    })
    .transpose()
}
fn cursor(after: Option<&str>, limit: usize) -> Result<()> {
    if !(1..=MAX_TICK_SCHEDULES).contains(&limit)
        || after.is_some_and(|v| !limeos_domain::opaque_id(v))
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(())
}
fn partial(conn: &Connection, actor: &Principal, now: i64) -> Result<bool> {
    let stream: Option<(bool, i64)> = conn
        .query_row(
            "SELECT complete,owner_revision FROM automation_streams WHERE owner=?",
            [&actor.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(durable)?;
    if stream.is_none_or(|(complete, revision)| !complete || revision != actor.grant_revision) {
        return Ok(true);
    }
    conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE principal=? AND state='queued' AND deadline<=? AND json_extract(intent,'$.operation')='automation_observe_health')",params![actor.id,now],|r|r.get(0)).map_err(durable)
}
fn authenticated(
    conn: &Connection,
    session: &str,
    csrf: Option<&str>,
    now: i64,
) -> Result<Principal> {
    let actor = identity_store::authenticate(conn, session, csrf, now)?;
    read_authorized(conn, &actor, "host:system")?;
    Ok(actor)
}
fn page_size<T: serde::Serialize>(page: &AutomationPage<T>) -> Result<()> {
    sized(page, PAGE_BYTES).map(|_| ())
}

impl Store {
    pub fn automation_publish_in_app(
        &mut self,
        after: Option<&str>,
        limit: usize,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<InAppPublication> {
        reservation_live(Some(deadline))?;
        cursor(after, limit)?;
        self.write(|tx| {
            reservation_live(Some(deadline))?;
            let now = clock();
            reservation_live(Some(deadline))?;
            time(now)?;
            let mut stmt = tx.prepare("SELECT id FROM automation_notifications WHERE id>? AND json_extract(body,'$.delivery.state')='pending' ORDER BY id LIMIT ?").map_err(durable)?;
            let ids = stmt.query_map(params![after.unwrap_or(""),limit as i64],|r|r.get::<_,String>(0)).map_err(durable)?.collect::<std::result::Result<Vec<_>,_>>().map_err(durable)?;
            let mut page = InAppPublication { scanned:ids.len(),published:0,blocked:0,next:if ids.len()==limit {ids.last().cloned()} else {None} };
            for id in ids {
                let row = notification(tx,&id)?.ok_or(Error(ErrorCode::StateNotDurable))?;
                let actor = principal(tx,&row.owner)?;
                let permitted = authorize(tx,&actor,true).and_then(|()|read_authorized(tx,&actor,&row.resource));
                match permitted {
                    Ok(()) if actor.grant_revision == row.revision => {},
                    Ok(()) | Err(Error(ErrorCode::Forbidden|ErrorCode::Expired)) => { page.blocked+=1; continue; },
                    Err(error) => return Err(error),
                }
                let opening = if row.value.event.event == AlertEventKind::Recovery {
                    let id: Option<String> = tx.query_row("SELECT id FROM automation_notifications WHERE owner=? AND key=? AND generation=? AND phase='incident'",params![row.owner,row.value.event.key,row.value.event.generation],|r|r.get(0)).optional().map_err(durable)?;
                    id.map(|id|notification(tx,&id)?.ok_or(Error(ErrorCode::StateNotDurable))).transpose()?
                } else {None};
                if opening.as_ref().is_some_and(|opening|opening.owner!=row.owner || opening.resource!=row.resource) { return Err(Error(ErrorCode::StateNotDurable)); }
                if plan_in_app_delivery(&row.value,opening.as_ref().map(|r|&r.value),now)? != DeliveryDecision::PublishInApp { page.blocked+=1; continue; }
                audit_capacity(tx,1)?;
                let next = row.value.begin(now)?.finish(DeliveryResult::Verified,now)?;
                if tx.execute("UPDATE automation_notifications SET body=? WHERE id=? AND json_extract(body,'$.revision')=?",params![json(&next)?,id,row.value.revision]).map_err(durable)?!=1 { return Err(Error(ErrorCode::Conflict)); }
                event(tx,Some(&row.owner),None,EventKind::AutomationNotificationPublished { notification:id },now)?;
                page.published+=1;
            }
            reservation_live(Some(deadline))?;
            Ok(page)
        })
    }
    pub fn observe_health_alert_page(
        &mut self,
        session: &str,
        after: Option<&str>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<AutomationPage<AlertState>> {
        reservation_live(Some(deadline))?;
        if after.is_some_and(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control)) {
            return Err(Error(ErrorCode::InvalidInput));
        }
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
        let actor = authenticated(&tx, session, None, now)?;
        let mut items = Vec::new();
        let mut next = None;
        {
            let mut stmt=tx.prepare("SELECT key,resource,body FROM automation_alerts WHERE owner=? AND key>? ORDER BY key LIMIT ?").map_err(durable)?;
            let mut rows = stmt
                .query(params![
                    actor.id,
                    after.unwrap_or(""),
                    MAX_ALERTS as i64 + 1
                ])
                .map_err(durable)?;
            let mut scanned = 0;
            while let Some(row) = rows.next().map_err(durable)? {
                scanned += 1;
                if scanned > MAX_ALERTS {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                let key: String = row.get(0).map_err(durable)?;
                let resource: String = row.get(1).map_err(durable)?;
                let body: String = row.get(2).map_err(durable)?;
                if body.len() > ROW_BYTES || !resource_valid(&resource) {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                let value: AlertState = parse(&body)?;
                value.validate()?;
                if value.key != key {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                match read_authorized(&tx, &actor, &resource) {
                    Ok(()) => {}
                    Err(Error(ErrorCode::Forbidden)) => continue,
                    Err(error) => return Err(error),
                }
                if items.len() == MAX_TICK_SCHEDULES {
                    next = items.last().map(|v: &AlertState| v.key.clone());
                    break;
                }
                items.push(value);
            }
        }
        let page = AutomationPage {
            items,
            next,
            partial: partial(&tx, &actor, now)?,
        };
        page_size(&page)?;
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        Ok(page)
    }
    pub fn observe_health_notification_page(
        &mut self,
        session: &str,
        after: Option<&str>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<AutomationPage<Notification>> {
        reservation_live(Some(deadline))?;
        cursor(after, MAX_TICK_SCHEDULES)?;
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
        let actor = authenticated(&tx, session, None, now)?;
        let mut items = Vec::new();
        let mut next = None;
        {
            let mut stmt=tx.prepare("SELECT id FROM automation_notifications WHERE owner=? AND id>? ORDER BY id LIMIT ?").map_err(durable)?;
            let mut rows = stmt
                .query(params![
                    actor.id,
                    after.unwrap_or(""),
                    MAX_NOTIFICATIONS as i64 + 1
                ])
                .map_err(durable)?;
            let mut scanned = 0;
            while let Some(row) = rows.next().map_err(durable)? {
                scanned += 1;
                if scanned > MAX_NOTIFICATIONS {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                let id: String = row.get(0).map_err(durable)?;
                let value = notification(&tx, &id)?.ok_or(Error(ErrorCode::StateNotDurable))?;
                match read_authorized(&tx, &actor, &value.resource) {
                    Ok(()) => {}
                    Err(Error(ErrorCode::Forbidden)) => continue,
                    Err(error) => return Err(error),
                }
                if items.len() == MAX_TICK_SCHEDULES {
                    next = items.last().map(|v: &Notification| v.id.clone());
                    break;
                }
                items.push(value.value);
            }
        }
        let page = AutomationPage {
            items,
            next,
            partial: partial(&tx, &actor, now)?,
        };
        page_size(&page)?;
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        Ok(page)
    }
    pub fn observe_health_notification_acknowledge(
        &mut self,
        session: &str,
        csrf: &str,
        id: &str,
        input: &AcknowledgeNotification,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<Notification> {
        reservation_live(Some(deadline))?;
        if !limeos_domain::opaque_id(id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.write(|tx| {
            reservation_live(Some(deadline))?; let now=clock(); reservation_live(Some(deadline))?; time(now)?;
            let actor=authenticated(tx,session,Some(csrf),now)?;
            let row=notification(tx,id)?.filter(|r|r.owner==actor.id).ok_or(Error(ErrorCode::NotFound))?;
            read_authorized(tx,&actor,&row.resource)?;
            let mut next=row.value;
            if next.acknowledged_at.is_some() {
                if input.revision!=next.revision && input.revision.checked_add(1)!=Some(next.revision) {return Err(Error(ErrorCode::Conflict));}
            } else {
                if next.revision!=input.revision || !matches!(next.delivery,DeliveryState::Delivered {at} if now>=at) {return Err(Error(ErrorCode::Conflict));}
                audit_capacity(tx,1)?;
                next.revision=next.revision.checked_add(1).filter(|v|*v<=MAX_REVISION).ok_or(Error(ErrorCode::Overloaded))?;
                next.acknowledged_at=Some(now); next.validate()?;
                if tx.execute("UPDATE automation_notifications SET body=? WHERE id=? AND json_extract(body,'$.revision')=?",params![json(&next)?,id,input.revision]).map_err(durable)?!=1 {return Err(Error(ErrorCode::Conflict));}
                event(tx,Some(&actor.id),None,EventKind::AutomationNotificationAcknowledged {notification:id.into()},now)?;
            }
            reservation_live(Some(deadline))?; Ok(next)
        })
    }
    pub fn automation_prune_notifications(
        &mut self,
        after: Option<&str>,
        limit: usize,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<(usize, Option<String>)> {
        reservation_live(Some(deadline))?;
        cursor(after, limit)?;
        self.write(|tx| {
            reservation_live(Some(deadline))?; let now=clock(); reservation_live(Some(deadline))?; time(now)?;
            let mut stmt=tx.prepare("SELECT id FROM automation_notifications WHERE id>? ORDER BY id LIMIT ?").map_err(durable)?;
            let ids=stmt.query_map(params![after.unwrap_or(""),limit as i64],|r|r.get::<_,String>(0)).map_err(durable)?.collect::<std::result::Result<Vec<_>,_>>().map_err(durable)?;
            let next=if ids.len()==limit {ids.last().cloned()} else {None}; let mut removed=0;
            for id in ids {
                let row=notification(tx,&id)?.ok_or(Error(ErrorCode::StateNotDurable))?;
                if !matches!(row.value.delivery,DeliveryState::Delivered {..}) || !row.value.retainable(now)? {continue;}
                if row.value.event.event==AlertEventKind::Incident {
                    let needed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM automation_notifications WHERE owner=? AND key=? AND generation=? AND phase='recovery') OR EXISTS(SELECT 1 FROM automation_alerts WHERE owner=? AND key=? AND json_extract(body,'$.incident.phase')='open' AND json_extract(body,'$.incident.generation')=?)",params![row.owner,row.value.event.key,row.value.event.generation,row.owner,row.value.event.key,row.value.event.generation],|r|r.get(0)).map_err(durable)?;
                    if needed {continue;}
                }
                audit_capacity(tx,1)?;
                removed+=tx.execute("DELETE FROM automation_notifications WHERE id=?",[&id]).map_err(durable)?;
                event(tx,Some(&row.owner),None,EventKind::AutomationNotificationPruned {notification:id},now)?;
            }
            reservation_live(Some(deadline))?; Ok((removed,next))
        })
    }
}
