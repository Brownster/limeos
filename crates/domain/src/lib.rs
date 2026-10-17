use schemars::JsonSchema;
pub mod automation;
pub mod backup_jobs;
pub use backup_jobs::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
mod operations;
pub use operations::*;
mod logs;
pub use logs::*;
mod compose;
pub use compose::*;
pub mod compose_override;
mod storage;
pub use storage::*;
mod storage_operations;
mod storage_targets;
pub use storage_operations::*;
pub use storage_targets::*;
mod storage_dependencies;
pub use storage_dependencies::*;
mod pools;
pub use pools::*;
mod protection;
pub use protection::*;
pub mod backups;
pub mod configured_dependencies;
pub mod shares;
pub mod storage_dependency_graph;
pub mod storage_impact;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    Unauthenticated,
    Forbidden,
    Conflict,
    Expired,
    Overloaded,
    Unavailable,
    StorageMountLimit,
    CorruptConfiguration,
    StateNotDurable,
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub ErrorCode);
impl Error {
    pub fn message(self) -> &'static str {
        match self.0 {
            ErrorCode::InvalidInput => "The request is invalid.",
            ErrorCode::Unauthenticated => "Authentication is required.",
            ErrorCode::Forbidden => "The request is outside the permitted scope.",
            ErrorCode::Conflict => "The request conflicts with current state.",
            ErrorCode::Expired => "The authority has expired or been revoked.",
            ErrorCode::Overloaded => "The bounded work queue is full. Retry later.",
            ErrorCode::Unavailable => "The service is unavailable.",
            ErrorCode::StorageMountLimit => {
                "This mount flow supports at most 4 running containers and 16 destination mounts. This host exceeds those limits."
            }
            ErrorCode::CorruptConfiguration => {
                "Configuration is invalid; the original file was preserved. Run limeosctl check-config before restarting."
            }
            ErrorCode::StateNotDurable => {
                "Durable state is unavailable. Check the job outcome before retrying."
            }
            ErrorCode::NotFound => "The resource was not found.",
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    MediaRequester,
    Operator,
    Administrator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: String,
    pub role: Role,
    #[ts(type = "number")]
    pub grant_revision: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    HealthRead,
    MediaRequest,
    ContainerManage,
    DeploymentManage,
    DeploymentElevated,
    StorageManage,
    IdentityManage,
    AutomationManage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub operation: Operation,
    pub resource: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Planned,
    WaitingApproval,
    Queued,
    Running,
    Verifying,
    Succeeded,
    Failed,
    Canceled,
    PreconditionChanged,
    OutcomeUnknown,
    NeedsIntervention,
}
impl JobState {
    pub fn allows(self, next: Self) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Planned, WaitingApproval | Queued | Canceled)
                | (WaitingApproval, Queued | Canceled | PreconditionChanged)
                | (Queued, Running | Canceled | PreconditionChanged)
                | (
                    Running,
                    Verifying | Failed | Canceled | OutcomeUnknown | NeedsIntervention
                )
                | (
                    Verifying,
                    Succeeded | Failed | PreconditionChanged | OutcomeUnknown | NeedsIntervention
                )
                | (OutcomeUnknown, Verifying | NeedsIntervention)
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::WaitingApproval => "waiting_approval",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::PreconditionChanged => "precondition_changed",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::NeedsIntervention => "needs_intervention",
        }
    }
}

/// Authority events contain stable types and identifiers, never credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    BootstrapIssued,
    AdministratorEnrolled,
    LegacyIdentityImported,
    SessionCreated,
    SessionRevoked,
    GrantsRevised,
    TaskIssued,
    TaskRevoked,
    JobQueued,
    JobRecoveryRequired,
    JobTransition {
        state: JobState,
    },
    PlanCreated {
        plan: String,
    },
    PlanApproved {
        plan: String,
    },
    PlanCanceled {
        plan: String,
    },
    AutomationScheduleCreated {
        schedule: String,
    },
    AutomationScheduleChanged {
        schedule: String,
        #[ts(type = "number")]
        revision: i64,
    },
    AutomationOccurrenceSettled {
        schedule: String,
        #[ts(type = "number")]
        revision: i64,
        #[ts(type = "number")]
        due: i64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Event {
    #[ts(type = "number")]
    pub cursor: i64,
    pub principal: Option<String>,
    pub job: Option<String>,
    pub event: EventKind,
    #[ts(type = "number")]
    pub created: i64,
}

/// Closed typed intent. Effectful intent requires the plan approval path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    HealthProbe {
        resource: String,
    },
    /// Only atomic occurrence reservation may queue this read-only intent.
    /// It remains undispatchable until protected observation integration exists.
    AutomationObserveHealth {
        occurrence: automation::OccurrenceKey,
    },
    ContainerRestart {
        plan: RestartPlan,
    },
    ContainerStart {
        plan: ContainerPlan,
    },
    ContainerStop {
        plan: ContainerPlan,
    },
    StoragePrepareTargets {
        plan: Box<StorageTargetPlan>,
    },
    StorageMount {
        plan: Box<StorageMountPlan>,
    },
    BackupPreparePrivate {
        plan: Box<BackupPreparationJobPlan>,
    },
}
impl Intent {
    pub fn resource(&self) -> &str {
        match self {
            Self::HealthProbe { resource } => resource,
            Self::AutomationObserveHealth { .. } => "system",
            Self::ContainerRestart { plan }
            | Self::ContainerStart { plan }
            | Self::ContainerStop { plan } => &plan.expected.resource,
            Self::StoragePrepareTargets { .. } | Self::StorageMount { .. } => {
                "storage:configuration"
            }
            Self::BackupPreparePrivate { .. } => BACKUP_PREPARATION_RESOURCE,
        }
    }
    pub fn container(plan: ContainerPlan) -> Self {
        match plan.operation {
            ContainerAction::Restart => Self::ContainerRestart { plan },
            ContainerAction::Start => Self::ContainerStart { plan },
            ContainerAction::Stop => Self::ContainerStop { plan },
        }
    }
    pub fn container_plan(self) -> Result<ContainerPlan> {
        let (expected, plan) = match self {
            Self::ContainerRestart { plan } => (ContainerAction::Restart, plan),
            Self::ContainerStart { plan } => (ContainerAction::Start, plan),
            Self::ContainerStop { plan } => (ContainerAction::Stop, plan),
            _ => return Err(Error(ErrorCode::InvalidInput)),
        };
        if plan.operation != expected {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(plan)
    }
}

pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
}
