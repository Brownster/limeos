//! The simulation's state: core (the real `Core`, its store's writer thread
//! on the simulated clock), the executor process, the Engine, the clock, the
//! workload's users and flows, the ledger of every job, and how every result
//! is judged.
use super::{
    engine::World,
    executor::{Crash, Executor, Opened},
    support::{Clock, DAY, HOUR, Io, MINUTE, Rng, START},
};
use crate::{Core, operations::Dispatcher, password_admission::PasswordWorkers};
use limeos_domain::{
    ContainerAction, DeclaredOutcome, Error, ErrorCode, PlannedContainerAction, PlannedStackAction,
    Principal, Result,
};
use limeos_persistence::{Store, test_support};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::Semaphore;

/// Refusals that mean a counter, table or cap has run out: never a legitimate
/// answer to ordinary use, whatever the call site expects.
pub const LIFETIME: [ErrorCode; 3] = [
    ErrorCode::StateNotDurable,
    ErrorCode::Overloaded,
    ErrorCode::BeyondReceiptHistory,
];

/// Errors core's dispatcher logs and moves on from: the executor is
/// unreachable or lost an answer, a lock is held, or a receipt is refused.
pub const DISPATCH_LEGIT: [ErrorCode; 4] = [
    ErrorCode::Conflict,
    ErrorCode::Unavailable,
    ErrorCode::NotFound,
    ErrorCode::Expired,
];

/// A core retention rule's limits, from the production rules
/// (`retention::RULES`): (rows it keeps, most rows one batch removes).
pub fn rule_limits(rule: &str) -> (i64, i64) {
    test_support::rules()
        .into_iter()
        .find(|(name, _, _)| *name == rule)
        .map(|(_, budget, batch)| (budget, batch))
        .unwrap_or_else(|| panic!("no retention rule named {rule}"))
}

/// What goes wrong around one job, decided when it is queued (or, for a
/// resolution, when the operator decides).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobFault {
    /// The executor handles the execute; core never sees the answer.
    AnswerLost,
    /// The executor dies with the execute unread (and restarts).
    LostBeforeStore,
    /// The executor stops between answering `ReceiptEpoch` and the execute:
    /// the execute is never sent.
    ExecutorDownAfterEpoch,
    /// The executor is killed inside the Engine call, after the insert that
    /// recorded `prepared` (and pruned, once the store is full).
    ExecutorKilledInEngineCall,
    /// The Engine answers an error; `applied`: the container changed anyway.
    EngineFailure { applied: bool },
    /// Core restarts after the claim, before the execute is sent.
    CoreCrashAfterClaim,
    /// Core restarts after the execute answer, before it reads it.
    CoreCrashAfterExecute,
    /// Core restarts after the executor verified, before it reads it.
    CoreCrashAfterVerify,
    /// The first recovery lookup's answer is lost.
    LookupLost,
    /// Core restarts after recording an operator's decision, before the
    /// executor sees it: the retry loop completes it after startup.
    ResolutionInterrupted,
}
impl JobFault {
    /// A simulated core restart.
    pub fn restarts_core(self) -> bool {
        matches!(
            self,
            Self::CoreCrashAfterClaim
                | Self::CoreCrashAfterExecute
                | Self::CoreCrashAfterVerify
                | Self::ResolutionInterrupted
        )
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::AnswerLost => "execute answer lost",
            Self::LostBeforeStore => "executor died with the execute unread",
            Self::ExecutorDownAfterEpoch => "executor stopped between epoch and execute",
            Self::ExecutorKilledInEngineCall => "executor killed inside the Engine call",
            Self::EngineFailure { applied: true } => "Engine error after acting",
            Self::EngineFailure { applied: false } => "Engine error before acting",
            Self::CoreCrashAfterClaim => "core restart between claim and execute",
            Self::CoreCrashAfterExecute => "core restart between execute answer and recording",
            Self::CoreCrashAfterVerify => "core restart between executor verify and recording",
            Self::LookupLost => "recovery lookup answer lost",
            Self::ResolutionInterrupted => "core restart between resolution request and executor",
        }
    }
}

/// Core died (at a point the executor signalled) while the harness drove it.
pub struct CoreCrash;

