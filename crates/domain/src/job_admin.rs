//! The host operator's view of every principal's jobs, and cancellation of a
//! queued job on its owner's behalf (`limeosctl job list`, `job cancel`).
//! Read-only listing plus the same queued-to-canceled step the owner can take;
//! nothing here starts, stops or reaches an executor.
use crate::JobState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Most rows one listing returns. 64 rows of the longest values fit well
/// inside one 64 KiB control-socket frame.
pub const MAX_OPERATOR_JOBS: u32 = 64;

/// The job families core can queue, one per group of intent operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobFamily {
    /// `container_restart`, `container_start`, `container_stop`.
    Container,
    /// `stack_restart`, `stack_start`, `stack_stop`.
    Stack,
    /// `storage_prepare_targets`.
    StorageTargets,
    /// `storage_mount`.
    StorageMount,
    /// `automation_observe_health`.
    AutomationObservation,
    /// `health_probe`.
    HealthProbe,
    /// `backup_prepare_private`.
    BackupPreparation,
}
impl JobFamily {
    /// The family of a stored intent's `operation` tag; `None` for a tag no
    /// intent has.
    pub fn of_operation(operation: &str) -> Option<Self> {
        Some(match operation {
            "container_restart" | "container_start" | "container_stop" => Self::Container,
            "stack_restart" | "stack_start" | "stack_stop" => Self::Stack,
            "storage_prepare_targets" => Self::StorageTargets,
            "storage_mount" => Self::StorageMount,
            "automation_observe_health" => Self::AutomationObservation,
            "health_probe" => Self::HealthProbe,
            "backup_prepare_private" => Self::BackupPreparation,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Stack => "stack",
            Self::StorageTargets => "storage_targets",
            Self::StorageMount => "storage_mount",
            Self::AutomationObservation => "automation_observation",
            Self::HealthProbe => "health_probe",
            Self::BackupPreparation => "backup_preparation",
        }
    }
}

/// One job as the host operator sees it. Names the job, its owner and what
/// it would act on; never the plan body, evidence or receipts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct OperatorJob {
    pub job: String,
    /// Username of the principal that queued the job.
    pub owner: String,
    pub family: JobFamily,
    /// The intent's operation tag, for example `container_restart`.
    pub operation: String,
    pub state: JobState,
    /// The job's headline resource, for example `container:<id>`,
    /// `stack:<name>`, `storage:configuration`.
    pub resource: String,
    /// Resources the job claims when dispatched (its `job_resources` rows,
    /// the headline resource included). A queued job holds none of them yet.
    pub claims: u32,
    /// Unix seconds of the job's first recorded event (when it was queued);
    /// 0 when none survives.
    #[ts(type = "number")]
    pub created_at: i64,
    /// Unix seconds after which a queued job is never dispatched: its approved
    /// plan's expiry, or a health probe's deadline. Backup preparation stores
    /// its plan's expiry here but has no dispatcher that reads it.
    #[ts(type = "number")]
    pub deadline: i64,
    /// An administrator's resolution request is recorded and not completed.
    pub resolution_open: bool,
}

/// One bounded listing, oldest job first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct OperatorJobPage {
    pub jobs: Vec<OperatorJob>,
    /// Every job in the requested states; more than `jobs.len()` means the
    /// listing was cut at its limit.
    pub matching: u32,
}

/// Why core refused to cancel a job on its owner's behalf. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum CancelRefusal {
    /// The job is not queued: it was dispatched (an execute may have been
    /// sent, so cancelling cannot prove nothing happened), or it has already
    /// settled. A held job is settled by `limeosctl job resolve`.
    NotQueued,
    /// Automation observations are cancelled only by pausing their schedule:
    /// startup refuses a store holding a cancellation of the current schedule
    /// revision that no pause explains.
    PauseScheduleInstead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum CancelOutcome {
    /// The job moved from `queued` to `canceled` now.
    Canceled,
    /// The job was already `canceled`; nothing was written.
    AlreadyCanceled,
    Refused,
}

/// The answer to one cancellation on the owner's behalf.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct OperatorCancel {
    pub job: String,
    pub family: JobFamily,
    /// The job's state after the call.
    pub state: JobState,
    pub outcome: CancelOutcome,
    /// Present exactly when `outcome` is `refused`.
    pub refusal: Option<CancelRefusal>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_intent_operation_has_a_family() {
        // Every tag `Intent` serializes to is known, so a listing never fails
        // on a well-formed job.
        for operation in [
            "health_probe",
            "automation_observe_health",
            "container_restart",
            "container_start",
            "container_stop",
            "stack_restart",
            "stack_start",
            "stack_stop",
            "storage_prepare_targets",
            "storage_mount",
            "backup_prepare_private",
        ] {
            assert!(JobFamily::of_operation(operation).is_some(), "{operation}");
        }
        assert_eq!(JobFamily::of_operation("container"), None);
        let tag = serde_json::to_value(crate::Intent::HealthProbe {
            resource: "system".into(),
        })
        .unwrap();
        assert_eq!(
            JobFamily::of_operation(tag["operation"].as_str().unwrap()),
            Some(JobFamily::HealthProbe)
        );
        assert_eq!(
            serde_json::to_value(JobFamily::StorageTargets).unwrap(),
            JobFamily::StorageTargets.as_str()
        );
    }
}
