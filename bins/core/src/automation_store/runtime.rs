//! Bounded runtime calls, sharing the existing admission and completion path.
use super::*;
use crate::health_collection::FreshContainerHealth;
use limeos_domain::automation::Schedule;
use limeos_persistence::{ObservationOwnerPage, ObservationWorkHint};

impl ObservationStoreBridge {
    pub(crate) async fn owner_page(
        &self,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<ObservationOwnerPage> {
        self.call(deadline, move |store| {
            store.automation_observation_owner_page(after.as_deref(), MAX_TICK_SCHEDULES)
        })
        .await
    }
    pub(crate) async fn owner_work(
        &self,
        owner: String,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<Option<ObservationWorkHint>> {
        self.call(deadline, move |store| {
            store.automation_observation_owner_work(&owner, after.as_deref())
        })
        .await
    }
    pub(crate) async fn prepare_container_poll(
        &self,
        schedule: String,
        deadline: Instant,
    ) -> Result<ContainerReadAdmission> {
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            store.automation_container_poll_begin(&schedule, || clock(), deadline)
        })
        .await
    }
    pub(crate) async fn prepare_container_job(
        &self,
        job: ObservationJob,
        deadline: Instant,
    ) -> Result<ContainerReadAdmission> {
        let selected = job.clone();
        let clock = self.clock.clone();
        match self
            .call(deadline, move |store| {
                store.automation_container_job_begin(&selected, || clock(), deadline)
            })
            .await
        {
            Err(Error(ErrorCode::Expired | ErrorCode::Forbidden)) => self
                .discard(job, deadline)
                .await
                .map(ContainerReadAdmission::Existing),
            answer => answer,
        }
    }
    pub(crate) async fn settle_unavailable_adapter(
        &self,
        schedule: Schedule,
        deadline: Instant,
    ) -> Result<()> {
        if schedule.definition.operation.observation_only() {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            let sampled = clock();
            let settlement = match schedule.due(sampled)? {
                DueDecision::Ready(key) => Settlement::AdapterUnavailable(key),
                DueDecision::Skipped(key) => Settlement::Skipped(key),
                _ => return Ok(()),
            };
            store
                .automation_reserve_current(&schedule, &settlement, || clock(), deadline)
                .map(|_| ())
        })
        .await
    }
    pub(crate) async fn complete_container_batch(
        &self,
        permits: Vec<ContainerReadPermit>,
        fresh: FreshContainerHealth,
    ) -> Result<Vec<Result<ContainerReadOutcome>>> {
        if permits.is_empty() || permits.len() > MAX_TICK_SCHEDULES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        fresh.ensure_live()?;
        let deadline = fresh.deadline();
        let cohort = fresh.cohort().clone();
        let clock = self.clock.clone();
        self.call(deadline, move |store| {
            let mut results = Vec::new();
            for permit in &permits {
                let result =
                    store.automation_container_read_complete(permit, &cohort, || clock(), deadline);
                match result {
                    Err(error) if !observation_retryable(&error) => return Err(error),
                    result => results.push(result),
                }
            }
            Ok(results)
        })
        .await
    }
    pub(crate) async fn abandon_container_batch(&self, permits: Vec<ContainerReadPermit>) {
        if permits.len() > MAX_TICK_SCHEDULES {
            return;
        }
        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        let _ = self
            .call(deadline, move |store| {
                for permit in permits {
                    store.automation_container_read_abandon(&permit);
                }
                Ok(())
            })
            .await;
    }

    #[cfg(test)]
    pub(crate) fn with_test_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }
}
