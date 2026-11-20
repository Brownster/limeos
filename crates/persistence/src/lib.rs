pub mod config;
use fs2::FileExt;
use limeos_domain::{
    Error, ErrorCode, EventKind, Intent, JobState, Operation, Principal, Result, Role, Scope,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const SCHEMA_VERSION: u32 = 17;
pub const QUEUE_CAPACITY: usize = 64;
/// The revocation lane: small, and awaited rather than refused when full.
const PRIORITY_CAPACITY: usize = 8;
const PRIORITY_WAIT: Duration = Duration::from_secs(5);
/// Revocations the writer serves before one ordinary item when both lanes
/// have work, so neither lane can starve the other.
const PRIORITY_BURST: usize = 4;
const MIN_FREE_BYTES: u64 = 8 * 1024 * 1024;
const AUDIT_LIMIT: i64 = 64 * 1024 * 1024;
type Work = Box<dyn FnOnce(&mut Store) + Send>;

/// Wakes the writer thread without taking a slot in either queue. A sender
/// rings after every successful send. The writer clears the flag when it
/// wakes, then tries both lanes, and waits again only after finding both
/// empty, so a ring between its last check and its wait is never lost.
struct Bell {
    rung: std::sync::Mutex<bool>,
    ring: std::sync::Condvar,
}
impl Bell {
    fn ring(&self) {
        if let Ok(mut rung) = self.rung.lock() {
            *rung = true;
            self.ring.notify_one();
        }
    }
    /// Returns once rung, clearing it, or after `timeout`.
    fn wait(&self, timeout: Duration) {
        let Ok(mut rung) = self.rung.lock() else {
            return;
        };
        let deadline = Instant::now() + timeout;
        while !*rung {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.ring.wait_timeout(rung, left) {
                Ok((guard, _)) => rung = guard,
                Err(_) => return,
            }
        }
        *rung = false;
    }
}
/// Rings when the last `Database` handle drops, after its senders, so the
/// writer finds both lanes closed and exits.
struct LastHandle(Arc<Bell>);
impl Drop for LastHandle {
    fn drop(&mut self) {
        self.0.ring();
    }
}

#[derive(Clone)]
pub struct Database {
    sender: mpsc::Sender<Work>,
    priority: mpsc::Sender<Work>,
    /// A query-only connection for revocation admission, so a request that
    /// fails it touches neither lane.
    reader: Arc<std::sync::Mutex<Connection>>,
    bell: Arc<Bell>,
    audit: Arc<AtomicI64>,
    retention: Arc<std::sync::Mutex<Option<RetentionFailure>>>,
    pub generation: i64,
    #[cfg(test)]
    admitted: Arc<std::sync::atomic::AtomicUsize>,
    // Last, so it drops after the senders.
    _last: Arc<LastHandle>,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let mut store = Store::open(path)?;
        let generation = store.generation;
        let audit = Arc::new(AtomicI64::new(store.audit_usage()?.bytes));
        let reader = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(durable)?;
        reader
            .busy_timeout(Duration::from_millis(500))
            .map_err(durable)?;
        reader
            .execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(durable)?;
        let (sender, mut receiver) = mpsc::channel::<Work>(QUEUE_CAPACITY);
        let (priority, mut urgent) = mpsc::channel::<Work>(PRIORITY_CAPACITY);
        let bell = Arc::new(Bell {
            rung: std::sync::Mutex::new(false),
            ring: std::sync::Condvar::new(),
        });
        let observed = audit.clone();
        let retention = Arc::new(std::sync::Mutex::new(None::<RetentionFailure>));
        let failures = retention.clone();
        let doorbell = bell.clone();
        std::thread::Builder::new()
            .name("limeos-db".into())
            .spawn(move || {
                let maintain = |store: &mut Store| {
                    let Some(pass) = store.maintain(Instant::now()) else {
                        return;
                    };
                    if let Some(usage) = pass.usage {
                        observed.store(usage.bytes, Ordering::SeqCst);
                    }
                    if let Ok(mut latest) = failures.lock() {
                        for (rule, code) in pass.failed {
                            let count = latest.map_or(0, |f| f.count) + 1;
                            *latest = Some(RetentionFailure { rule, code, count });
                        }
                    }
                };
                maintain(&mut store);
                loop {
                    // Up to a burst of revocations, then one ordinary item:
                    // revocations wait behind at most one ordinary item, and
                    // ordinary work gets at least one turn in five.
                    let mut served = false;
                    for _ in 0..PRIORITY_BURST {
                        let Ok(work) = urgent.try_recv() else {
                            break;
                        };
                        work(&mut store);
                        served = true;
                    }
                    if let Ok(work) = receiver.try_recv() {
                        work(&mut store);
                        served = true;
                    }
                    maintain(&mut store);
                    if !served {
                        if receiver.is_closed()
                            && urgent.is_closed()
                            && receiver.is_empty()
                            && urgent.is_empty()
                        {
                            break;
                        }
                        // Idle: sleep until a send rings, and at least once a
                        // retention interval so an idle store is maintained.
                        doorbell.wait(retention::INTERVAL);
                    }
                }
                let _ = store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Self {
            sender,
            priority,
            reader: Arc::new(std::sync::Mutex::new(reader)),
            _last: Arc::new(LastHandle(bell.clone())),
            bell,
            audit,
            retention,
            generation,
            #[cfg(test)]
            admitted: Arc::default(),
        })
    }
    /// Audit usage as of the writer's last retention pass (at most a minute
    /// old). Never queues, so health reporting works when the queue is full.
    pub fn audit_usage(&self) -> AuditUsage {
        AuditUsage {
            bytes: self.audit.load(Ordering::SeqCst),
        }
    }
    /// The latest failed retention batch and how many have failed since the
    /// store opened, for the daemon to log. Never queues.
    pub fn retention_failure(&self) -> Option<RetentionFailure> {
        self.retention.lock().ok().and_then(|latest| *latest)
    }
    /// Session and task revocation. Admission first reads the session (with
    /// its CSRF digest) or the token on a query-only connection: an unknown or
    /// mismatched session is refused exactly as `authenticate` refuses it, and
    /// an unknown or already revoked token needs nothing, so neither touches
    /// the writer. An admitted revocation takes a separate lane that the writer
    /// serves ahead of ordinary work, and waits for space in it (up to a
    /// deadline) instead of failing when the ordinary queue is full.
    pub async fn revoke(&self, revocation: Revocation, now: i64) -> Result<()> {
        let reader = self.reader.clone();
        let (revocation, admitted) = tokio::task::spawn_blocking(move || {
            let conn = reader.lock().map_err(|_| Error(ErrorCode::Unavailable))?;
            let admitted = revocation.admit(&conn, now)?;
            Ok::<_, Error>((revocation, admitted))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))??;
        if !admitted {
            return Ok(());
        }
        #[cfg(test)]
        self.admitted.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        let work: Work = Box::new(move |s| {
            let _ = sender.send(s.apply_revocation(revocation, now));
        });
        tokio::time::timeout(PRIORITY_WAIT, self.priority.send(work))
            .await
            .map_err(|_| Error(ErrorCode::Overloaded))?
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        self.bell.ring();
        receiver.await.map_err(|_| Error(ErrorCode::Unavailable))?
    }
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .try_send(Box::new(move |s| {
                let _ = sender.send(f(s));
            }))
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        self.bell.ring();
        receiver.await.map_err(|_| Error(ErrorCode::Unavailable))?
    }
}

