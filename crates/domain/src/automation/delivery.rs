use super::{AlertEvent, MAX_REVISION, valid_time};
use crate::{Error, ErrorCode, Result, identifier};
use serde::{Deserialize, Serialize};

pub const MAX_DELIVERY_ATTEMPTS: u8 = 4;
pub const DELIVERED_RETENTION_SECONDS: i64 = 7 * 86400;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryChannel {
    InApp,
    IdempotentReceiver,
    External,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryState {
    Pending,
    InFlight { started_at: i64 },
    Delivered { at: i64 },
    Failed { retry_at: Option<i64> },
    Ambiguous { at: i64 },
    Blocked,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub id: String,
    pub revision: i64,
    pub event: AlertEvent,
    pub channel: DeliveryChannel,
    pub delivery: DeliveryState,
    pub attempts: u8,
    pub acknowledged_at: Option<i64>,
}
impl Notification {
    pub fn validate(&self) -> Result<()> {
        self.event.validate()?;
        if !identifier(&self.id)
            || !(1..=MAX_REVISION).contains(&self.revision)
            || self.attempts > MAX_DELIVERY_ATTEMPTS
            || self
                .acknowledged_at
                .is_some_and(|at| !valid_time(at) || at < self.event.at)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let at = match self.delivery {
            DeliveryState::InFlight { started_at } => Some(started_at),
            DeliveryState::Delivered { at } | DeliveryState::Ambiguous { at } => Some(at),
            DeliveryState::Failed { retry_at } => retry_at,
            _ => None,
        };
        if at.is_some_and(|at| !valid_time(at) || at < self.event.at)
            || (matches!(
                self.delivery,
                DeliveryState::InFlight { .. }
                    | DeliveryState::Delivered { .. }
                    | DeliveryState::Ambiguous { .. }
                    | DeliveryState::Failed { .. }
            ) && self.attempts == 0)
            || (matches!(self.delivery, DeliveryState::Pending) && self.attempts != 0)
            || (matches!(self.delivery, DeliveryState::Failed { retry_at: Some(_) })
                && self.attempts == MAX_DELIVERY_ATTEMPTS)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(())
    }
    pub fn retry_due(&self, now: i64) -> Result<bool> {
        self.validate()?;
        if !valid_time(now) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if now < self.event.at || self.attempts >= MAX_DELIVERY_ATTEMPTS {
            return Ok(false);
        }
        Ok(match self.delivery {
            DeliveryState::Pending => true,
            DeliveryState::Failed { retry_at: Some(at) } => now >= at,
            DeliveryState::Ambiguous { at } => {
                self.channel == DeliveryChannel::IdempotentReceiver && now >= at + 600
            }
            _ => false,
        })
    }
    pub fn begin(&self, now: i64) -> Result<Self> {
        if !self.retry_due(now)? {
            return Err(Error(ErrorCode::Conflict));
        }
        let mut next = self.clone();
        next.revision = self
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        next.attempts += 1;
        next.delivery = DeliveryState::InFlight { started_at: now };
        Ok(next)
    }
    pub fn finish(&self, result: DeliveryResult, now: i64) -> Result<Self> {
        self.validate()?;
        let DeliveryState::InFlight { started_at } = self.delivery else {
            return Err(Error(ErrorCode::Conflict));
        };
        if !valid_time(now) || now < started_at {
            return Err(Error(ErrorCode::Conflict));
        }
        let mut next = self.clone();
        next.revision = self
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        next.delivery = match result {
            DeliveryResult::Verified => DeliveryState::Delivered { at: now },
            DeliveryResult::Unknown => DeliveryState::Ambiguous { at: now },
            DeliveryResult::Rejected => DeliveryState::Failed {
                retry_at: if self.attempts < MAX_DELIVERY_ATTEMPTS {
                    let delay = [30, 120, 600][usize::from(self.attempts - 1)];
                    now.checked_add(delay).filter(|at| valid_time(*at))
                } else {
                    None
                },
            },
        };
        Ok(next)
    }
    /// Only resolved rows qualify for automatic retention. Pending/failed/ambiguous rows remain visible.
    pub fn retainable(&self, now: i64) -> Result<bool> {
        self.validate()?;
        if !valid_time(now) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let at = self.acknowledged_at.or(match self.delivery {
            DeliveryState::Delivered { at } => Some(at),
            _ => None,
        });
        Ok(at.is_some_and(|at| now >= at && now - at >= DELIVERED_RETENTION_SECONDS))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryResult {
    Verified,
    Rejected,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeNotification {
    pub revision: i64,
}
