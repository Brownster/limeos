//! Bounded container execution and protected receipts.
//! The executor records uncertainty before a host effect and never blindly replays.
use fs2::FileExt;
use limeos_domain::{ContainerAction, ContainerSnapshot, Error, ErrorCode, Result, opaque_id};
pub use limeos_domain::{
    ContainerReceipt, ContainerRequest, ExecutionState, RestartReceipt, RestartRequest,
};
pub mod docker;
mod logs;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    future::Future,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

/// Core's authorization cannot expand this independently loaded ceiling.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerCeiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_restart: bool,
    #[serde(default)]
    pub allow_start: bool,
    #[serde(default)]
    pub allow_stop: bool,
    pub managed_containers: Vec<String>,
}
impl ContainerCeiling {
    pub fn authorize(
        &self,
        peer_uid: u32,
        resource: &str,
        operation: ContainerAction,
    ) -> Result<()> {
        let id = resource.strip_prefix("container:").unwrap_or_default();
        if self.version != 1
            || peer_uid != self.core_uid
            || !match operation {
                ContainerAction::Restart => self.allow_restart,
                ContainerAction::Start => self.allow_start,
                ContainerAction::Stop => self.allow_stop,
            }
            || !opaque_id(id)
            || self.managed_containers.len() > 64
            || self.managed_containers.iter().any(|id| !opaque_id(id))
            || !self.managed_containers.iter().any(|allowed| allowed == id)
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
}
pub type RestartCeiling = ContainerCeiling;

pub trait Engine {
    fn inspect(&self, id: &str) -> impl Future<Output = Result<ContainerSnapshot>> + Send;
    fn restart(&self, id: &str) -> impl Future<Output = Result<()>> + Send;
    fn start(&self, _id: &str) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn stop(&self, _id: &str) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
}

pub struct ReceiptStore {
    conn: Connection,
    _lock: File,
    directory: PathBuf,
}
fn durable(_: rusqlite::Error) -> Error {
    Error(ErrorCode::StateNotDurable)
}
impl ReceiptStore {
    /// The packaging layer supplies a fixed private directory owned by this
    /// executor. Core's account must not be able to read or write it.
    pub fn open(directory: &Path) -> Result<Self> {
        let uid = std::fs::metadata("/proc/self")
            .map_err(|_| Error(ErrorCode::Unavailable))?
            .uid();
        let meta =
            std::fs::symlink_metadata(directory).map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(Error(ErrorCode::Forbidden));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(directory.join("receipts.lock"))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let meta = lock
            .metadata()
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(Error(ErrorCode::Forbidden));
        }
        lock.try_lock_exclusive()
            .map_err(|_| Error(ErrorCode::Conflict))?;
        let path = directory.join("receipts.sqlite");
        if std::fs::symlink_metadata(&path).is_ok_and(|m| {
            !m.is_file() || m.uid() != uid || m.mode() & 0o077 != 0 || m.len() > 16 * 1024 * 1024
        }) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let conn = Connection::open(&path).map_err(durable)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let page_size: u32 = conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(durable)?;
        if page_size != 4096 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        conn.busy_timeout(Duration::from_millis(200))
            .map_err(durable)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; PRAGMA max_page_count=4096; PRAGMA wal_autocheckpoint=128; PRAGMA journal_size_limit=1048576;").map_err(durable)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(durable)?;
        match version {
            0 => {
                conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL); CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','outcome_unknown'); PRAGMA user_version=1; COMMIT;").map_err(durable)?;
            }
            1..=3 => {}
            _ => return Err(Error(ErrorCode::Conflict)),
        }
        if version < 2 {
            conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE actions RENAME TO actions_v1; DROP INDEX actions_resource_lock; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL); INSERT INTO actions SELECT * FROM actions_v1; DROP TABLE actions_v1; CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); PRAGMA user_version=2; COMMIT;").map_err(durable)?;
        }
        if version < 3 {
            conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE actions ADD COLUMN operation TEXT NOT NULL DEFAULT 'restart' CHECK(operation IN ('restart','start','stop')); PRAGMA user_version=3; COMMIT;").map_err(durable)?;
        }
        if conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
            .map_err(durable)?
            != "ok"
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // Check the expected schema before any effect can be considered.
        let _ = conn
            .prepare("SELECT action,digest,resource,state,receipt,operation FROM actions LIMIT 0")
            .map_err(durable)?;
        File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        Ok(Self {
            conn,
            _lock: lock,
            directory: directory.into(),
        })
    }

    pub fn receipt(&self, action: &str, digest: &str) -> Result<Option<RestartReceipt>> {
        if !opaque_id(action) || !opaque_id(digest) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let row: Option<(String, String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT digest,resource,state,receipt,operation FROM actions WHERE action=?",
                [action],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(durable)?;
        let Some((stored, resource, state, body, operation)) = row else {
            return Ok(None);
        };
        if stored != digest {
            return Err(Error(ErrorCode::Conflict));
        }
        let receipt: RestartReceipt =
            serde_json::from_str(&body).map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if receipt.action != action
            || receipt.plan_digest != digest
            || receipt.before.resource != resource
            || state != receipt.state.as_str()
            || operation != receipt.operation.as_str()
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(Some(receipt))
    }

    fn prepare(&mut self, receipt: &RestartReceipt) -> Result<()> {
        if fs2::available_space(&self.directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
            < 8 * 1024 * 1024
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = self.conn.transaction().map_err(durable)?;
        let count: u32 = tx
            .query_row("SELECT count(*) FROM actions", [], |r| r.get(0))
            .map_err(durable)?;
        if count >= 8192 {
            return Err(Error(ErrorCode::Overloaded));
        }
        tx.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation) VALUES(?,?,?,?,?,?)",
            params![
                receipt.action,
                receipt.plan_digest,
                receipt.before.resource,
                receipt.state.as_str(),
                encode(receipt)?,
                receipt.operation.as_str()
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(code, _)
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                Error(ErrorCode::Conflict)
            }
            _ => durable(e),
        })?;
        tx.commit().map_err(durable)?;
        File::open(&self.directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error(ErrorCode::StateNotDurable))
    }
    fn finish(&mut self, receipt: &RestartReceipt) -> Result<()> {
        let tx = self.conn.transaction().map_err(durable)?;
        if tx
            .execute(
                "UPDATE actions SET state=?,receipt=? WHERE action=? AND digest=? AND state=?",
                params![
                    receipt.state.as_str(),
                    encode(receipt)?,
                    receipt.action,
                    receipt.plan_digest,
                    ExecutionState::Prepared.as_str()
                ],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Error(ErrorCode::Conflict));
        }
        tx.commit().map_err(durable)
    }

    pub async fn execute(
        &mut self,
        engine: &impl Engine,
        ceiling: &RestartCeiling,
        peer_uid: u32,
        request: &RestartRequest,
        clock: impl Fn() -> i64,
    ) -> Result<RestartReceipt> {
        ceiling.authorize(
            peer_uid,
            &request.plan.expected.resource,
            request.plan.operation,
        )?;
        if !opaque_id(&request.action)
            || limeos_identity::digest(&encode(&request.plan)?) != request.plan_digest
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if let Some(receipt) = self.receipt(&request.action, &request.plan_digest)? {
            if receipt.operation != request.plan.operation {
                return Err(Error(ErrorCode::Conflict));
            }
            return Ok(receipt);
        }
        request.plan.validate(clock())?;
        let id = request
            .plan
            .expected
            .resource
            .strip_prefix("container:")
            .ok_or(Error(ErrorCode::InvalidInput))?;
        let actual = tokio::time::timeout(Duration::from_secs(2), engine.inspect(id))
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))??;
        request.plan.check_current(&actual, clock())?;
        let mut receipt = RestartReceipt {
            action: request.action.clone(),
            plan_digest: request.plan_digest.clone(),
            operation: request.plan.operation,
            before: actual,
            state: ExecutionState::Prepared,
            error: None,
        };
        self.prepare(&receipt)?;
        // Sync may take time. Recheck expiry immediately before initiating IPC.
        if let Err(e) = request.plan.validate(clock()) {
            receipt.state = ExecutionState::PreconditionChanged;
            receipt.error = Some(e.0);
            self.finish(&receipt)?;
            return Ok(receipt);
        }
        let result = tokio::time::timeout(
            Duration::from_secs(request.plan.operation.definition().timeout_seconds.into()),
            async {
                match request.plan.operation {
                    ContainerAction::Restart => engine.restart(id).await,
                    ContainerAction::Start => engine.start(id).await,
                    ContainerAction::Stop => engine.stop(id).await,
                }
            },
        )
        .await;
        match result {
            Ok(Ok(())) => receipt.state = ExecutionState::EffectAccepted,
            Ok(Err(e)) => {
                receipt.state = ExecutionState::OutcomeUnknown;
                receipt.error = Some(e.0);
            }
            Err(_) => {
                receipt.state = ExecutionState::OutcomeUnknown;
                receipt.error = Some(ErrorCode::Unavailable);
            }
        }
        if self.finish(&receipt).is_err() {
            // The effect may have occurred. The protected Prepared row remains
            // the barrier to replay; never claim that no effect was dispatched.
            receipt.state = ExecutionState::OutcomeUnknown;
            receipt.error = Some(ErrorCode::StateNotDurable);
        }
        Ok(receipt)
    }

    /// Release the executor's resource lock only after a separate fresh Engine
    /// inspection confirms the result supplied by core. Never replay an effect.
    pub async fn verify(
        &mut self,
        engine: &impl Engine,
        ceiling: &RestartCeiling,
        peer_uid: u32,
        action: &str,
        digest: &str,
        after: &ContainerSnapshot,
    ) -> Result<RestartReceipt> {
        let mut receipt = self
            .receipt(action, digest)?
            .ok_or(Error(ErrorCode::NotFound))?;
        ceiling.authorize(peer_uid, &receipt.before.resource, receipt.operation)?;
        if receipt.state == ExecutionState::Verified {
            return Ok(receipt);
        }
        if receipt.state != ExecutionState::EffectAccepted {
            return Err(Error(ErrorCode::Conflict));
        }
        limeos_domain::verify_container(receipt.operation, &receipt.before, after)?;
        let id = receipt
            .before
            .resource
            .strip_prefix("container:")
            .ok_or(Error(ErrorCode::InvalidInput))?;
        let current = tokio::time::timeout(Duration::from_secs(2), engine.inspect(id))
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))??;
        if current != *after {
            return Err(Error(ErrorCode::Conflict));
        }
        receipt.state = ExecutionState::Verified;
        let tx = self.conn.transaction().map_err(durable)?;
        if tx.execute("UPDATE actions SET state='verified',receipt=? WHERE action=? AND digest=? AND state='effect_accepted'", params![encode(&receipt)?, action, digest]).map_err(durable)? != 1 {
            return Err(Error(ErrorCode::Conflict));
        }
        tx.commit().map_err(durable)?;
        Ok(receipt)
    }
}
fn encode(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error(ErrorCode::InvalidInput))
}

#[cfg(test)]
mod tests;
