//! Operation-owned read settlement. No collector, timer or effect dispatch.
use super::*;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

mod check;
/// The per-row startup queries on `events` of this module's checks.
#[cfg(test)]
pub(super) fn startup_event_queries() -> Vec<&'static str> {
    vec![
        check::JOB_PROOF,
        check::JOBLESS_PROOF,
        check::NOTIFICATION_PROOF,
        check::CREATED_PROOFS,
        check::DELIVERY_TIMES,
        check::QUEUED_PROOF,
        check::SETTLEMENT_COUNT,
        check::NOTIFICATION_COUNT,
        polling::POLL_PROOFS,
    ]
}
mod container_read;
mod in_app;
mod polling;
mod runtime_hints;
pub use container_read::{ContainerReadAdmission, ContainerReadOutcome, ContainerReadPermit};
pub use in_app::InAppPublication;
pub(crate) use polling::PollWatermark;
pub use polling::{ObservationPollAdmission, ObservationPollLease, ObservationPollOutcome};
pub use runtime_hints::{ObservationOwnerPage, ObservationWorkHint};

const ROW_BYTES: usize = 4096;
const RESOURCE_BYTES: usize = 512;
const BATCH_BYTES: usize = 1024 * 1024;
const LEASE_SECONDS: u64 = 10;
const MAX_RESULTS: i64 = AUDIT_LIMIT / (4 * 256);

pub(crate) struct ActiveObservation {
    nonce: String,
    job: Option<String>,
    deadline: Instant,
}

/// Internal read capability; never serialized or accepted from an API caller.
#[derive(Clone)]
pub struct ObservationLease {
    nonce: String,
    job: ObservationJob,
    owner: String,
    revision: i64,
    generation: i64,
    sequence: i64,
    resources: Vec<String>,
    policy: Box<HealthPolicyView>,
    deadline: Instant,
}
impl std::fmt::Debug for ObservationLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservationLease")
            .field("job", &self.job)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}