/// Runs core's `work` unless core dies first: the crash signal is polled
/// ahead of the work, so once the executor has signalled it, core's work is
/// never polled again (its future is dropped where it stands).
pub async fn guarded<F: Future>(
    crash: &Crash,
    work: F,
) -> std::result::Result<F::Output, CoreCrash> {
    tokio::select! {
        biased;
        () = crash.wait() => Err(CoreCrash),
        value = work => Ok(value),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Container,
    Stack,
}

/// Every job the workload queued, with what the harness knows about it.
pub struct JobRecord {
    pub kind: Kind,
    pub operation: ContainerAction,
    pub targets: usize,
    pub fault: Option<JobFault>,
    pub declared: Option<DeclaredOutcome>,
    /// The user who queued it (its progress is polled with their session).
    pub user: usize,
}

pub struct Session {
    pub token: String,
    pub csrf: String,
}
impl Session {
    pub fn digest(&self) -> String {
        limeos_identity::digest(&self.token)
    }
    pub fn csrf_digest(&self) -> String {
        limeos_identity::digest(&self.csrf)
    }
}
pub struct User {
    pub name: &'static str,
    pub principal: Principal,
    pub session: Option<Session>,
}

pub enum Planned {
    Container(PlannedContainerAction),
    Stack(PlannedStackAction),
}
/// One user's proposal on its way to a job.
pub struct Flow {
    pub user: usize,
    pub planned: Planned,
    pub approval: Option<String>,
    pub key: String,
    pub fate: super::workload::Fate,
}

/// A store open, measured, with the simulated time it happened.
#[derive(Clone, Debug)]
pub struct Startup {
    pub at: i64,
    pub reason: &'static str,
    /// The database file and its WAL when the open began.
    pub file: u64,
    pub wal: u64,
    pub opened: Opened,
    /// For an open on a copy of core's store: what each of its statements
    /// read, from the open's own statement trace.
    pub trace: Option<super::startup::Trace>,
}

/// Sizes of a database file and its WAL.
pub fn sizes(path: &std::path::Path) -> (u64, u64) {
    let len = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    (len(path), len(std::path::Path::new(&wal)))
}

#[derive(Default)]
pub struct Stats {
    /// Successful operations by name.
    pub ops: BTreeMap<String, u64>,
    /// Legitimate refusals by operation and code.
    pub refused: BTreeMap<(String, String), u64>,
    /// Failed checks as they happen: the check's name, then when and what.
    pub violations: Vec<(&'static str, String)>,
    /// Faults the harness itself caused (the executor counts its own).
    pub faults: BTreeMap<&'static str, u64>,
    pub restarts: BTreeMap<&'static str, u64>,
    /// Peak rows of core's `events` table seen at any retention pass.
    pub peak_events: i64,
    pub retention_removed: BTreeMap<&'static str, i64>,
    /// The largest batch each rule removed in one transaction.
    pub largest_batch: BTreeMap<&'static str, i64>,
    pub dispatcher_passes: u64,
    /// Wall time spent per part of the run (for the runtime budget only).
    pub timing: BTreeMap<&'static str, std::time::Duration>,
    pub retention_passes: u64,
    pub reads: u64,
    /// Failures core logged as `InvalidInput`: each a lost executor answer
    /// (`checks::LOST_ANSWERS`).
    pub lost_as_invalid: u64,
}

impl Stats {
    pub fn time(&mut self, part: &'static str, since: Instant) {
        *self.timing.entry(part).or_default() += since.elapsed();
    }
    pub fn op(&mut self, op: &str) {
        *self.ops.entry(op.to_owned()).or_default() += 1;
    }
}

/// A warning or error core logged while the harness drove it.
pub struct Logged {
    pub error: bool,
    pub message: String,
    pub code: Option<String>,
}
/// Collects what core logs on the test thread: its dispatcher, recovery and
/// resolution report failures with the error's code and move on, so this is
/// where a lifetime refusal inside core's own paths shows.
struct Capture(Arc<Mutex<Vec<Logged>>>);
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let level = *event.metadata().level();
        if level > tracing::Level::WARN {
            return;
        }
        struct Fields(String, Option<String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                match field.name() {
                    "message" => self.0 = format!("{value:?}"),
                    "code" => self.1 = Some(format!("{value:?}")),
                    _ => {}
                }
            }
        }
        let mut fields = Fields(String::new(), None);
        event.record(&mut fields);
        self.0.lock().unwrap().push(Logged {
            error: level == tracing::Level::ERROR,
            message: fields.0,
            code: fields.1,
        });
    }
}

