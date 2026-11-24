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
mod executor_socket;
pub use executor_socket::{ExecutorOwner, ExecutorSocket, SocketRefusal};

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
    /// The storage reader has a storage read stuck past its deadline (a
    /// failing disk, typically); it refuses storage inventory until the read
    /// returns and serves everything else. Or core refuses the reader's
    /// socket (code `forbidden`): it is not served by the reader's own
    /// account from its runtime directory (`ExecutorOwner`), so core takes
    /// no storage, stack or container-health answer from it.
    StorageReader,
    /// The store's single writer thread has been on one item for too long
    /// (a hung fsync, a lock held from outside), or did not answer a probe
    /// in time. Requests that need the store fail at their deadline
    /// meanwhile, and core reports itself not ready.
    Store,
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
    /// runtime, the one that ended its latest run; for the storage reader,
    /// `unavailable` for a stuck read and `forbidden` for a socket core
    /// refuses; for the store, `unavailable`.
    pub code: ErrorCode,
    /// When the current state began (Unix seconds): for the storage reader,
    /// when its oldest stuck read was abandoned, or when core first refused
    /// its socket; for the store, when its
    /// writer started the item it is still on.
    #[ts(type = "number")]
    pub since: i64,
    /// For the observation runtime, failures in the current run; it restarts
    /// after a longer wait each time. For the storage reader, reads stuck
    /// (0 for a refused socket).
    /// For the store, calls waiting for its writer.
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
    /// Root-peer only. Asks the storage reader for its health now, under the
    /// rule core trusts its socket by, and says whether core accepted the
    /// answer; with `read`, core then also makes that read through the
    /// reader as it would for a user and says whether it succeeded. For
    /// deployment checks (`scripts/cutover/activate.sh`).
    ReaderStatus {
        version: u16,
        #[serde(default)]
        read: Option<ReaderRead>,
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
    /// Root-peer only. Every principal's jobs that `filter` matches, oldest
    /// first, at most `limit` (1 to `limeos_domain::MAX_OPERATOR_JOBS`) from
    /// `filter.after` on. Read-only.
    ListJobs {
        version: u16,
        filter: limeos_domain::OperatorJobFilter,
        limit: u32,
    },
    /// Root-peer only. Cancel one queued job on its owner's behalf. As for
    /// `ResolveJob`, `admin` names a current administrator, recorded as the
    /// actor; the root peer is the re-authentication.
    CancelJob {
        version: u16,
        admin: String,
        job: String,
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
    Jobs(limeos_domain::OperatorJobPage),
    JobCanceled(limeos_domain::OperatorCancel),
    RestartPlan(limeos_domain::PlannedRestart),
    RestartJob(limeos_domain::RestartJob),
    ContainerPlan(limeos_domain::PlannedContainerAction),
    ContainerJob(limeos_domain::ContainerJob),
    ContainerLogs(limeos_domain::ContainerLogs),
    ComposePlan(limeos_domain::PlannedCompose),
    ReaderStatus(ReaderStatus),
    Error(ErrorEnvelope),
}
/// What happened when core asked the storage reader just now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReaderState {
    /// Core trusted the socket and took a well-formed answer.
    Accepted,
    /// A socket is there but core refuses it, or may not reach it (`reason`
    /// says why).
    Refused,
    /// Core could not try (out of descriptors or memory, a system call
    /// failed), or trusted the socket but got no well-formed answer in time.
    Unanswered,
    /// No socket, or nothing listening on it: the reader is not running.
    Unreachable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ReaderStatus {
    pub state: ReaderState,
    /// Why core refused the socket (`refused`) or got no answer (`unanswered`).
    pub reason: Option<String>,
    /// The read asked for, if any, and how it went.
    #[serde(default)]
    pub read: Option<ReaderReadOutcome>,
}
/// A read the storage reader serves, made by core through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReaderRead {
    /// Stack status: the reader's latest stack scan (catalog and Engine,
    /// through its worker), which must be fresh in both.
    Stack,
    /// A storage inventory (sysfs, block devices, blkid), as core reads it
    /// for storage planning.
    Storage,
    /// A container-health read (Engine, through the health worker).
    ContainerHealth,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReadResult {
    /// Core got a valid, fresh answer.
    Accepted,
    /// The reader's ceiling does not grant this read.
    NotGranted,
    /// It failed; `reason` says how.
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ReaderReadOutcome {
    pub read: ReaderRead,
    pub result: ReadResult,
    pub reason: Option<String>,
}

/// Reads one frame: a big-endian `u32` length, then that many bytes of JSON.
///
/// A connection that ends or fails before the whole frame arrived (the peer
/// closed it, crashed or restarted, or gave up at its deadline, whether
/// before the first byte or part-way through the length or the body) is a
/// transport failure: `Unavailable`, which callers may retry and which never
/// proves the request was refused. A frame that did arrive but is malformed
/// (a zero or oversized length, refused before its body is read, or a body
/// that is not the expected JSON) is `InvalidInput`.
pub async fn read_frame<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> limeos_domain::Result<T> {
    let len = reader
        .read_u32()
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))? as usize;
    if len == 0 || len > FRAME_LIMIT {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let mut bytes = vec![0; len];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?;
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
    /// What `read_frame` makes of `bytes` followed by the end of the stream.
    async fn read(bytes: &[u8]) -> limeos_domain::Result<Health> {
        let (mut a, mut b) = tokio::io::duplex(FRAME_LIMIT + 8);
        a.write_all(bytes).await.unwrap();
        drop(a);
        read_frame::<Health>(&mut b).await
    }
    fn frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }
    fn health() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": VERSION,
            "generation": 7,
            "schema_version": 17,
            "ready": true,
            "degraded": [],
        }))
        .unwrap()
    }
    #[tokio::test]
    async fn a_connection_that_ends_before_the_whole_frame_is_unavailable() {
        let body = health();
        let whole = frame(&body);
        // Nothing at all, part of the length, the length alone, part of the
        // body: the peer went away, which is no fault of the caller's input.
        for cut in [0, 1, 3, 4, 4 + body.len() / 2, whole.len() - 1] {
            assert_eq!(
                read(&whole[..cut]).await.err(),
                Some(Error(ErrorCode::Unavailable)),
                "a stream cut after {cut} of {} bytes",
                whole.len()
            );
        }
    }
    #[tokio::test]
    async fn a_frame_that_arrived_malformed_is_invalid_input() {
        let oversized = ((FRAME_LIMIT + 1) as u32).to_be_bytes();
        for (what, bytes) in [
            ("a zero length", 0u32.to_be_bytes().to_vec()),
            (
                "an oversized length, refused before its body",
                oversized.to_vec(),
            ),
            ("a body that is not JSON", frame(b"not json")),
            ("JSON of another shape", frame(br#"{"unexpected":true}"#)),
        ] {
            assert_eq!(
                read(&bytes).await.err(),
                Some(Error(ErrorCode::InvalidInput)),
                "{what}"
            );
        }
    }
    #[tokio::test]
    async fn a_whole_frame_is_read_and_a_written_frame_reads_back() {
        let health = read(&frame(&health())).await.expect("a whole frame");
        let (mut a, mut b) = tokio::io::duplex(FRAME_LIMIT + 8);
        write_frame(&mut a, &health).await.unwrap();
        let again: Health = read_frame(&mut b).await.unwrap();
        assert_eq!(
            serde_json::to_value(&again).unwrap(),
            serde_json::to_value(&health).unwrap()
        );
    }
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
