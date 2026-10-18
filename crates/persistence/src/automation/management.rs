//! Authenticated fixed schedule metadata; no runner or producer registration.
use super::*;
use std::time::Instant;

fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(())
}

impl Store {
    pub fn observe_health_schedule_create(
        &mut self,
        session: &str,
        csrf: &str,
        input: &CreateSchedule,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        live(deadline)?;
        self.write(|tx| {
            live(deadline)?;
            // BEGIN IMMEDIATE has succeeded; no pre-lock expiry sample is reused.
            let now = clock();
            time(now)?;
            let actor = identity_store::authenticate(tx, session, Some(csrf), now)?;
            authorize(tx, &actor, true)?;
            if !input.definition.operation.observation_only() {
                return Err(Error(ErrorCode::InvalidInput));
            }
            let value = create_schedule(tx, &actor, input, now)?;
            live(deadline)?;
            Ok(value)
        })
    }

    pub fn observe_health_schedule_page(
        &mut self,
        session: &str,
        after: Option<&str>,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<AutomationPage<ScheduleView>> {
        live(deadline)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(durable)?;
        // BEGIN DEFERRED alone does not select a snapshot. Acquire it before
        // sampling expiry, then resolve the whole owner-scoped page within it.
        let _: i64 = tx
            .query_row("SELECT generation FROM meta WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(durable)?;
        live(deadline)?;
        let now = clock();
        time(now)?;
        let actor = identity_store::authenticate(&tx, session, None, now)?;
        let value = schedule_page(&tx, &actor, after, true)?;
        live(deadline)?;
        tx.commit().map_err(durable)?;
        Ok(value)
    }

    pub fn observe_health_schedule_pause(
        &mut self,
        session: &str,
        csrf: &str,
        id: &str,
        input: &PauseSchedule,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        live(deadline)?;
        if !limeos_domain::opaque_id(id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.write(|tx| {
            live(deadline)?;
            let now = clock();
            time(now)?;
            let actor = identity_store::authenticate(tx, session, Some(csrf), now)?;
            authorize(tx, &actor, true)?;
            let _row = load(tx, id)?
                .filter(|row| {
                    row.owner == actor.id && row.schedule.definition.operation.observation_only()
                })
                .ok_or(Error(ErrorCode::NotFound))?;
            let value = pause_schedule(tx, &actor, id, input, now)?;
            live(deadline)?;
            Ok(value)
        })
    }
}

#[cfg(test)]
mod tests;
