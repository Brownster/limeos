//! One owned, fair observation future. No effect dispatch or detached producer.
use crate::automation_store::observation_retryable as recoverable;
use crate::{automation_store::ObservationStoreBridge, health_collection::HealthReader};
use limeos_domain::{Error, ErrorCode, Result, automation::*};
use limeos_persistence::{
    ContainerReadAdmission, ContainerReadOutcome, ContainerReadPermit, ObservationWorkHint,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct Status(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    running: bool,
    rows: BTreeMap<String, (i64, ObservationRuntimeView)>,
}
struct Running(Status);
impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.0.lock() {
            state.running = false;
            state.rows.clear();
        }
    }
}
impl Status {
    fn enter(&self) -> Result<Running> {
        let mut state = self.0.lock().map_err(|_| Error(ErrorCode::Unavailable))?;
        if state.running {
            return Err(Error(ErrorCode::Conflict));
        }
        state.running = true;
        Ok(Running(self.clone()))
    }
    fn mark(&self, id: &str, revision: i64, phase: ObservationPhase, at: i64) -> Result<()> {
        let view = ObservationRuntimeView {
            phase,
            last_attempt: Some(at),
        };
        view.validate()?;
        let mut state = self.0.lock().map_err(|_| Error(ErrorCode::Unavailable))?;
        if !state.running {
            return Ok(());
        }
        if !state.rows.contains_key(id) && state.rows.len() >= MAX_SCHEDULES {
            state.rows.pop_first();
        }
        state.rows.insert(id.into(), (revision, view));
        Ok(())
    }
    fn apply(state: &State, view: &mut ScheduleView) -> Result<()> {
        view.validate()?;
        if state.running && view.status == ScheduleStatus::AwaitingProducer {
            view.status = ScheduleStatus::Scheduled;
            view.runtime = Some(
                state
                    .rows
                    .get(&view.schedule.id)
                    .filter(|(revision, _)| *revision == view.schedule.revision)
                    .map(|(_, view)| view.clone())
                    .unwrap_or(ObservationRuntimeView {
                        phase: ObservationPhase::Waiting,
                        last_attempt: None,
                    }),
            );
        }
        view.validate()
    }
    pub(crate) fn project(&self, mut view: ScheduleView) -> Result<ScheduleView> {
        let state = self.0.lock().map_err(|_| Error(ErrorCode::Unavailable))?;
        Self::apply(&state, &mut view)?;
        Ok(view)
    }
    pub(crate) fn project_page(
        &self,
        mut page: AutomationPage<ScheduleView>,
    ) -> Result<AutomationPage<ScheduleView>> {
        let state = self.0.lock().map_err(|_| Error(ErrorCode::Unavailable))?;
        for view in &mut page.items {
            Self::apply(&state, view)?;
        }
        page.partial |= state.running;
        Ok(page)
    }
}

