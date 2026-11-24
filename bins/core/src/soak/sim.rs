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
    /// The execute answer is lost (the job is held), and then the first
    /// recovery lookup's answer too.
    LookupLost,
    /// Core restarts after recording an operator's decision, before the
    /// executor sees it: the retry loop completes it after startup.
    ResolutionInterrupted,
    /// The executor handles the execute, its answer is lost, and it comes
    /// back with its receipt store replaced: by an empty one, or (`older`)
    /// by a copy from an earlier night, as a restore from a backup or a lost
    /// disk would leave it.
    ReceiptStoreLost { older: bool },
    /// The execute answer is lost (the job is held), and the job is then
    /// held open (`workload::HeldOpen`): the executor refuses its recovery
    /// lookups (`Conflict`, its store busy) and its resolution (`Unavailable`)
    /// until the harness releases them.
    HeldOpen,
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
            Self::LookupLost => "execute and recovery lookup answers lost",
            Self::ResolutionInterrupted => "core restart between resolution request and executor",
            Self::ReceiptStoreLost { older: false } => {
                "execute answer lost, receipt store replaced by an empty one"
            }
            Self::ReceiptStoreLost { older: true } => {
                "execute answer lost, receipt store replaced by an older copy"
            }
            Self::HeldOpen => "execute answer lost, the job held open for its resolution",
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
    /// Queued by the storm's script.
    pub scripted: bool,
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
    /// Committed WAL frames the open replayed (crash images).
    pub replay: u64,
}

/// The reason of an orderly core restart.
pub const DAILY: &str = "daily restart";

/// Core's store files as a crash leaves them (`Sim::crash_image`).
pub struct CrashImage {
    pub db: Vec<u8>,
    pub wal: Option<Vec<u8>>,
}

/// `path` with `suffix` appended (`core.sqlite-wal`).
pub fn suffixed(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
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
    /// The storm's jobs whose progress poll showed them settled as refused
    /// by the effect budget (`executor_budget_spent`), with when.
    pub budget_refused_seen: Vec<i64>,
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
    /// Events job-row retention removed with the jobs (counted apart from
    /// the events rules').
    pub job_events: i64,
    /// The largest batch each rule removed in one transaction.
    pub largest_batch: BTreeMap<&'static str, i64>,
    pub dispatcher_passes: u64,
    /// Wall time spent per part of the run (for the runtime budget only).
    pub timing: BTreeMap<&'static str, std::time::Duration>,
    pub retention_passes: u64,
    pub reads: u64,
    /// Lost recovery-lookup answers checked against what core logged in the
    /// same dispatcher pass, and those that fell in a pass the executor died
    /// in (unchecked: a dead executor's refused connections log the same).
    pub lookups_checked: u64,
    pub lookups_unchecked: u64,
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
    /// The module that logged it (its log site, with the message).
    pub site: String,
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
            site: event.metadata().module_path().unwrap_or("?").to_owned(),
        });
    }
}

