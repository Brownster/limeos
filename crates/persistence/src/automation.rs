//! Durable observation reservations only. No producer or dispatcher is registered.
use super::*;
use limeos_domain::automation::*;
use std::io::Write;

const POLICY: &str = "health-v1";
const DEFINITION_BYTES: usize = 4096;
const PAGE_BYTES: usize = 64 * 1024;
const MAX_OCCURRENCES: i64 = 4096;
const JOB_SECONDS: i64 = 60;

struct Stored {
    owner: String,
    owner_revision: i64,
    schedule: Schedule,
}
type ScheduleHeader = (String, i64, i64, bool, Option<i64>, i64, String, String);
type LastOccurrence = (i64, i64, String, Option<String>, Option<String>);
fn time(now: i64) -> Result<()> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(())
}
fn operation(value: ScheduledOperation) -> &'static str {
    match value {
        ScheduledOperation::ObserveHealth => "observe_health",
        ScheduledOperation::Backup => "backup",
        ScheduledOperation::ProtectionSync => "protection_sync",
        ScheduledOperation::ProtectionScrub => "protection_scrub",
        ScheduledOperation::ContainerUpdate => "container_update",
        ScheduledOperation::PackageReconcile => "package_reconcile",
    }
}
fn authorize(conn: &Connection, actor: &Principal, manage: bool) -> Result<()> {
    let grants = super::identity_store::current_grants(conn, actor)?;
    limeos_policy::authorize(
        actor,
        &grants,
        &Scope {
            operation: Operation::HealthRead,
            resource: "host:system".into(),
        },
    )?;
    if manage {
        limeos_policy::authorize(
            actor,
            &grants,
            &Scope {
                operation: Operation::AutomationManage,
                resource: "automation:observe-health".into(),
            },
        )?;
    }
    Ok(())
}
fn principal(conn: &Connection, id: &str) -> Result<Principal> {
    let (role, revision): (String, i64) = conn
        .query_row(
            "SELECT role,grant_revision FROM users WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(durable)?;
    Ok(Principal {
        id: id.into(),
        role: parse(&role)?,
        grant_revision: revision,
    })
}
fn audit_capacity(conn: &Connection, count: usize) -> Result<()> {
    let bytes: i64 = conn
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(durable)?;
    let extra = i64::try_from(count)
        .ok()
        .and_then(|n| n.checked_mul(256))
        .ok_or(Error(ErrorCode::StateNotDurable))?;
    if bytes
        .checked_add(extra)
        .is_none_or(|total| total > AUDIT_LIMIT)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}
fn load(conn: &Connection, id: &str) -> Result<Option<Stored>> {
    let header: Option<ScheduleHeader> = conn.query_row(
        "SELECT owner,owner_revision,revision,paused,settled_through,length(CAST(definition AS BLOB)),operation,policy FROM automation_schedules WHERE id=?",
        [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
    ).optional().map_err(durable)?;
    let Some((owner, owner_revision, revision, paused, settled_through, size, kind, policy)) =
        header
    else {
        return Ok(None);
    };
    if !(1..=DEFINITION_BYTES as i64).contains(&size)
        || policy != POLICY
        || !limeos_domain::opaque_id(id)
        || !limeos_domain::identifier(&owner)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let body: String = conn
        .query_row(
            "SELECT definition FROM automation_schedules WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .map_err(durable)?;
    let definition: ScheduleDefinition = parse(&body)?;
    let schedule = Schedule {
        id: id.into(),
        revision,
        paused,
        definition,
        settled_through,
    };
    schedule
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if kind != operation(schedule.definition.operation)
        || !(1..=MAX_REVISION).contains(&owner_revision)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(Some(Stored {
        owner,
        owner_revision,
        schedule,
    }))
}
fn view(conn: &Connection, row: &Stored, actor: &Principal) -> Result<ScheduleView> {
    let last: Option<LastOccurrence> = conn.query_row(
        "SELECT o.revision,o.due,o.settlement,o.job,j.state FROM automation_occurrences o LEFT JOIN jobs j ON j.id=o.job WHERE o.schedule=? ORDER BY o.revision DESC,o.due DESC LIMIT 1",
        [&row.schedule.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(durable)?;
    let last_occurrence = last
        .map(|(revision, due, kind, job, state)| {
            let outcome = match (kind.as_str(), job, state) {
                ("observation", Some(id), Some(state)) => OccurrenceOutcome::Job {
                    id,
                    state: parse(&json(&state)?)?,
                },
                ("skipped", None, None) => OccurrenceOutcome::Skipped,
                ("blocked", None, None) => OccurrenceOutcome::Blocked,
                _ => return Err(Error(ErrorCode::StateNotDurable)),
            };
            Ok(OccurrenceView {
                key: OccurrenceKey {
                    schedule: row.schedule.id.clone(),
                    revision,
                    due,
                },
                outcome,
            })
        })
        .transpose()?;
    let status = if row.schedule.paused {
        ScheduleStatus::Paused
    } else if !row.schedule.definition.operation.observation_only() {
        ScheduleStatus::Blocked
    } else {
        let grants = identity_store::current_grants(conn, actor)?;
        let management = limeos_policy::authorize(
            actor,
            &grants,
            &Scope {
                operation: Operation::AutomationManage,
                resource: "automation:observe-health".into(),
            },
        );
        match management {
            Ok(()) if row.owner_revision == actor.grant_revision => {
                ScheduleStatus::AwaitingProducer
            }
            Ok(()) | Err(Error(ErrorCode::Forbidden)) => ScheduleStatus::AuthoritySuspended,
            Err(error) => return Err(error),
        }
    };
    let value = ScheduleView {
        schedule: row.schedule.clone(),
        status,
        last_occurrence,
    };
    value
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    Ok(value)
}
fn proposed(schedule: &Schedule, now: i64) -> Result<Option<Settlement>> {
    Ok(match schedule.due(now)? {
        DueDecision::Ready(key) if schedule.definition.operation.observation_only() => {
            Some(Settlement::Observation(key))
        }
        DueDecision::Ready(key) => Some(Settlement::AdapterUnavailable(key)),
        DueDecision::Skipped(key) => Some(Settlement::Skipped(key)),
        _ => None,
    })
}
fn job_key(key: &OccurrenceKey) -> Result<String> {
    Ok(format!("b1:{}", limeos_identity::digest(&json(key)?)))
}
fn sized<T: serde::Serialize>(value: &T, limit: usize) -> Result<usize> {
    struct Counter {
        used: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit - self.used {
                return Err(std::io::Error::other("automation page bound"));
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { used: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| Error(ErrorCode::Overloaded))?;
    Ok(counter.used)
}

fn create_schedule(
    tx: &Transaction<'_>,
    actor: &Principal,
    input: &CreateSchedule,
    now: i64,
) -> Result<ScheduleView> {
    time(now)?;
    input.definition.validate()?;
    let mut definition = input.definition.clone();
    definition.anchor = now;
    let body = json(&definition)?;
    if body.len() > DEFINITION_BYTES {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
    authorize(tx, actor, true)?;
    let count: i64 = tx
        .query_row("SELECT count(*) FROM automation_schedules", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    if count >= MAX_SCHEDULES as i64 {
        return Err(Error(ErrorCode::Overloaded));
    }
    audit_capacity(tx, 1)?;
    tx.execute(
        "INSERT INTO automation_schedules VALUES(?,?,?,?,1,?,?,0,NULL,?,?)",
        params![
            id,
            actor.id,
            POLICY,
            actor.grant_revision,
            operation(definition.operation),
            body,
            now,
            now
        ],
    )
    .map_err(durable)?;
    event(
        tx,
        Some(&actor.id),
        None,
        EventKind::AutomationScheduleCreated {
            schedule: id.clone(),
        },
        now,
    )?;
    view(
        tx,
        &load(tx, &id)?.ok_or(Error(ErrorCode::StateNotDurable))?,
        actor,
    )
}
fn pause_schedule(
    tx: &Transaction<'_>,
    actor: &Principal,
    id: &str,
    input: &PauseSchedule,
    now: i64,
) -> Result<ScheduleView> {
    time(now)?;
    if !limeos_domain::opaque_id(id) || !(1..=MAX_REVISION).contains(&input.revision) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    authorize(tx, actor, true)?;
    let row = load(tx, id)?
        .filter(|r| r.owner == actor.id)
        .ok_or(Error(ErrorCode::NotFound))?;
    if row.schedule.revision != input.revision {
        return Err(Error(ErrorCode::Conflict));
    }
    if row.schedule.paused == input.paused && row.owner_revision == actor.grant_revision {
        return view(tx, &row, actor);
    }
    let revision = row
        .schedule
        .revision
        .checked_add(1)
        .filter(|r| *r <= MAX_REVISION)
        .ok_or(Error(ErrorCode::StateNotDurable))?;
    let mut definition = row.schedule.definition.clone();
    let owner_revision = if input.paused {
        row.owner_revision
    } else {
        definition.anchor = now;
        actor.grant_revision
    };
    let mut stmt=tx.prepare("SELECT j.id FROM automation_occurrences o JOIN jobs j ON j.id=o.job WHERE o.schedule=? AND j.state='queued' ORDER BY j.id LIMIT 65").map_err(durable)?;
    let jobs = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(durable)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(durable)?;
    if jobs.len() > QUEUE_CAPACITY {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    audit_capacity(tx, jobs.len() + 1)?;
    for job in jobs {
        if tx
            .execute(
                "UPDATE jobs SET state='canceled' WHERE id=? AND state='queued'",
                [&job],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Error(ErrorCode::Conflict));
        }
        event(
            tx,
            Some(&actor.id),
            Some(&job),
            EventKind::JobTransition {
                state: JobState::Canceled,
            },
            now,
        )?;
    }
    if tx.execute("UPDATE automation_schedules SET revision=?,owner_revision=?,paused=?,definition=?,settled_through=NULL,updated=? WHERE id=? AND revision=?",params![revision,owner_revision,input.paused,json(&definition)?,now,id,input.revision]).map_err(durable)?!=1 { return Err(Error(ErrorCode::Conflict)); }
    event(
        tx,
        Some(&actor.id),
        None,
        EventKind::AutomationScheduleChanged {
            schedule: id.into(),
            revision,
        },
        now,
    )?;
    view(
        tx,
        &load(tx, id)?.ok_or(Error(ErrorCode::StateNotDurable))?,
        actor,
    )
}
fn schedule_page(
    conn: &Connection,
    actor: &Principal,
    after: Option<&str>,
    observe_only: bool,
) -> Result<AutomationPage<ScheduleView>> {
    authorize(conn, actor, false)?;
    if after.is_some_and(|id| !limeos_domain::opaque_id(id)) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let mut stmt = conn
            .prepare(
                if observe_only {
                    "SELECT id FROM automation_schedules WHERE owner=? AND id>? AND operation='observe_health' ORDER BY id LIMIT 33"
                } else {
                    "SELECT id FROM automation_schedules WHERE owner=? AND id>? ORDER BY id LIMIT 33"
                },
            )
            .map_err(durable)?;
    let ids = stmt
        .query_map(params![actor.id, after.unwrap_or("")], |r| {
            r.get::<_, String>(0)
        })
        .map_err(durable)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(durable)?;
    let mut items = Vec::new();
    let mut used = 256;
    for id in ids.iter().take(MAX_TICK_SCHEDULES) {
        let row = load(conn, id)?.ok_or(Error(ErrorCode::StateNotDurable))?;
        let value = view(conn, &row, actor)?;
        if used >= PAGE_BYTES {
            break;
        }
        match sized(&value, PAGE_BYTES - used - 1) {
            Ok(size) => used += size + 1,
            Err(_) if !items.is_empty() => break,
            Err(error) => return Err(error),
        }
        items.push(value);
    }
    let next = if items.len() < ids.len() {
        items.last().map(|v| v.schedule.id.clone())
    } else {
        None
    };
    Ok(AutomationPage {
        items,
        next,
        partial: false,
    })
}

impl Store {
    pub fn automation_create(
        &mut self,
        actor: &Principal,
        input: &CreateSchedule,
        now: i64,
    ) -> Result<ScheduleView> {
        self.write(|tx| create_schedule(tx, actor, input, now))
    }
    pub fn automation_pause(
        &mut self,
        actor: &Principal,
        id: &str,
        input: &PauseSchedule,
        now: i64,
    ) -> Result<ScheduleView> {
        self.write(|tx| pause_schedule(tx, actor, id, input, now))
    }
    pub fn automation_schedules(
        &self,
        actor: &Principal,
        after: Option<&str>,
    ) -> Result<AutomationPage<ScheduleView>> {
        schedule_page(&self.conn, actor, after, false)
    }
    /// Internal scheduling hints only; actual reserve reloads current authority.
    pub fn automation_schedule_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Schedule>> {
        if !(1..=MAX_TICK_SCHEDULES).contains(&limit)
            || after.is_some_and(|id| !limeos_domain::opaque_id(id))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM automation_schedules WHERE id>? ORDER BY id LIMIT ?")
            .map_err(durable)?;
        let ids = stmt
            .query_map(params![after.unwrap_or(""), limit as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        ids.iter()
            .map(|id| {
                load(&self.conn, id)?
                    .map(|row| row.schedule)
                    .ok_or(Error(ErrorCode::StateNotDurable))
            })
            .collect()
    }
    pub fn automation_reserve(
        &mut self,
        expected: &Schedule,
        settlement: &Settlement,
        now: i64,
    ) -> Result<Reservation> {
        time(now)?;
        expected.validate()?;
        let key = settlement.key();
        if key.schedule != expected.id
            || key.revision != expected.revision
            || key.due < expected.definition.anchor
            || key.due > now
            || (key.due - expected.definition.anchor)
                % i64::from(expected.definition.interval_seconds)
                != 0
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let generation = self.generation;
        self.write(|tx| {
            let Some(row)=load(tx,&expected.id)? else { return Ok(Reservation::Stale); };
            let actor=principal(tx,&row.owner)?;
            match authorize(tx,&actor,true) {
                Ok(())=>{},
                Err(Error(ErrorCode::Forbidden|ErrorCode::Expired))=>return Ok(Reservation::Stale),
                Err(error)=>return Err(error),
            }
            if row.owner_revision!=actor.grant_revision || row.schedule.revision!=expected.revision
                || row.schedule.paused || row.schedule.paused!=expected.paused
                || row.schedule.definition!=expected.definition { return Ok(Reservation::Stale); }
            let old:Option<String>=tx.query_row("SELECT settlement FROM automation_occurrences WHERE schedule=? AND revision=? AND due=?",params![key.schedule,key.revision,key.due],|r|r.get(0)).optional().map_err(durable)?;
            let kind=match settlement { Settlement::Observation(_)=>"observation",Settlement::Skipped(_)=>"skipped",Settlement::AdapterUnavailable(_)=>"blocked" };
            if let Some(old)=old { return if old==kind { Ok(Reservation::Existing) } else { Err(Error(ErrorCode::Conflict)) }; }
            if row.schedule.settled_through.is_some_and(|through|key.due<=through) { return Ok(Reservation::Existing); }
            if proposed(&row.schedule,now)?.as_ref()!=Some(settlement) { return Ok(Reservation::Stale); }
            let count:i64=tx.query_row("SELECT count(*) FROM automation_occurrences",[],|r|r.get(0)).map_err(durable)?;
            if count>=MAX_OCCURRENCES { return Err(Error(ErrorCode::Overloaded)); }
            audit_capacity(tx,if kind=="observation" {2} else {1})?;
            let job=if kind=="observation" {
                let active:i64=tx.query_row("SELECT count(*) FROM jobs WHERE state IN ('queued','running','verifying','outcome_unknown','needs_intervention')",[],|r|r.get(0)).map_err(durable)?;
                if active>=QUEUE_CAPACITY as i64 { return Err(Error(ErrorCode::Overloaded)); }
                let id=limeos_identity::opaque().map_err(|_|Error(ErrorCode::Unavailable))?;
                let idempotency=job_key(key)?;
                let collision:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE principal=? AND idempotency=?)",params![actor.id,idempotency],|r|r.get(0)).map_err(durable)?;
                if collision { return Err(Error(ErrorCode::Conflict)); }
                let deadline=now.checked_add(JOB_SECONDS).filter(|v|*v<=MAX_TIME).ok_or(Error(ErrorCode::InvalidInput))?;
                let body=json(&Intent::AutomationObserveHealth { occurrence:key.clone() })?;
                tx.execute("INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)",params![id,actor.id,idempotency,body,limeos_identity::digest(&body),"system",generation,actor.grant_revision,deadline]).map_err(durable)?;
                Some(id)
            } else { None };
            tx.execute("INSERT INTO automation_occurrences VALUES(?,?,?,?,?,?,?,?,?)",params![key.schedule,key.revision,key.due,actor.id,POLICY,actor.grant_revision,kind,job,now]).map_err(durable)?;
            if tx.execute("UPDATE automation_schedules SET settled_through=?,updated=? WHERE id=? AND revision=?",params![key.due,now,key.schedule,key.revision]).map_err(durable)?!=1 { return Err(Error(ErrorCode::Conflict)); }
            if let Some(id)=&job { event(tx,Some(&actor.id),Some(id),EventKind::JobQueued,now)?; }
            event(tx,Some(&actor.id),job.as_deref(),EventKind::AutomationOccurrenceSettled { schedule:key.schedule.clone(),revision:key.revision,due:key.due },now)?;
            Ok(match job { Some(id)=>Reservation::Queued(ObservationJob { id,occurrence:key.clone() }),None if kind=="skipped"=>Reservation::Skipped,None=>Reservation::Blocked })
        })
    }
}

pub(super) fn check(conn: &Connection) -> Result<()> {
    let schedules: i64 = conn
        .query_row("SELECT count(*) FROM automation_schedules", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    let occurrences: i64 = conn
        .query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    if schedules > MAX_SCHEDULES as i64 || occurrences > MAX_OCCURRENCES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut stmt = conn
        .prepare("SELECT id FROM automation_schedules ORDER BY id LIMIT 129")
        .map_err(durable)?;
    for id in stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(durable)?
    {
        load(conn, &id.map_err(durable)?)?.ok_or(Error(ErrorCode::StateNotDurable))?;
    }
    let invalid:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM automation_occurrences o JOIN automation_schedules s ON s.id=o.schedule WHERE o.owner<>s.owner OR o.policy<>s.policy OR o.revision>s.revision OR (o.revision=s.revision AND (o.owner_revision<>s.owner_revision OR s.settled_through IS NULL OR o.due>s.settled_through))) OR EXISTS(SELECT 1 FROM automation_occurrences o JOIN jobs j ON j.id=o.job WHERE json_extract(j.intent,'$.operation') IS NOT 'automation_observe_health')",[],|r|r.get(0)).map_err(durable)?;
    if invalid {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut stmt=conn.prepare("SELECT j.id,j.principal,j.idempotency,j.intent,j.digest,j.resource,j.state,j.revision,j.deadline,o.schedule,o.revision,o.due,o.owner,o.owner_revision,o.created FROM jobs j LEFT JOIN automation_occurrences o ON o.job=j.id WHERE json_extract(j.intent,'$.operation')='automation_observe_health'").map_err(durable)?;
    let mut rows = stmt.query([]).map_err(durable)?;
    while let Some(row) = rows.next().map_err(durable)? {
        let id: String = row.get(0).map_err(durable)?;
        let owner: String = row.get(1).map_err(durable)?;
        let intent: String = row.get(3).map_err(durable)?;
        let Intent::AutomationObserveHealth { occurrence: key } = parse::<Intent>(&intent)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        let schedule = load(conn, &key.schedule)?.ok_or(Error(ErrorCode::StateNotDurable))?;
        let state: String = row.get(6).map_err(durable)?;
        let joined: Option<String> = row.get(9).map_err(durable)?;
        let deadline: i64 = row.get(8).map_err(durable)?;
        let queued: (i64, i64, i64) = conn
            .query_row(
                "SELECT count(*),coalesce(min(created),-1),coalesce(sum(principal IS NOT ?),0) FROM events WHERE job=? AND kind=?",
                params![owner, id, json(&EventKind::JobQueued)?],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(durable)?;
        if !limeos_domain::opaque_id(&key.schedule)
            || !(1..=MAX_REVISION).contains(&key.revision)
            || !(0..=MAX_TIME).contains(&key.due)
            || key.revision > schedule.schedule.revision
            || schedule.owner != owner
            || !schedule.schedule.definition.operation.observation_only()
            || queued.0 != 1
            || queued.2 != 0
            || !(0..=MAX_TIME).contains(&queued.1)
            || !(0..=MAX_TIME).contains(&deadline)
            || queued.1 < key.due
            || queued.1.checked_add(JOB_SECONDS) != Some(deadline)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if joined.is_some() {
            if row.get::<_, String>(9).map_err(durable)? != key.schedule
                || row.get::<_, i64>(10).map_err(durable)? != key.revision
                || row.get::<_, i64>(11).map_err(durable)? != key.due
                || row.get::<_, String>(12).map_err(durable)? != owner
                || row.get::<_, i64>(13).map_err(durable)?
                    != row.get::<_, i64>(7).map_err(durable)?
                || row.get::<_, i64>(14).map_err(durable)? != queued.1
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        } else if state != "canceled" {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if state == "canceled" {
            // Pause/review is the sole cancellation path in this foundation.
            // It atomically cancels the old job and advances schedule revision.
            let canceled: (i64,i64) = conn.query_row(
                "SELECT count(*),coalesce(min(created),-1) FROM events WHERE principal=? AND job=? AND kind=?",
                params![owner,id,json(&EventKind::JobTransition { state:JobState::Canceled })?],
                |r|Ok((r.get(0)?,r.get(1)?)),
            ).map_err(durable)?;
            let reviewed:bool=conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE principal=? AND job IS NULL AND created=? AND json_extract(kind,'$.kind')='automation_schedule_changed' AND json_extract(kind,'$.schedule')=? AND json_extract(kind,'$.revision')>? AND json_extract(kind,'$.revision')<=?)",
                params![owner,canceled.1,key.schedule,key.revision,schedule.schedule.revision],|r|r.get(0),
            ).map_err(durable)?;
            if key.revision >= schedule.schedule.revision
                || canceled.0 != 1
                || !(0..=MAX_TIME).contains(&canceled.1)
                || canceled.1 < queued.1
                || !reviewed
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        let matching:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM job_resources WHERE job=? AND resource='system') AND (SELECT count(*) FROM job_resources WHERE job=?)=1",params![id,id],|r|r.get(0)).map_err(durable)?;
        if !matching
            || row.get::<_, String>(2).map_err(durable)? != job_key(&key)?
            || row.get::<_, String>(4).map_err(durable)? != limeos_identity::digest(&intent)
            || row.get::<_, String>(5).map_err(durable)? != "system"
            || !matches!(state.as_str(), "queued" | "canceled")
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    Ok(())
}

mod management;

#[cfg(test)]
mod tests;
