//! Shared runner values. These grant no dispatch or effect authority.
use super::{AlertSignal, MAX_ALERTS, OccurrenceKey};
use crate::{Error, ErrorCode, Result};

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