impl ObservationLease {
    pub fn job(&self) -> &ObservationJob {
        &self.job
    }
    pub fn resources(&self) -> &[String] {
        &self.resources
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn health_policy(&self) -> &HealthPolicyView {
        &self.policy
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationResult {
    pub job: String,
    pub sequence: i64,
    pub state: JobState,
    pub complete: bool,
    pub recorded: i64,
    pub notifications: u16,
}
#[derive(Debug)]
pub enum ObservationAdmission {
    Lease(ObservationLease),
    Existing(ObservationResult),
}
/// Trusted normalized collector output plus its concrete HealthRead resource.
/// The protected collector adapter supplies this binding; it is not HTTP input.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ScopedHealthSignal {
    pub resource: String,
    pub signal: AlertSignal,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthObservation {
    pub signals: Vec<ScopedHealthSignal>,
    pub complete: bool,
}
fn resource_valid(value: &str) -> bool {
    !value.is_empty()
        && value != "*"
        && value.len() <= RESOURCE_BYTES
        && !value.chars().any(char::is_control)
}
fn read_authorized(conn: &Connection, actor: &Principal, resource: &str) -> Result<()> {
    limeos_policy::authorize(
        actor,
        &identity_store::current_grants(conn, actor)?,
        &Scope {
            operation: Operation::HealthRead,
            resource: resource.into(),
        },
    )
}
fn resources(mut values: Vec<String>) -> Result<Vec<String>> {
    if values.is_empty() || values.len() > MAX_ALERTS || values.iter().any(|v| !resource_valid(v)) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(values)
}
fn observation_digest(batch: &HealthObservation) -> Result<String> {
    if batch.signals.len() > MAX_ALERTS {
        return Err(Error(ErrorCode::Overloaded));
    }
    let mut ordered: Vec<_> = batch.signals.iter().collect();
    ordered.sort_by(|a, b| a.signal.key.cmp(&b.signal.key));
    for (index, entry) in ordered.iter().enumerate() {
        entry.signal.validate()?;
        if !resource_valid(&entry.resource)
            || (entry.signal.kind == AlertKind::Host && entry.resource != "host:system")
            || (entry.signal.kind != AlertKind::Host && entry.resource == "host:system")
            || index > 0 && ordered[index - 1].signal.key == entry.signal.key
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
    }
    let body = json(&(batch.complete, ordered))?;
    if body.len() > BATCH_BYTES {
        return Err(Error(ErrorCode::Overloaded));
    }
    Ok(limeos_identity::digest(&body))
}
struct Context {
    owner: String,
    revision: i64,
    deadline: i64,
    queued: i64,
    state: JobState,
    schedule: Stored,
    policy: HealthPolicyView,
}
fn context(conn: &Connection, job: &ObservationJob) -> Result<Context> {
    if !limeos_domain::opaque_id(&job.id) || !limeos_domain::opaque_id(&job.occurrence.schedule) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let row: Option<(String, i64, i64, String, String)> = conn
        .query_row(
            "SELECT principal,revision,deadline,state,intent FROM jobs WHERE id=?",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(durable)?;
    let (owner, revision, deadline, state, intent) = row.ok_or(Error(ErrorCode::NotFound))?;
    if parse::<Intent>(&intent)?
        != (Intent::AutomationObserveHealth {
            occurrence: job.occurrence.clone(),
        })
    {
        return Err(Error(ErrorCode::Conflict));
    }
    let schedule =
        load(conn, &job.occurrence.schedule)?.ok_or(Error(ErrorCode::StateNotDurable))?;
    if schedule.owner != owner {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let queued = conn
        .query_row(
            "SELECT created FROM events WHERE job=? AND kind=?",
            params![job.id, json(&EventKind::JobQueued)?],
            |r| r.get(0),
        )
        .map_err(durable)?;
    let bound:Option<i64>=conn.query_row("SELECT coalesce((SELECT policy_revision FROM automation_occurrences WHERE job=?),(SELECT policy_revision FROM automation_observation_results WHERE job=?))",params![job.id,job.id],|r|r.get(0)).map_err(durable)?;
    let bound = bound.ok_or(Error(ErrorCode::StateNotDurable))?;
    let policy = super::policy::at(conn, &owner, bound)?;
    Ok(Context {
        owner,
        revision,
        deadline,
        queued,
        state: parse(&json(&state)?)?,
        schedule,
        policy,
    })
}
fn current(conn: &Connection, ctx: &Context, job: &ObservationJob) -> Result<Principal> {
    let actor = principal(conn, &ctx.owner)?;
    authorize(conn, &actor, true)?;
    if actor.grant_revision != ctx.revision
        || ctx.schedule.owner_revision != ctx.revision
        || ctx.schedule.schedule.revision != job.occurrence.revision
        || ctx.schedule.schedule.paused
        || (ctx.state == JobState::Queued
            && super::policy::current(conn, &ctx.owner)?.revision != ctx.policy.revision)
    {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(actor)
}
fn oldest(conn: &Connection, ctx: &Context, job: &ObservationJob) -> Result<()> {
    let id: Option<String> = conn.query_row("SELECT j.id FROM jobs j JOIN automation_occurrences o ON o.job=j.id WHERE j.principal=? AND j.state='queued' AND json_extract(j.intent,'$.operation')='automation_observe_health' ORDER BY o.created,o.due,j.id LIMIT 1", [&ctx.owner], |r|r.get(0)).optional().map_err(durable)?;
    if id.as_deref() != Some(&job.id) {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(())
}
fn next_sequence(conn: &Connection, owner: &str) -> Result<i64> {
    let previous: Option<i64> = conn
        .query_row(
            "SELECT sequence FROM automation_streams WHERE owner=?",
            [owner],
            |r| r.get(0),
        )
        .optional()
        .map_err(durable)?;
    previous
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| *n <= MAX_REVISION)
        .ok_or(Error(ErrorCode::Overloaded))
}
type ResultRow = (i64, String, bool, i64, String, u16);
type StoredResult = (ObservationResult, String);
fn result(conn: &Connection, job: &str) -> Result<Option<StoredResult>> {
    let row: Option<ResultRow> = conn.query_row("SELECT sequence,outcome,complete,recorded,digest,notifications FROM automation_observation_results WHERE job=?", [job], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(durable)?;
    row.map(
        |(sequence, state, complete, recorded, digest, notifications)| {
            Ok((
                ObservationResult {
                    job: job.into(),
                    sequence,
                    state: parse(&json(&state)?)?,
                    complete,
                    recorded,
                    notifications,
                },
                digest,
            ))
        },
    )
    .transpose()
}
fn save_result(
    tx: &Transaction<'_>,
    job: &ObservationJob,
    ctx: &Context,
    receipt: &ObservationResult,
    reason: &str,
    digest: &str,
) -> Result<()> {
    let previous: Option<i64> = tx
        .query_row(
            "SELECT recorded FROM automation_streams WHERE owner=?",
            [&ctx.owner],
            |r| r.get(0),
        )
        .optional()
        .map_err(durable)?;
    if previous.is_some_and(|at| receipt.recorded < at) {
        return Err(Error(ErrorCode::Conflict));
    }
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM automation_observation_results",
            [],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if count >= MAX_RESULTS {
        return Err(Error(ErrorCode::Overloaded));
    }
    audit_capacity(tx, 2)?;
    let alerts_digest = material_digest(tx, &ctx.owner)?;
    tx.execute(
        "INSERT INTO automation_observation_results VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            job.id,
            ctx.owner,
            receipt.sequence,
            ctx.revision,
            receipt.state.as_str(),
            reason,
            receipt.complete,
            digest,
            receipt.recorded,
            receipt.notifications,
            alerts_digest,
            ctx.policy.revision
        ],
    )
    .map_err(durable)?;
    tx.execute("INSERT INTO automation_streams VALUES(?,?,?,?,?,?,?) ON CONFLICT(owner) DO UPDATE SET sequence=excluded.sequence,complete=excluded.complete,owner_revision=excluded.owner_revision,recorded=excluded.recorded,alerts_digest=excluded.alerts_digest,policy_revision=excluded.policy_revision", params![ctx.owner,receipt.sequence,receipt.complete,ctx.revision,receipt.recorded,alerts_digest,ctx.policy.revision]).map_err(durable)?;
    Ok(())
}
fn finish_job(tx: &Transaction<'_>, ctx: &Context, receipt: &ObservationResult) -> Result<()> {
    if tx
        .execute(
            "UPDATE jobs SET state=? WHERE id=? AND state='queued'",
            params![receipt.state.as_str(), receipt.job],
        )
        .map_err(durable)?
        != 1
    {
        return Err(Error(ErrorCode::Conflict));
    }
    event(
        tx,
        Some(&ctx.owner),
        Some(&receipt.job),
        EventKind::JobTransition {
            state: receipt.state,
        },
        receipt.recorded,
    )?;
    event(
        tx,
        Some(&ctx.owner),
        Some(&receipt.job),
        EventKind::AutomationObservationSettled {
            sequence: receipt.sequence,
            complete: receipt.complete,
        },
        receipt.recorded,
    )
}
fn material_digest(conn: &Connection, owner: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"health-v1-alert-material\0");
    let mut stmt=conn.prepare("SELECT key,resource,body,updated FROM automation_alerts WHERE owner=? ORDER BY key LIMIT ?").map_err(durable)?;
    let mut rows = stmt
        .query(params![owner, MAX_ALERTS as i64 + 1])
        .map_err(durable)?;
    let mut count = 0u64;
    while let Some(row) = rows.next().map_err(durable)? {
        count += 1;
        if count > MAX_ALERTS as u64 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        for column in 0..3 {
            let text: String = row.get(column).map_err(durable)?;
            if text.len() > ROW_BYTES {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            hasher.update((text.len() as u64).to_be_bytes());
            hasher.update(text.as_bytes());
        }
        hasher.update(row.get::<_, i64>(3).map_err(durable)?.to_be_bytes());
    }
    hasher.update(count.to_be_bytes());
    Ok(hex::encode(hasher.finalize()))
}
fn notification_digest(
    owner: &str,
    revision: i64,
    resource: &str,
    job: &str,
    value: &Notification,
) -> Result<String> {
    Ok(limeos_identity::digest(&json(&(
        &value.id,
        owner,
        revision,
        resource,
        job,
        &value.event,
    ))?))
}
fn old_alert(
    conn: &Connection,
    owner: &str,
    entry: &ScopedHealthSignal,
) -> Result<Option<AlertState>> {
    let old: Option<(String, String)> = conn
        .query_row(
            "SELECT resource,body FROM automation_alerts WHERE owner=? AND key=?",
            params![owner, entry.signal.key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(durable)?;
    old.map(|(resource, body)| {
        if resource != entry.resource {
            return Err(Error(ErrorCode::Conflict));
        }
        if body.len() > ROW_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let state: AlertState = parse(&body)?;
        state.validate()?;
        Ok(state)
    })
    .transpose()
}
fn fold(
    tx: &Transaction<'_>,
    ctx: &Context,
    receipt: &ObservationResult,
    batch: &HealthObservation,
) -> Result<u16> {
    let mut created = 0;
    let policy = ctx.policy.definition.runtime_policy()?;
    for entry in &batch.signals {
        let state = old_alert(tx, &ctx.owner, entry)?;
        let folded = fold_alert(
            state.as_ref(),
            &entry.signal,
            &policy,
            receipt.sequence,
            receipt.recorded,
        )?;
        if folded.changed {
            if state.is_none() {
                let count: i64 = tx
                    .query_row("SELECT count(*) FROM automation_alerts", [], |r| r.get(0))
                    .map_err(durable)?;
                if count >= MAX_ALERTS as i64 {
                    return Err(Error(ErrorCode::Overloaded));
                }
            }
            tx.execute("INSERT INTO automation_alerts VALUES(?,?,?,?,?) ON CONFLICT(owner,key) DO UPDATE SET body=excluded.body,updated=excluded.updated",params![ctx.owner,entry.signal.key,entry.resource,json(&folded.next)?,receipt.recorded]).map_err(durable)?;
        }
        if let Some(alert) = folded.event {
            audit_capacity(tx, 3)?;
            let count: i64 = tx
                .query_row("SELECT count(*) FROM automation_notifications", [], |r| {
                    r.get(0)
                })
                .map_err(durable)?;
            if count >= MAX_NOTIFICATIONS as i64 {
                return Err(Error(ErrorCode::Overloaded));
            }
            let notification = Notification {
                id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
                revision: 1,
                event: alert,
                channel: DeliveryChannel::InApp,
                delivery: DeliveryState::Pending,
                attempts: 0,
                acknowledged_at: None,
            };
            notification.validate()?;
            let phase = match notification.event.event {
                AlertEventKind::Incident => "incident",
                AlertEventKind::Recovery => "recovery",
            };
            tx.execute(
                "INSERT INTO automation_notifications VALUES(?,?,?,?,?,?,?,?,?)",
                params![
                    notification.id,
                    ctx.owner,
                    ctx.revision,
                    entry.resource,
                    receipt.job,
                    notification.event.key,
                    notification.event.generation,
                    phase,
                    json(&notification)?
                ],
            )
            .map_err(durable)?;
            event(
                tx,
                Some(&ctx.owner),
                Some(&receipt.job),
                EventKind::AutomationNotificationCreated {
                    notification: notification.id.clone(),
                    digest: notification_digest(
                        &ctx.owner,
                        ctx.revision,
                        &entry.resource,
                        &receipt.job,
                        &notification,
                    )?,
                },
                receipt.recorded,
            )?;
            created += 1;
        }
    }
    Ok(created)
}

impl Store {
    /// Internal query-only recovery hints, never a grant to collect or complete.
    pub fn automation_queued_observation_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<AutomationPage<ObservationJob>> {
        if !(1..=MAX_TICK_SCHEDULES).contains(&limit)
            || after.is_some_and(|id| !limeos_domain::opaque_id(id))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut statement = self.conn.prepare("SELECT id,intent FROM jobs WHERE id>? AND state='queued' AND json_extract(intent,'$.operation')='automation_observe_health' ORDER BY id LIMIT ?").map_err(durable)?;
        let mut rows = statement
            .query(params![after.unwrap_or(""), limit as i64 + 1])
            .map_err(durable)?;
        let mut items = Vec::new();
        let mut next = None;
        while let Some(row) = rows.next().map_err(durable)? {
            if items.len() == limit {
                next = items.last().map(|job: &ObservationJob| job.id.clone());
                break;
            }
            let id = row.get_ref(0).map_err(durable)?;
            let id = id.as_str().map_err(|_| Error(ErrorCode::StateNotDurable))?;
            let intent = row.get_ref(1).map_err(durable)?;
            let intent = intent
                .as_str()
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
            items.push(queued_hint(id, intent)?);
        }
        Ok(AutomationPage {
            items,
            next,
            partial: false,
        })
    }

    pub fn automation_observation_begin(
        &mut self,
        job: &ObservationJob,
        selected: Vec<String>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ObservationAdmission> {
        reservation_live(Some(deadline))?;
        let selected = resources(selected)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        reservation_live(Some(deadline))?;
        let now = clock();
        reservation_live(Some(deadline))?;
        time(now)?;
        let ctx = context(&tx, job)?;
        let actor = current(&tx, &ctx, job)?;
        for resource in &selected {
            read_authorized(&tx, &actor, resource)?;
        }
        if let Some((receipt, _)) = result(&tx, &job.id)? {
            reservation_live(Some(deadline))?;
            tx.commit().map_err(durable)?;
            return Ok(ObservationAdmission::Existing(receipt));
        }
        if ctx.state != JobState::Queued || now < ctx.queued {
            return Err(Error(ErrorCode::Conflict));
        }
        if now >= ctx.deadline {
            return Err(Error(ErrorCode::Expired));
        }
        oldest(&tx, &ctx, job)?;
        self.observation_leases
            .retain(|_, entry| Instant::now() < entry.deadline);
        if self.observation_leases.contains_key(&ctx.owner) {
            return Err(Error(ErrorCode::Conflict));
        }
        if self.observation_leases.len() >= QUEUE_CAPACITY {
            return Err(Error(ErrorCode::Overloaded));
        }
        let sequence = next_sequence(&tx, &ctx.owner)?;
        let nonce = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let deadline = deadline
            .min(Instant::now() + Duration::from_secs(LEASE_SECONDS))
            .min(Instant::now() + Duration::from_secs((ctx.deadline - now) as u64));
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        self.observation_leases.insert(
            ctx.owner.clone(),
            ActiveObservation {
                nonce: nonce.clone(),
                job: Some(job.id.clone()),
                deadline,
            },
        );
        Ok(ObservationAdmission::Lease(ObservationLease {
            nonce,
            job: job.clone(),
            owner: ctx.owner,
            revision: actor.grant_revision,
            generation: self.generation,
            sequence,
            resources: selected,
            policy: Box::new(ctx.policy),
            deadline,
        }))
    }
    pub fn automation_observation_complete(
        &mut self,
        lease: &ObservationLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
    ) -> Result<ObservationResult> {
        self.complete_observation_before(lease, batch, clock, lease.deadline, None)
    }
    fn complete_observation_before(
        &mut self,
        lease: &ObservationLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
        read_deadline: Instant,
        source: Option<&ContainerHealthCohort>,
    ) -> Result<ObservationResult> {
        let deadline = read_deadline.min(lease.deadline);
        reservation_live(Some(deadline))?;
        let digest = observation_digest(batch)?;
        let active = self
            .observation_leases
            .get(&lease.owner)
            .is_some_and(|entry| {
                entry.nonce == lease.nonce
                    && entry.job.as_deref() == Some(lease.job.id.as_str())
                    && entry.deadline == lease.deadline
            });
        let generation = self.generation;
        let answer = self.write(|tx| {
            reservation_live(Some(deadline))?;
            let now = clock();
            reservation_live(Some(deadline))?;
            time(now)?;
            let ctx = context(tx, &lease.job)?;
            let actor = current(tx, &ctx, &lease.job)?;
            for resource in &lease.resources {
                read_authorized(tx, &actor, resource)?;
            }
            if let Some(cohort) = source {
                polling::container_bindings(
                    tx,
                    &lease.owner,
                    &lease.resources,
                    &lease.policy,
                    cohort,
                    now,
                )?;
            }
            if let Some((receipt, old_digest)) = result(tx, &lease.job.id)? {
                reservation_live(Some(deadline))?;
                return if old_digest == digest {
                    Ok(receipt)
                } else {
                    Err(Error(ErrorCode::Conflict))
                };
            }
            if !active
                || generation != lease.generation
                || actor.grant_revision != lease.revision
                || ctx.owner != lease.owner
                || ctx.policy != *lease.policy
            {
                return Err(Error(ErrorCode::Expired));
            }
            if now >= ctx.deadline {
                return Err(Error(ErrorCode::Expired));
            }
            if now < ctx.queued || ctx.state != JobState::Queued {
                return Err(Error(ErrorCode::Conflict));
            }
            oldest(tx, &ctx, &lease.job)?;
            if next_sequence(tx, &ctx.owner)? != lease.sequence {
                return Err(Error(ErrorCode::Conflict));
            }
            for signal in &batch.signals {
                if lease.resources.binary_search(&signal.resource).is_err() {
                    return Err(Error(ErrorCode::Forbidden));
                }
            }
            let mut receipt = ObservationResult {
                job: lease.job.id.clone(),
                sequence: lease.sequence,
                state: JobState::Succeeded,
                complete: batch.complete,
                recorded: now,
                notifications: 0,
            };
            audit_capacity(tx, 2)?;
            receipt.notifications = fold(tx, &ctx, &receipt, batch)?;
            save_result(tx, &lease.job, &ctx, &receipt, "observed", &digest)?;
            finish_job(tx, &ctx, &receipt)?;
            reservation_live(Some(deadline))?;
            Ok(receipt)
        });
        if answer.is_ok()
            || matches!(
                answer,
                Err(Error(ErrorCode::Expired | ErrorCode::Forbidden))
            )
        {
            self.release_observation(lease);
        }
        answer
    }
    fn release_observation(&mut self, lease: &ObservationLease) {
        if self
            .observation_leases
            .get(&lease.owner)
            .is_some_and(|entry| entry.nonce == lease.nonce)
        {
            self.observation_leases.remove(&lease.owner);
        }
    }
    pub fn automation_observation_discard(
        &mut self,
        job: &ObservationJob,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ObservationResult> {
        reservation_live(Some(deadline))?;
        let receipt = self.write(|tx| {
            reservation_live(Some(deadline))?;
            let now = clock();
            reservation_live(Some(deadline))?;
            time(now)?;
            let ctx = context(tx, job)?;
            if let Some((receipt, _)) = result(tx, &job.id)? {
                reservation_live(Some(deadline))?;
                return Ok(receipt);
            }
            if ctx.state != JobState::Queued || now < ctx.queued {
                return Err(Error(ErrorCode::Conflict));
            }
            oldest(tx, &ctx, job)?;
            let (state, reason) = match current(tx, &ctx, job) {
                Err(Error(ErrorCode::Forbidden | ErrorCode::Expired)) => {
                    (JobState::PreconditionChanged, "authority_changed")
                }
                Err(error) => return Err(error),
                Ok(_) if now >= ctx.deadline => (JobState::Failed, "expired"),
                Ok(_) => return Err(Error(ErrorCode::Conflict)),
            };
            let receipt = ObservationResult {
                job: job.id.clone(),
                sequence: next_sequence(tx, &ctx.owner)?,
                state,
                complete: false,
                recorded: now,
                notifications: 0,
            };
            let digest = limeos_identity::digest(&format!("{reason}:{}", job.id));
            save_result(tx, job, &ctx, &receipt, reason, &digest)?;
            finish_job(tx, &ctx, &receipt)?;
            reservation_live(Some(deadline))?;
            Ok(receipt)
        })?;
        self.observation_leases
            .retain(|_, entry| entry.job.as_deref() != Some(job.id.as_str()));
        Ok(receipt)
    }
}

fn queued_hint(id: &str, intent: &str) -> Result<ObservationJob> {
    if !limeos_domain::opaque_id(id) || intent.len() > DEFINITION_BYTES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let Intent::AutomationObserveHealth { occurrence } = parse(intent)? else {
        return Err(Error(ErrorCode::StateNotDurable));
    };
    if !limeos_domain::opaque_id(&occurrence.schedule)
        || !(1..=MAX_REVISION).contains(&occurrence.revision)
        || !(0..=MAX_TIME).contains(&occurrence.due)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(ObservationJob {
        id: id.into(),
        occurrence,
    })
}

pub(super) fn check(conn: &Connection) -> Result<()> {
    check::check(conn)?;
    polling::check(conn)
}

#[cfg(test)]
mod tests;
