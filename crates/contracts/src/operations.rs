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
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct JobProgress {
    pub job: limeos_domain::RestartJob,
    pub events: Vec<limeos_domain::Event>,
}
