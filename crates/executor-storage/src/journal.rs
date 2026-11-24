//! The storage roles' receipt journal: one implementation behind target
//! preparation (`targets.sqlite`, `targets`) and mounts (`mounts.sqlite`,
//! `effects`). Each receipt type keeps its own file, schema text and state
//! names ([`Receipt`]); the discipline is shared:
//!
//! - One root-owned directory (`STATE`, mode without group or world bits) and
//!   one process lock (`targets.lock`) for both journals, taken
//!   non-blocking: exclusive for a writer, shared for a status reader. In-memory
//!   mutexes cannot serialize separate privileged services or operator
//!   processes. [`ProcessLock`] releases it (`LOCK_UN`) before closing.
//! - Every file is opened beneath the held directory without following a
//!   symlink and must be a private regular file with one link of at most
//!   16 MiB, the database and its `-wal`, `-shm` and `-journal`, checked
//!   before SQLite opens the database and again after it set WAL mode.
//! - SQLite in WAL mode with `synchronous=FULL`, a 4 KiB page, at most 4096
//!   pages and `trusted_schema=OFF`; foreign keys on.
//! - At every open: the whole schema equals the reference built from the
//!   receipt type's schema text (no extra index or trigger), `quick_check`,
//!   the foreign-key check, at most [`MAX_RECEIPTS`] receipts, every receipt
//!   reads back valid, and each receipt holds exactly its claims while it
//!   holds any (`locks`), none once settled.
//! - Durability: a writer's open syncs the database file, then the
//!   directory; every commit (the prepared receipt with all its claims, and
//!   each later state with its claims released when settled) is followed by
//!   a directory sync. A commit that fails leaves the previous state, so a
//!   prepared receipt stays the barrier to any replay.
//!
//! Bounds: at most [`MAX_RECEIPTS`] receipts and no retention, so a journal
//! that reaches the cap refuses every new effect (`StateNotDurable`) until an
//! operator intervenes. Both roles are dormant; retention must exist before
//! either is enabled for long.
use crate::{Failure, Result, path};
use rusqlite::{Connection, OptionalExtension, params};
use rustix::{
    fd::OwnedFd,
    fs::{FlockOperation, Mode, OFlags},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Write, marker::PhantomData, os::fd::AsRawFd, path::Path, time::Duration};

pub(crate) const STATE: &str = "/var/lib/limeos/executors/storage";
/// Shared by both journals: directory preparation and mounts never run at once.
pub(crate) const LOCK: &str = "targets.lock";
/// Receipts one journal holds at most (no retention).
pub const MAX_RECEIPTS: u32 = 1024;
type SchemaRows = Vec<(String, String, String, Option<String>)>;

