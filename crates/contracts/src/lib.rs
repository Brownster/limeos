use limeos_domain::{Error, ErrorCode, Principal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use ts_rs::TS;
mod observations;
pub use observations::*;
mod operations;
pub use operations::*;
mod resolution;
pub use resolution::*;
mod service;
pub use service::*;

pub const VERSION: u16 = 1;
pub const FRAME_LIMIT: usize = 64 * 1024;
pub const RPC_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub username: String,
    pub password: String,
}
impl Login {
    /// Could name an account and be checked. Anything else is refused before
    /// any attempt budget or password work, by core and the HTTP layer alike.
    pub fn well_formed(&self) -> bool {
        limeos_domain::identifier(&self.username)
            && self.username.len() <= 64
            && password_well_formed(&self.password)
    }
}
/// A password worth checking: non-empty and at most 1024 bytes. Every
/// password-gated path refuses anything else before charging an attempt.
pub fn password_well_formed(password: &str) -> bool {
    !password.is_empty() && password.len() <= 1024
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ObserveHealthEnable {
    pub password: String,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ObserveHealthEnabled {
    pub enabled: bool,
    pub reauthenticate: bool,
}
/// Fixed metadata only: owner, operation, anchor and readiness are server-owned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ObserveHealthScheduleInput {
    pub interval_seconds: u32,
    pub grace_seconds: u32,
    pub missed: limeos_domain::automation::MissedRunPolicy,
    pub window: Option<limeos_domain::automation::MaintenanceWindow>,
}
impl ObserveHealthScheduleInput {
    pub fn definition(&self) -> limeos_domain::automation::ScheduleDefinition {
        limeos_domain::automation::ScheduleDefinition {
            operation: limeos_domain::automation::ScheduledOperation::ObserveHealth,
            anchor: 0,
            interval_seconds: self.interval_seconds,
            grace_seconds: self.grace_seconds,
            missed: self.missed,
            window: self.window,
        }
    }
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub token: String,
    pub username: String,
    pub password: String,
}
/// One legacy user as read from the frozen app's env file. The hash is secret
/// material: no `Debug`, and it never appears in errors or logs.
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct LegacyUser {
    pub username: String,
    pub password_hash: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub principal: Principal,
    pub csrf_token: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub version: u16,
    #[ts(type = "number")]
    pub generation: i64,
    pub schema_version: u32,
    pub ready: bool,
    /// Subsystems not working as they should right now; an entry clears as
    /// soon as its subsystem recovers. Core keeps serving requests while any
    /// is listed; an empty list is healthy.
    pub degraded: Vec<DegradedSubsystem>,
}
/// A part of the service that can fail on its own while core keeps serving.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    /// The 1 Hz health-observation and automation runtime.
    ObservationRuntime,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DegradedSubsystem {
    pub subsystem: Subsystem,
    /// What is wrong and since when. Given to an authenticated caller and to
    /// `limeosctl status`; the unauthenticated health check names only the
    /// subsystem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<DegradedDetail>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DegradedDetail {
    /// The error that describes the current state: for the observation
    /// runtime, the one that ended its latest run.
    pub code: ErrorCode,
    /// When the current state began (Unix seconds).
    #[ts(type = "number")]
    pub since: i64,
    /// For the observation runtime, failures in the current run; it restarts
    /// after a longer wait each time.
    pub count: u32,
}
impl DegradedSubsystem {
    /// The entry as the unauthenticated health check shows it.
    pub fn redacted(self) -> Self {
        Self {
            subsystem: self.subsystem,
            detail: None,
        }
    }
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub code: ErrorCode,
    pub message: String,
    pub retry: bool,
    pub audit_id: String,
}

/// Internal password IPC. No authority, actor or filesystem parameter is accepted.
#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PasswordRequest {
    Hash { password: String },
    Verify { password: String, encoded: String },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordResponse {
    pub valid: bool,
    pub upgraded: Option<String>,
}

/// No principal name or role is accepted from a local caller. Privileged local
/// enrollment is gated by kernel credentials in the composition root.
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "request", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreRequest {
    Health {
        version: u16,
    },
    IssueBootstrap {
        version: u16,
    },
    Enroll {
        version: u16,
        enrollment: Enrollment,
    },
    /// Root-peer only, like `Enroll`. All-or-nothing and idempotent.
    ImportLegacyUsers {
        version: u16,
        users: Vec<LegacyUser>,
    },
    /// Root-peer only. Read-only evidence for a stuck job, as the named
    /// administrator would see it over HTTP. `admin` is a username; core
    /// refuses anyone who is not currently an administrator.
    ReadResolution {
        version: u16,
        admin: String,
        job: String,
    },
    /// Root-peer only. The root peer is the re-authentication, so no password
    /// is sent. Core stores `admin` as the resolving principal and marks the
    /// reason as entered through `limeosctl`.
    ResolveJob {
        version: u16,
        admin: String,
        job: String,
        declared: limeos_domain::DeclaredOutcome,
        reason: String,
        evidence_digest: String,
    },
    CheckTask {
        version: u16,
        token: String,
        task: String,
        operation: limeos_domain::Operation,
        resource: String,
    },
    ProposeRestart {
        version: u16,
        token: String,
        task: String,
        resource: String,
    },
    QueueRestart {
        version: u16,
        token: String,
        task: String,
        input: Box<QueueRestartInput>,
    },
    ProposeContainer {
        version: u16,
        token: String,
        task: String,
        resource: String,
        operation: limeos_domain::ContainerAction,
    },
    QueueContainer {
        version: u16,
        token: String,
        task: String,
        input: Box<QueueRestartInput>,
    },
    ReadContainerLogs {
        version: u16,
        token: String,
        task: String,
        resource: String,
        options: limeos_domain::LogOptions,
    },
    ProposeCompose {
        version: u16,
        token: String,
        task: String,
        selection: limeos_domain::ComposeSelection,
    },
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "response", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreResponse {
    Health(Health),
    Bootstrap {
        token: String,
        #[ts(type = "number")]
        expires_at: i64,
    },
    Enrolled,
    LegacyUsersImported {
        imported: u32,
        skipped_existing: Vec<String>,
    },
    Authorized,
    Resolution(ResolutionView),
    Resolved(ResolveJobResponse),
    RestartPlan(limeos_domain::PlannedRestart),
    RestartJob(limeos_domain::RestartJob),
    ContainerPlan(limeos_domain::PlannedContainerAction),
    ContainerJob(limeos_domain::ContainerJob),
    ContainerLogs(limeos_domain::ContainerLogs),
    ComposePlan(limeos_domain::PlannedCompose),
    Error(ErrorEnvelope),
}

pub async fn read_frame<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> limeos_domain::Result<T> {
    let len = reader
        .read_u32()
        .await
        .map_err(|_| Error(ErrorCode::InvalidInput))? as usize;
    if len == 0 || len > FRAME_LIMIT {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let mut bytes = vec![0; len];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|_| Error(ErrorCode::InvalidInput))?;
    serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::InvalidInput))
}
pub async fn write_frame<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> limeos_domain::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error(ErrorCode::InvalidInput))?;
    if bytes.is_empty() || bytes.len() > FRAME_LIMIT {
        return Err(Error(ErrorCode::InvalidInput));
    }
    writer
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn oversized_frame_rejected_before_body_and_forged_actor_rejected() {
        let (mut a, mut b) = tokio::io::duplex(8);
        a.write_u32((FRAME_LIMIT + 1) as u32).await.unwrap();
        assert!(read_frame::<CoreRequest>(&mut b).await.is_err());
        assert!(
            serde_json::from_str::<CoreRequest>(
                r#"{"request":"health","version":1,"username":"admin"}"#
            )
            .is_err()
        );
    }
}
