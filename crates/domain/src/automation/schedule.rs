use super::{MAX_REVISION, valid_time};
use crate::{Error, ErrorCode, Result, identifier};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledOperation {
    ObserveHealth,
    Backup,
    ProtectionSync,
    ProtectionScrub,
    ContainerUpdate,
    PackageReconcile,
}
impl ScheduledOperation {
    pub fn observation_only(self) -> bool {
        self == Self::ObserveHealth
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    CoalesceLatest,
    Skip,
}

/// UTC minutes. Start is inclusive, end exclusive; a crossing window spans midnight.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceWindow {
    pub start_minute: u16,
    pub end_minute: u16,
}
impl MaintenanceWindow {
    pub fn validate(self) -> Result<()> {
        if self.start_minute >= 1440
            || self.end_minute >= 1440
            || self.start_minute == self.end_minute
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
    pub fn contains(self, at: i64) -> bool {
        let second = at.rem_euclid(86400);
        let start = i64::from(self.start_minute) * 60;
        let end = i64::from(self.end_minute) * 60;
        if start < end {
            (start..end).contains(&second)
        } else {
            second >= start || second < end
        }
    }
    fn last_allowed(self, at: i64) -> i64 {
        if self.contains(at) {
            return at;
        }
        let base = at - at.rem_euclid(86400);
        let end = i64::from(self.end_minute) * 60;
        if self.start_minute < self.end_minute && at.rem_euclid(86400) < end {
            base - 86400 + end - 1
        } else {
            base + end - 1
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDefinition {
    pub operation: ScheduledOperation,
    #[ts(type = "number")]
    pub anchor: i64,
    pub interval_seconds: u32,
    pub grace_seconds: u32,
    pub missed: MissedRunPolicy,
    pub window: Option<MaintenanceWindow>,
}
impl ScheduleDefinition {
    pub fn validate(&self) -> Result<()> {
        if !valid_time(self.anchor)
            || !(30..=2_678_400).contains(&self.interval_seconds)
            || self.grace_seconds > 86400
            || self.grace_seconds >= self.interval_seconds
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if let Some(window) = self.window {
            window.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub id: String,
    #[ts(type = "number")]
    pub revision: i64,
    pub paused: bool,
    pub definition: ScheduleDefinition,
    /// Last atomically settled occurrence, including skipped/blocked ones.
    #[ts(type = "number | null")]
    pub settled_through: Option<i64>,
}
impl Schedule {
    pub fn validate(&self) -> Result<()> {
        self.definition.validate()?;
        if !identifier(&self.id) || !(1..=MAX_REVISION).contains(&self.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if let Some(last) = self.settled_through {
            if !valid_time(last)
                || last < self.definition.anchor
                || (last - self.definition.anchor) % i64::from(self.definition.interval_seconds)
                    != 0
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        Ok(())
    }

    pub fn due(&self, now: i64) -> Result<DueDecision> {
        self.validate()?;
        if !valid_time(now) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if self.paused {
            return Ok(DueDecision::Paused);
        }
        let d = &self.definition;
        if self.settled_through.is_some_and(|last| now < last) {
            return Ok(DueDecision::ClockRollback);
        }
        if now < d.anchor {
            return Ok(DueDecision::NotDue);
        }
        let interval = i64::from(d.interval_seconds);
        let latest = d.anchor + (now - d.anchor) / interval * interval;
        if self.settled_through.is_some_and(|last| latest <= last) {
            return Ok(DueDecision::NotDue);
        }
        let mut due = latest;
        // A one-day grace admits at most today's and yesterday's window.
        for _ in 0..3 {
            if d.window.is_none_or(|window| window.contains(due)) {
                break;
            }
            let allowed = d.window.expect("checked window").last_allowed(due);
            if allowed < d.anchor {
                return Ok(DueDecision::Skipped(self.key(latest)));
            }
            due = d.anchor + (allowed - d.anchor) / interval * interval;
        }
        let missed = self
            .settled_through
            .map_or(due != d.anchor, |last| due > last + interval);
        if now - due > i64::from(d.grace_seconds)
            || self.settled_through.is_some_and(|last| due <= last)
            || d.window
                .is_some_and(|window| !window.contains(due) || !window.contains(now))
            || (d.missed == MissedRunPolicy::Skip && missed)
        {
            return Ok(DueDecision::Skipped(self.key(latest)));
        }
        Ok(DueDecision::Ready(self.key(due)))
    }
    fn key(&self, due: i64) -> OccurrenceKey {
        OccurrenceKey {
            schedule: self.id.clone(),
            revision: self.revision,
            due,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceKey {
    pub schedule: String,
    #[ts(type = "number")]
    pub revision: i64,
    #[ts(type = "number")]
    pub due: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DueDecision {
    NotDue,
    Paused,
    ClockRollback,
    Skipped(OccurrenceKey),
    Ready(OccurrenceKey),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSchedule {
    pub definition: ScheduleDefinition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct PauseSchedule {
    #[ts(type = "number")]
    pub revision: i64,
    pub paused: bool,
}
