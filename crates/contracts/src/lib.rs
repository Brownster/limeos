use limeos_domain::{Error, ErrorCode, Principal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use ts_rs::TS;
mod observations;
pub use observations::*;
mod operations;
pub use operations::*;

pub const VERSION: u16 = 1;
pub const FRAME_LIMIT: usize = 64 * 1024;
pub const RPC_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub username: String,
    pub password: String,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub token: String,
    pub username: String,
    pub password: String,
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
    Authorized,
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