pub struct Sim {
    pub name: &'static str,
    pub clock: Clock,
    pub rng: Rng,
    /// Holds both stores; removed with the run.
    pub _dir: tempfile::TempDir,
    pub core_path: PathBuf,
    /// The running core, if any; its dispatcher's state between ticks.
    pub core: Option<Core>,
    pub dispatcher: Dispatcher,
    pub exec: Executor,
    pub world: Arc<World>,
    log: Arc<Mutex<Vec<Logged>>>,
    _log: tracing::subscriber::DefaultGuard,
    pub users: Vec<User>,
    /// A session kept past its expiry, to provoke its refusal.
    pub stale: Option<Session>,
    pub flows: BTreeMap<u64, Flow>,
    pub next_flow: u64,
    pub ledger: BTreeMap<String, JobRecord>,
    /// Operator attempts per job, so a refused declaration is not repeated.
    pub attempts: std::collections::HashMap<String, u32>,
    pub stats: Stats,
    /// What core logged without a code, by message (reported).
    pub logged: BTreeMap<String, u64>,
    pub core_starts: Vec<Startup>,
    pub executor_starts: Vec<Startup>,
    /// When the next dispatcher pass is due.
    pub next_pass: i64,
    /// When a stopped executor comes back.
    pub executor_back_at: Option<i64>,
    pub samples: Vec<super::checks::Sample>,
    /// Per-job growth over the last third (bytes a day) and the years until
    /// core's file reaches its bound at that rate.
    pub projection: Option<(f64, f64)>,
    /// Per store: startup reads added a day over the last third, and the
    /// years until an open reads its bound at that rate.
    pub startup_projection: Vec<(&'static str, f64, f64)>,
    /// What the trend and startup checks measured, by name: what the pins of
    /// `checks::EXPECTED_FAILURES` are compared with.
    pub measures: BTreeMap<&'static str, f64>,
    /// Idle windows measured: (simulated start, the store files written).
    pub idle: Vec<(i64, Vec<String>)>,
    /// Idle windows scheduled, and those skipped with the reason.
    pub idle_scheduled: u64,
    pub idle_skipped: Vec<(i64, String)>,
    pub started: Instant,
    /// Core's store restarts only before this (simulated) time; after it
    /// the store stays open and startup is measured on copies.
    pub core_restarts_until: i64,
    /// Whether standalone and stack receipts have reached what retention
    /// keeps, and the highest prune watermark seen.
    pub receipts_reached: (bool, bool),
    pub last_watermark: i64,
    /// Authority events at the last checkpoint: they never decrease.
    pub authority_seen: i64,
    side: Option<rusqlite::Connection>,
}

/// Real time a stopped core's store may take to close (its writer thread
/// finishes its item and checkpoints; the runtime drops aborted tasks): a
/// bound on a hang, never reached by a store that closes.
const STOP_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Free space a tmpfs needs before the soak puts its stores there: every
/// test of the binary at once peaks near 300 MiB.
const TMPFS_ROOM: u64 = 1 << 30;

/// Where a test's temporary stores go: `LIMEOS_SOAK_DIR` when set; else
/// `/dev/shm` when it has room (a tmpfs keeps fsync out of the run time, and
/// a GitHub-hosted runner's is large), else the system temp directory (Docker
/// gives a container a 64 MiB `/dev/shm`). Every check counts work, not
/// time, so the choice changes how long a run takes, not what it measures.
pub fn soak_dir() -> tempfile::TempDir {
    let base = std::env::var_os("LIMEOS_SOAK_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let shm = PathBuf::from("/dev/shm");
            let stat = rustix::fs::statvfs(&shm).ok()?;
            (stat.f_bavail.saturating_mul(stat.f_frsize) >= TMPFS_ROOM).then_some(shm)
        })
        .unwrap_or_else(std::env::temp_dir);
    tempfile::Builder::new()
        .prefix("limeos-soak-")
        .tempdir_in(base)
        .expect("soak directory")
}

