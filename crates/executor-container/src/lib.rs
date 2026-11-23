//! Bounded container execution and protected receipts.
//! The executor records uncertainty before a host effect and never blindly replays.
use fs2::FileExt;
use limeos_domain::{ContainerAction, ContainerSnapshot, Error, ErrorCode, Result, opaque_id};
pub use limeos_domain::{
    ContainerReceipt, ContainerRequest, ExecutionState, RestartReceipt, RestartRequest,
    StackReceipt, StackRequest,
};
mod answer;
mod closure;
pub mod compose_files;
pub mod compose_management;
#[cfg(feature = "deferred")]
pub mod compose_source;
pub mod deployment;
pub mod docker;
mod logs;
mod resolution;
mod retention;
mod stack;
pub use answer::Answer;
pub use closure::Lookup;
pub use resolution::{RESOLVED, ResolveOutcome};
pub use retention::verified_expiry;
/// Rows of `actions` the store holds at most: an execute that would pass it is
/// refused `Overloaded`, a last resort retention keeps far away.
const MAX_ACTION_ROWS: u32 = 8192;
/// The store's page cap (`PRAGMA max_page_count`): an insert past it fails.
const MAX_PAGES: u32 = 4096;
/// The largest store file `open` accepts.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// The connection's page cache, not SQLite's 2000 KiB default: the open-time
/// `quick_check` reads every page and the executor runs for months, so the
/// default would keep up to 2000 KiB of a grown store for good. Steady use writes
/// one receipt or one `PRUNE_BATCH`, which fits with room to spare (the test
/// `a_full_prune_batch_writes_no_more_with_the_small_cache`); migrations run
/// before it is set, with the default.
const CACHE_KIB: u32 = 256;
/// The store's limits, for the long-uptime soak to assert against rather
/// than copies of them.
#[doc(hidden)]
pub mod limits {
    pub const MAX_ACTION_ROWS: u32 = super::MAX_ACTION_ROWS;
    pub const MAX_STACK_ROWS: u32 = super::stack::MAX_STACK_ROWS;
    pub const MAX_PAGES: u32 = super::MAX_PAGES;
    pub const MAX_FILE_BYTES: u64 = super::MAX_FILE_BYTES;
    pub const RETAINED_ACTIONS: u32 = super::retention::RETAINED_ACTIONS;
    pub const RETAINED_STACKS: u32 = super::retention::RETAINED_STACKS;
    pub const PRUNE_BATCH: u32 = super::retention::PRUNE_BATCH;
}
/// The receipt store's schema (`PRAGMA user_version`) after every migration.
/// Opening a newer store refuses; tests and documentation name this constant.
pub const RECEIPT_SCHEMA_VERSION: u32 = 8;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
pub use stack::member_action;
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
    #[serde(default)]
    pub allow_stack: bool,
    pub managed_containers: Vec<String>,
    #[serde(default)]
    pub managed_stacks: Vec<String>,
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
            || self.core_uid == 0
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
impl ContainerCeiling {
    /// Defense in depth after the protocol ceiling: the project must be listed
    /// and every target must independently pass the container ceiling.
    pub fn authorize_stack(
        &self,
        peer_uid: u32,
        expected: &limeos_domain::StackSnapshot,
        operation: ContainerAction,
    ) -> Result<()> {
        expected.validate_for(operation)?;
        let project = expected.project()?;
        if !self.allow_stack
            || self.core_uid == 0
            || peer_uid != self.core_uid
            || self.managed_stacks.len() > 16
            || self
                .managed_stacks
                .iter()
                .any(|p| !limeos_domain::stack_project(p))
            || !self.managed_stacks.iter().any(|allowed| allowed == project)
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        for target in expected.targets(operation) {
            self.authorize(peer_uid, &target.resource, operation)?;
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
    /// Fresh listing of a Compose project's ordinary (non one-off) members,
    /// each identified by full ID, image, start time and running state.
    fn project_members(
        &self,
        _project: &str,
    ) -> impl Future<Output = Result<Vec<ContainerSnapshot>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
}

pub struct ReceiptStore {
    // Declared before `_lock`: fields drop in order, so the connection is closed
    // before the lock is released for another executor.
    conn: Connection,
    _lock: ReceiptLock,
    directory: PathBuf,
    /// This open's epoch (`epoch`).
    epoch: String,
}
/// Owns the locked `receipts.lock` descriptor; on drop the lock is released, then
/// the descriptor is closed.
struct ReceiptLock(File);
impl Drop for ReceiptLock {
    fn drop(&mut self) {
        // An flock belongs to the open file description, which a child made by
        // fork/posix_spawn shares until it execs. Closing only our descriptor would
        // leave such a child holding the lock, and a reopen in this process would be
        // refused with Conflict. LOCK_UN releases it for the whole description now.
        // If it fails, the close that follows still releases it once the last
        // descriptor is gone; never panic here (release builds abort).
        let _ = FileExt::unlock(&self.0);
    }
}
fn durable(_: rusqlite::Error) -> Error {
    Error(ErrorCode::StateNotDurable)
}
impl ReceiptStore {
    /// The packaging layer supplies a fixed private directory owned by this
    /// executor. Core's account must not be able to read or write it. A store
    /// created now starts with nothing pruned (`retention::initial_watermark`);
    /// opening reads no clock. Every open draws a new random epoch (`epoch`).
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
        let lock = ReceiptLock(lock);
        let path = directory.join("receipts.sqlite");
        if std::fs::symlink_metadata(&path).is_ok_and(|m| {
            !m.is_file() || m.uid() != uid || m.mode() & 0o077 != 0 || m.len() > MAX_FILE_BYTES
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
        conn.execute_batch(&format!("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; PRAGMA max_page_count={MAX_PAGES}; PRAGMA wal_autocheckpoint=128; PRAGMA journal_size_limit=1048576;")).map_err(durable)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(durable)?;
        match version {
            0 => {
                conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL); CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','outcome_unknown'); PRAGMA user_version=1; COMMIT;").map_err(durable)?;
            }
            1..=RECEIPT_SCHEMA_VERSION => {}
            _ => return Err(Error(ErrorCode::Conflict)),
        }
        if version < 2 {
            conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE actions RENAME TO actions_v1; DROP INDEX actions_resource_lock; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL); INSERT INTO actions SELECT * FROM actions_v1; DROP TABLE actions_v1; CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); PRAGMA user_version=2; COMMIT;").map_err(durable)?;
        }
        if version < 3 {
            conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE actions ADD COLUMN operation TEXT NOT NULL DEFAULT 'restart' CHECK(operation IN ('restart','start','stop')); PRAGMA user_version=3; COMMIT;").map_err(durable)?;
        }
        if version < 4 {
            conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE stack_actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','partly_done')),receipt TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); PRAGMA user_version=4; COMMIT;").map_err(durable)?;
        }
        if version < 5 {
            // `resolved` is added to both CHECK lists and kept out of both
            // unique-index WHERE lists, so a resolved action frees its resource.
            // `resolution` holds the operator-resolution confirmation.
            conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE actions RENAME TO actions_v4; DROP INDEX actions_resource_lock; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','resolved')),receipt TEXT NOT NULL,operation TEXT NOT NULL DEFAULT 'restart' CHECK(operation IN ('restart','start','stop')),resolution TEXT); INSERT INTO actions(action,digest,resource,state,receipt,operation) SELECT action,digest,resource,state,receipt,operation FROM actions_v4; DROP TABLE actions_v4; CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); ALTER TABLE stack_actions RENAME TO stack_actions_v4; DROP INDEX stack_actions_lock; CREATE TABLE stack_actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','partly_done','resolved')),receipt TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop')),resolution TEXT); INSERT INTO stack_actions(action,digest,resource,state,receipt,operation) SELECT action,digest,resource,state,receipt,operation FROM stack_actions_v4; DROP TABLE stack_actions_v4; CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); PRAGMA user_version=5; COMMIT;").map_err(durable)?;
        }
        if version < 6 {
            // Each receipt records its plan's expiry, so retention can keep it
            // for as long as that plan could still present the action again.
            let legacy = retention::legacy_expiry_sql();
            conn.execute_batch(&format!("BEGIN IMMEDIATE; ALTER TABLE actions ADD COLUMN expires_at INTEGER NOT NULL DEFAULT 0; ALTER TABLE stack_actions ADD COLUMN expires_at INTEGER NOT NULL DEFAULT 0; UPDATE actions SET expires_at={legacy}; UPDATE stack_actions SET expires_at={legacy}; CREATE INDEX actions_digest ON actions(digest); CREATE INDEX stack_actions_digest ON stack_actions(digest); PRAGMA user_version=6; COMMIT;")).map_err(durable)?;
        }
        if version < 7 {
            // The prune watermark and the legacy tombstones (see `retention`).
            // Every row present now is legacy: its `expires_at` may be a
            // back-filled estimate, not its plan's. A store already at 6 may
            // have pruned without a watermark, so its history is unknown.
            let watermark = retention::initial_watermark(version);
            conn.execute_batch(&format!("BEGIN IMMEDIATE; ALTER TABLE actions ADD COLUMN legacy INTEGER NOT NULL DEFAULT 0 CHECK(legacy IN (0,1)); ALTER TABLE stack_actions ADD COLUMN legacy INTEGER NOT NULL DEFAULT 0 CHECK(legacy IN (0,1)); UPDATE actions SET legacy=1; UPDATE stack_actions SET legacy=1; CREATE TABLE retention(id INTEGER PRIMARY KEY CHECK(id=1),watermark INTEGER NOT NULL); INSERT INTO retention(id,watermark) VALUES(1,{watermark}); CREATE TABLE pruned_legacy(action TEXT PRIMARY KEY) WITHOUT ROWID; PRAGMA user_version=7; COMMIT;")).map_err(durable)?;
        }
        if version < 8 {
            // A closure (see `closure`) is a terminal `precondition_changed`
            // row a receipt lookup wrote for an action it held no row for;
            // `closed` keeps what the store knew then. Every existing row was
            // written by an execute, so it is NULL.
            let check = closure::CLOSED_CHECK;
            conn.execute_batch(&format!("BEGIN IMMEDIATE; ALTER TABLE actions ADD COLUMN closed TEXT {check}; ALTER TABLE stack_actions ADD COLUMN closed TEXT {check}; PRAGMA user_version=8; COMMIT;")).map_err(durable)?;
        }
        // Every migration step ends at the named schema, or the store is refused.
        if conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(durable)?
            != RECEIPT_SCHEMA_VERSION
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        conn.execute_batch(&format!(
            "PRAGMA cache_size=-{CACHE_KIB}; PRAGMA shrink_memory;"
        ))
        .map_err(durable)?;
        if conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
            .map_err(durable)?
            != "ok"
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // Check the expected schema before any effect can be considered.
        let _ = conn
            .prepare("SELECT action,digest,resource,state,receipt,operation,resolution,expires_at,legacy,closed FROM actions LIMIT 0")
            .map_err(durable)?;
        let _ = conn
            .prepare(
                "SELECT action,digest,resource,state,receipt,operation,resolution,expires_at,legacy,closed FROM stack_actions LIMIT 0",
            )
            .map_err(durable)?;
        let _ = conn
            .prepare("SELECT action FROM pruned_legacy LIMIT 0")
            .map_err(durable)?;
        retention::watermark(&conn)?;
        File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let epoch = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Self {
            conn,
            _lock: lock,
            directory: directory.into(),
            epoch,
        })
    }

    /// A random identifier drawn when this store was opened, returned with
    /// every receipt answer. Any discontinuity in the store core talks to (a
    /// lost store recreated, a store restored from an older copy, any other
    /// file swapped in) can only be seen after a reopen, so a new epoch per
    /// open covers them all: core treats the jobs it held under an earlier
    /// epoch as needing an operator rather than accept a `never_recorded`
    /// closure for them. An epoch fixed at creation would not do, because a
    /// restored copy of the same store would carry it too. A restart of the
    /// executor with the same store changes it as well: core asks for the
    /// epoch (`ReceiptEpoch`) before each dispatch, so that costs only an
    /// operator decision for a job held across the restart whose execute
    /// never reached the store.
    pub fn epoch(&self) -> &str {
        &self.epoch
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
            || (state != receipt.state.as_str() && state != RESOLVED)
            || operation != receipt.operation.as_str()
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        Ok(Some(receipt))
    }

    /// `expires_at` is the plan's expiry; `now` is the clock that validated it.
    fn prepare(&mut self, receipt: &RestartReceipt, expires_at: i64, now: i64) -> Result<()> {
        self.insert_action(receipt, expires_at, now, false)
            .map(|_| ())
    }
    /// The first write of a container receipt, in one transaction with the
    /// retention it triggers. With `close`, the row is a closure: the absence
    /// is judged in the same transaction, before pruning can raise the
    /// watermark, and recorded in `closed`.
    pub(crate) fn insert_action(
        &mut self,
        receipt: &RestartReceipt,
        expires_at: i64,
        now: i64,
        close: bool,
    ) -> Result<Option<limeos_domain::ReceiptAbsence>> {
        if fs2::available_space(&self.directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
            < 8 * 1024 * 1024
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = self.conn.transaction().map_err(durable)?;
        let closed = close
            .then(|| {
                retention::absence_in(
                    &tx,
                    limeos_domain::ResolveKind::Container,
                    &receipt.action,
                    expires_at,
                )
            })
            .transpose()?;
        retention::prune(&tx, now)?;
        // Last resort only: retention keeps normal operation far below this.
        let count: u32 = tx
            .query_row("SELECT count(*) FROM actions", [], |r| r.get(0))
            .map_err(durable)?;
        if count >= MAX_ACTION_ROWS {
            return Err(Error(ErrorCode::Overloaded));
        }
        tx.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at,closed) VALUES(?,?,?,?,?,?,?,?)",
            params![
                receipt.action,
                receipt.plan_digest,
                receipt.before.resource,
                receipt.state.as_str(),
                encode(receipt)?,
                receipt.operation.as_str(),
                expires_at,
                closed.map(|c| c.as_str())
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
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        Ok(closed)
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
            // Taken by a closure: refused whatever the clock says (`closure`).
            closure::refuse_if_unproven(
                self.closure(limeos_domain::ResolveKind::Container, &request.action)?,
            )?;
            return Ok(receipt);
        }
        // No row: refused if a receipt for this plan may have been pruned,
        // whatever the clock says (`retention`, the execute barrier).
        self.admit(
            limeos_domain::ResolveKind::Container,
            &request.action,
            request.plan.expires_at,
        )?;
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
        self.prepare(&receipt, request.plan.expires_at, clock())?;
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
        // An operator-resolved action is closed; its receipt stays as recorded.
        if self
            .resolution(limeos_domain::ResolveKind::Container, action)?
            .is_some()
        {
            return Err(Error(ErrorCode::Conflict));
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
