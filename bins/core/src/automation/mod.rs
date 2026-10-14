//! Observation-only runner. The integrator implements this boundary on the existing Store.
use limeos_domain::automation::*;
use limeos_domain::{Error, ErrorCode, Result, identifier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settlement {
    Observation(OccurrenceKey),
    Skipped(OccurrenceKey),
    AdapterUnavailable(OccurrenceKey),
}
impl Settlement {
    pub fn key(&self) -> &OccurrenceKey {
        match self {
            Self::Observation(key) | Self::Skipped(key) | Self::AdapterUnavailable(key) => key,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationJob {
    pub id: String,
    pub occurrence: OccurrenceKey,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reservation {
    Queued(ObservationJob),
    Existing,
    Skipped,
    Blocked,
    Stale,
}

/// Proposed adapter to the sole central writer. Implementations must not use a
/// separate database, queue or recovery ledger. No method here grants effect authority.
pub trait ObservationStore {
    /// Sorted ascending, unique schedule IDs. Never writes or advances a cursor.
    fn schedule_page(&mut self, after: Option<&str>, limit: usize) -> Result<Vec<Schedule>>;

    /// Reauthenticate current owner and recheck revision/pause/definition/current
    /// due decision under the transaction's trusted clock. Unique occurrence + job
    /// insertion + watermark + audit are one transaction. Full queue rolls back all.
    /// Non-observation settlements create no executable job. Return Existing for
    /// a previously settled tuple, including after restart or concurrent ticks.
    fn reserve(
        &mut self,
        expected: &Schedule,
        settlement: &Settlement,
        now: i64,
    ) -> Result<Reservation>;

    /// Complete the existing job once. Atomically consume its observation sequence,
    /// fold changed alert states, insert unique events/in-app outbox, finish job and
    /// audit. Duplicate completion returns the original result; failures roll back.
    fn complete_observation(
        &mut self,
        job: &ObservationJob,
        batch: &ObservationBatch,
        now: i64,
    ) -> Result<()>;

    /// Publish bounded in-app outbox rows in the existing transaction. Mark delivered
    /// only when the authenticated notification view can read the committed row.
    /// No external delivery, model invocation, acknowledgment or retry as a GET effect.
    fn publish_in_app(&mut self, limit: usize, now: i64) -> Result<usize>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationBatch {
    pub signals: Vec<AlertSignal>,
    pub complete: bool,
}
impl ObservationBatch {
    pub fn validate(&self) -> Result<()> {
        if self.signals.len() > MAX_ALERTS {
            return Err(Error(ErrorCode::Overloaded));
        }
        for (i, signal) in self.signals.iter().enumerate() {
            signal.validate()?;
            if self.signals[..i]
                .iter()
                .any(|other| other.key == signal.key)
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        Ok(())
    }
}
pub trait HealthReader {
    /// The registered protected read-worker supplies observation facts and partial
    /// collection status. Missing data must not be represented as healthy.
    fn observe(&mut self) -> Result<ObservationBatch>;
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub scanned: usize,
    pub queued: Vec<ObservationJob>,
    pub existing: usize,
    pub skipped: usize,
    pub blocked: usize,
    pub stale: usize,
    pub clock_rollback: usize,
    pub next_cursor: Option<String>,
}

/// One bounded page per invocation; the central supervisor schedules fair subsequent
/// pages, cancels/shuts down between calls and supplies the real clock. No sleeps/threads.
pub fn tick(
    store: &mut impl ObservationStore,
    after: Option<&str>,
    now: i64,
) -> Result<TickReport> {
    if !(0..=MAX_TIME).contains(&now) || after.is_some_and(|id| !identifier(id)) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let rows = store.schedule_page(after, MAX_TICK_SCHEDULES)?;
    if rows.len() > MAX_TICK_SCHEDULES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    // Validate the entire admitted page before any reservation.
    let mut previous = after;
    for row in &rows {
        row.validate()?;
        if previous.is_some_and(|id| row.id.as_str() <= id) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        previous = Some(&row.id);
    }
    let mut report = TickReport {
        scanned: rows.len(),
        ..Default::default()
    };
    if rows.len() == MAX_TICK_SCHEDULES {
        report.next_cursor = rows.last().map(|row| row.id.clone());
    }
    for row in rows {
        let settlement = match row.due(now)? {
            DueDecision::NotDue | DueDecision::Paused => continue,
            DueDecision::ClockRollback => {
                report.clock_rollback += 1;
                continue;
            }
            DueDecision::Skipped(key) => Settlement::Skipped(key),
            DueDecision::Ready(key) if row.definition.operation.observation_only() => {
                Settlement::Observation(key)
            }
            DueDecision::Ready(key) => Settlement::AdapterUnavailable(key),
        };
        match store.reserve(&row, &settlement, now)? {
            Reservation::Queued(job)
                if matches!(settlement, Settlement::Observation(_))
                    && identifier(&job.id)
                    && job.occurrence == *settlement.key() =>
            {
                report.queued.push(job)
            }
            Reservation::Existing => report.existing += 1,
            Reservation::Skipped if matches!(settlement, Settlement::Skipped(_)) => {
                report.skipped += 1
            }
            Reservation::Blocked if matches!(settlement, Settlement::AdapterUnavailable(_)) => {
                report.blocked += 1
            }
            Reservation::Stale => report.stale += 1,
            _ => return Err(Error(ErrorCode::StateNotDurable)),
        }
    }
    Ok(report)
}

pub fn observe_job(
    store: &mut impl ObservationStore,
    reader: &mut impl HealthReader,
    job: &ObservationJob,
    now: i64,
) -> Result<()> {
    if !identifier(&job.id)
        || !identifier(&job.occurrence.schedule)
        || !(1..=MAX_REVISION).contains(&job.occurrence.revision)
        || !(0..=MAX_TIME).contains(&now)
        || !(0..=now).contains(&job.occurrence.due)
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let batch = reader.observe()?;
    batch.validate()?;
    store.complete_observation(job, &batch, now)
}

pub fn publish_notifications(store: &mut impl ObservationStore, now: i64) -> Result<usize> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let count = store.publish_in_app(MAX_TICK_SCHEDULES, now)?;
    if count > MAX_TICK_SCHEDULES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(count)
}

#[cfg(test)]
mod tests;