pub struct Store {
    // Declared before `_lock`: fields drop in order, so the connection is closed
    // (and its WAL checkpointed) before the lock is released for another core.
    conn: Connection,
    _lock: core_lock::CoreLock,
    directory: PathBuf,
    pub generation: i64,
    observation_leases: std::collections::HashMap<String, automation::ActiveObservation>,
    observation_watermarks: std::collections::HashMap<String, automation::PollWatermark>,
    retained_at: Option<Instant>,
    #[cfg(test)]
    free_bytes: Option<u64>,
    /// Tests: this rule's batches fail.
    #[cfg(test)]
    failing_rule: Option<&'static str>,
}
pub struct TaskGrant<'a> {
    pub service_uid: u32,
    pub task: &'a str,
    pub scopes: &'a [Scope],
    pub expires: i64,
}

pub struct LoginRecord {
    pub principal: Principal,
    pub password_hash: String,
}
/// Free space in the store's directory. Tests on this thread may pretend the
/// disk is fuller.
fn available_space(directory: &Path) -> Result<u64> {
    #[cfg(test)]
    if let Some(bytes) = OPEN_FREE_BYTES.get() {
        return Ok(bytes);
    }
    fs2::available_space(directory).map_err(|_| Error(ErrorCode::StateNotDurable))
}
#[cfg(test)]
thread_local! {
    static OPEN_FREE_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}
