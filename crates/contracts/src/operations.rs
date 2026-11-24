use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartInput {
    pub resource: String,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerInput {
    pub resource: String,
    pub operation: limeos_domain::ContainerAction,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ApprovalInput {
    pub digest: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct QueueRestartInput {
    pub proposal: limeos_domain::PlannedRestart,
    pub approval: String,
    pub idempotency_key: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct QueueStorageTargetsInput {
    pub proposal: limeos_domain::PlannedStorageTargets,
    pub approval: String,
    pub idempotency_key: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct QueueStorageMountInput {
    pub proposal: limeos_domain::PlannedStorageMount,
    pub approval: String,
    pub idempotency_key: String,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct JobProgress {
    pub job: limeos_domain::RestartJob,
    pub events: Vec<limeos_domain::Event>,
}
/// Whole-stack lifecycle: exact Compose project name and one of start, stop
/// or restart. No path, file, service or container selection is accepted.
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackInput {
    pub project: String,
    pub operation: limeos_domain::ContainerAction,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct QueueStackInput {
    pub proposal: limeos_domain::PlannedStackAction,
    pub approval: String,
    pub idempotency_key: String,
}
/// Owner-scoped progress. `receipt` carries the per-member outcomes recorded
/// by the executor once any exist; it is absent before the job runs.
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackJobProgress {
    pub job: limeos_domain::StackJob,
    pub events: Vec<limeos_domain::Event>,
    pub receipt: Option<limeos_domain::StackReceipt>,
}
