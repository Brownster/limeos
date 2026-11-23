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

/// The schema version a store has once `Store::open` has run: the last entry
/// of the migration table (`migrations::MIGRATIONS`).
pub const SCHEMA_VERSION: u32 = migrations::MIGRATIONS[migrations::MIGRATIONS.len() - 1].version;
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
/// How long a session confirmation waits before trying the query-only
/// connection again while another lookup holds it (each takes microseconds).
const READER_RETRY: Duration = Duration::from_millis(2);
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
        let mut store = Store::open_at(path, clock.wall())?;
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
                time.turn();
                let maintain = |store: &mut Store, time: &writer_clock::WriterTime| {
                    let Some(pass) = working.run(|| store.maintain(time.now(), time.wall())) else {
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
                maintain(&mut store, &time);
                loop {
                    time.turn();
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
                    maintain(&mut store, &time);
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
    /// Whether `digest` names a live session, for health to decide how much
    /// it shows while the writer may be stuck: [`Database::session_confirm`]
    /// without a CSRF value; false when unsure.
    pub async fn session_live(&self, digest: String, now: i64, bound: Duration) -> bool {
        self.session_confirm(digest, None, now, bound).await.is_ok()
    }
    /// Whether `digest` names a live session and, with `csrf`, whether that
    /// is its CSRF digest: the checks `Store::authenticate` makes, with its
    /// answers (`Unauthenticated` for no live session, `Forbidden` for a
    /// mismatched CSRF digest), read on the query-only connection. It puts
    /// nothing on either lane, so it answers with the writer stuck and its
    /// queue full, and an anonymous caller's made-up cookie adds no store
    /// work. It never waits on the connection's lock: while another lookup
    /// holds it, it tries again every `READER_RETRY`, and gives up at
    /// `bound` with `Unavailable` ("cannot confirm"), as it does when the
    /// connection is gone or the blocking pool cannot run the lookup in
    /// time. It checks; it renews and records nothing.
    pub async fn session_confirm(
        &self,
        digest: String,
        csrf: Option<String>,
        now: i64,
        bound: Duration,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + bound;
        loop {
            let (reader, digest, csrf) = (self.reader.clone(), digest.clone(), csrf.clone());
            let lookup = tokio::task::spawn_blocking(move || match reader.try_lock() {
                Ok(conn) => Some(
                    conn.as_ref()
                        .ok_or(Error(ErrorCode::Unavailable))
                        .and_then(|conn| {
                            identity_store::authenticate(conn, &digest, csrf.as_deref(), now)
                        })
                        .map(|_| ()),
                ),
                Err(std::sync::TryLockError::WouldBlock) => None,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    Some(Err(Error(ErrorCode::Unavailable)))
                }
            });
            match tokio::time::timeout_at(deadline, lookup).await {
                Ok(Ok(Some(answer))) => return answer,
                Ok(Ok(None)) if tokio::time::Instant::now() + READER_RETRY < deadline => {
                    tokio::time::sleep(READER_RETRY).await;
                }
                _ => return Err(Error(ErrorCode::Unavailable)),
            }
        }
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

/// Puts the store's connection in WAL mode and fails unless SQLite reports
/// that it is: `PRAGMA journal_mode=WAL` does not fail when it cannot switch
/// (a VFS without shared memory, a database it cannot hold in WAL); it
/// answers with the mode it kept. The writer, the query-only reader beside
/// it and the snapshot before an upgrade all rely on WAL. There is no
/// exception: `Store::open` only ever opens a database file, never an
/// in-memory one (whose mode is `memory`).
fn require_wal(conn: &Connection) -> Result<()> {
    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .map_err(durable)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
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
    /// Where job-row retention's scan resumes (`retention/jobs.rs`).
    job_scan: retention::JobScan,
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
    #[cfg(test)]
    if let Some(capacity) = OPEN_DISK_CAPACITY.get() {
        let used = std::fs::read_dir(directory)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?
            .filter_map(|e| e.ok()?.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| std::os::unix::fs::MetadataExt::blocks(&m) * 512)
            .sum::<u64>();
        return Ok(capacity.saturating_sub(used));
    }
    fs2::available_space(directory).map_err(|_| Error(ErrorCode::StateNotDurable))
}
#[cfg(test)]
thread_local! {
    static OPEN_FREE_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// Tests: the store's directory is a disk of this many bytes holding
    /// only its own files, so writing and removing files changes the space.
    static OPEN_DISK_CAPACITY: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// Tests: `Store::open` opens its database with this SQLite VFS.
    static OPEN_VFS: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
    /// Tests: runs between the snapshot and the migration transaction.
    static BEFORE_MIGRATING: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
    /// Tests: the size of the upgrade's WAL just before its connection
    /// closed: with the database file then, the upgrade's peak on disk.
    static UPGRADE_WAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Tests: the migrations the last `Store::open` on this thread ran.
    static APPLIED: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
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
/// Bytes of one page in the WAL: the page and its 24-byte frame header.
fn frame_bytes(conn: &Connection) -> Result<u64> {
    let size: i64 = conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(durable)?;
    Ok(u64::try_from(size).unwrap_or(0).saturating_add(24))
}
/// The upgrade to schema 19 (`migration-v19.sql`): it sets `settled` on
/// every settled job and builds `jobs_retention` over them.
///
/// Bounded from the pages of `jobs` (leaf, interior and overflow, counted
/// by `dbstat`), not its rows: setting `settled` enlarges each record, and
/// in the worst case every page of the table is rewritten and split, as a
/// leaf that held two records does when they no longer fit. Each page is
/// then written once to the WAL with its new sibling (two frames), and the
/// sibling is added to the database file by the checkpoint (one page), so
/// three pages per page of `jobs`. The index adds one entry per row (two
/// integers and the rowid, plus [`ENTRY_OVERHEAD`]), with the schema-17
/// build's 4/3 for page fill and its two writes. Both are what remains on
/// disk at the peak, after the commit's checkpoint and before the WAL is
/// next reset.
///
/// This rests on two facts, not on a proof for every database: a record
/// grows by at most ten bytes (a serial type and an integer) and every job
/// record is far larger, so no page splits more than once, and an interior
/// page at most doubles its cells and splits once too; and SQLite rewrites
/// a page's WAL frame in place when the same transaction writes it again,
/// so a page spilled from the cache and changed later costs one frame. An
/// overflow chain rewritten elsewhere takes no more pages than it had.
fn retention_writes(conn: &Connection) -> Result<u64> {
    let (pages, rows): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM dbstat WHERE name='jobs'),(SELECT count(*) FROM jobs)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(durable)?;
    let pages = u64::try_from(pages).unwrap_or(0);
    let rows = u64::try_from(rows).unwrap_or(0);
    let table = pages.saturating_mul(3).saturating_mul(frame_bytes(conn)?);
    let index = rows
        .saturating_mul(ENTRY_OVERHEAD + 2 * 9)
        .saturating_mul(4)
        / 3
        * 2;
    Ok(table.saturating_add(index))
}
/// The b-trees SQLite's plan for startup recovery's two statements opens
/// for writing, per interrupted job: `events`, its four indexes and its
/// AUTOINCREMENT row in `sqlite_sequence`; `jobs` and four of its indexes
/// (its primary key's, `jobs_state`, `jobs_resource_lock`, `jobs_retention`);
/// and `resource_locks` with its two indexes, which the plan opens for the
/// foreign keys. Not all of them change for a move from `running` or
/// `verifying` to `needs_intervention` (no trigger fires for it, and a
/// recovery event has no notification), so counting every opened tree is
/// an upper bound. `recovery_trees_match_the_schema` reads the count from
/// SQLite's plan, so a new index fails it.
const RECOVERY_TREES: u64 = 14;
/// What startup recovery writes for `interrupted` jobs, bounded from pages:
/// one insert or move in each of [`RECOVERY_TREES`] b-trees per job. An
/// insert into a packed index can split its leaf and,
/// in the worst case, every page up to the root; each split costs the page
/// and its sibling in the WAL and the sibling in the database file. A tree
/// of `pages` pages, every interior page holding at least four cells, is at
/// most `1 + log4(pages)` deep.
fn recovery_writes(conn: &Connection, interrupted: u64) -> Result<u64> {
    if interrupted == 0 {
        return Ok(0);
    }
    let pages: i64 = conn
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .map_err(durable)?;
    let mut depth = 1u64;
    let mut reach = 1u64;
    while reach < u64::try_from(pages).unwrap_or(0) {
        reach = reach.saturating_mul(4);
        depth += 1;
    }
    Ok(interrupted
        .saturating_mul(RECOVERY_TREES * 3)
        .saturating_mul(depth)
        .saturating_mul(frame_bytes(conn)?))
}
/// What an upgrade from `from` writes before it commits, beyond the
/// snapshot: the schema-17 index build when it crosses 17, and the
/// schema-19 rewrite when it crosses 19 (like `INDEXED_SCHEMA`, keyed by
/// version here rather than in the migration table), plus startup
/// recovery ([`recovery_writes`]).
///
/// Retention's `make_room` is not counted. It runs only when the audit
/// history is at its cap and even the revocation reserve cannot hold the
/// recovery records, and then deletes whole batches of history until they
/// fit. Deleting never grows the database file (freed pages go to its
/// freelist), but it does write the pages it changes to the WAL, so on such
/// a store an upgrade can need more than its estimate. That is a residual
/// risk, accepted because it needs the audit cap reached and jobs
/// interrupted at the moment of an upgrade, and because running out then
/// still changes nothing and is reported with figures.
fn upgrade_writes(conn: &Connection, from: u32) -> Result<(IndexBuild, u64)> {
    let build = if (1..INDEXED_SCHEMA).contains(&from) {
        IndexBuild::measure(conn)?
    } else {
        IndexBuild::default()
    };
    let retention = if (1..19).contains(&from) {
        retention_writes(conn)?
    } else {
        0
    };
    // A new store has no jobs (and no table to count them in).
    let recovery = if from >= 1 {
        recovery_writes(conn, u64::try_from(interrupted_jobs(conn)?).unwrap_or(0))?
    } else {
        0
    };
    Ok((build, retention.saturating_add(recovery)))
}
/// Free space the migrations of an upgrade from `from` need once the
/// snapshot is in place: their writes ([`upgrade_writes`]: the index build
/// and `others`), plus the fixed allowance and the write reserve that must
/// remain afterwards.
fn upgrade_remaining(from: u32, build: IndexBuild, others: u64) -> u64 {
    let base = if (1..INDEXED_SCHEMA).contains(&from) {
        build.needed()
    } else {
        MIN_FREE_BYTES.saturating_add(UPGRADE_FIXED_BYTES)
    };
    base.saturating_add(others)
}
/// Free space, and space needed, before an upgrade from `from` starts, as
/// `Store::open` measures them: `available` is the free space plus what
/// removing the store's other snapshot files gives back, and `needed` is
/// the new snapshot (none when an unchanged one is kept) plus
/// [`upgrade_remaining`]. Also returns the free space alone.
fn upgrade_figures(
    conn: &Connection,
    path: &Path,
    from: u32,
    look: migrations::snapshot::Look<'_>,
    build: IndexBuild,
    others: u64,
) -> Result<(migrations::snapshot::Plan, u64, UpgradeSpace)> {
    let directory = path.parent().ok_or(Error(ErrorCode::InvalidInput))?;
    let plan = migrations::snapshot::plan(conn, path, from, look)?;
    let free = available_space(directory)?;
    Ok((
        plan,
        free,
        UpgradeSpace {
            from,
            needed: plan
                .bytes
                .saturating_add(upgrade_remaining(from, build, others)),
            available: free.saturating_add(plan.freed),
        },
    ))
}
/// What `Store::open` found about an upgrade's space, for [`upgrade_space`]
/// to report without measuring again: the database path, and the shortfall
/// (`None`: the space was checked and fitted).
static UPGRADE_NOTE: std::sync::Mutex<Option<(PathBuf, Option<UpgradeSpace>)>> =
    std::sync::Mutex::new(None);
fn note_upgrade(path: &Path, space: Option<UpgradeSpace>) {
    if let Ok(mut note) = UPGRADE_NOTE.lock() {
        *note = Some((path.to_path_buf(), space));
    }
}
/// After `Database::open` fails: whether a pending upgrade lacks free space,
/// so the daemon can say how much it needs; `None` when no upgrade is
/// pending, it fits, or the store is unreadable. Reports what `Store::open`
/// noted when it got as far as checking (once), so a failing start costs no
/// second look. Otherwise it estimates on a read-only connection without
/// hashing anything: the store's other snapshot files count as freed (open
/// removes them), and a snapshot whose record matches on lengths, inode and
/// modification time, with the WAL empty, as kept.
pub fn upgrade_space(path: &Path) -> Option<UpgradeSpace> {
    if let Some((_, space)) = UPGRADE_NOTE
        .lock()
        .ok()
        .and_then(|mut note| note.take_if(|(noted, _)| noted == path))
    {
        return space;
    }
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let from: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .ok()?;
    if !(1..SCHEMA_VERSION).contains(&from) {
        return None;
    }
    let (build, others) = upgrade_writes(&conn, from).ok()?;
    let look = migrations::snapshot::Look::Estimate;
    let (_, _, space) = upgrade_figures(&conn, path, from, look, build, others).ok()?;
    (space.available < space.needed).then_some(space)
}
/// The store whose upgrade could not take its exclusive lock, noted for
/// [`upgrade_blocked`].
static BLOCKED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
fn note_blocked(path: &Path) {
    if let Ok(mut noted) = BLOCKED.lock() {
        *noted = Some(path.to_path_buf());
    }
}
/// After `Database::open` fails: whether a pending upgrade could not start
/// because another program has the store open (it waits half a second for
/// it). Reported once.
pub fn upgrade_blocked(path: &Path) -> bool {
    BLOCKED
        .lock()
        .ok()
        .and_then(|mut noted| noted.take_if(|noted| noted == path))
        .is_some()
}
/// The store whose last pre-upgrade snapshot was written without a durable
/// record, noted for [`unrecorded_snapshot`].
static UNRECORDED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
pub(crate) fn note_unrecorded(path: &Path) {
    if let Ok(mut noted) = UNRECORDED.lock() {
        *noted = Some(path.to_path_buf());
    }
}
/// After `Database::open` fails: whether it wrote a pre-upgrade snapshot it
/// could not record, so that the next attempt cannot keep it and writes a
/// new copy. Reported once.
pub fn unrecorded_snapshot(path: &Path) -> bool {
    UNRECORDED
        .lock()
        .ok()
        .and_then(|mut noted| noted.take_if(|noted| noted == path))
        .is_some()
}
/// A schema object of the store that differs from this build's: a trigger,
/// or a table or index the migration table names, missing or with other SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaMismatch {
    /// `trigger`, `table` or `index`.
    pub kind: &'static str,
    pub name: String,
}
/// The mismatch `Store::open` refused, noted for [`schema_mismatch`].
static SCHEMA_MISMATCH: std::sync::Mutex<Option<(PathBuf, SchemaMismatch)>> =
    std::sync::Mutex::new(None);
pub(crate) fn note_mismatch(path: &Path, mismatch: SchemaMismatch) {
    if let Ok(mut noted) = SCHEMA_MISMATCH.lock() {
        *noted = Some((path.to_path_buf(), mismatch));
    }
}
/// After `Database::open` fails: the schema object it refused the store
/// over, if that was the reason, so the daemon can name it. Reported once.
pub fn schema_mismatch(path: &Path) -> Option<SchemaMismatch> {
    SCHEMA_MISMATCH
        .lock()
        .ok()?
        .take_if(|(noted, _)| noted == path)
        .map(|(_, mismatch)| mismatch)
}
fn interrupted_jobs(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM jobs WHERE state IN ('running','verifying')",
        [],
        |r| r.get(0),
    )
    .map_err(durable)
}
thread_local! {
    /// Set when a statement on this thread failed for lack of space: how
    /// `Store::open` tells an upgrade that ran out of space from one that
    /// failed otherwise.
    static DISK_FULL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn durable(error: rusqlite::Error) -> Error {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull) {
        DISK_FULL.set(true);
    }
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
fn user_version(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(durable)
}
/// Opens the store's database for `Store::open`, in WAL mode with its
/// settings. `exclusive`: the upgrade's connection, which sets
/// `locking_mode=EXCLUSIVE` and makes an empty write transaction its first
/// access, so that it holds the database file locked against every other
/// connection, reading or writing, in this process or another, until it
/// closes. Like the normal connection it has a WAL-index file (`-shm`):
/// SQLite creates it already when the `locking_mode` statement runs, so
/// the WAL index is not kept in heap memory
/// (`the_upgrade_connection_has_a_wal_index_file`).
fn connect(path: &Path, directory: &Path, exclusive: bool) -> Result<Connection> {
    let flags = rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
    #[cfg(test)]
    let opened = match OPEN_VFS.get() {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    };
    #[cfg(not(test))]
    let opened = Connection::open_with_flags(path, flags);
    #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
    let mut conn = opened.map_err(durable)?;
    #[cfg(feature = "test-support")]
    retention::test_support::opened(&mut conn);
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if rusqlite::version_number() < 3_051_003 {
        return Err(Error(ErrorCode::Unavailable));
    }
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(durable)?;
    if exclusive {
        let mode: String = conn
            .pragma_update_and_check(None, "locking_mode", "EXCLUSIVE", |r| r.get(0))
            .map_err(durable)?;
        if !mode.eq_ignore_ascii_case("exclusive") {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // In exclusive locking mode a read takes only a shared lock, which
        // lets another connection read and even write; the exclusive lock
        // comes with the first write transaction, and is then held until
        // the connection closes. Take it now, as the first access.
        conn.execute_batch("BEGIN IMMEDIATE; COMMIT;")
            .map_err(|error| {
                // Another connection has the store open (a WAL connection
                // holds a shared lock for as long as it is open).
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) {
                    note_blocked(path);
                }
                durable(error)
            })?;
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(durable)?;
    require_wal(&conn)?;
    conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=4194304; PRAGMA trusted_schema=OFF;").map_err(durable)?;
    // Below the free-space reserve, the only space this store can give back
    // without a VACUUM (which needs more free space, not less) is a WAL left
    // by a crash: checkpoint and truncate it before the check decides.
    // Pruning cannot help here: freed pages stay in the file's freelist.
    if available_space(directory)? < MIN_FREE_BYTES {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(durable)?;
    }
    Ok(conn)
}
/// What `startup` needs to know about an upgrade under way: the index
/// build and the other writes it was admitted for.
#[derive(Clone, Copy)]
struct Upgrading {
    build: IndexBuild,
    others: u64,
}
/// Upgrades the store at `path` from the version it is at, on a connection
/// that holds it exclusively for the whole of it: the snapshot (hash, copy
/// and record) and the startup transaction that migrates, checks, recovers
/// and commits all see one state, and no other connection can read or
/// write the store meanwhile. Returns the new generation.
///
/// The store's database file is read for its hash only through `store`, a
/// descriptor opened here and closed after the connection: closing any
/// descriptor of a file drops every POSIX lock this process holds on it,
/// the exclusive connection's included.
fn upgrade(
    path: &Path,
    directory: &Path,
    wall: i64,
    migrations: &[migrations::Migration],
) -> Result<i64> {
    let store = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    let mut conn = connect(path, directory, true)?;
    let generation = (|| {
        let version = user_version(&conn)?;
        let target = migrations.last().map_or(0, |m| m.version);
        if !(1..target).contains(&version) {
            return Err(Error(ErrorCode::Conflict));
        }
        let (build, others) = upgrade_writes(&conn, version)?;
        let look = migrations::snapshot::Look::Writer { store: &store };
        let (plan, free, space) = upgrade_figures(&conn, path, version, look, build, others)?;
        // Refused, removing and writing nothing, unless the snapshot and
        // then the migrations fit above the free-space reserve, with what
        // removing the store's other snapshot files gives back
        // (`upgrade_space` reports the same figures).
        if space.available < space.needed {
            note_upgrade(path, Some(space));
            return Err(Error(ErrorCode::StateNotDurable));
        }
        note_upgrade(path, None);
        let write_first = free >= plan.bytes.saturating_add(MIN_FREE_BYTES);
        migrations::snapshot::take(&conn, &store, path, plan, write_first)?;
        #[cfg(test)]
        if let Some(hook) = BEFORE_MIGRATING.get() {
            hook(path);
        }
        let available = available_space(directory)?;
        let upgrading = Upgrading { build, others };
        DISK_FULL.set(false);
        startup(
            &mut conn,
            path,
            directory,
            wall,
            migrations,
            version,
            Some(upgrading),
        )
        .inspect_err(|_| {
            // Out of space somewhere in the transaction (a migration, the
            // checks, recovery or the commit): asks for another estimate's
            // worth on top of what was free once the snapshot was in place.
            // The snapshot is not asked for again: the next start keeps it,
            // the store being unchanged, or counts removing it as freed.
            if DISK_FULL.take() {
                let remaining = upgrade_remaining(version, build, others);
                note_upgrade(
                    path,
                    Some(UpgradeSpace {
                        from: version,
                        needed: available.saturating_add(remaining - MIN_FREE_BYTES),
                        available,
                    }),
                );
            }
        })
    })();
    #[cfg(test)]
    {
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        UPGRADE_WAL.set(std::fs::metadata(PathBuf::from(wal)).map_or(0, |m| m.len()));
    }
    // The connection closes (checkpointing its WAL) before the descriptor.
    drop(conn);
    drop(store);
    generation
}
/// `Store::open`'s startup transaction on `conn`: the migrations above
/// `version` when `upgrading`, the schema check, the data checks, retention
/// reconciliation, the new generation and startup recovery, committed
/// together or not at all. Returns the new generation.
fn startup(
    conn: &mut Connection,
    path: &Path,
    directory: &Path,
    wall: i64,
    migrations: &[migrations::Migration],
    version: u32,
    upgrading: Option<Upgrading>,
) -> Result<i64> {
    // Set before the transaction: `temp_store` cannot change inside one.
    let indexing = upgrading.is_some_and(|_| (1..INDEXED_SCHEMA).contains(&version));
    if let Some(Upgrading { build, .. }) = upgrading.filter(|_| indexing) {
        let store = if build.in_memory() { "MEMORY" } else { "FILE" };
        conn.execute_batch(&format!("PRAGMA temp_store={store}"))
            .map_err(durable)?;
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(durable)?;
    #[cfg(test)]
    APPLIED.with(|applied| applied.borrow_mut().clear());
    #[cfg(test)]
    if let Some(pages) = UPGRADE_MAX_PAGES.get() {
        tx.pragma_update(None, "max_page_count", pages)
            .map_err(durable)?;
    }
    for (index, migration) in migrations.iter().enumerate() {
        if migration.version <= version {
            continue;
        }
        #[cfg(test)]
        APPLIED.with(|applied| applied.borrow_mut().push(migration.version));
        // Version 17 builds three indexes over `events` inside this
        // transaction. Refuse before building them, writing nothing, when
        // the rest of the upgrade would not fit above the free-space
        // reserve. A new store has nothing to index and is checked against
        // the fixed allowance alone.
        if migration.version == INDEXED_SCHEMA {
            let (build, others) =
                upgrading.map_or((IndexBuild::default(), 0), |u| (u.build, u.others));
            if available_space(directory)? < upgrade_remaining(version, build, others) {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        migrations::apply(&tx, path, migrations, index)?;
    }
    let check: String = tx
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(durable)?;
    if check != "ok" {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    if let Some(mismatch) = migrations::mismatch(&tx, migrations)? {
        note_mismatch(path, mismatch);
        return Err(Error(ErrorCode::StateNotDurable));
    }
    resources::check_data(&tx)?;
    if available_space(directory)? < MIN_FREE_BYTES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let audit_bytes = retention::reconcile(&tx)?;
    let interrupted = interrupted_jobs(&tx)?;
    // Recovery records use the revocation reserve past the cap. When even
    // that is full, retention runs first, in this transaction (so a
    // refusal still changes nothing, upgrade included), and startup is
    // refused only when nothing more can be pruned.
    if !retention::recovery_fits(interrupted, audit_bytes) {
        retention::make_room(&tx, interrupted, wall)?;
    }
    tx.execute("UPDATE meta SET generation=generation+1 WHERE id=1", [])
        .map_err(durable)?;
    let generation = tx
        .query_row("SELECT generation FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(durable)?;
    // Set-based recovery stays bounded in memory. Every uncertain job
    // keeps its complete resource set; recovery never redispatches effects.
    let recovered_at = wall;
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
    if indexing {
        conn.execute_batch("PRAGMA temp_store=DEFAULT")
            .map_err(durable)?;
    }
    Ok(generation)
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_at(path, writer_clock::unix_now())
    }
    /// `open`, at `wall` (seconds since the Unix epoch, from the clock the
    /// writer thread will read): the time startup recovery records and the
    /// time any batch it must run to make room is taken at.
    fn open_at(path: &Path, wall: i64) -> Result<Self> {
        Self::open_migrating_at(path, wall, migrations::MIGRATIONS)
    }
    /// Tests: `open`, applying and checking `migrations` (the migration
    /// table with more entries after it).
    #[cfg(test)]
    fn open_migrating(path: &Path, migrations: &[migrations::Migration]) -> Result<Self> {
        Self::open_migrating_at(path, writer_clock::unix_now(), migrations)
    }
    /// `open_at`, applying and checking `migrations`.
    fn open_migrating_at(
        path: &Path,
        wall: i64,
        migrations: &[migrations::Migration],
    ) -> Result<Self> {
        let target = migrations
            .last()
            .ok_or(Error(ErrorCode::Unavailable))?
            .version;
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
        // Under the core lock, before SQLite opens anything.
        let _ = migrations::snapshot::remove_leftovers(path);
        let mut conn = connect(path, &directory, false)?;
        let version = user_version(&conn)?;
        if version > target {
            return Err(Error(ErrorCode::Conflict));
        }
        let generation = if (1..target).contains(&version) {
            // The upgrade runs on its own connection, holding the store
            // exclusively; the normal one opens again after it has closed.
            drop(conn);
            let generation = upgrade(path, &directory, wall, migrations)?;
            conn = connect(path, &directory, false)?;
            generation
        } else {
            startup(&mut conn, path, &directory, wall, migrations, version, None)?
        };
        Ok(Self {
            conn,
            _lock: lock,
            directory,
            generation,
            observation_leases: std::collections::HashMap::new(),
            observation_watermarks: std::collections::HashMap::new(),
            retained_at: None,
            job_scan: retention::JobScan::default(),
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
mod migrations;
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

/// Set by every shipped binary's `main` before anything else
/// ([`refuse_test_support`]).
static TEST_SUPPORT_REFUSED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Called first thing by the `main` of every binary that links this crate
/// (core, limeosctl): from then on, the soak's seams (`test_support::open`'s
/// simulated writer clock and `test_support::on_open`'s connection hook)
/// cannot be selected in this process, even in a build where feature
/// unification compiled them in (`cargo build --workspace --all-targets`
/// enables the `test-support` feature that limeos-core's dev-dependencies
/// ask for). A unit test's process never runs a binary's `main` (the test
/// harness brings its own), so the soak can still select them there. Without
/// the feature it only sets the flag.
pub fn refuse_test_support() {
    TEST_SUPPORT_REFUSED.store(true, Ordering::SeqCst);
}
/// Panics if this process's `main` refused the test seams: reached only by
/// selecting one, which no shipped code does.
#[cfg(feature = "test-support")]
pub(crate) fn assert_test_support_allowed(seam: &str) {
    assert!(
        !TEST_SUPPORT_REFUSED.load(Ordering::SeqCst),
        "{seam} selected in a process whose main refused the test seams: test-support is for unit tests only"
    );
}
pub use retention::{AuditLevel, AuditUsage, RetentionFailure};
pub use storage::storage_scope;

#[cfg(test)]
fn remove_v10_schema(conn: &Connection) {
    remove_v11_schema(conn);
    conn.execute_batch("DROP TRIGGER IF EXISTS storage_mount_terminal_result; DROP TRIGGER IF EXISTS storage_mount_result_identity; DROP TRIGGER IF EXISTS storage_mount_plan_delete; DROP TRIGGER IF EXISTS storage_mount_plan_consumed; DROP TRIGGER IF EXISTS storage_mount_plan_identity; DROP TABLE IF EXISTS storage_mount_results; DROP TABLE IF EXISTS storage_mount_plans;").unwrap();
}

#[cfg(test)]
fn remove_v19_schema(conn: &Connection) {
    // Back to the version-7 and version-16 delete guards, no job delete
    // guard and no settlement time.
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS jobs_delete; DROP TRIGGER IF EXISTS job_resources_delete; DROP TRIGGER IF EXISTS job_resolutions_delete;
DROP TRIGGER IF EXISTS jobs_settled; DROP TRIGGER IF EXISTS jobs_settled_now; DROP INDEX IF EXISTS jobs_retention;
ALTER TABLE jobs DROP COLUMN settled;
CREATE TRIGGER job_resources_delete BEFORE DELETE ON job_resources
WHEN (SELECT state FROM jobs WHERE id=OLD.job) IS NOT 'queued'
    OR OLD.resource=(SELECT resource FROM jobs WHERE id=OLD.job)
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;
CREATE TRIGGER job_resolutions_delete BEFORE DELETE ON job_resolutions
WHEN OLD.confirmation IS NOT NULL
BEGIN SELECT RAISE(ABORT,'completed resolution is retained'); END;",
    )
    .unwrap();
}

#[cfg(test)]
fn remove_v18_schema(conn: &Connection) {
    remove_v19_schema(conn);
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