pub struct Sim {
    pub name: &'static str,
    pub clock: Clock,
    pub rng: Rng,
    /// The host operator's own decisions (`workload::operator_cancel`), drawn
    /// apart from `rng` so that adding them left the household's stream, and
    /// so its jobs, faults and timings, as they were.
    pub operator_rng: Rng,
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
    /// Every warning and error core logged: by site, message and code.
    pub logged: BTreeMap<(String, String, Option<String>), u64>,
    /// Of the last batch `absorb` judged: the dispatcher's "remains pending"
    /// warnings with the transport code (`checks::PENDING`), for container
    /// and for stack jobs.
    pub batch_pending_unavailable: (u64, u64),
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
    /// The flatness check's events model, b-tree by b-tree, for the report.
    pub flat_lines: Vec<String>,
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
    /// Jobs the writer thread's job-row retention reported removing.
    pub pruned: std::collections::BTreeSet<String>,
    /// Jobs startup recovery marked (`job_recovery_required`), as seen at
    /// the checkpoints.
    pub recovered: std::collections::BTreeSet<String>,
    /// Each job's state when the invariants last read it: what a job that
    /// retention removed since is judged by.
    pub last_state: BTreeMap<String, String>,
    /// Jobs core held (dispatched, unsettled) when the executor's receipt
    /// store changed (any executor start draws a new epoch; a replaced store
    /// has also lost receipts): none may be released on a closure alone.
    pub exposed: std::collections::HashSet<String>,
    /// Days on which the next container job queued gets its receipt store
    /// replaced (`JobFault::ReceiptStoreLost`), and whether by an older copy.
    pub replacements: Vec<(i64, bool)>,
    /// The job held open across retention passes with a resolution request
    /// open (`workload::release_held`), once armed.
    pub held: Option<String>,
    pub held_open: Option<(i64, i64)>,
    /// Until when the storm's script ignores job histories and keeps
    /// hammering containers the effect budget refused (`workload::busy`).
    pub careless_until: i64,
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
            operator_rng: Rng::new(seed ^ 0x6f70_6572_6174_6f72),
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
            batch_pending_unavailable: (0, 0),
            core_starts: Vec::new(),
            executor_starts: Vec::new(),
            next_pass: START,
            executor_back_at: None,
            samples: Vec::new(),
            projection: None,
            startup_projection: Vec::new(),
            measures: BTreeMap::new(),
            flat_lines: Vec::new(),
            idle: Vec::new(),
            idle_scheduled: 0,
            idle_skipped: Vec::new(),
            started: Instant::now(),
            core_restarts_until: core_restarts_until.map_or(i64::MAX, |day| START + day * DAY),
            receipts_reached: (false, false),
            last_watermark: i64::MIN,
            authority_seen: 0,
            pruned: Default::default(),
            recovered: Default::default(),
            last_state: Default::default(),
            exposed: Default::default(),
            replacements: Vec::new(),
            held: None,
            held_open: None,
            careless_until: i64::MIN,
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
            // Core's clock: its stack proposals judge the inventory cohort's
            // freshness on simulated time, as `propose_stack` does on the
            // system clock.
            stacks: limeos_observations::stacks::Cache::with_clock(|| {
                (crate::now(), crate::monotonic())
            }),
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
            replay: 0,
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
    /// Core restarts. A daily restart is orderly (the writer checkpoints and
    /// truncates its WAL as it stops); any other restart is a crash: core
    /// reopens the store from its files as they were at that moment, the
    /// database with its WAL (and no wal-index), so its startup replays
    /// every frame committed since the last checkpoint, as after a kill.
    pub async fn restart_core(&mut self, reason: &'static str) {
        assert!(
            self.core_restarts(),
            "[{}] {}: core restarted ({reason}) after its restarts ended",
            self.name,
            self.when()
        );
        self.absorb_passes();
        super::checks::core_accounting(self, reason);
        let image = (reason != DAILY).then(|| self.crash_image());
        if let Some(image) = &image {
            *self
                .stats
                .faults
                .entry("core crash with its WAL as it was")
                .or_default() += 1;
            if image
                .wal
                .as_deref()
                .is_some_and(|w| super::startup::committed_frames(w) > 0)
            {
                *self
                    .stats
                    .faults
                    .entry("of them: with committed frames to replay")
                    .or_default() += 1;
            }
        }
        self.stop_core().await;
        if let Some(image) = image {
            self.restore(image);
        }
        self.start_core(reason).await;
        super::checks::after_restart(self, reason);
    }
    /// Core's store files as a crash would leave them now: the writer idle
    /// (every call it was given has committed or rolled back), the database
    /// and its WAL as they are on disk. The wal-index (`-shm`) is shared
    /// memory a crash loses; it is not kept.
    pub fn crash_image(&mut self) -> CrashImage {
        self.clock.sim().settle();
        let read = |path: &std::path::Path| std::fs::read(path).ok();
        let db = read(&self.core_path).expect("core.sqlite");
        let wal = read(&suffixed(&self.core_path, "-wal"));
        CrashImage { db, wal }
    }
    /// Puts `image` in place of core's store files (core stopped).
    fn restore(&mut self, image: CrashImage) {
        self.side = None;
        std::fs::write(&self.core_path, &image.db).expect("core.sqlite from the crash image");
        let wal = suffixed(&self.core_path, "-wal");
        match &image.wal {
            Some(bytes) => std::fs::write(&wal, bytes).expect("its WAL from the crash image"),
            None => {
                let _ = std::fs::remove_file(&wal);
            }
        }
        let _ = std::fs::remove_file(suffixed(&self.core_path, "-shm"));
    }
    pub fn start_executor(&mut self, reason: &'static str) {
        // A new open is a new store epoch: every job core holds now was
        // dispatched under an earlier one.
        if self.core.is_some() {
            let held: Vec<String> = self
                .side()
                .prepare("SELECT id FROM jobs WHERE state IN ('running','verifying','outcome_unknown','needs_intervention')")
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .unwrap_or_default();
            self.exposed.extend(held);
        }
        let now = self.now();
        let (file, wal) = sizes(&self.exec.dir.join("receipts.sqlite"));
        match self.exec.start() {
            Ok(opened) => self.executor_starts.push(Startup {
                at: now,
                reason,
                file,
                wal,
                opened,
                trace: None,
                replay: 0,
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
            self.stats.job_events += pass.job_events;
            events += pass.job_events;
            total += pass.job_events;
            self.pruned.extend(pass.pruned_jobs);
            self.stats.peak_events = self.stats.peak_events.max(events);
        }
        total
    }
    /// After core's work: what it logged and what the executor saw, judged;
    /// an executor that died is restarted (or comes back later).
    pub async fn absorb(&mut self) {
        let logged = std::mem::take(&mut *self.log.lock().unwrap());
        self.batch_pending_unavailable = (0, 0);
        for entry in logged {
            let code = entry.code.as_deref();
            if code == Some("Unavailable") {
                if entry.message == super::checks::PENDING[0] {
                    self.batch_pending_unavailable.0 += 1;
                } else if entry.message == super::checks::PENDING[1] {
                    self.batch_pending_unavailable.1 += 1;
                }
            }
            let lifetime = code.is_some_and(|c| LIFETIME.iter().any(|l| format!("{l:?}") == c));
            if lifetime {
                self.violation(
                    super::checks::REFUSALS,
                    format!(
                        "core logged a lifetime refusal: {} ({code:?}, {})",
                        entry.message, entry.site
                    ),
                );
            } else if super::checks::expected_log(&entry).is_none() {
                let level = if entry.error { "error" } else { "warning" };
                self.violation(
                    super::checks::LOGS,
                    format!(
                        "core logged an unexpected {level}: {} ({code:?}, {})",
                        entry.message, entry.site
                    ),
                );
            }
            *self
                .logged
                .entry((entry.site, entry.message, entry.code))
                .or_default() += 1;
        }
        let violations = std::mem::take(&mut self.exec.faults().violations);
        for violation in violations {
            self.violation(super::checks::REFUSALS, violation);
        }
        if let Some(death) = self.exec.take_death() {
            super::checks::receipt_accounting(self, death.reason);
            self.exec.stop().await;
            if let Some(older) = death.replace {
                self.replace_receipts(older);
            }
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
    /// The stopped executor's receipt store replaced: by an empty one, or
    /// (`older`) by the copy of an earlier night (`snapshot_receipts`), if
    /// there is one. What retention promised of the old store no longer
    /// binds the new one, so the accounting starts again.
    fn replace_receipts(&mut self, older: bool) {
        let path = self.exec.dir.join("receipts.sqlite");
        for file in [
            path.clone(),
            suffixed(&path, "-wal"),
            suffixed(&path, "-shm"),
        ] {
            let _ = std::fs::remove_file(file);
        }
        let snapshot = self.older_receipts();
        if older && snapshot.exists() {
            std::fs::copy(&snapshot, &path).expect("the older receipt store");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        self.receipts_reached = (false, false);
        self.last_watermark = i64::MIN;
    }
    fn older_receipts(&self) -> PathBuf {
        self._dir.path().join("receipts-older.sqlite")
    }
    /// At a checkpoint: a consistent copy of the receipt store, kept for an
    /// older-copy replacement, when no action in it is active (an active row
    /// is a lock a restore would bring back for a job core has settled).
    pub fn snapshot_receipts(&mut self) {
        let path = self.exec.dir.join("receipts.sqlite");
        let Ok(conn) = super::checks::reader(&path) else {
            return;
        };
        let active: i64 = conn
            .query_row(
                "SELECT (SELECT count(*) FROM actions WHERE state IN ('prepared','effect_accepted','outcome_unknown'))+(SELECT count(*) FROM stack_actions WHERE state IN ('prepared','effect_accepted','outcome_unknown'))",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1);
        drop(conn);
        if active == 0 {
            let snapshot = self.older_receipts();
            let _ = std::fs::remove_file(&snapshot);
            super::startup::copy(&path, &snapshot);
        }
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

use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