pub(crate) struct ObservationRuntime {
    bridge: ObservationStoreBridge,
    reader: HealthReader,
    status: Status,
    owner_after: Option<String>,
    schedule_after: BTreeMap<String, String>,
    publish_after: Option<String>,
    prune_after: Option<String>,
}
struct Work {
    id: String,
    revision: i64,
    permit: ContainerReadPermit,
}
#[derive(Default, Debug)]
struct Pass {
    owners: usize,
    admitted: usize,
    reads: usize,
    completed: usize,
    unavailable: usize,
}
impl ObservationRuntime {
    pub(crate) fn new(
        bridge: ObservationStoreBridge,
        reader: HealthReader,
        status: Status,
    ) -> Self {
        Self {
            bridge,
            reader,
            status,
            owner_after: None,
            schedule_after: BTreeMap::new(),
            publish_after: None,
            prune_after: None,
        }
    }
    pub(crate) async fn run(mut self) -> Result<()> {
        let _running = self.status.enter()?;
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            match self.step(Instant::now() + Duration::from_secs(8)).await {
                Ok(pass) => tracing::trace!(
                    owners = pass.owners,
                    admitted = pass.admitted,
                    reads = pass.reads,
                    completed = pass.completed,
                    unavailable = pass.unavailable,
                    "observation pass settled"
                ),
                Err(error) => {
                    if !recoverable(&error) {
                        return Err(error);
                    }
                }
            }
        }
    }
    async fn step(&mut self, deadline: Instant) -> Result<Pass> {
        let mut work = Vec::new();
        let answer = self.step_inner(deadline, &mut work).await;
        if answer.is_err() {
            for item in &work {
                let _ = self.status.mark(
                    &item.id,
                    item.revision,
                    ObservationPhase::Unavailable,
                    super::now(),
                );
            }
        }
        // Matching nonce/generation only; earlier commits or newer work cannot
        // be undone. Cancellation instead leaves original volatile expiry.
        self.bridge
            .abandon_container_batch(work.into_iter().map(|w| w.permit).collect())
            .await;
        answer
    }
    async fn step_inner(&mut self, deadline: Instant, work: &mut Vec<Work>) -> Result<Pass> {
        // Maintenance progresses before any potentially slow protected read.
        match self
            .bridge
            .publish_page(self.publish_after.clone(), deadline)
            .await
        {
            Ok(page) => self.publish_after = page.next,
            Err(error) if recoverable(&error) => {}
            Err(error) => return Err(error),
        }
        match self
            .bridge
            .prune_page(self.prune_after.clone(), deadline)
            .await
        {
            Ok((_, next)) => self.prune_after = next,
            Err(error) if recoverable(&error) => {}
            Err(error) => return Err(error),
        }
        let page = self
            .bridge
            .owner_page(self.owner_after.clone(), deadline)
            .await?;
        if page.owners.len() > MAX_TICK_SCHEDULES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        self.owner_after = page.next;
        let mut pass = Pass {
            owners: page.owners.len(),
            ..Default::default()
        };
        for owner in page.owners {
            let Some(hint) = self
                .bridge
                .owner_work(
                    owner.clone(),
                    self.schedule_after.get(&owner).cloned(),
                    deadline,
                )
                .await?
            else {
                continue;
            };
            let (id, revision, admission) = match hint {
                ObservationWorkHint::Queued(job) => {
                    let id = job.occurrence.schedule.clone();
                    let revision = job.occurrence.revision;
                    (
                        id,
                        revision,
                        self.bridge.prepare_container_job(job, deadline).await,
                    )
                }
                ObservationWorkHint::Schedule(schedule) => {
                    if !self.schedule_after.contains_key(&owner)
                        && self.schedule_after.len() >= MAX_SCHEDULES
                    {
                        self.schedule_after.pop_first();
                    }
                    self.schedule_after.insert(owner, schedule.id.clone());
                    if !schedule.definition.operation.observation_only() {
                        if let Err(error) = self
                            .bridge
                            .settle_unavailable_adapter(schedule, deadline)
                            .await
                        {
                            if !recoverable(&error) {
                                return Err(error);
                            }
                        }
                        continue;
                    }
                    let id = schedule.id.clone();
                    let revision = schedule.revision;
                    (
                        id.clone(),
                        revision,
                        self.bridge.prepare_container_poll(id, deadline).await,
                    )
                }
            };
            match admission {
                Ok(ContainerReadAdmission::Read(permit)) => {
                    self.status
                        .mark(&id, revision, ObservationPhase::Checking, super::now())?;
                    work.push(Work {
                        id,
                        revision,
                        permit: *permit,
                    });
                }
                Ok(ContainerReadAdmission::Existing(receipt)) => {
                    self.status.mark(
                        &id,
                        revision,
                        if receipt.state == limeos_domain::JobState::Succeeded {
                            ObservationPhase::Partial
                        } else {
                            ObservationPhase::Unavailable
                        },
                        super::now(),
                    )?;
                }
                Ok(_) => {}
                Err(error) if recoverable(&error) => {
                    self.status
                        .mark(&id, revision, ObservationPhase::Unavailable, super::now())?;
                    pass.unavailable += 1;
                }
                Err(error) => return Err(error),
            }
        }
        pass.admitted = work.len();
        if work.is_empty() {
            return Ok(pass);
        }
        let earliest = work
            .iter()
            .map(|w| w.permit.deadline())
            .min()
            .unwrap()
            .min(deadline);
        pass.reads = 1;
        let answer = match self.reader.read(earliest).await {
            Ok(fresh) => {
                self.bridge
                    .complete_container_batch(
                        work.iter().map(|w| w.permit.clone()).collect(),
                        fresh,
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        match answer {
            Ok(results) => {
                if results.len() != work.len() {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                for (item, result) in work.iter().zip(results) {
                    match result {
                        Ok(ContainerReadOutcome::Poll(_) | ContainerReadOutcome::Queued(_)) => {
                            self.status.mark(
                                &item.id,
                                item.revision,
                                ObservationPhase::Partial,
                                super::now(),
                            )?;
                            pass.completed += 1;
                        }
                        Err(error) => {
                            self.status.mark(
                                &item.id,
                                item.revision,
                                ObservationPhase::Unavailable,
                                super::now(),
                            )?;
                            pass.unavailable += 1;
                            if !recoverable(&error) {
                                return Err(error);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                for item in work.iter() {
                    self.status.mark(
                        &item.id,
                        item.revision,
                        ObservationPhase::Unavailable,
                        super::now(),
                    )?;
                    pass.unavailable += 1;
                }
                if !recoverable(&error) {
                    return Err(error);
                }
            }
        }
        Ok(pass)
    }
}

#[cfg(test)]
mod tests;