impl Sim {
    pub fn new(name: &'static str, seed: u64, core_restarts_until: Option<i64>) -> Self {
        let clock = Clock::new(START);
        // Core's own code on this thread reads the simulated clock.
        crate::test_clock::install(Some(clock.sim().clone()));
        let world = Arc::new(World::new(clock.clone()));
        let dir = soak_dir();
        let executor_dir = dir.path().join("executor");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&executor_dir)
            .unwrap();
        let uid = std::fs::metadata("/proc/self").unwrap().uid();
        let exec = Executor::new(
            &executor_dir,
            dir.path().join("executor.sock"),
            world.clone(),
            clock.clone(),
            uid,
        );
        let core_path = dir.path().join("core").join("core.sqlite");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(core_path.parent().unwrap())
            .unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let subscriber = {
            use tracing_subscriber::layer::SubscriberExt;
            tracing_subscriber::registry().with(Capture(log.clone()))
        };
        let guard = tracing::subscriber::set_default(subscriber);
        Self {
            name,
            clock,
            rng: Rng::new(seed),
            _dir: dir,
            core_path,
            core: None,
            dispatcher: Dispatcher::default(),
            exec,
            world,
            log,
            _log: guard,
            users: Vec::new(),
            stale: None,
            flows: BTreeMap::new(),
            next_flow: 0,
            ledger: BTreeMap::new(),
            attempts: Default::default(),
            stats: Stats::default(),
            logged: BTreeMap::new(),
            core_starts: Vec::new(),
            executor_starts: Vec::new(),
            next_pass: START,
            executor_back_at: None,
            samples: Vec::new(),
            projection: None,
            startup_projection: Vec::new(),
            measures: BTreeMap::new(),
            idle: Vec::new(),
            idle_scheduled: 0,
            idle_skipped: Vec::new(),
            started: Instant::now(),
            core_restarts_until: core_restarts_until.map_or(i64::MAX, |day| START + day * DAY),
            receipts_reached: (false, false),
            last_watermark: i64::MIN,
            authority_seen: 0,
            side: None,
        }
    }
    pub fn now(&self) -> i64 {
        self.clock.now()
    }
    pub fn when(&self) -> String {
        let t = self.now() - START;
        format!(
            "day {} {:02}:{:02}:{:02}",
            t / DAY,
            t % DAY / HOUR,
            t % HOUR / MINUTE,
            t % MINUTE
        )
    }
    pub fn core(&self) -> Core {
        self.core.clone().expect("core is running")
    }
    /// One call on core's store, through its writer thread.
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.core().db.call(f).await
    }
    /// Advances the clock to `at`: the writer thread runs its retention gate
    /// there before this returns, and what it did is judged.
    pub fn advance(&mut self, at: i64) -> i64 {
        self.clock.set(at.max(self.now()));
        // Core-death faults fire only while core may restart.
        self.exec.faults().core_restarts = self.core_restarts();
        self.absorb_passes()
    }
    /// A read-only side connection to core's store, kept open: it can
    /// neither write nor checkpoint, and holds no snapshot between reads.
    pub fn side(&mut self) -> &rusqlite::Connection {
        if self.side.is_none() {
            self.side = Some(super::checks::reader(&self.core_path).expect("core side connection"));
        }
        self.side.as_ref().unwrap()
    }

    /// Judges one result: success, a refusal the workload provoked or the
    /// system rightly gives (`legit`), or a violation. A lifetime refusal is
    /// a violation wherever it comes from.
    pub fn judge<T>(&mut self, op: &str, result: Result<T>, legit: &[ErrorCode]) -> Option<T> {
        match result {
            Ok(value) => {
                self.stats.op(op);
                Some(value)
            }
            Err(Error(code)) if legit.contains(&code) && !LIFETIME.contains(&code) => {
                *self
                    .stats
                    .refused
                    .entry((op.to_owned(), format!("{code:?}")))
                    .or_default() += 1;
                None
            }
            Err(Error(code)) => {
                let kind = if LIFETIME.contains(&code) {
                    "lifetime refusal"
                } else {
                    "unexpected refusal"
                };
                self.violation(super::checks::REFUSALS, format!("{op}: {kind} {code:?}"));
                None
            }
        }
    }
    pub fn violation(&mut self, check: &'static str, what: String) {
        let line = format!("{}: {what}", self.when());
        eprintln!("[{}] {check}: {line}", self.name);
        self.stats.violations.push((check, line));
    }

    /// Core startup: `Database::open` (with its writer on the simulated
    /// clock): `Store::open`'s schema, quick_check, every startup invariant
    /// check, audit reconciliation and recovery of interrupted jobs,
    /// measured; then the writer thread's own first retention pass. Fresh
    /// in-memory state: core's rotations do not survive it.
    pub async fn start_core(&mut self, reason: &'static str) {
        assert!(self.core.is_none());
        let (file, wal) = sizes(&self.core_path);
        let (io, at) = (Io::now(), Instant::now());
        let opened = test_support::open(&self.core_path, self.clock.sim());
        let opened_at = Opened {
            io: Io::now().since(io),
            elapsed: at.elapsed(),
        };
        let db = match opened {
            Ok(db) => db,
            Err(Error(code)) => panic!(
                "[{}] {}: core store refused to open ({reason}): {code:?}",
                self.name,
                self.when()
            ),
        };
        // Core's code that runs on the writer thread reads the same clock.
        let clock = self.clock.sim().clone();
        db.call(move |_| {
            crate::test_clock::install(Some(clock));
            Ok(())
        })
        .await
        .expect("the writer thread serves its first call");
        self.clock.sim().settle();
        self.core = Some(Core {
            automation_status: Default::default(),
            reader_health: Default::default(),
            db,
            password_workers: Arc::new(PasswordWorkers::new(2)),
            dummy_hash: "unused".into(),
            observations: Default::default(),
            telemetry: None,
            container_socket: Arc::new(self.exec.socket.clone()),
            storage_socket: Arc::new("/nonexistent/limeos-soak-storage".into()),
            storage_target_socket: Arc::new("/nonexistent/limeos-soak-targets".into()),
            storage_mount_socket: Arc::new("/nonexistent/limeos-soak-mounts".into()),
            storage_readers: Arc::new(Semaphore::new(1)),
            claims_in_doubt: Default::default(),
            stacks: Default::default(),
            compose_catalog: None,
        });
        self.dispatcher = Dispatcher::default();
        *self.stats.restarts.entry(reason).or_default() += 1;
        self.core_starts.push(Startup {
            at: self.now(),
            reason,
            file,
            wal,
            opened: opened_at,
            trace: None,
        });
        self.absorb_passes();
        // The dispatcher's first tick is immediate: startup leftovers.
        self.next_pass = self.now();
    }
    /// Core stops: every task it had is dropped with it, and the store's
    /// writer thread finishes and releases the core lock.
    async fn stop_core(&mut self) {
        self.core = None;
        // A task core detached (a resolution's executor step) dies with the
        // process: aborted, it is never polled again, and the runtime drops
        // it (and its handle on the store) at its next turn.
        let aborted = crate::test_clock::abort_detached();
        if aborted > 0 {
            *self
                .stats
                .faults
                .entry("detached task ended by a core stop")
                .or_default() += aborted as u64;
        }
        let deadline = Instant::now() + STOP_LIMIT;
        while self.clock.sim().writers() > 0 {
            assert!(
                Instant::now() < deadline,
                "[{}] {}: core's store is still open {} s after core stopped: something other than core's detached tasks holds it",
                self.name,
                self.when(),
                STOP_LIMIT.as_secs()
            );
            tokio::task::yield_now().await;
        }
        self.exec.crash.clear();
        self.absorb_passes();
    }
    /// Whether core may restart now (the main run keeps its store open from
    /// day 30, so a drifting counter is not recomputed away by `Store::open`).
    pub fn core_restarts(&self) -> bool {
        self.now() < self.core_restarts_until
    }
    pub async fn restart_core(&mut self, reason: &'static str) {
        assert!(
            self.core_restarts(),
            "[{}] {}: core restarted ({reason}) after its restarts ended",
            self.name,
            self.when()
        );
        self.absorb_passes();
        super::checks::core_accounting(self, reason);
        self.stop_core().await;
        self.start_core(reason).await;
        super::checks::after_restart(self, reason);
    }
    pub fn start_executor(&mut self, reason: &'static str) {
        let now = self.now();
        let (file, wal) = sizes(&self.exec.dir.join("receipts.sqlite"));
        match self.exec.start(now) {
            Ok(opened) => self.executor_starts.push(Startup {
                at: now,
                reason,
                file,
                wal,
                opened,
                trace: None,
            }),
            Err(Error(code)) => panic!(
                "[{}] {}: receipt store refused to open ({reason}): {code:?}",
                self.name,
                self.when()
            ),
        }
        *self.stats.restarts.entry(reason).or_default() += 1;
    }
    pub async fn restart_executor(&mut self, reason: &'static str) {
        super::checks::receipt_accounting(self, reason);
        self.exec.stop().await;
        self.start_executor(reason);
        super::checks::after_restart(self, reason);
    }

    /// Judges every retention pass the writer thread ran since the last
    /// call: a failed rule is a violation like any refusal, and every batch
    /// must stay within its rule's limit. Returns the rows they removed.
    pub fn absorb_passes(&mut self) -> i64 {
        let mut total = 0;
        for pass in self.clock.sim().take_passes() {
            self.stats.retention_passes += 1;
            for (rule, code) in pass.failed {
                self.violation(
                    super::checks::REFUSALS,
                    format!("retention pass: the {rule} batch failed with {code:?}"),
                );
            }
            let mut events = pass
                .usage
                .map_or(0, |usage| usage.bytes / test_support::EVENT_BYTES);
            for (rule, rows) in pass.removed {
                let limit = rule_limits(rule).1;
                if rows > limit {
                    self.violation(
                        super::checks::BATCHES,
                        format!(
                            "the {rule} batch removed {rows} rows in one transaction (limit {limit})"
                        ),
                    );
                }
                if rows > 0 {
                    *self.stats.retention_removed.entry(rule).or_default() += rows;
                    let largest = self.stats.largest_batch.entry(rule).or_default();
                    *largest = (*largest).max(rows);
                }
                if rule == "events" || rule == "authority events" {
                    events += rows;
                }
                total += rows;
            }
            self.stats.peak_events = self.stats.peak_events.max(events);
        }
        total
    }
    /// After core's work: what it logged and what the executor saw, judged;
    /// an executor that died is restarted (or comes back later).
    pub async fn absorb(&mut self) {
        let logged = std::mem::take(&mut *self.log.lock().unwrap());
        for entry in logged {
            let code = entry.code.as_deref();
            // A connection the executor closed unanswered: `read_frame`
            // reports the end of the stream as `InvalidInput`, a defect of
            // its own (`checks::LOST_ANSWERS`), judged there.
            if code == Some("InvalidInput") && !entry.error {
                self.stats.lost_as_invalid += 1;
                self.violation(
                    super::checks::LOST_ANSWERS,
                    format!(
                        "core logged a lost executor answer as InvalidInput: {}",
                        entry.message
                    ),
                );
                continue;
            }
            let lifetime = code.is_some_and(|c| LIFETIME.iter().any(|l| format!("{l:?}") == c));
            let legit = code.is_some_and(|c| DISPATCH_LEGIT.iter().any(|l| format!("{l:?}") == c));
            if entry.error || lifetime || (code.is_some() && !legit) {
                let kind = if lifetime {
                    "lifetime refusal"
                } else if entry.error {
                    "error"
                } else {
                    "unexpected refusal"
                };
                self.violation(
                    super::checks::REFUSALS,
                    format!("core logged {kind}: {} ({code:?})", entry.message),
                );
            } else if let Some(code) = code {
                *self
                    .stats
                    .refused
                    .entry((format!("core: {}", entry.message), code.to_owned()))
                    .or_default() += 1;
            } else {
                *self.logged.entry(entry.message).or_default() += 1;
            }
        }
        let violations = std::mem::take(&mut self.exec.faults().violations);
        for violation in violations {
            self.violation(super::checks::REFUSALS, violation);
        }
        if let Some(death) = self.exec.take_death() {
            super::checks::receipt_accounting(self, death.reason);
            self.exec.stop().await;
            match death.back_at {
                Some(at) => self.executor_back_at = Some(at),
                None => {
                    self.start_executor(death.reason);
                    super::checks::after_restart(self, death.reason);
                }
            }
        }
        self.absorb_passes();
    }
    /// Every fault that fired, the executor's and the harness's.
    pub fn faults(&self) -> BTreeMap<&'static str, u64> {
        let mut all = self.exec.faults().fired.clone();
        for (name, count) in &self.stats.faults {
            *all.entry(name).or_default() += count;
        }
        all
    }
}

use std::os::unix::fs::{DirBuilderExt, MetadataExt};