/// Free space the upgrade to schema 17 needs for `events` rows: its two
/// indexes take about 52 bytes per event (13 MB at 250,000 events), written
/// once to the WAL and once to the database file, plus margin, above the
/// 8 MiB reserve. At most about 42 MiB, at the audit cap.
const UPGRADE_BYTES_PER_EVENT: u64 = 128;
fn upgrade_bytes(rows: i64) -> u64 {
    u64::try_from(rows)
        .unwrap_or(u64::MAX)
        .saturating_mul(UPGRADE_BYTES_PER_EVENT)
        .saturating_add(MIN_FREE_BYTES)
}
/// A pending schema upgrade that needs more free space than the store's
/// directory has: what `Store::open` refused, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpgradeSpace {
    pub from: u32,
    pub needed: u64,
    pub available: u64,
}
/// After `Database::open` fails: whether a pending upgrade lacks free space,
/// so the daemon can say how much it needs. Reads the store without changing
/// it; `None` when no upgrade is pending, it fits, or the store is unreadable.
pub fn upgrade_space(path: &Path) -> Option<UpgradeSpace> {
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let from: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .ok()?;
    if !(1..SCHEMA_VERSION).contains(&from) {
        return None;
    }
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .ok()?;
    let available = available_space(path.parent()?).ok()?;
    let needed = upgrade_bytes(rows);
    (available < needed).then_some(UpgradeSpace {
        from,
        needed,
        available,
    })
}
fn interrupted_jobs(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM jobs WHERE state IN ('running','verifying')",
        [],
        |r| r.get(0),
    )
    .map_err(durable)
}
fn durable(_: rusqlite::Error) -> Error {
    Error(ErrorCode::StateNotDurable)
}
fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error(ErrorCode::InvalidInput))
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|_| Error(ErrorCode::StateNotDurable))
}
fn event(
    tx: &Transaction<'_>,
    principal: Option<&str>,
    job: Option<&str>,
    kind: EventKind,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(principal,job,kind,created) VALUES (?,?,?,?)",
        params![principal, job, json(&kind)?, now],
    )
    .map_err(durable)?;
    retention::account(tx, 1)
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let directory = path
            .parent()
            .ok_or(Error(ErrorCode::InvalidInput))?
            .to_path_buf();
        if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path.with_extension("lock"))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error(ErrorCode::Conflict))?;
        let lock = core_lock::CoreLock(lock);
        let mut conn = Connection::open(path).map_err(durable)?;
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if rusqlite::version_number() < 3_051_003 {
            return Err(Error(ErrorCode::Unavailable));
        }
        conn.busy_timeout(Duration::from_millis(500))
            .map_err(durable)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=4194304; PRAGMA trusted_schema=OFF;").map_err(durable)?;
        // Below the free-space reserve, the only space this store can give back
        // without a VACUUM (which needs more free space, not less) is a WAL left
        // by a crash: checkpoint and truncate it before the check decides.
        // Pruning cannot help here: freed pages stay in the file's freelist.
        if available_space(&directory)? < MIN_FREE_BYTES {
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .map_err(durable)?;
        }
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(durable)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        match version {
            0..=16 => {
                if version < 1 {
                    tx.execute_batch(include_str!("schema.sql"))
                        .map_err(durable)?;
                }
                if version < 2 {
                    tx.execute_batch(include_str!("migration-v2.sql"))
                        .map_err(durable)?;
                }
                if version < 3 {
                    tx.execute_batch(include_str!("migration-v3.sql"))
                        .map_err(durable)?;
                }
                if version < 4 {
                    tx.execute_batch(include_str!("migration-v4.sql"))
                        .map_err(durable)?;
                }
                if version < 5 {
                    tx.execute_batch(include_str!("migration-v5.sql"))
                        .map_err(durable)?;
                }
                if version < 6 {
                    tx.execute_batch(include_str!("migration-v6.sql"))
                        .map_err(durable)?;
                }
                if version < 7 {
                    tx.execute_batch(include_str!("migration-v7.sql"))
                        .map_err(durable)?;
                }
                if version < 8 {
                    tx.execute_batch(include_str!("migration-v8.sql"))
                        .map_err(durable)?;
                }
                if version < 9 {
                    tx.execute_batch(include_str!("migration-v9.sql"))
                        .map_err(durable)?;
                }
                if version < 10 {
                    tx.execute_batch(include_str!("migration-v10.sql"))
                        .map_err(durable)?;
                }
                if version < 11 {
                    tx.execute_batch(include_str!("migration-v11.sql"))
                        .map_err(durable)?;
                }
                if version < 12 {
                    tx.execute_batch(include_str!("migration-v12.sql"))
                        .map_err(durable)?;
                }
                if version < 13 {
                    tx.execute_batch(include_str!("migration-v13.sql"))
                        .map_err(durable)?;
                }
                if version < 14 {
                    tx.execute_batch(include_str!("migration-v14.sql"))
                        .map_err(durable)?;
                }
                if version < 15 {
                    tx.execute_batch(include_str!("migration-v15.sql"))
                        .map_err(durable)?;
                }
                if version < 16 {
                    tx.execute_batch(include_str!("migration-v16.sql"))
                        .map_err(durable)?;
                }
                // Version 17 builds two indexes over `events` inside this
                // transaction. Refuse before building them, writing nothing,
                // when they would not fit above the free-space reserve; core
                // then logs the space needed (`upgrade_space`).
                let rows: i64 = tx
                    .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
                    .map_err(durable)?;
                if available_space(&directory)? < upgrade_bytes(rows) {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                tx.execute_batch(include_str!("migration-v17.sql"))
                    .map_err(durable)?;
            }
            SCHEMA_VERSION => {}
            _ => return Err(Error(ErrorCode::Conflict)),
        }
        let check: String = tx
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(durable)?;
        if check != "ok" {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        resources::check(&tx)?;
        if available_space(&directory)? < MIN_FREE_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let audit_bytes = retention::reconcile(&tx)?;
        let interrupted = interrupted_jobs(&tx)?;
        // Recovery records use the revocation reserve past the cap. When even
        // that is full, retention runs first, in its ordinary bounded batches
        // (each its own transaction; the checks above have passed and the core
        // lock keeps every other writer out), and startup is refused only when
        // nothing more can be pruned.
        let tx = if retention::recovery_fits(interrupted, audit_bytes) {
            tx
        } else {
            tx.commit().map_err(durable)?;
            retention::make_room(&mut conn, interrupted)?;
            conn.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(durable)?
        };
        tx.execute("UPDATE meta SET generation=generation+1 WHERE id=1", [])
            .map_err(durable)?;
        let generation = tx
            .query_row("SELECT generation FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        // Set-based recovery stays bounded in memory. Every uncertain job
        // keeps its complete resource set; recovery never redispatches effects.
        let recovered_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        let recovered = tx.execute(
            "INSERT INTO events(principal,job,kind,created) SELECT principal,id,?,? FROM jobs WHERE state IN ('running','verifying')",
            params![json(&EventKind::JobRecoveryRequired)?, recovered_at],
        ).map_err(durable)?;
        retention::account(&tx, recovered as i64)?;
        tx.execute(
            "UPDATE jobs SET state='needs_intervention' WHERE state IN ('running','verifying')",
            [],
        )
        .map_err(durable)?;
        tx.commit().map_err(durable)?;
        Ok(Self {
            conn,
            _lock: lock,
            directory,
            generation,
            observation_leases: std::collections::HashMap::new(),
            observation_watermarks: std::collections::HashMap::new(),
            retained_at: None,
            #[cfg(test)]
            free_bytes: None,
            #[cfg(test)]
            failing_rule: None,
        })
    }
    fn free_bytes(&self) -> Result<u64> {
        #[cfg(test)]
        if let Some(bytes) = self.free_bytes {
            return Ok(bytes);
        }
        fs2::available_space(&self.directory).map_err(|_| Error(ErrorCode::StateNotDurable))
    }
    fn write<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        if self.free_bytes()? < MIN_FREE_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        let audit: i64 = tx
            .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        if audit >= AUDIT_LIMIT {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let value = f(&tx)?;
        tx.commit().map_err(durable)?;
        Ok(value)
    }
    /// Revocation removes or invalidates authority, so neither gate of `write`
    /// may refuse it. It skips the free-space check (its few pages come out of
    /// that 8 MiB reserve) and the audit cap. `f` learns whether its one event
    /// still fits within `AUDIT_RESERVE` past the cap.
    fn revocation_write<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>, bool) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        let audit: i64 = tx
            .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        let recordable = audit <= AUDIT_LIMIT + retention::AUDIT_RESERVE - retention::EVENT_BYTES;
        let value = f(&tx, recordable)?;
        tx.commit().map_err(durable)?;
        Ok(value)
    }
}

