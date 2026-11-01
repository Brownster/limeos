use super::*;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    AwaitingProducer,
    Scheduled,
    AuthoritySuspended,
    Paused,
    Blocked,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ObservationPhase {
    Waiting,
    Checking,
    Partial,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ObservationRuntimeView {
    pub phase: ObservationPhase,
    #[ts(type = "number | null")]
    pub last_attempt: Option<i64>,
}
impl ObservationRuntimeView {
    pub fn validate(&self) -> crate::Result<()> {
        if self.last_attempt.is_some_and(|at| !valid_time(at))
            || (self.phase == ObservationPhase::Waiting) != self.last_attempt.is_none()
        {
            return Err(crate::Error(crate::ErrorCode::StateNotDurable));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum OccurrenceOutcome {
    Job { id: String, state: crate::JobState },
    Blocked,
    Skipped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceView {
    pub key: OccurrenceKey,
    pub outcome: OccurrenceOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ScheduleView {
    pub schedule: Schedule,
    pub status: ScheduleStatus,
    pub last_occurrence: Option<OccurrenceView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<ObservationRuntimeView>,
}
impl ScheduleView {
    pub fn validate(&self) -> crate::Result<()> {
        self.schedule.validate()?;
        // Shape validation is not an authority decision. Store derives the two
        // unpaused observation states from actual current identity and grants.
        let valid_status = match self.status {
            ScheduleStatus::Paused => self.schedule.paused,
            ScheduleStatus::Blocked => {
                !self.schedule.paused && !self.schedule.definition.operation.observation_only()
            }
            ScheduleStatus::AwaitingProducer
            | ScheduleStatus::Scheduled
            | ScheduleStatus::AuthoritySuspended => {
                !self.schedule.paused && self.schedule.definition.operation.observation_only()
            }
        };
        if !valid_status {
            return Err(crate::Error(crate::ErrorCode::StateNotDurable));
        }
        if (self.status == ScheduleStatus::Scheduled) != self.runtime.is_some() {
            return Err(crate::Error(crate::ErrorCode::StateNotDurable));
        }
        if let Some(runtime) = &self.runtime {
            runtime.validate()?;
        }
        if let Some(occurrence) = &self.last_occurrence {
            if occurrence.key.schedule != self.schedule.id
                || occurrence.key.revision < 1
                || occurrence.key.revision > self.schedule.revision
                || !valid_time(occurrence.key.due)
            {
                return Err(crate::Error(crate::ErrorCode::StateNotDurable));
            }
            match &occurrence.outcome {
                OccurrenceOutcome::Job { id, .. }
                    if !crate::identifier(id)
                        || !self.schedule.definition.operation.observation_only() =>
                {
                    return Err(crate::Error(crate::ErrorCode::StateNotDurable));
                }
                OccurrenceOutcome::Blocked
                    if self.schedule.definition.operation.observation_only() =>
                {
                    return Err(crate::Error(crate::ErrorCode::StateNotDurable));
                }
                _ => {}
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct AutomationPage<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
    pub partial: bool,
}
