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
        atomic::{AtomicI64, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const SCHEMA_VERSION: u32 = 18;
/// The schema whose upgrade builds the event indexes (`migration-v17.sql`):
/// an upgrade from below it needs the free space `IndexBuild` measures.
const INDEXED_SCHEMA: u32 = 17;
pub const QUEUE_CAPACITY: usize = 64;
/// Queued jobs, and separately held jobs, one recovery candidate listing
/// returns at most (`Store::stack_candidates_after`).
pub const RECOVERY_BATCH: usize = 64;
/// The revocation lane: small, and awaited rather than refused when full.
const PRIORITY_CAPACITY: usize = 8;
const PRIORITY_WAIT: Duration = Duration::from_secs(5);
/// Revocations the writer serves before one ordinary item when both lanes
/// have work, so neither lane can starve the other.
const PRIORITY_BURST: usize = 4;
/// How long a caller waits for the writer to answer one call, queueing
/// included. The longest legitimate item is a sub-second commit (retention
/// passes delete in bounded batches, SQLite gives up on a held lock after
/// 500 ms, and the startup migration is not a call), so a full queue of 64
/// still answers well inside it on SD storage. Past it the call fails: see
/// [`Unanswered`] for what the caller then knows.
pub const CALL_DEADLINE: Duration = Duration::from_secs(30);
/// Why [`Database::try_call`] returned without the writer's answer. What
/// each means for the work is exact, so a caller can tell "it did not
/// happen" from "it may yet happen".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unanswered {
    /// The queue was full: the work was never queued and never runs.
    /// [`Database::call`] reports it as `Overloaded`.
    Overloaded,
    /// The work never ran and never will: the caller gave up before the
    /// writer started it (the writer skips abandoned work), or the writer
    /// has stopped. Safe to treat as a plain failure or to retry.
    /// [`Database::call`] reports it as `Unavailable`.
    NotExecuted,
    /// The caller gave up while the writer was running the work: it may
    /// still commit. Never a refusal and never proof that the write failed:
    /// read the durable state again and continue from what is there.
    /// [`Database::call`] reports it as `StateNotDurable` ("check the job
    /// outcome before retrying").
    InDoubt,
}
impl From<Unanswered> for Error {
    fn from(unanswered: Unanswered) -> Self {
        Error(match unanswered {
            Unanswered::Overloaded => ErrorCode::Overloaded,
            Unanswered::NotExecuted => ErrorCode::Unavailable,
            Unanswered::InDoubt => ErrorCode::StateNotDurable,
        })
    }
}
/// Abandons a call's work that has not started when its caller stops
/// waiting, however it stops.
struct Abandon(Arc<AtomicU8>);
impl Drop for Abandon {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(WAITING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst);
    }
}
/// A call's work, not yet started.
const WAITING: u8 = 0;
/// The writer has started the work.
const STARTED: u8 = 1;
/// The caller gave up first; the writer skips the work.
const ABANDONED: u8 = 2;
const MIN_FREE_BYTES: u64 = 8 * 1024 * 1024;
const AUDIT_LIMIT: i64 = 64 * 1024 * 1024;
type Work = Box<dyn FnOnce(&mut Store) + Send>;