mod automation;
pub use automation::{
    ContainerReadAdmission, ContainerReadOutcome, ContainerReadPermit, HealthObservation,
    InAppPublication, ObservationAdmission, ObservationLease, ObservationOwnerPage,
    ObservationPollAdmission, ObservationPollLease, ObservationPollOutcome, ObservationResult,
    ObservationWorkHint, ScopedHealthSignal,
};
mod backup_preparation;
mod budgets;
mod compose;
mod core_lock;
mod identity_enablement;
mod identity_store;
mod job_resolutions;
mod jobs;
mod plans;
mod resources;
mod retention;
mod stack_plans;
mod storage;
mod storage_mounts;
mod storage_targets;
pub use compose::{
    AuthenticatedComposePreview, ComposeAuthorityKind, ComposeAuthorityObservation, compose_scopes,
};
pub use identity_store::Revocation;
pub use job_resolutions::{PendingResolution, ResolutionSubject};
pub use retention::{AuditLevel, AuditUsage, RetentionFailure};
pub use storage::storage_scope;

#[cfg(test)]
fn remove_v10_schema(conn: &Connection) {
    remove_v11_schema(conn);
    conn.execute_batch("DROP TRIGGER IF EXISTS storage_mount_terminal_result; DROP TRIGGER IF EXISTS storage_mount_result_identity; DROP TRIGGER IF EXISTS storage_mount_plan_delete; DROP TRIGGER IF EXISTS storage_mount_plan_consumed; DROP TRIGGER IF EXISTS storage_mount_plan_identity; DROP TABLE IF EXISTS storage_mount_results; DROP TABLE IF EXISTS storage_mount_plans;").unwrap();
}

