//! Fixed authenticated schedule metadata; no timer or observation adapter.
use super::*;
use limeos_domain::automation::{AutomationPage, CreateSchedule, PauseSchedule, ScheduleView};
use std::time::Instant;

impl Core {
    pub(super) async fn human_schedule_page(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<ScheduleView>> {
        let session = limeos_identity::digest(&token);
        self.db
            .call(move |store| {
                store.observe_health_schedule_page(&session, after.as_deref(), now, deadline)
            })
            .await
    }

    pub(super) async fn human_schedule_create(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthScheduleInput,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        let session = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        let input = CreateSchedule {
            definition: input.definition(),
        };
        self.db
            .call(move |store| {
                store.observe_health_schedule_create(&session, &csrf, &input, now, deadline)
            })
            .await
    }

    pub(super) async fn human_schedule_pause(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: PauseSchedule,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        let session = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |store| {
                store.observe_health_schedule_pause(&session, &csrf, &id, &input, now, deadline)
            })
            .await
    }
}

#[cfg(test)]
mod tests;
