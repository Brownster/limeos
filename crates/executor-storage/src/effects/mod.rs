//! Protected effect receipts. Persistence is not consumer or launch authority.
pub mod mounts;

use crate::{Failure, Result, path};
use limeos_domain::{
    MountEffectQuiescence, MountExecutionState, MountHelperIdentity, STORAGE_MOUNT_RECEIPT_BYTES,
    StorageMountEffectPlan, StorageMountReceipt,
};
use rusqlite::{Connection, OptionalExtension, params};
use rustix::{
    fd::OwnedFd,
    fs::{FlockOperation, Mode, OFlags},
};
use std::{io::Write, os::fd::AsRawFd, path::Path, time::Duration};

const STATE: &str = "/var/lib/limeos/executors/storage";
const DATABASE: &str = "mounts.sqlite";
// Share directory-preparation's process lock. In-memory mutexes cannot serialize
// separate privileged services or operator processes.
const LOCK: &str = "targets.lock";
type SchemaRows = Vec<(String, String, String, Option<String>)>;
const SCHEMA: &str = "BEGIN IMMEDIATE; CREATE TABLE receipts(action TEXT PRIMARY KEY,digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),body TEXT NOT NULL) STRICT; CREATE TABLE locks(resource TEXT PRIMARY KEY,action TEXT NOT NULL REFERENCES receipts(action)) STRICT; PRAGMA user_version=1; COMMIT;";