/// The process lock's descriptor. Dropping it releases the flock (LOCK_UN)
/// before closing: a child that another thread is spawning shares the open
/// file description until it execs (CLOEXEC acts only then), so a plain
/// close would leave the lock held by that child and an immediate reopen in
/// this process would be refused with `Conflict`. The container executor's
/// receipt lock does the same.
pub(crate) struct ProcessLock(pub(crate) OwnedFd);
impl ProcessLock {
    /// Takes the flock and wraps the descriptor in one step, so every error
    /// between locking and building the journal unlocks too, not only the
    /// journal's own drop.
    pub(crate) fn acquire(lock: OwnedFd, operation: FlockOperation) -> Result<Self> {
        rustix::fs::flock(&lock, operation).map_err(|_| Failure::Conflict)?;
        Ok(Self(lock))
    }
}
impl Drop for ProcessLock {
    fn drop(&mut self) {
        // If it fails, the close still releases the lock once the last
        // descriptor is gone; never panic here.
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}

/// A receipt one journal stores: its file, schema and stored facts.
pub trait Receipt: Serialize + DeserializeOwned + PartialEq + Sized {
    /// The database's file name in `STATE`.
    const DATABASE: &'static str;
    /// The schema, created at version 1 in one transaction. Tables `receipts`
    /// (`action`, `digest`, `state`, `body`) and `locks` (`resource`,
    /// `action`). It is also the reference every open compares against.
    const SCHEMA: &'static str;
    /// The largest encoded receipt (and plan) the journal admits.
    const BODY_BYTES: usize;
    /// What an open refuses a group- or world-accessible `STATE` with.
    const UNSAFE_DIRECTORY: Failure;
    fn action(&self) -> &str;
    fn digest(&self) -> &str;
    /// The `state` column's value for this receipt.
    fn state_name(&self) -> &'static str;
    /// While true the receipt holds every one of `claims`; once false, none.
    fn holds_claims(&self) -> bool;
    /// The resources claimed, sorted as `ORDER BY resource` returns them.
    fn claims(&self) -> Vec<String>;
    /// The digest of the encoded plan, which `digest` must equal.
    fn plan_digest(&self) -> Result<String>;
    /// The receipt's own validation, applied to every receipt read back.
    fn admit(&self) -> bool;
    /// Whether `next` describes the same plan and digest as `self`.
    fn same_subject(&self, next: &Self) -> bool;
}

/// Serialize `value` into at most `limit` bytes, never allocating more.
pub(crate) fn encoded_within(value: &impl Serialize, limit: usize) -> Result<String> {
    struct Limited {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("receipt limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Limited {
        bytes: vec![],
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| Failure::InvalidPlan)?;
    String::from_utf8(output.bytes).map_err(|_| Failure::InvalidPlan)
}
fn durable(_: rusqlite::Error) -> Failure {
    Failure::StateNotDurable
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
/// The database and every auxiliary file SQLite may leave beside it: each
/// absent, or a private single-link regular file reached without a symlink.
fn check_files(directory: &OwnedFd, database: &str, owner: u32) -> Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let name = format!("{database}{suffix}");
        match rustix::fs::openat(
            directory,
            name.as_str(),
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
fn root() -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    Ok(())
}
fn private_directory(unsafe_directory: Failure) -> Result<OwnedFd> {
    let directory = path::open(Path::new(STATE), true, true)?;
    if path::stat(&directory)?.stx_mode & 0o077 != 0 {
        return Err(unsafe_directory);
    }
    Ok(directory)
}

/// A root-only fixed-location receipt journal for one receipt type `R`.
/// `S` is state the receipt type keeps for the life of one open (the
/// mounts' one-use launch cursor); it starts at its default on every open.
/// Only trusted runtime orchestration writes it; stored receipts are facts,
/// never authority to execute.
pub struct Journal<R, S = ()> {
    // Declared before `_lock`: fields drop in order, so the connection is
    // closed before the process lock is released.
    pub(crate) conn: Connection,
    pub(crate) _lock: ProcessLock,
    pub(crate) directory: OwnedFd,
    pub(crate) cursor: S,
    receipts: PhantomData<fn() -> R>,
}
impl<R: Receipt, S: Default> Journal<R, S> {
    /// Retained directory, process lock and SQLite database/WAL/shared memory.
    /// Constructor and transaction scratch must fit the caller's separate
    /// transient allowance. This bound grants no execution or replay authority.
    pub const RETAINED_DESCRIPTOR_BOUND: usize = 5;

    /// The writer's open of the fixed journal, creating it if absent.
    pub fn open() -> Result<Self> {
        root()?;
        Self::from_directory(private_directory(R::UNSAFE_DIRECTORY)?, 0)
    }
    /// Status/replay lookups open only existing files and never initialize state.
    pub fn readonly() -> Result<Self> {
        root()?;
        Self::readonly_from(private_directory(R::UNSAFE_DIRECTORY)?, 0)
    }
    pub(crate) fn readonly_from(directory: OwnedFd, owner: u32) -> Result<Self> {
        let lock = rustix::fs::openat(
            &directory,
            LOCK,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, owner)?;
        let lock = ProcessLock::acquire(lock, FlockOperation::NonBlockingLockShared)?;
        check_files(&directory, R::DATABASE, owner)?;
        let file = rustix::fs::openat(
            &directory,
            R::DATABASE,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&file, owner)?;
        let conn = Connection::open_with_flags(
            format!("/proc/self/fd/{}/{}", directory.as_raw_fd(), R::DATABASE),
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
        let journal = Self {
            conn,
            _lock: lock,
            directory,
            cursor: S::default(),
            receipts: PhantomData,
        };
        journal.check()?;
        Ok(journal)
    }
    pub(crate) fn from_directory(directory: OwnedFd, owner: u32) -> Result<Self> {
        let lock = rustix::fs::openat(
            &directory,
            LOCK,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, owner)?;
        let lock = ProcessLock::acquire(lock, FlockOperation::NonBlockingLockExclusive)?;
        check_files(&directory, R::DATABASE, owner)?;
        let file = rustix::fs::openat(
            &directory,
            R::DATABASE,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&file, owner)?;
        let conn = Connection::open(format!(
            "/proc/self/fd/{}/{}",
            directory.as_raw_fd(),
            R::DATABASE
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
            0 => conn.execute_batch(R::SCHEMA).map_err(durable)?,
            1 => {}
            _ => return Err(Failure::StateNotDurable),
        }
        // WAL mode created the auxiliary files; they must be private too.
        check_files(&directory, R::DATABASE, owner)?;
        rustix::fs::fsync(&file).map_err(|_| Failure::StateNotDurable)?;
        // O_PATH cannot be fsynced. Reopen the retained directory descriptor.
        let directory = rustix::fs::openat(
            &directory,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        rustix::fs::fsync(&directory).map_err(|_| Failure::StateNotDurable)?;
        let journal = Self {
            conn,
            _lock: lock,
            directory,
            cursor: S::default(),
            receipts: PhantomData,
        };
        journal.check()?;
        Ok(journal)
    }
    fn check(&self) -> Result<()> {
        let reference = Connection::open_in_memory().map_err(durable)?;
        reference.execute_batch(R::SCHEMA).map_err(durable)?;
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
        if count > MAX_RECEIPTS
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
                .stored(&action, None)?
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
            let expected = if receipt.holds_claims() {
                receipt.claims()
            } else {
                vec![]
            };
            if actual != expected {
                return Err(Failure::StateNotDurable);
            }
        }
        Ok(())
    }
    /// The receipt stored for `action`, read back and checked whole: its
    /// lengths are admitted before SQLite text becomes Rust strings, and the
    /// body must be a valid receipt for this action, digest, plan and state.
    /// A different `digest` is `Conflict`.
    pub(crate) fn stored(&self, action: &str, digest: Option<&str>) -> Result<Option<R>> {
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
                        || !(1..=R::BODY_BYTES as i64).contains(&body) {
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
        if body.len() > R::BODY_BYTES {
            return Err(Failure::StateNotDurable);
        }
        let receipt: R = serde_json::from_str(&body).map_err(|_| Failure::StateNotDurable)?;
        if !receipt.admit() {
            return Err(Failure::StateNotDurable);
        }
        if receipt.action() != action
            || receipt.digest() != stored
            || state != receipt.state_name()
            || receipt.plan_digest()? != stored
        {
            return Err(Failure::StateNotDurable);
        }
        Ok(Some(receipt))
    }
    /// Record a new prepared receipt with every claim it holds, in one
    /// transaction, then sync the directory. A claim another receipt holds
    /// is `Conflict`, and nothing is written.
    pub(crate) fn insert(&mut self, receipt: &R) -> Result<()> {
        if receipt.state_name() != "prepared" || !receipt.holds_claims() {
            return Err(Failure::InvalidPlan);
        }
        let body = encoded_within(receipt, R::BODY_BYTES)?;
        let tx = self.conn.transaction().map_err(durable)?;
        let count: u32 = tx
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
            .map_err(durable)?;
        if count >= MAX_RECEIPTS {
            return Err(Failure::StateNotDurable);
        }
        tx.execute(
            "INSERT INTO receipts VALUES(?,?,'prepared',?)",
            params![receipt.action(), receipt.digest(), body],
        )
        .map_err(durable)?;
        for resource in receipt.claims() {
            tx.execute("INSERT INTO locks VALUES(?,?)", params![resource, receipt.action()]).map_err(|e| {
                if matches!(e, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::ConstraintViolation) { Failure::Conflict } else { durable(e) }
            })?;
        }
        tx.commit().map_err(durable)?;
        rustix::fs::fsync(&self.directory).map_err(|_| Failure::StateNotDurable)
    }
    /// Replace `before`, which must be exactly the stored receipt and still
    /// hold its claims, with `next` for the same plan, if `permitted` allows
    /// the change. A settled `next` releases every claim in the same
    /// transaction. The directory is synced after the commit.
    pub(crate) fn replace(
        &mut self,
        before: &R,
        next: &R,
        permitted: impl FnOnce(&R, &R) -> bool,
    ) -> Result<()> {
        if !before.same_subject(next)
            || !before.holds_claims()
            || self
                .stored(before.action(), Some(before.digest()))?
                .as_ref()
                != Some(before)
            || !permitted(before, next)
        {
            return Err(Failure::Conflict);
        }
        let body = encoded_within(next, R::BODY_BYTES)?;
        let tx = self.conn.transaction().map_err(durable)?;
        if tx
            .execute(
                "UPDATE receipts SET state=?,body=? WHERE action=? AND digest=? AND body=?",
                params![
                    next.state_name(),
                    body,
                    next.action(),
                    next.digest(),
                    encoded_within(before, R::BODY_BYTES)?
                ],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Failure::Conflict);
        }
        if !next.holds_claims() {
            tx.execute("DELETE FROM locks WHERE action=?", [next.action()])
                .map_err(durable)?;
        }
        tx.commit().map_err(durable)?;
        rustix::fs::fsync(&self.directory).map_err(|_| Failure::StateNotDurable)
    }
}

#[cfg(test)]
mod tests;