#[cfg(test)]
fn remove_v17_schema(conn: &Connection) {
    conn.execute_batch("DROP INDEX IF EXISTS events_kind; DROP INDEX IF EXISTS events_job;")
        .unwrap();
}

#[cfg(test)]
fn remove_v16_schema(conn: &Connection) {
    remove_v17_schema(conn);
    // Back to the version-15 jobs CHECK (no `resolved`) and no resolution table.
    conn.execute_batch("DROP TRIGGER IF EXISTS jobs_resolved_update; DROP TRIGGER IF EXISTS jobs_resolved_insert; DROP TABLE IF EXISTS job_resolutions; PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,',''resolved'')',')') WHERE type='table' AND name='jobs'; PRAGMA writable_schema=RESET;").unwrap();
}

#[cfg(test)]
fn remove_v11_schema(conn: &Connection) {
    // Legacy migration fixtures must remove every later schema addition.
    remove_v16_schema(conn);
    conn.execute_batch("DROP TABLE IF EXISTS stack_results; DROP TABLE IF EXISTS stack_plans;")
        .unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS automation_poll_origins;")
        .unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS automation_health_policies;")
        .unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS automation_notifications; DROP TABLE IF EXISTS automation_alerts; DROP TABLE IF EXISTS automation_observation_results; DROP TABLE IF EXISTS automation_streams;").unwrap();
    conn.execute_batch("DROP TRIGGER IF EXISTS automation_job_update_state; DROP TRIGGER IF EXISTS automation_job_insert_state; DROP TRIGGER IF EXISTS automation_occurrence_delete; DROP TRIGGER IF EXISTS automation_occurrence_identity; DROP TRIGGER IF EXISTS automation_schedule_delete; DROP TRIGGER IF EXISTS automation_schedule_identity; DROP TABLE IF EXISTS automation_occurrences; DROP TABLE IF EXISTS automation_schedules;").unwrap();
}

#[cfg(test)]
fn remove_v9_schema(conn: &Connection) {
    remove_v10_schema(conn);
    conn.execute_batch("DROP TRIGGER backup_preparation_plan_immutable; DROP TRIGGER backup_preparation_insert_state; DROP TRIGGER backup_preparation_update_state; DROP TABLE backup_preparation_plans;").unwrap();
}

#[cfg(test)]
fn remove_v7_schema(conn: &Connection) {
    remove_v9_schema(conn);
    conn.execute_batch("DROP TABLE storage_target_results; DROP TABLE storage_target_plans;")
        .unwrap();
    // Existing migration fixtures must actually represent the older schema.
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND name IN ('jobs_primary_resource','jobs_identity_immutable','job_resources_insert','job_resources_update','job_resources_delete','jobs_acquire_resources','jobs_release_resources','resource_locks_insert','resource_locks_update','resource_locks_delete')")
        .unwrap();
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    drop(stmt);
    for name in names {
        conn.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    }
    conn.execute_batch("DROP TABLE resource_locks; DROP TABLE job_resources;")
        .unwrap();
}

#[cfg(test)]
mod scale;
#[cfg(test)]
mod tests;
