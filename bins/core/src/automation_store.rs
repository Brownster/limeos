//! Bounded calls to the actual Store worker. No timer or collector is started.
use limeos_domain::automation::{
    AutomationPage, DueDecision, MAX_TICK_SCHEDULES, MAX_TIME, ObservationJob, Reservation,
    Settlement,
};
use limeos_domain::{Error, ErrorCode, Result};
use limeos_persistence::{
    ContainerReadAdmission, ContainerReadOutcome, ContainerReadPermit, Database, HealthObservation,
    InAppPublication, ObservationAdmission, ObservationLease, ObservationPollAdmission,
    ObservationPollLease, ObservationPollOutcome, ObservationResult, Store,
};
use std::{sync::Arc, time::Instant};

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
mod runtime;

pub(crate) fn observation_retryable(error: &Error) -> bool {
    matches!(
        error.0,
        ErrorCode::Expired
            | ErrorCode::Forbidden
            | ErrorCode::Conflict
            | ErrorCode::Overloaded
            | ErrorCode::NotFound
            | ErrorCode::Unavailable
    )
}

/// Uses the sole database worker; it has no independent authority or queue.
#[derive(Clone)]
pub struct ObservationStoreBridge {
    db: Database,
    clock: Clock,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchedulePass {
    pub scanned: usize,
    pub queued: Vec<ObservationJob>,
    pub existing: usize,
    pub skipped: usize,
    pub blocked: usize,
    pub stale: usize,
    pub clock_rollback: usize,
    pub next: Option<String>,
}

fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error(ErrorCode::Expired))
    } else {
        Ok(())
    }
}

