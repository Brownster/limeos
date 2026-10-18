use super::*;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    AwaitingProducer,
    AuthoritySuspended,
    Paused,
    Blocked,
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
            ScheduleStatus::AwaitingProducer | ScheduleStatus::AuthoritySuspended => {
                !self.schedule.paused && self.schedule.definition.operation.observation_only()
            }
        };
        if !valid_status {
            return Err(crate::Error(crate::ErrorCode::StateNotDurable));
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
