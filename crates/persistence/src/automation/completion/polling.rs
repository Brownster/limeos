//! Volatile read admission; only material transitions enter the durable ledger.
use super::*;

pub(crate) struct PollWatermark {
    revision: i64,
    due: i64,
}

/// Private, owner-exclusive read capability. Never accepted from HTTP or serialized.
#[derive(Clone)]
pub struct ObservationPollLease {
    nonce: String,
    schedule: Schedule,
    occurrence: OccurrenceKey,
    owner: String,
    revision: i64,
    generation: i64,
    admitted: i64,
    resources: Vec<String>,
    policy: Box<HealthPolicyView>,
    deadline: Instant,
}
impl std::fmt::Debug for ObservationPollLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservationPollLease")
            .field("occurrence", &self.occurrence)
            .finish_non_exhaustive()
    }
}
impl ObservationPollLease {
    pub(super) fn with_resources(&self, resources: Vec<String>) -> Self {
        Self {
            resources,
            ..self.clone()
        }
    }
    pub fn occurrence(&self) -> &OccurrenceKey {
        &self.occurrence
    }
    pub fn resources(&self) -> &[String] {
        &self.resources
    }
    pub fn health_policy(&self) -> &HealthPolicyView {
        &self.policy
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}
#[derive(Debug)]
pub enum ObservationPollAdmission {
    Lease(Box<ObservationPollLease>),
    NotDue,
    Paused,
    ClockRollback,
    Skipped(OccurrenceKey),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationPollOutcome {
    /// No durable receipt exists; this sample can be repeated after restart.
    Unchanged {
        complete: bool,
        sampled_at: i64,
    },
    Committed(ObservationResult),
}

#[derive(serde::Serialize)]
struct PollOrigin<'a> {
    definition: &'a ScheduleDefinition,
    admitted: i64,
    previous_due: Option<i64>,
    lease_digest: &'a str,
}
fn origin_digest(job: &str, origin: &PollOrigin<'_>) -> Result<String> {
    Ok(limeos_identity::digest(&json(&(1, job, origin))?))
}
fn no_queued(conn: &Connection, owner: &str) -> Result<()> {
    let queued:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE principal=? AND state='queued' AND json_extract(intent,'$.operation')='automation_observe_health')",[owner],|r|r.get(0)).map_err(durable)?;
    if queued {
        Err(Error(ErrorCode::Conflict))
    } else {
        Ok(())
    }
}
fn writable(conn: &Connection, directory: &std::path::Path) -> Result<()> {
    if fs2::available_space(directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
        < MIN_FREE_BYTES
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    audit_capacity(conn, 5)
}
fn poll_current(
    conn: &Connection,
    lease: &ObservationPollLease,
    generation: i64,
    now: i64,
) -> Result<Stored> {
    reservation_live(Some(lease.deadline))?;
    time(now)?;
    if generation != lease.generation
        || now < lease.admitted
        || now - lease.admitted > LEASE_SECONDS as i64
    {
        return Err(Error(ErrorCode::Expired));
    }
    let row = load(conn, &lease.schedule.id)?.ok_or(Error(ErrorCode::Expired))?;
    let actor = principal(conn, &row.owner)?;
    authorize(conn, &actor, true)?;
    if row.owner != lease.owner
        || row.owner_revision != lease.revision
        || actor.grant_revision != lease.revision
        || row.schedule.revision != lease.schedule.revision
        || row.schedule.paused
        || row.schedule.definition != lease.schedule.definition
        || super::super::policy::current(conn, &actor.id)? != *lease.policy
    {
        return Err(Error(ErrorCode::Expired));
    }
    for resource in &lease.resources {
        read_authorized(conn, &actor, resource)?;
    }
    Ok(row)
}
fn material_change(
    conn: &Connection,
    lease: &ObservationPollLease,
    batch: &HealthObservation,
    now: i64,
) -> Result<bool> {
    let stream:Option<(bool,i64,i64,i64)>=conn.query_row("SELECT complete,owner_revision,policy_revision,recorded FROM automation_streams WHERE owner=?",[&lease.owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(durable)?;
    if stream.is_some_and(|(_, _, _, at)| now < at) {
        return Err(Error(ErrorCode::Conflict));
    }
    let mut changed = stream.is_none_or(|(complete, revision, policy, _)| {
        complete != batch.complete || revision != lease.revision || policy != lease.policy.revision
    });
    let sequence = next_sequence(conn, &lease.owner)?;
    let policy = lease.policy.definition.runtime_policy()?;
    // Validate every binding and fold, even after finding an earlier change.
    for entry in &batch.signals {
        if lease.resources.binary_search(&entry.resource).is_err() {
            return Err(Error(ErrorCode::Forbidden));
        }
        let state = old_alert(conn, &lease.owner, entry)?;
        changed |= fold_alert(state.as_ref(), &entry.signal, &policy, sequence, now)?.changed;
    }
    Ok(changed)
}

pub(super) fn container_bindings(
    conn: &Connection,
    owner: &str,
    resources: &[String],
    policy: &HealthPolicyView,
    cohort: &ContainerHealthCohort,
    now: i64,
) -> Result<()> {
    cohort.check_time(cohort.sampled_at(), now)?;
    let mut scopes = vec!["host:system".into()];
    scopes.extend(cohort.members().iter().map(|m| m.resource.clone()));
    scopes.sort();
    if scopes != resources {
        return Err(Error(ErrorCode::Forbidden));
    }
    let present = |resource: &str| {
        resource
            .strip_prefix("container:")
            .is_some_and(limeos_domain::opaque_id)
            && cohort
                .members()
                .binary_search_by(|m| m.resource.as_str().cmp(resource))
                .is_ok()
    };
    if policy
        .definition
        .silences
        .iter()
        .any(|silence| silence.kind == AlertKind::Container && !present(&silence.key))
    {
        return Err(Error(ErrorCode::Conflict));
    }
    let mut statement=conn.prepare("SELECT key,resource FROM automation_alerts WHERE owner=? AND json_extract(body,'$.kind')='container' LIMIT ?").map_err(durable)?;
    let mut rows = statement
        .query(params![owner, MAX_ALERTS as i64 + 1])
        .map_err(durable)?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(durable)? {
        count += 1;
        if count > MAX_ALERTS {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let key: String = row.get(0).map_err(durable)?;
        let resource: String = row.get(1).map_err(durable)?;
        if key != resource || !present(&resource) {
            return Err(Error(ErrorCode::Conflict));
        }
    }
    Ok(())
}

impl Store {
    /// Read-only admission for an ObserveHealth schedule. Hints convey no authority.
    pub fn automation_poll_begin(
        &mut self,
        id: &str,
        selected: Vec<String>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ObservationPollAdmission> {
        reservation_live(Some(deadline))?;
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let selected = resources(selected)?;
        let deadline = deadline.min(Instant::now() + Duration::from_secs(LEASE_SECONDS));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        reservation_live(Some(deadline))?;
        let now = clock();
        time(now)?;
        reservation_live(Some(deadline))?;
        let row = load(&tx, id)?.ok_or(Error(ErrorCode::NotFound))?;
        if !row.schedule.definition.operation.observation_only() {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let actor = principal(&tx, &row.owner)?;
        authorize(&tx, &actor, true)?;
        if row.owner_revision != actor.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        for resource in &selected {
            read_authorized(&tx, &actor, resource)?;
        }
        let policy = super::super::policy::current(&tx, &actor.id)?;
        if policy.updated.is_some_and(|at| now < at) {
            return Err(Error(ErrorCode::Conflict));
        }
        let mut schedule = row.schedule;
        if let Some(mark) = self
            .observation_watermarks
            .get(id)
            .filter(|m| m.revision == schedule.revision)
        {
            schedule.settled_through = Some(
                schedule
                    .settled_through
                    .map_or(mark.due, |due| due.max(mark.due)),
            );
        }
        let occurrence = match schedule.due(now)? {
            DueDecision::NotDue => return Ok(ObservationPollAdmission::NotDue),
            DueDecision::Paused => return Ok(ObservationPollAdmission::Paused),
            DueDecision::ClockRollback => return Ok(ObservationPollAdmission::ClockRollback),
            DueDecision::Skipped(key) => {
                no_queued(&tx, &actor.id)?;
                writable(&tx, &self.directory)?;
                let reserved = reserve_admitted_occurrence(
                    &tx,
                    &schedule,
                    &Settlement::Skipped(key.clone()),
                    now,
                    self.generation,
                    Some((now, schedule.settled_through)),
                )?;
                if !matches!(reserved, Reservation::Skipped) {
                    return Err(Error(ErrorCode::Conflict));
                }
                reservation_live(Some(deadline))?;
                tx.commit().map_err(durable)?;
                self.observation_watermarks.insert(
                    id.into(),
                    PollWatermark {
                        revision: key.revision,
                        due: key.due,
                    },
                );
                return Ok(ObservationPollAdmission::Skipped(key));
            }
            DueDecision::Ready(key) => key,
        };
        no_queued(&tx, &actor.id)?;
        self.observation_leases
            .retain(|_, entry| Instant::now() < entry.deadline);
        if self.observation_leases.contains_key(&actor.id) {
            return Err(Error(ErrorCode::Conflict));
        }
        if self.observation_leases.len() >= QUEUE_CAPACITY {
            return Err(Error(ErrorCode::Overloaded));
        }
        let nonce = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        self.observation_leases.insert(
            actor.id.clone(),
            ActiveObservation {
                nonce: nonce.clone(),
                job: None,
                deadline,
            },
        );
        Ok(ObservationPollAdmission::Lease(Box::new(
            ObservationPollLease {
                nonce,
                schedule,
                occurrence,
                owner: actor.id,
                revision: actor.grant_revision,
                generation: self.generation,
                admitted: now,
                resources: selected,
                policy: Box::new(policy),
                deadline,
            },
        )))
    }

    /// Consume one sample; unchanged observations perform no SQL writes.
    pub fn automation_poll_complete(
        &mut self,
        lease: &ObservationPollLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
    ) -> Result<ObservationPollOutcome> {
        self.automation_poll_complete_before(lease, batch, clock, lease.deadline)
    }
    /// A trusted fresh reader may shorten completion lifetime, never renew it.
    pub fn automation_poll_complete_before(
        &mut self,
        lease: &ObservationPollLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
        read_deadline: Instant,
    ) -> Result<ObservationPollOutcome> {
        self.settle_poll(lease, batch, clock, read_deadline, None)
    }
    /// A partial container pass cannot silently rebind legacy names or retire
    /// prior subjects. Check current source time and bindings in the commit snapshot.
    pub fn automation_poll_containers_complete_before(
        &mut self,
        lease: &ObservationPollLease,
        cohort: &ContainerHealthCohort,
        clock: impl Fn() -> i64,
        read_deadline: Instant,
    ) -> Result<ObservationPollOutcome> {
        let batch = HealthObservation {
            signals: cohort
                .members()
                .iter()
                .filter_map(|m| {
                    m.signal.clone().map(|signal| ScopedHealthSignal {
                        resource: m.resource.clone(),
                        signal,
                    })
                })
                .collect(),
            complete: false,
        };
        self.settle_poll(lease, &batch, clock, read_deadline, Some(cohort))
    }
    fn settle_poll(
        &mut self,
        lease: &ObservationPollLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
        read_deadline: Instant,
        source: Option<&ContainerHealthCohort>,
    ) -> Result<ObservationPollOutcome> {
        let answer = self.complete_poll(
            lease,
            batch,
            clock,
            read_deadline.min(lease.deadline),
            source,
        );
        // Any failure requires fresh admission. This cannot release other work.
        self.automation_poll_abandon(lease);
        if answer.is_ok() {
            let mark = self
                .observation_watermarks
                .entry(lease.schedule.id.clone())
                .or_insert(PollWatermark {
                    revision: lease.occurrence.revision,
                    due: lease.occurrence.due,
                });
            if mark.revision != lease.occurrence.revision {
                mark.revision = lease.occurrence.revision;
                mark.due = lease.occurrence.due;
            } else {
                mark.due = mark.due.max(lease.occurrence.due);
            }
        }
        answer
    }
    fn complete_poll(
        &mut self,
        lease: &ObservationPollLease,
        batch: &HealthObservation,
        clock: impl Fn() -> i64,
        deadline: Instant,
        source: Option<&ContainerHealthCohort>,
    ) -> Result<ObservationPollOutcome> {
        reservation_live(Some(deadline))?;
        let digest = observation_digest(batch)?;
        if batch.complete
            && batch
                .signals
                .iter()
                .any(|entry| entry.signal.status == SignalStatus::Unavailable)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        reservation_live(Some(deadline))?;
        let now = clock();
        reservation_live(Some(deadline))?;
        let row = poll_current(&tx, lease, self.generation, now)?;
        if let Some(cohort) = source {
            container_bindings(
                &tx,
                &lease.owner,
                &lease.resources,
                &lease.policy,
                cohort,
                now,
            )?;
        }
        // A replay needs the original private nonce, current authority and body.
        let prior:Option<(String,String)>=tx.query_row("SELECT o.job,p.lease_digest FROM automation_occurrences o JOIN automation_poll_origins p ON p.job=o.job WHERE o.schedule=? AND o.revision=? AND o.due=?",params![lease.occurrence.schedule,lease.occurrence.revision,lease.occurrence.due],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(durable)?;
        if let Some((job, nonce)) = prior {
            let (receipt, body) = result(&tx, &job)?.ok_or(Error(ErrorCode::StateNotDurable))?;
            if nonce != limeos_identity::digest(&lease.nonce) || body != digest {
                return Err(Error(ErrorCode::Conflict));
            }
            reservation_live(Some(deadline))?;
            tx.commit().map_err(durable)?;
            return Ok(ObservationPollOutcome::Committed(receipt));
        }
        let active = self
            .observation_leases
            .get(&lease.owner)
            .is_some_and(|entry| {
                entry.job.is_none()
                    && entry.nonce == lease.nonce
                    && entry.deadline == lease.deadline
            });
        if !active
            || row.schedule.settled_through.is_some_and(|due| {
                lease
                    .schedule
                    .settled_through
                    .is_none_or(|previous| previous < due)
            })
        {
            return Err(Error(ErrorCode::Expired));
        }
        no_queued(&tx, &lease.owner)?;
        if !material_change(&tx, lease, batch, now)? {
            reservation_live(Some(deadline))?;
            tx.commit().map_err(durable)?;
            return Ok(ObservationPollOutcome::Unchanged {
                complete: batch.complete,
                sampled_at: now,
            });
        }
        writable(&tx, &self.directory)?;
        let Reservation::Queued(job) = reserve_admitted_occurrence(
            &tx,
            &lease.schedule,
            &Settlement::Observation(lease.occurrence.clone()),
            now,
            self.generation,
            Some((lease.admitted, lease.schedule.settled_through)),
        )?
        else {
            return Err(Error(ErrorCode::Conflict));
        };
        let nonce = limeos_identity::digest(&lease.nonce);
        let origin = PollOrigin {
            definition: &lease.schedule.definition,
            admitted: lease.admitted,
            previous_due: lease.schedule.settled_through,
            lease_digest: &nonce,
        };
        tx.execute(
            "INSERT INTO automation_poll_origins VALUES(?,?,?,?,?)",
            params![
                job.id,
                json(origin.definition)?,
                origin.admitted,
                origin.previous_due,
                origin.lease_digest
            ],
        )
        .map_err(durable)?;
        event(
            &tx,
            Some(&lease.owner),
            Some(&job.id),
            EventKind::AutomationObservationPolled {
                digest: origin_digest(&job.id, &origin)?,
            },
            now,
        )?;
        let ctx = context(&tx, &job)?;
        let mut receipt = ObservationResult {
            job: job.id.clone(),
            sequence: next_sequence(&tx, &lease.owner)?,
            state: JobState::Succeeded,
            complete: batch.complete,
            recorded: now,
            notifications: 0,
        };
        receipt.notifications = fold(&tx, &ctx, &receipt, batch)?;
        save_result(&tx, &job, &ctx, &receipt, "observed", &digest)?;
        finish_job(&tx, &ctx, &receipt)?;
        reservation_live(Some(deadline))?;
        tx.commit().map_err(durable)?;
        Ok(ObservationPollOutcome::Committed(receipt))
    }

    /// Release only this generation's matching poll, including after expiry.
    pub fn automation_poll_abandon(&mut self, lease: &ObservationPollLease) {
        if self.generation == lease.generation
            && self
                .observation_leases
                .get(&lease.owner)
                .is_some_and(|entry| {
                    entry.job.is_none()
                        && entry.nonce == lease.nonce
                        && entry.deadline == lease.deadline
                })
        {
            self.observation_leases.remove(&lease.owner);
        }
    }
}

pub(super) fn check(conn: &Connection) -> Result<()> {
    let count: i64 = conn
        .query_row("SELECT count(*) FROM automation_poll_origins", [], |r| {
            r.get(0)
        })
        .map_err(durable)?;
    check::require(count <= MAX_RESULTS)?;
    let orphan:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM events e WHERE json_extract(e.kind,'$.kind')='automation_observation_polled' AND NOT EXISTS(SELECT 1 FROM automation_poll_origins p WHERE p.job=e.job))",[],|r|r.get(0)).map_err(durable)?;
    check::require(!orphan)?;
    // Historical occurrence detail may already have been pruned legally. The
    // immutable job intent retains its occurrence identity and all proof joins.
    let mut statement=conn.prepare("SELECT p.job,p.definition,p.admitted,p.previous_due,p.lease_digest,j.intent,r.owner,r.recorded,r.reason,r.outcome FROM automation_poll_origins p JOIN jobs j ON j.id=p.job JOIN automation_observation_results r ON r.job=p.job").map_err(durable)?;
    let mut rows = statement.query([]).map_err(durable)?;
    let mut seen = 0;
    while let Some(row) = rows.next().map_err(durable)? {
        seen += 1;
        let job: String = row.get(0).map_err(durable)?;
        let body: String = row.get(1).map_err(durable)?;
        check::require(body.len() <= DEFINITION_BYTES)?;
        let definition: ScheduleDefinition = parse(&body)?;
        check::require(json(&definition)? == body && definition.operation.observation_only())?;
        let admitted = row.get(2).map_err(durable)?;
        let previous_due = row.get(3).map_err(durable)?;
        let nonce: String = row.get(4).map_err(durable)?;
        check::require(limeos_domain::opaque_id(&nonce))?;
        let intent: String = row.get(5).map_err(durable)?;
        check::require(intent.len() <= DEFINITION_BYTES)?;
        let Intent::AutomationObserveHealth { occurrence: key } = parse(&intent)? else {
            return Err(Error(ErrorCode::StateNotDurable));
        };
        let schedule = Schedule {
            id: key.schedule.clone(),
            revision: key.revision,
            paused: false,
            definition,
            settled_through: previous_due,
        };
        check::require(schedule.due(admitted)? == DueDecision::Ready(key))?;
        let owner: String = row.get(6).map_err(durable)?;
        let recorded: i64 = row.get(7).map_err(durable)?;
        check::require(
            (admitted..=admitted + LEASE_SECONDS as i64).contains(&recorded)
                && row.get::<_, String>(8).map_err(durable)? == "observed"
                && row.get::<_, String>(9).map_err(durable)? == "succeeded",
        )?;
        let origin = PollOrigin {
            definition: &schedule.definition,
            admitted,
            previous_due,
            lease_digest: &nonce,
        };
        check::audit(
            conn,
            &owner,
            Some(&job),
            EventKind::AutomationObservationPolled {
                digest: origin_digest(&job, &origin)?,
            },
            recorded,
        )?;
        let proofs:i64=conn.query_row("SELECT count(*) FROM events WHERE job=? AND json_extract(kind,'$.kind')='automation_observation_polled'",[job],|r|r.get(0)).map_err(durable)?;
        check::require(proofs == 1)?;
    }
    check::require(seen == count)
}

#[cfg(test)]
mod tests;