/// What the writer thread is doing, readable without queueing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriterActivity {
    /// How long it has been on its current item (a call, a revocation or a
    /// retention pass); `None` while it waits for work.
    pub busy_for: Option<Duration>,
    /// Calls queued behind it.
    pub queued: usize,
}
/// When the writer started its current item, as milliseconds after
/// `origin` plus one; zero while idle.
struct Busy {
    origin: Instant,
    started: AtomicU64,
}
impl Busy {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            started: AtomicU64::new(0),
        }
    }
    fn run<T>(&self, item: impl FnOnce() -> T) -> T {
        let since = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX - 1);
        self.started.store(since + 1, Ordering::SeqCst);
        let value = item();
        self.started.store(0, Ordering::SeqCst);
        value
    }
    fn busy_for(&self) -> Option<Duration> {
        let started = self.started.load(Ordering::SeqCst).checked_sub(1)?;
        Some(
            self.origin
                .elapsed()
                .saturating_sub(Duration::from_millis(started)),
        )
    }
}

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
    /// fails it touches neither lane. The writer thread closes it (`None`)
    /// before its final checkpoint, so a blocking task that still holds it
    /// finds it gone rather than closing it after the store.
    reader: Arc<std::sync::Mutex<Option<Connection>>>,
    bell: Arc<Bell>,
    audit: Arc<AtomicI64>,
    retention: Arc<std::sync::Mutex<Option<RetentionFailure>>>,
    busy: Arc<Busy>,
    /// `CALL_DEADLINE`; shorter in tests.
    deadline: Duration,
    pub generation: i64,
    #[cfg(test)]
    admitted: Arc<std::sync::atomic::AtomicUsize>,
    // Last, so it drops after the senders.
    _last: Arc<LastHandle>,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, writer_clock::Clock::System)
    }
    /// `open`, with the writer thread on `clock` (the system clock, except
    /// for the soak's `test_support::open`).
    fn open_with(path: &Path, clock: writer_clock::Clock) -> Result<Self> {
        let mut store = Store::open(path)?;
        let generation = store.generation;
        let audit = Arc::new(AtomicI64::new(store.audit_usage()?.bytes));
        let reader = Arc::new(std::sync::Mutex::new(Some(open_reader(path)?)));
        let closing = reader.clone();
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
        let busy = Arc::new(Busy::new());
        let working = busy.clone();
        let mut time = writer_clock::WriterTime::new(clock, doorbell);
        std::thread::Builder::new()
            .name("limeos-db".into())
            .spawn(move || {
                time.begin();
                let maintain = |store: &mut Store, time: &mut writer_clock::WriterTime| {
                    let Some(pass) = working.run(|| store.maintain(time.now())) else {
                        return;
                    };
                    time.passed(&pass);
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
                maintain(&mut store, &mut time);
                loop {
                    // Up to a burst of revocations, then one ordinary item:
                    // revocations wait behind at most one ordinary item, and
                    // ordinary work gets at least one turn in five.
                    let mut served = false;
                    for _ in 0..PRIORITY_BURST {
                        let Ok(work) = urgent.try_recv() else {
                            break;
                        };
                        working.run(|| work(&mut store));
                        served = true;
                    }
                    if let Ok(work) = receiver.try_recv() {
                        working.run(|| work(&mut store));
                        served = true;
                    }
                    maintain(&mut store, &mut time);
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
                        time.idle(retention::INTERVAL);
                    }
                }
                // Shutdown, in order: the reader closes first (without a
                // checkpoint), then the writer checkpoints, then the store
                // closes its connection and releases the core lock.
                drop(
                    closing
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take(),
                );
                let _ = store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
                // The store (and with it the core lock) goes before the clock
                // learns the writer has stopped.
                drop(store);
                drop(time);
            })
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Self {
            sender,
            priority,
            reader,
            _last: Arc::new(LastHandle(bell.clone())),
            bell,
            audit,
            retention,
            busy,
            deadline: CALL_DEADLINE,
            generation,
            #[cfg(test)]
            admitted: Arc::default(),
        })
    }
    /// What the writer is doing now. Never queues, so core can report a
    /// writer stuck on one item (a hung fsync, a lock held from outside)
    /// while every call waits behind it.
    pub fn writer_activity(&self) -> WriterActivity {
        WriterActivity {
            busy_for: self.busy.busy_for(),
            queued: self.sender.max_capacity() - self.sender.capacity(),
        }
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
            let conn = conn.as_ref().ok_or(Error(ErrorCode::Unavailable))?;
            let admitted = revocation.admit(conn, now)?;
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
        tokio::time::timeout(self.deadline, receiver)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?
            .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    /// Whether `digest` names a live session, read on the query-only
    /// connection, so it puts nothing on either lane: for health to decide
    /// how much it shows while the writer may be stuck. Never waits for the
    /// connection (in use, the answer is no) nor past `bound`; false when
    /// unsure.
    pub async fn session_live(&self, digest: String, now: i64, bound: Duration) -> bool {
        let reader = self.reader.clone();
        let lookup = tokio::task::spawn_blocking(move || {
            let Ok(conn) = reader.try_lock() else {
                return false;
            };
            conn.as_ref()
                .is_some_and(|conn| identity_store::authenticate(conn, &digest, None, now).is_ok())
        });
        matches!(tokio::time::timeout(bound, lookup).await, Ok(Ok(true)))
    }
    /// Runs `f` on the writer thread, as [`Database::try_call`], reporting
    /// an unanswered call as an error: `Overloaded` (never queued),
    /// `Unavailable` (never ran and never will) or `StateNotDurable` (may
    /// still commit). See [`Unanswered`].
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.try_call(f).await?
    }
    /// Runs `f` on the writer thread and returns its answer, or why there is
    /// none. A caller gives up after `CALL_DEADLINE`, queueing included. If
    /// the writer has not started `f` by then, `f` never runs
    /// ([`Unanswered::NotExecuted`]); if it has, `f` may still commit
    /// ([`Unanswered::InDoubt`]). Dropping the returned future before it
    /// finishes abandons `f` the same way: it never runs unless the writer
    /// had already started it.
    pub async fn try_call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> std::result::Result<Result<T>, Unanswered> {
        let (sender, mut receiver) = oneshot::channel();
        let ticket = Arc::new(AtomicU8::new(WAITING));
        let start = ticket.clone();
        self.sender
            .try_send(Box::new(move |s| {
                // Checked immediately before the work runs: work its caller
                // has abandoned is dropped unrun.
                if start
                    .compare_exchange(WAITING, STARTED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    let _ = sender.send(f(s));
                }
            }))
            .map_err(|_| Unanswered::Overloaded)?;
        self.bell.ring();
        // A caller that stops waiting early (its own future dropped, as at
        // an HTTP deadline) abandons the work the same way.
        let _abandon = Abandon(ticket.clone());
        match tokio::time::timeout(self.deadline, &mut receiver).await {
            Ok(Ok(answer)) => Ok(answer),
            // Dropped without an answer: never run (the writer stopped), or
            // stopped while running it.
            Ok(Err(_)) => Err(if ticket.load(Ordering::SeqCst) == STARTED {
                Unanswered::InDoubt
            } else {
                Unanswered::NotExecuted
            }),
            Err(_) => {
                if ticket
                    .compare_exchange(WAITING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    return Err(Unanswered::NotExecuted);
                }
                // Started: it may have answered just now.
                receiver.try_recv().map_err(|_| Unanswered::InDoubt)
            }
        }
    }
    /// The same handle with a shorter call deadline than `CALL_DEADLINE`,
    /// for tests that stall the writer.
    pub fn with_call_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