/// A root-only fixed-location journal. Only trusted runtime orchestration writes
/// it, after consuming genuine admission/supervision owners; wire receipts are
/// stored facts, never a public request to grant execution or termination proof.
pub struct MountJournal {
    conn: Connection,
    _lock: OwnedFd,
    directory: OwnedFd,
    fresh_launch: Option<FreshLaunch>,
}
struct FreshLaunch {
    action: String,
    digest: String,
    armed: bool,
}
/// One-use release authority, minted by a fresh durable helper-binding commit.
/// Serialized receipts, replay lookups and a matching digest cannot construct it.
/// ```compile_fail
/// use limeos_executor_storage::effects::MountLaunchPermit;
/// let imported: MountLaunchPermit = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use limeos_executor_storage::effects::MountLaunchPermit;
/// fn consume(_: MountLaunchPermit) {}
/// fn replay(permit: MountLaunchPermit) { consume(permit); consume(permit); }
/// ```
pub struct MountLaunchPermit {
    action: String,
    digest: String,
    helper: MountHelperIdentity,
    expires_at: i64,
}
impl MountLaunchPermit {
    pub fn action(&self) -> &str {
        &self.action
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn helper(&self) -> &MountHelperIdentity {
        &self.helper
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn check_binding(
        &self,
        action: &str,
        digest: &str,
        helper: &MountHelperIdentity,
        at: i64,
    ) -> Result<()> {
        if self.action != action
            || self.digest != digest
            || self.helper != *helper
            || at >= self.expires_at
        {
            return Err(Failure::Conflict);
        }
        Ok(())
    }
}
fn durable(_: rusqlite::Error) -> Failure {
    Failure::StateNotDurable
}
fn encoded(value: &impl serde::Serialize) -> Result<String> {
    struct Limited {
        bytes: Vec<u8>,
    }
    impl Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > STORAGE_MOUNT_RECEIPT_BYTES.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("mount receipt limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Limited { bytes: vec![] };
    serde_json::to_writer(&mut output, value).map_err(|_| Failure::InvalidPlan)?;
    String::from_utf8(output.bytes).map_err(|_| Failure::InvalidPlan)
}
fn private_file(fd: &OwnedFd, owner: u32) -> Result<()> {
    let stat = path::stat(fd)?;
    if stat.stx_mode & 0o170000 != 0o100000
        || stat.stx_uid != owner
        || stat.stx_nlink != 1
        || stat.stx_mode & 0o077 != 0
        || stat.stx_size > 16 * 1024 * 1024
    {
        return Err(Failure::StateNotDurable);
    }
    Ok(())
}
impl MountJournal {
    pub fn open() -> Result<Self> {
        if !rustix::process::geteuid().is_root() {
            return Err(Failure::Unavailable);
        }
        let directory = path::open(Path::new(STATE), true, true)?;
        let stat = path::stat(&directory)?;
        if stat.stx_mode & 0o077 != 0 {
            return Err(Failure::StateNotDurable);
        }
        Self::from_directory(directory, 0)
    }
    /// Status/replay lookups open only existing files and never initialize state.
    pub fn readonly() -> Result<Self> {
        if !rustix::process::geteuid().is_root() {
            return Err(Failure::Unavailable);
        }
        let directory = path::open(Path::new(STATE), true, true)?;
        if path::stat(&directory)?.stx_mode & 0o077 != 0 {
            return Err(Failure::StateNotDurable);
        }
        let lock = rustix::fs::openat(
            &directory,
            LOCK,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, 0)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockShared)
            .map_err(|_| Failure::Conflict)?;
        check_files(&directory, 0)?;
        let file = rustix::fs::openat(
            &directory,
            DATABASE,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&file, 0)?;
        let conn = Connection::open_with_flags(
            format!("/proc/self/fd/{}/{DATABASE}", directory.as_raw_fd()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(durable)?;
        conn.busy_timeout(Duration::from_millis(200))
            .map_err(durable)?;
        conn.pragma_update(None, "trusted_schema", "OFF")
            .map_err(durable)?;
        if conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(durable)?
            != 1
        {
            return Err(Failure::StateNotDurable);
        }
        let result = Self {
            conn,
            _lock: lock,
            directory,
            fresh_launch: None,
        };
        result.check()?;
        Ok(result)
    }
    fn from_directory(directory: OwnedFd, owner: u32) -> Result<Self> {
        let lock = rustix::fs::openat(
            &directory,
            LOCK,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, owner)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Failure::Conflict)?;
        check_files(&directory, owner)?;
        let file = rustix::fs::openat(
            &directory,
            DATABASE,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&file, owner)?;
        let conn = Connection::open(format!(
            "/proc/self/fd/{}/{DATABASE}",
            directory.as_raw_fd()
        ))
        .map_err(durable)?;
        conn.busy_timeout(Duration::from_millis(200))
            .map_err(durable)?;
        if conn
            .pragma_query_value(None, "page_size", |r| r.get::<_, u32>(0))
            .map_err(durable)?
            != 4096
        {
            return Err(Failure::StateNotDurable);
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA max_page_count=4096; PRAGMA wal_autocheckpoint=128; PRAGMA journal_size_limit=1048576;").map_err(durable)?;
        match conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(durable)?
        {
            0 => conn.execute_batch(SCHEMA).map_err(durable)?,
            1 => {}
            _ => return Err(Failure::StateNotDurable),
        }
        check_files(&directory, owner)?;
        rustix::fs::fsync(&file).map_err(|_| Failure::StateNotDurable)?;
        let directory = rustix::fs::openat(
            &directory,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        rustix::fs::fsync(&directory).map_err(|_| Failure::StateNotDurable)?;
        let result = Self {
            conn,
            _lock: lock,
            directory,
            fresh_launch: None,
        };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<()> {
        let reference = Connection::open_in_memory().map_err(durable)?;
        reference.execute_batch(SCHEMA).map_err(durable)?;
        for table in ["receipts", "locks"] {
            let expected: String = reference
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?",
                    [table],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            let actual: Option<String> = self
                .conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?",
                    [table],
                    |r| r.get(0),
                )
                .optional()
                .map_err(durable)?;
            if actual.as_ref() != Some(&expected) {
                return Err(Failure::StateNotDurable);
            }
        }
        let schema = |connection: &Connection| -> Result<SchemaRows> {
            let mut statement = connection
                .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")
                .map_err(durable)?;
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(durable)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(durable)
        };
        if schema(&self.conn)? != schema(&reference)? {
            return Err(Failure::StateNotDurable);
        }
        let count: u32 = self
            .conn
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
            .map_err(durable)?;
        if count > 1024
            || self
                .conn
                .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                .map_err(durable)?
                != "ok"
            || self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(durable)?
        {
            return Err(Failure::StateNotDurable);
        }
        if self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM receipts WHERE length(CAST(action AS BLOB))!=64)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(durable)?
        {
            return Err(Failure::StateNotDurable);
        }
        let mut statement = self
            .conn
            .prepare("SELECT action FROM receipts ORDER BY action")
            .map_err(durable)?;
        for row in statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(durable)?
        {
            let action = row.map_err(durable)?;
            let receipt = self
                .lookup(&action, None)?
                .ok_or(Failure::StateNotDurable)?;
            let mut statement = self
                .conn
                .prepare("SELECT resource FROM locks WHERE action=? ORDER BY resource")
                .map_err(durable)?;
            let actual = statement
                .query_map([&action], |r| r.get::<_, String>(0))
                .map_err(durable)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(durable)?;
            let expected = if terminal(receipt.state) {
                vec![]
            } else {
                receipt.plan.resources()
            };
            if actual != expected {
                return Err(Failure::StateNotDurable);
            }
        }
        Ok(())
    }
    pub fn lookup(
        &self,
        action: &str,
        digest: Option<&str>,
    ) -> Result<Option<StorageMountReceipt>> {
        if !limeos_domain::opaque_id(action) || digest.is_some_and(|d| !limeos_domain::opaque_id(d))
        {
            return Err(Failure::InvalidPlan);
        }
        let row: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT length(CAST(digest AS BLOB)),length(CAST(state AS BLOB)),length(CAST(body AS BLOB)),digest,state,body FROM receipts WHERE action=?",
                [action],
                |r| {
                    // Admit stored byte lengths before converting SQLite text
                    // into owned Rust strings, including corrupt journal rows.
                    let digest: i64 = r.get(0)?;
                    let state: i64 = r.get(1)?;
                    let body: i64 = r.get(2)?;
                    if digest != 64 || !(1..=24).contains(&state)
                        || !(1..=STORAGE_MOUNT_RECEIPT_BYTES as i64).contains(&body) {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    Ok((r.get(3)?, r.get(4)?, r.get(5)?))
                },
            )
            .optional()
            .map_err(durable)?;
        let Some((stored, state, body)) = row else {
            return Ok(None);
        };
        if digest.is_some_and(|d| d != stored) {
            return Err(Failure::Conflict);
        }
        if body.len() > STORAGE_MOUNT_RECEIPT_BYTES {
            return Err(Failure::StateNotDurable);
        }
        let receipt: StorageMountReceipt =
            serde_json::from_str(&body).map_err(|_| Failure::StateNotDurable)?;
        receipt.validate().map_err(|_| Failure::StateNotDurable)?;
        if receipt.plan.action != action
            || receipt.digest != stored
            || state != name(receipt.state)
            || limeos_identity::digest(&encoded(&receipt.plan)?) != stored
        {
            return Err(Failure::StateNotDurable);
        }
        Ok(Some(receipt))
    }
    /// Durable receipt and all root claims precede any possible helper launch.
    pub fn begin(&mut self, plan: &StorageMountEffectPlan, at: i64) -> Result<StorageMountReceipt> {
        // Replay or failure cannot preserve a prior publication's release cursor.
        self.fresh_launch = None;
        plan.validate(plan.created_at)
            .map_err(|_| Failure::InvalidPlan)?;
        let digest = limeos_identity::digest(&encoded(plan)?);
        if let Some(existing) = self.lookup(&plan.action, Some(&digest))? {
            return Ok(existing);
        }
        plan.validate(at).map_err(|_| Failure::InvalidPlan)?;
        let receipt = StorageMountReceipt {
            plan: plan.clone(),
            digest,
            state: MountExecutionState::Prepared,
            quiescence: MountEffectQuiescence::NotLaunched {},
            after: None,
        };
        let body = encoded(&receipt)?;
        let tx = self.conn.transaction().map_err(durable)?;
        let count: u32 = tx
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
            .map_err(durable)?;
        if count >= 1024 {
            return Err(Failure::StateNotDurable);
        }
        tx.execute(
            "INSERT INTO receipts VALUES(?,?,'prepared',?)",
            params![receipt.plan.action, receipt.digest, body],
        )
        .map_err(durable)?;
        for resource in plan.resources() {
            tx.execute("INSERT INTO locks VALUES(?,?)", params![resource,plan.action]).map_err(|e| {
                if matches!(e, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::ConstraintViolation) { Failure::Conflict } else { durable(e) }
            })?;
        }
        tx.commit().map_err(durable)?;
        rustix::fs::fsync(&self.directory).map_err(|_| Failure::StateNotDurable)?;
        self.fresh_launch = Some(FreshLaunch {
            action: receipt.plan.action.clone(),
            digest: receipt.digest.clone(),
            armed: false,
        });
        Ok(receipt)
    }
    /// Mark uncertainty durably before creation/release of any gated helper.
    pub fn arm(&mut self, receipt: &StorageMountReceipt) -> Result<StorageMountReceipt> {
        if receipt.state != MountExecutionState::Prepared
            || receipt.quiescence != (MountEffectQuiescence::NotLaunched {})
            || !self.fresh_matches(receipt, false)
        {
            return Err(Failure::Conflict);
        }
        let mut next = receipt.clone();
        next.quiescence = MountEffectQuiescence::Pending { helper: None };
        if let Err(error) = self.record(receipt, &next) {
            self.fresh_launch = None;
            return Err(error);
        }
        self.fresh_launch.as_mut().ok_or(Failure::Conflict)?.armed = true;
        Ok(next)
    }
    /// A gated supervisor supplies this actual identity before releasing mount.
    pub fn bind_helper(
        &mut self,
        receipt: &StorageMountReceipt,
        helper: MountHelperIdentity,
    ) -> Result<(StorageMountReceipt, MountLaunchPermit)> {
        if receipt.state != MountExecutionState::Prepared
            || receipt.quiescence != (MountEffectQuiescence::Pending { helper: None })
            || !self.fresh_matches(receipt, true)
        {
            return Err(Failure::Conflict);
        }
        let mut next = receipt.clone();
        next.quiescence = MountEffectQuiescence::Pending {
            helper: Some(helper.clone()),
        };
        if let Err(error) = self.record(receipt, &next) {
            self.fresh_launch = None;
            return Err(error);
        }
        self.fresh_launch.take();
        let permit = MountLaunchPermit {
            action: next.plan.action.clone(),
            digest: next.digest.clone(),
            helper,
            expires_at: next.plan.expires_at,
        };
        Ok((next, permit))
    }
    fn fresh_matches(&self, receipt: &StorageMountReceipt, armed: bool) -> bool {
        self.fresh_launch.as_ref().is_some_and(|fresh| {
            fresh.action == receipt.plan.action
                && fresh.digest == receipt.digest
                && fresh.armed == armed
        })
    }
    /// Refine only a known accepted effect with already recorded owned-helper
    /// termination. The composition root supplies a separate genuine fresh
    /// mounted observation; this never launches, retries or refreshes expiry.
    pub fn verify_accepted(
        &mut self,
        receipt: &StorageMountReceipt,
        independent: &limeos_domain::StorageMountedSnapshot,
    ) -> Result<StorageMountReceipt> {
        if receipt.state != MountExecutionState::EffectAccepted
            || !receipt.quiescence.succeeded()
            || receipt.after.is_none()
        {
            return Err(Failure::Conflict);
        }
        limeos_domain::verify_storage_mount(&receipt.plan, receipt, independent)
            .map_err(|_| Failure::Conflict)?;
        let mut verified = receipt.clone();
        verified.state = MountExecutionState::Verified;
        self.record(receipt, &verified)?;
        Ok(verified)
    }
    /// Persist trusted orchestration's verified facts. This is not an RPC and
    /// must never be called with client-supplied receipt/quiescence bodies.
    pub fn record(
        &mut self,
        before: &StorageMountReceipt,
        next: &StorageMountReceipt,
    ) -> Result<()> {
        let result = self.record_inner(before, next);
        if result.is_err() {
            self.fresh_launch = None;
        }
        result
    }
    fn record_inner(
        &mut self,
        before: &StorageMountReceipt,
        next: &StorageMountReceipt,
    ) -> Result<()> {
        before.validate().map_err(|_| Failure::InvalidPlan)?;
        next.validate().map_err(|_| Failure::InvalidPlan)?;
        if before.plan != next.plan
            || before.digest != next.digest
            || terminal(before.state)
            || self
                .lookup(&before.plan.action, Some(&before.digest))?
                .as_ref()
                != Some(before)
            || !transition(before, next)
        {
            return Err(Failure::Conflict);
        }
        let body = encoded(next)?;
        let tx = self.conn.transaction().map_err(durable)?;
        if tx
            .execute(
                "UPDATE receipts SET state=?,body=? WHERE action=? AND digest=? AND body=?",
                params![
                    name(next.state),
                    body,
                    next.plan.action,
                    next.digest,
                    encoded(before)?
                ],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Failure::Conflict);
        }
        if terminal(next.state) {
            tx.execute("DELETE FROM locks WHERE action=?", [&next.plan.action])
                .map_err(durable)?;
        }
        tx.commit().map_err(durable)?;
        rustix::fs::fsync(&self.directory).map_err(|_| Failure::StateNotDurable)
    }
}
fn check_files(directory: &OwnedFd, owner: u32) -> Result<()> {
    for name in [
        "mounts.sqlite",
        "mounts.sqlite-wal",
        "mounts.sqlite-shm",
        "mounts.sqlite-journal",
    ] {
        match rustix::fs::openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => private_file(&fd, owner)?,
            Err(e) if e == rustix::io::Errno::NOENT => {}
            Err(_) => return Err(Failure::StateNotDurable),
        }
    }
    Ok(())
}
fn terminal(state: MountExecutionState) -> bool {
    matches!(
        state,
        MountExecutionState::Verified | MountExecutionState::PreconditionChanged
    )
}
fn name(state: MountExecutionState) -> &'static str {
    match state {
        MountExecutionState::Prepared => "prepared",
        MountExecutionState::EffectAccepted => "effect_accepted",
        MountExecutionState::Verified => "verified",
        MountExecutionState::PreconditionChanged => "precondition_changed",
        MountExecutionState::OutcomeUnknown => "outcome_unknown",
    }
}
fn transition(before: &StorageMountReceipt, next: &StorageMountReceipt) -> bool {
    let quiescence = match (&before.quiescence, &next.quiescence) {
        (a, b) if a == b => true,
        (
            MountEffectQuiescence::NotLaunched {},
            MountEffectQuiescence::Pending { helper: None },
        ) => true,
        (
            MountEffectQuiescence::Pending { helper: None },
            MountEffectQuiescence::Pending { helper: Some(_) },
        ) => true,
        (
            MountEffectQuiescence::Pending { helper: Some(a) },
            MountEffectQuiescence::Reaped { helper: b, .. },
        ) => a == b,
        _ => false,
    };
    let state = matches!(
        (before.state, next.state),
        (
            MountExecutionState::Prepared,
            MountExecutionState::Prepared
                | MountExecutionState::OutcomeUnknown
                | MountExecutionState::EffectAccepted
                | MountExecutionState::PreconditionChanged,
        ) | (
            MountExecutionState::OutcomeUnknown,
            MountExecutionState::OutcomeUnknown
                | MountExecutionState::EffectAccepted
                | MountExecutionState::Verified
                | MountExecutionState::PreconditionChanged,
        ) | (
            MountExecutionState::EffectAccepted,
            MountExecutionState::EffectAccepted
                | MountExecutionState::Verified
                | MountExecutionState::OutcomeUnknown,
        )
    );
    quiescence && state
}

#[cfg(test)]
mod tests;