impl ObservationStoreBridge {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            clock: Arc::new(super::now),
        }
    }

    async fn call<T: Send + 'static>(
        &self,
        deadline: Instant,
        work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        live(deadline)?;
        let value = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.db.call(move |store| {
                // A dropped response future cannot remove already queued work.
                live(deadline)?;
                work(store)
            }),
        )
        .await
        .map_err(|_| Error(ErrorCode::Expired))??;
        // A late commit may have succeeded. Never present it as a timely reply.
        live(deadline)?;
        Ok(value)
    }

    /// A page may commit earlier rows before a later refusal or lost reply.
    /// Rediscover original queued jobs instead of trusting this advisory vector.
    pub async fn reserve_page(
        &self,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<SchedulePass> {
        let clock = self.clock.clone();
        let (rows, sampled) = self
            .call(deadline, move |store| {
                let rows = store.automation_schedule_page(after.as_deref(), MAX_TICK_SCHEDULES)?;
                let sampled = clock();
                if !(0..=MAX_TIME).contains(&sampled) {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                let mut previous = after.as_deref();
                for row in &rows {
                    row.validate()?;
                    if previous.is_some_and(|id| row.id.as_str() <= id) {
                        return Err(Error(ErrorCode::StateNotDurable));
                    }
                    previous = Some(&row.id);
                }
                Ok((rows, sampled))
            })
            .await?;
        let mut pass = SchedulePass {
            scanned: rows.len(),
            next: (rows.len() == MAX_TICK_SCHEDULES)
                .then(|| rows.last().expect("nonempty bounded page").id.clone()),
            ..Default::default()
        };
        for row in rows {
            let settlement = match row.due(sampled)? {
                DueDecision::NotDue | DueDecision::Paused => continue,
                DueDecision::ClockRollback => {
                    pass.clock_rollback += 1;
                    continue;
                }
                DueDecision::Skipped(key) => Settlement::Skipped(key),
                DueDecision::Ready(key) if row.definition.operation.observation_only() => {
                    Settlement::Observation(key)
                }
                DueDecision::Ready(key) => Settlement::AdapterUnavailable(key),
            };
            let clock = self.clock.clone();
            match self
                .call(deadline, move |store| {
                    store.automation_reserve_current(&row, &settlement, || clock(), deadline)
                })
                .await?
            {
                Reservation::Queued(job) => pass.queued.push(job),
                Reservation::Existing => pass.existing += 1,
                Reservation::Skipped => pass.skipped += 1,
                Reservation::Blocked => pass.blocked += 1,
                Reservation::Stale => pass.stale += 1,
            }
        }
        Ok(pass)
    }

    pub async fn queued_page(
        &self,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<ObservationJob>> {
        self.call(deadline, move |store| {
            store.automation_queued_observation_page(after.as_deref(), MAX_TICK_SCHEDULES)
        })
        .await
    }

    /// Production polling seam: no durable reservation for an unchanged sample.
    /// Concrete resources must come from the protected collector adapter.
    pub async fn poll_begin(
        &self,
        schedule: String,
        resources: Vec<String>,
        deadline: Instant,
    ) -> Result<ObservationPollAdmission> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_poll_begin(&schedule, resources, || clock(), deadline)
        })
        .await
    }

    pub async fn poll_complete(
        &self,
        lease: ObservationPollLease,
        batch: HealthObservation,
    ) -> Result<ObservationPollOutcome> {
        let clock = self.clock.clone();
        self.call(lease.deadline(), move |store| {
            store.automation_poll_complete(&lease, &batch, || clock())
        })
        .await
    }

    /// Internal partial container pass. Remaining health sources and normal
    /// scheduling are registered separately; this cannot claim global completeness.
    pub async fn poll_containers(
        &self,
        reader: &crate::health_collection::HealthReader,
        schedule: String,
        deadline: Instant,
    ) -> Result<Option<ObservationPollOutcome>> {
        let admission = self.prepare_container_poll(schedule, deadline).await?;
        let ContainerReadAdmission::Read(permit) = admission else {
            return Ok(None);
        };
        match self.collect_containers(reader, *permit).await? {
            ContainerReadOutcome::Poll(outcome) => Ok(Some(outcome)),
            ContainerReadOutcome::Queued(_) => Err(Error(ErrorCode::StateNotDurable)),
        }
    }

    /// Recover only the original queued observation. Invalid/expired work is
    /// independently discarded under current Store checks before any root read.
    pub async fn recover_container_job(
        &self,
        reader: &crate::health_collection::HealthReader,
        job: ObservationJob,
        deadline: Instant,
    ) -> Result<ObservationResult> {
        match self.prepare_container_job(job, deadline).await? {
            ContainerReadAdmission::Existing(receipt) => Ok(receipt),
            ContainerReadAdmission::Read(permit) => {
                match self.collect_containers(reader, *permit).await? {
                    ContainerReadOutcome::Queued(receipt) => Ok(receipt),
                    ContainerReadOutcome::Poll(_) => Err(Error(ErrorCode::StateNotDurable)),
                }
            }
            _ => Err(Error(ErrorCode::StateNotDurable)),
        }
    }

    async fn collect_containers(
        &self,
        reader: &crate::health_collection::HealthReader,
        permit: ContainerReadPermit,
    ) -> Result<ContainerReadOutcome> {
        let answer = async {
            let fresh = reader.read(permit.deadline()).await?;
            fresh.ensure_live()?;
            let original = fresh.deadline();
            let cohort = fresh.cohort().clone();
            let clock = self.clock.clone();
            let selected = permit.clone();
            self.call(original, move |store| {
                store.automation_container_read_complete(&selected, &cohort, || clock(), original)
            })
            .await
        }
        .await;
        if answer.is_err() {
            // Bounded best effort only. Cancellation can leave this private
            // lease until its original expiry; it never renews or settles work.
            let cleanup = Instant::now() + std::time::Duration::from_millis(100);
            let _ = self
                .call(cleanup, move |store| {
                    store.automation_container_read_abandon(&permit);
                    Ok(())
                })
                .await;
        }
        answer
    }

    pub async fn poll_abandon(&self, lease: ObservationPollLease, deadline: Instant) -> Result<()> {
        self.call(deadline, move |store| {
            store.automation_poll_abandon(&lease);
            Ok(())
        })
        .await
    }

    pub async fn begin(
        &self,
        job: ObservationJob,
        resources: Vec<String>,
        deadline: Instant,
    ) -> Result<ObservationAdmission> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_observation_begin(&job, resources, || clock(), deadline)
        })
        .await
    }

    pub async fn complete(
        &self,
        lease: ObservationLease,
        batch: HealthObservation,
    ) -> Result<ObservationResult> {
        let clock = self.clock.clone();
        self.call(lease.deadline(), move |store| {
            store.automation_observation_complete(&lease, &batch, || clock())
        })
        .await
    }

    pub async fn discard(
        &self,
        job: ObservationJob,
        deadline: Instant,
    ) -> Result<ObservationResult> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_observation_discard(&job, || clock(), deadline)
        })
        .await
    }

    pub async fn publish_page(
        &self,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<InAppPublication> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_publish_in_app(
                after.as_deref(),
                MAX_TICK_SCHEDULES,
                || clock(),
                deadline,
            )
        })
        .await
    }

    pub async fn prune_page(
        &self,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<(usize, Option<String>)> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_prune_notifications(
                after.as_deref(),
                MAX_TICK_SCHEDULES,
                || clock(),
                deadline,
            )
        })
        .await
    }
}

#[cfg(test)]
mod tests;