/// The query-only connection for revocation admission, opened with the
/// store's path checks: the database must already be a regular file, and
/// SQLite refuses a symlink put there since (`SQLITE_OPEN_NOFOLLOW`). It
/// cannot write (`query_only`) and never checkpoints, not even on close when
/// it is the last connection (`NO_CKPT_ON_CLOSE`): the writer's own close
/// checkpoints, under the core lock.
fn open_reader(path: &Path) -> Result<Connection> {
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let reader = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(durable)?;
    reader
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .map_err(durable)?;
    reader
        .busy_timeout(Duration::from_millis(500))
        .map_err(durable)?;
    reader
        .execute_batch(
            "PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA wal_autocheckpoint=0;",
        )
        .map_err(durable)?;
    Ok(reader)
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
    /// Tests: the schema-17 index build runs with this page limit, which
    /// fails it as a full disk would.
    static UPGRADE_MAX_PAGES: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}
/// The upgrade to schema 17 (`migration-v17.sql`): what its three indexes
/// over `events` need, measured from the rows they will hold.
///
/// An index entry is its key plus at most [`ENTRY_OVERHEAD`] bytes. The keys
/// are measured: `job` (64 bytes for a job's event, none for a job-less one),
/// the event kind's name, and the notification id where there is one. A
/// sorted build fills its pages nearly full; the safety factor of 4/3 covers
/// interior pages, page and WAL frame headers and partly filled pages. The
/// new pages are written once to the WAL and once to the database file (the
/// WAL keeps its size until the first write after the checkpoint), and the
/// 8 MiB write reserve must remain afterwards:
///
/// `needed = reserve + 2 × 4/3 × entries`, plus twice the largest sort when
/// it spills to temporary files.
///
/// The sort that orders each index's entries runs in memory (`temp_store`),
/// so it needs no space outside the state directory and cannot fail there
/// after this check passed. It holds one index at a time: the largest
/// index's entries plus [`SORT_OVERHEAD`] per row, about 31 MiB at most for
/// a table at the audit cap whose every event has a job. Above
/// [`SORT_MEMORY`] (only a table far past the cap) it spills to temporary
/// files instead, counted as twice its size: in core's private `/tmp`, which
/// on Raspberry Pi OS shares the root filesystem with the state directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IndexBuild {
    /// Bytes of the three indexes' entries.
    entries: u64,
    /// Bytes of the largest index's sort.
    sort: u64,
}
/// A record header (up to 5 bytes), `cursor` and the rowid (up to 8 each),
/// the cell's length (2) and its pointer (2).
const ENTRY_OVERHEAD: u64 = 25;
/// Per sorted row: SQLite's sorter record (16 bytes) and the allocator's
/// overhead for it.
const SORT_OVERHEAD: u64 = 32;
/// The largest sort kept in memory: a quarter of core's 128 MiB memory limit.
const SORT_MEMORY: u64 = 32 * 1024 * 1024;
impl IndexBuild {
    fn measure(conn: &Connection) -> Result<Self> {
        let (rows, job, kind, notifications, notification): (i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT count(*),coalesce(sum(octet_length(job)),0),coalesce(sum(octet_length(json_extract(kind,'$.kind'))),0),count(json_extract(kind,'$.notification')),coalesce(sum(octet_length(json_extract(kind,'$.notification'))),0) FROM events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(durable)?;
        let bytes = |value: i64| u64::try_from(value).unwrap_or(0);
        Ok(Self::of([
            (bytes(rows), bytes(job)),
            (bytes(rows), bytes(kind)),
            (bytes(notifications), bytes(notification)),
        ]))
    }
    /// From the rows and key bytes of `events_job`, `events_kind` and
    /// `events_notification`.
    fn of(indexes: [(u64, u64); 3]) -> Self {
        let indexes = indexes.map(|(rows, keys)| {
            (
                rows,
                rows.saturating_mul(ENTRY_OVERHEAD).saturating_add(keys),
            )
        });
        Self {
            entries: indexes
                .iter()
                .fold(0, |sum, (_, entries)| sum.saturating_add(*entries)),
            sort: indexes
                .iter()
                .map(|(rows, entries)| entries.saturating_add(rows.saturating_mul(SORT_OVERHEAD)))
                .max()
                .unwrap_or(0),
        }
    }
    fn in_memory(self) -> bool {
        self.sort <= SORT_MEMORY
    }
    /// Free space the upgrade needs, the write reserve included.
    fn needed(self) -> u64 {
        let pages = self.entries.saturating_mul(4) / 3;
        let spill = if self.in_memory() {
            0
        } else {
            self.sort.saturating_mul(2)
        };
        MIN_FREE_BYTES
            .saturating_add(UPGRADE_FIXED_BYTES)
            .saturating_add(pages.saturating_mul(2))
            .saturating_add(spill)
    }
}
/// What the upgrade writes however few events there are: the root page of each
/// new index, the rewritten schema page, and their WAL frames and header. A
/// few 4 KiB pages in truth; 256 KiB leaves room for a larger page size and for
/// the WAL's own bookkeeping, so that even an empty store keeps the full write
/// reserve after the upgrade commits.
const UPGRADE_FIXED_BYTES: u64 = 256 * 1024;
/// A pending schema upgrade that needs more free space than the store's
/// directory has: what `Store::open` refused, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpgradeSpace {
    pub from: u32,
    pub needed: u64,
    pub available: u64,
}
/// An index build that ran out of space although the check passed, noted by
/// `Store::open` for [`upgrade_space`]: the database path and the shortfall.
static UPGRADE_SHORT: std::sync::Mutex<Option<(PathBuf, UpgradeSpace)>> =
    std::sync::Mutex::new(None);
