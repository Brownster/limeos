//! Operator resolution of `needs_intervention` container and stack jobs: the
//! evidence view, the decision and the outcome. See `docs/p06-job-resolution.md`.
use limeos_domain::{
    ContainerAction, ContainerSnapshot, DeclaredOutcome, JobResolution, JobState, ResolutionMember,
    ResolutionRequest, ResolutionTarget, ResolveKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One target, plan against fresh measurement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionTargetView {
    /// What the plan expected before the action.
    pub expected: ContainerSnapshot,
    /// What a read-only inspection found just now.
    pub measured: ResolutionTarget,
}

/// `GET /api/v1/jobs/{id}/resolution`. Evidence is fresh on every read and its
/// digest binds a later decision to exactly what was shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionView {
    pub job: String,
    pub state: JobState,
    pub kind: ResolveKind,
    pub operation: ContainerAction,
    /// Empty when the job is not awaiting resolution or is already resolved.
    pub targets: Vec<ResolutionTargetView>,
    /// Present exactly when `targets` is. Send it back unchanged in the POST.
    pub evidence_digest: Option<String>,
    /// Declarations the evidence supports now. `accepted_divergence` is always
    /// listed while the job is awaiting resolution.
    pub allowed: Vec<DeclaredOutcome>,
    /// The executor's recorded per-target outcomes; absent before any receipt.
    pub members: Option<Vec<ResolutionMember>>,
    /// A request that is recorded but not yet completed.
    pub pending: Option<ResolutionRequest>,
    /// The completed record, once resolved.
    pub resolution: Option<JobResolution>,
}

/// `POST /api/v1/jobs/{id}/resolution`.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolveJobInput {
    pub declared: DeclaredOutcome,
    /// 1 to 500 characters, no control characters; at least 20 for
    /// `accepted_divergence`.
    pub reason: String,
    /// The `evidence_digest` of the `ResolutionView` the administrator saw.
    pub evidence_digest: String,
    /// The administrator's own password: fresh re-authentication.
    pub password: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResolveStatus {
    /// HTTP 200. The executor and core both released the job.
    Resolved,
    /// HTTP 202. The request is recorded and the job stays held; the executor
    /// was unreachable or its answer was lost. Core retries on its own, and the
    /// same POST may be repeated.
    Pending,
    /// HTTP 409. The state changed since the evidence was shown. Nothing is
    /// held; `view` carries the fresh evidence.
    StaleEvidence,
    /// HTTP 409. The evidence does not support `declared`; `blocking` names the
    /// targets that differ and `view` carries the evidence.
    UnsupportedDeclaration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolveJobResponse {
    pub status: ResolveStatus,
    pub message: String,
    pub resolution: Option<JobResolution>,
    pub view: Option<ResolutionView>,
    /// Container IDs whose measured state blocks the declaration.
    pub blocking: Vec<String>,
}