/// The error for a failed index build or startup commit while upgrading: a
/// full disk is noted (asking for another estimate's worth on top of what
/// was free) so core logs the same message as for a refused upgrade.
fn upgrade_failed(
    path: &Path,
    from: u32,
    available: u64,
    build: IndexBuild,
    error: rusqlite::Error,
) -> Error {
    if (1..INDEXED_SCHEMA).contains(&from)
        && error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull)
        && let Ok(mut short) = UPGRADE_SHORT.lock()
    {
        *short = Some((
            path.to_path_buf(),
            UpgradeSpace {
                from,
                needed: available.saturating_add(build.needed() - MIN_FREE_BYTES),
                available,
            },
        ));
    }
    Error(ErrorCode::StateNotDurable)
}
/// After `Database::open` fails: whether a pending upgrade lacks free space,
/// so the daemon can say how much it needs. Reads the store without changing
/// it; `None` when no upgrade is pending, it fits, or the store is unreadable.
/// An upgrade that `Store::open` found short of space mid-build is reported
/// once, as noted then.
pub fn upgrade_space(path: &Path) -> Option<UpgradeSpace> {
    if let Some((_, space)) = UPGRADE_SHORT
        .lock()
        .ok()
        .and_then(|mut short| short.take_if(|(noted, _)| noted == path))
    {
        return Some(space);
    }
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let from: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .ok()?;
    if !(1..INDEXED_SCHEMA).contains(&from) {
        return None;
    }
    let needed = IndexBuild::measure(&conn).ok()?.needed();
    let available = available_space(path.parent()?).ok()?;
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
        let mut conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(durable)?;
        #[cfg(feature = "test-support")]
        retention::test_support::opened(&conn);
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
        // Measured before the transaction: `temp_store` cannot change inside
        // one, and no earlier migration touches `events`.
        let upgrading = (1..INDEXED_SCHEMA).contains(&version);
        let build = if upgrading {
            IndexBuild::measure(&conn)?
        } else {
            IndexBuild::default()
        };
        let mut available = 0;
        if upgrading {
            let store = if build.in_memory() { "MEMORY" } else { "FILE" };
            conn.execute_batch(&format!("PRAGMA temp_store={store}"))
                .map_err(durable)?;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        match version {
            0..=17 => {
                if version < INDEXED_SCHEMA {
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
                    // Version 17 builds three indexes over `events` inside this
                    // transaction. Refuse before building them, writing nothing,
                    // when they would not fit above the free-space reserve; core
                    // then logs the space needed (`upgrade_space`), and also when
                    // the build or the commit runs out of space all the same.
                    available = available_space(&directory)?;
                    if available < build.needed() {
                        return Err(Error(ErrorCode::StateNotDurable));
                    }
                    #[cfg(test)]
                    if let Some(pages) = UPGRADE_MAX_PAGES.get() {
                        tx.pragma_update(None, "max_page_count", pages)
                            .map_err(durable)?;
                    }
                    tx.execute_batch(include_str!("migration-v17.sql"))
                        .map_err(|error| upgrade_failed(path, version, available, build, error))?;
                }
                tx.execute_batch(include_str!("migration-v18.sql"))
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
        // that is full, retention runs first, in this transaction (so a
        // refusal still changes nothing, upgrade included), and startup is
        // refused only when nothing more can be pruned.
        if !retention::recovery_fits(interrupted, audit_bytes) {
            retention::make_room(&tx, interrupted)?;
        }
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
        tx.commit()
            .map_err(|error| upgrade_failed(path, version, available, build, error))?;
        if upgrading {
            conn.execute_batch("PRAGMA temp_store=DEFAULT")
                .map_err(durable)?;
        }
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
mod writer_clock;
pub use compose::{
    AuthenticatedComposePreview, ComposeAuthorityKind, ComposeAuthorityObservation, compose_scopes,
};
pub use identity_store::Revocation;
pub use job_resolutions::{PendingResolution, ResolutionSubject};
#[cfg(feature = "test-support")]
pub use retention::test_support;
pub use retention::{AuditLevel, AuditUsage, RetentionFailure};
pub use storage::storage_scope;

#[cfg(test)]
fn remove_v10_schema(conn: &Connection) {
    remove_v11_schema(conn);
    conn.execute_batch("DROP TRIGGER IF EXISTS storage_mount_terminal_result; DROP TRIGGER IF EXISTS storage_mount_result_identity; DROP TRIGGER IF EXISTS storage_mount_plan_delete; DROP TRIGGER IF EXISTS storage_mount_plan_consumed; DROP TRIGGER IF EXISTS storage_mount_plan_identity; DROP TABLE IF EXISTS storage_mount_results; DROP TABLE IF EXISTS storage_mount_plans;").unwrap();
}

#[cfg(test)]
fn remove_v18_schema(conn: &Connection) {
    conn.execute_batch("ALTER TABLE meta DROP COLUMN executor_epoch;")
        .unwrap();
}

#[cfg(test)]
fn remove_v17_schema(conn: &Connection) {
    remove_v18_schema(conn);
    conn.execute_batch("DROP INDEX IF EXISTS events_notification; DROP INDEX IF EXISTS events_kind; DROP INDEX IF EXISTS events_job;")
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
