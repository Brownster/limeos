//! The simulation's state: both stores, the Engine, the clock, the workload's
//! users and flows, the ledger of every job, and how every result is judged.
use crate::{
    engine::World,
    executor::{Executor, Opened},
    support::{Clock, DAY, HOUR, Io, MINUTE, Rng, START},
};
use limeos_contracts::VERSION;
use limeos_domain::{
    ContainerAction, DeclaredOutcome, Error, ErrorCode, PlannedContainerAction, PlannedStackAction,
    Principal, Result,
};
use limeos_executor_protocol::{Receipt, Request};
use limeos_persistence::{Store, test_support};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

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

/// Least time between two receipt lookups for one held job (core's
/// `operations::RECOVERY_PAUSE`).
pub const RECOVERY_PAUSE: i64 = 30;

/// Core's `operations::Rotation` on the simulated clock: fair paging through
/// held jobs, each looked up at most once per `RECOVERY_PAUSE`.
#[derive(Default)]
pub struct Rotation {
    cursor: String,
    polled: HashMap<String, i64>,
}
impl Rotation {
    pub fn cursor(&self) -> String {
        self.cursor.clone()
    }
    pub fn advance<'a>(&mut self, held: impl IntoIterator<Item = &'a str>, now: i64) {
        let held: Vec<&str> = held.into_iter().collect();
        self.cursor = match held.last() {
            Some(last) if held.len() >= limeos_persistence::RECOVERY_BATCH => (*last).to_owned(),
            _ => String::new(),
        };
        self.polled.retain(|_, at| now - *at < RECOVERY_PAUSE);
    }
    pub fn due(&mut self, id: &str, now: i64) -> bool {
        if self
            .polled
            .get(id)
            .is_some_and(|at| now - *at < RECOVERY_PAUSE)
        {
            return false;
        }
        self.polled.insert(id.to_owned(), now);
        true
    }
}

/// What goes wrong around one job's dispatch, decided when it is queued.
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
    /// Core restarts after the execute answer, before marking `verifying`.
    CoreCrashAfterExecute,
    /// Core restarts after the executor verified, before core records it.
    CoreCrashAfterVerify,
    /// The first recovery lookup's answer is lost.
    LookupLost,
}
impl JobFault {
    /// A simulated core restart.
    pub fn restarts_core(self) -> bool {
        matches!(
            self,
            Self::CoreCrashAfterClaim | Self::CoreCrashAfterExecute | Self::CoreCrashAfterVerify
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
        }
    }
}

/// Raised inside a mirrored core path: an error (logged by core, which moves
/// on) or a simulated core crash at that exact point.
pub enum Halt {
    /// An error at a named step (`at`), or anywhere else in the job's
    /// dispatch (an empty step).
    Error(&'static str, Error),
    CoreCrash,
}
impl From<Error> for Halt {
    fn from(error: Error) -> Self {
        Self::Error("", error)
    }
}
/// Names the step an error came from, for the report.
pub fn at(step: &'static str) -> impl Fn(Error) -> Halt {
    move |error| Halt::Error(step, error)
}

/// A failed executor call and whether the request may have reached it
/// (core's `operations::Failure`).
pub struct Failure {
    pub error: Error,
    pub sent: bool,
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
    pub fate: crate::workload::Fate,
}

/// A store open, measured, with the simulated time it happened.
#[derive(Clone, Copy, Debug)]
pub struct Startup {
    pub at: i64,
    pub reason: &'static str,
    /// The database file and its WAL when the open began.
    pub file: u64,
    pub wal: u64,
    pub opened: Opened,
    /// For an open on a copy of core's store: its startup statements cold.
    pub probe: Option<crate::startup::Probe>,
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
    pub ops: BTreeMap<&'static str, u64>,
    /// Legitimate refusals by operation and code.
    pub refused: BTreeMap<(&'static str, String), u64>,
    /// Failed checks as they happen: the check's name, then when and what.
    pub violations: Vec<(&'static str, String)>,
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
}

impl Stats {
    pub fn time(&mut self, part: &'static str, since: Instant) {
        *self.timing.entry(part).or_default() += since.elapsed();
    }
}

pub struct Sim {
    pub name: &'static str,
    pub clock: Clock,
    pub rng: Rng,
    /// Holds both stores; removed with the run.
    pub _dir: tempfile::TempDir,
    pub core_path: PathBuf,
    pub store: Option<Store>,
    pub exec: Executor,
    pub world: Arc<World>,
    pub rotation: Rotation,
    pub stack_rotation: Rotation,
    pub resolution_rotation: Rotation,
    pub users: Vec<User>,
    /// A session kept past its expiry, to provoke its refusal.
    pub stale: Option<Session>,
    pub flows: BTreeMap<u64, Flow>,
    pub next_flow: u64,
    pub ledger: BTreeMap<String, JobRecord>,
    pub armed: HashMap<String, JobFault>,
    /// Operator attempts per job, so a refused declaration is not repeated.
    pub attempts: HashMap<String, u32>,
    pub stats: Stats,
    pub core_starts: Vec<Startup>,
    pub executor_starts: Vec<Startup>,
    pub last_retention: i64,
    /// When the next dispatcher pass is due.
    pub next_pass: i64,
    /// Queued and held jobs the last pass listed.
    pub seen_queued: usize,
    pub seen_held: usize,
    /// When a stopped executor comes back.
    pub executor_back_at: Option<i64>,
    pub samples: Vec<crate::checks::Sample>,
    /// Per-job growth over the last third (bytes a day) and the years until
    /// core's file reaches its bound at that rate.
    pub projection: Option<(f64, f64)>,
    /// Per store: startup reads added a day over the last third, and the
    /// years until an open reads its bound at that rate.
    pub startup_projection: Vec<(&'static str, f64, f64)>,
    /// What the trend and startup checks measured, by name: what the pins of
    /// `checks::EXPECTED_FAILURES` are compared with.
    pub measures: BTreeMap<&'static str, f64>,
    /// Idle windows measured: (simulated start, thread I/O over the window).
    pub idle: Vec<(i64, Io)>,
    /// Idle windows scheduled, and those skipped with the reason.
    pub idle_scheduled: u64,
    pub idle_skipped: Vec<(i64, String)>,
    pub started: Instant,
    /// The instant that stands for `START` (see `instant`).
    pub epoch: Instant,
    /// Core's store restarts only before this (simulated) time; after it
    /// the store stays open and startup is measured on copies.
    pub core_restarts_until: i64,
    /// Whether standalone and stack receipts have reached what retention
    /// keeps, and the highest prune watermark seen.
    pub receipts_reached: (bool, bool),
    pub last_watermark: i64,
}

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
        let world = Arc::new(World::new(clock.clone()));
        let dir = soak_dir();
        let executor_dir = dir.path().join("executor");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&executor_dir)
            .unwrap();
        let uid = std::fs::metadata("/proc/self").unwrap().uid();
        let exec = Executor::new(&executor_dir, world.clone(), uid);
        let core_path = dir.path().join("core").join("core.sqlite");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(core_path.parent().unwrap())
            .unwrap();
        Self {
            name,
            clock,
            rng: Rng::new(seed),
            _dir: dir,
            core_path,
            store: None,
            exec,
            world,
            rotation: Rotation::default(),
            stack_rotation: Rotation::default(),
            resolution_rotation: Rotation::default(),
            users: Vec::new(),
            stale: None,
            flows: BTreeMap::new(),
            next_flow: 0,
            ledger: BTreeMap::new(),
            armed: HashMap::new(),
            attempts: HashMap::new(),
            stats: Stats::default(),
            core_starts: Vec::new(),
            executor_starts: Vec::new(),
            last_retention: i64::MIN,
            next_pass: START,
            seen_queued: 0,
            seen_held: 0,
            executor_back_at: None,
            samples: Vec::new(),
            projection: None,
            startup_projection: Vec::new(),
            measures: BTreeMap::new(),
            idle: Vec::new(),
            idle_scheduled: 0,
            idle_skipped: Vec::new(),
            started: Instant::now(),
            epoch: Instant::now(),
            core_restarts_until: core_restarts_until.map_or(i64::MAX, |day| START + day * DAY),
            receipts_reached: (false, false),
            last_watermark: i64::MIN,
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
    pub fn db(&mut self) -> &mut Store {
        self.store.as_mut().expect("core is running")
    }

    /// Judges one result: success, a refusal the workload provoked or the
    /// system rightly gives (`legit`), or a violation. A lifetime refusal is
    /// a violation wherever it comes from.
    pub fn judge<T>(
        &mut self,
        op: &'static str,
        result: Result<T>,
        legit: &[ErrorCode],
    ) -> Option<T> {
        match result {
            Ok(value) => {
                *self.stats.ops.entry(op).or_default() += 1;
                Some(value)
            }
            Err(Error(code)) if legit.contains(&code) && !LIFETIME.contains(&code) => {
                *self
                    .stats
                    .refused
                    .entry((op, format!("{code:?}")))
                    .or_default() += 1;
                None
            }
            Err(Error(code)) => {
                let kind = if LIFETIME.contains(&code) {
                    "lifetime refusal"
                } else {
                    "unexpected refusal"
                };
                self.violation(crate::checks::REFUSALS, format!("{op}: {kind} {code:?}"));
                None
            }
        }
    }
    pub fn violation(&mut self, check: &'static str, what: String) {
        let line = format!("{}: {what}", self.when());
        eprintln!("[{}] {check}: {line}", self.name);
        self.stats.violations.push((check, line));
    }

    /// Core startup: `Database::open`'s `Store::open` (schema, quick_check,
    /// every startup invariant check, audit reconciliation, recovery of
    /// interrupted jobs), measured, then the writer thread's first retention
    /// pass. Fresh in-memory state: core's rotations do not survive it.
    pub fn start_core(&mut self, reason: &'static str) {
        assert!(self.store.is_none());
        let (file, wal) = sizes(&self.core_path);
        let (io, at) = (Io::now(), Instant::now());
        let opened = Store::open(&self.core_path);
        let opened_at = Opened {
            io: Io::now().since(io),
            elapsed: at.elapsed(),
        };
        match opened {
            Ok(store) => self.store = Some(store),
            Err(Error(code)) => panic!(
                "[{}] {}: core store refused to open ({reason}): {code:?}",
                self.name,
                self.when()
            ),
        }
        *self.stats.restarts.entry(reason).or_default() += 1;
        self.core_starts.push(Startup {
            at: self.now(),
            reason,
            file,
            wal,
            opened: opened_at,
            probe: None,
        });
        self.rotation = Rotation::default();
        self.stack_rotation = Rotation::default();
        self.resolution_rotation = Rotation::default();
        self.retention_pass();
        // The dispatcher's first tick is immediate: startup leftovers.
        self.next_pass = self.now();
    }
    /// Whether core may restart now (the main run keeps its store open from
    /// day 30, so a drifting counter is not recomputed away by `Store::open`).
    pub fn core_restarts(&self) -> bool {
        self.now() < self.core_restarts_until
    }
    pub fn restart_core(&mut self, reason: &'static str) {
        assert!(
            self.core_restarts(),
            "[{}] {}: core restarted ({reason}) after its restarts ended",
            self.name,
            self.when()
        );
        crate::checks::core_accounting(self, reason);
        self.store = None;
        self.start_core(reason);
        crate::checks::after_restart(self, reason);
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
                probe: None,
            }),
            Err(Error(code)) => panic!(
                "[{}] {}: receipt store refused to open ({reason}): {code:?}",
                self.name,
                self.when()
            ),
        }
        *self.stats.restarts.entry(reason).or_default() += 1;
    }
    pub fn restart_executor(&mut self, reason: &'static str) {
        crate::checks::receipt_accounting(self, reason);
        self.exec.stop();
        self.start_executor(reason);
        crate::checks::after_restart(self, reason);
    }

    /// The instant the writer thread would read at simulated time `at`: the
    /// gate of `maintain` compares instants only with each other.
    pub fn instant(&self, at: i64) -> Instant {
        self.epoch + std::time::Duration::from_secs((at - START) as u64)
    }

    /// One retention pass of the core store: the writer thread's own
    /// `maintain`, with its once-a-minute gate, on the simulated clock. A
    /// failed rule is a violation like any refusal, and every batch must stay
    /// within its rule's limit. Returns the rows it removed.
    pub fn retention_pass(&mut self) -> i64 {
        let events: i64 = self.count_events();
        self.stats.peak_events = self.stats.peak_events.max(events);
        let now = self.now();
        let at = self.instant(now);
        self.last_retention = now;
        let Some(pass) = test_support::maintain(self.db(), at) else {
            return 0;
        };
        self.stats.retention_passes += 1;
        for (rule, code) in pass.failed {
            self.violation(
                crate::checks::REFUSALS,
                format!("retention pass: the {rule} batch failed with {code:?}"),
            );
        }
        let mut total = 0;
        for (rule, rows) in pass.removed {
            let limit = rule_limits(rule).1;
            if rows > limit {
                self.violation(
                    crate::checks::BATCHES,
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
            total += rows;
        }
        total
    }
    fn count_events(&mut self) -> i64 {
        self.db()
            .audit_usage()
            .map(|usage| usage.bytes / test_support::EVENT_BYTES)
            .unwrap_or_default()
    }

    /// Core's `operations::exchange`: one executor call over the socket,
    /// with `fault` deciding what the transport does with it. The executor
    /// being down is a refused connection (`sent: false`).
    pub async fn exchange(
        &mut self,
        request: Request,
        fault: Option<JobFault>,
    ) -> std::result::Result<Receipt, Failure> {
        let now = self.now();
        if !self.exec.up() {
            return Err(Failure {
                error: Error(ErrorCode::Unavailable),
                sent: false,
            });
        }
        match fault {
            Some(JobFault::LostBeforeStore) => {
                self.count_fault(JobFault::LostBeforeStore);
                self.restart_executor("executor crash with an execute unread");
                return Err(Failure {
                    error: Error(ErrorCode::Unavailable),
                    sent: true,
                });
            }
            Some(JobFault::ExecutorKilledInEngineCall) => {
                self.count_fault(JobFault::ExecutorKilledInEngineCall);
                crate::checks::receipt_accounting(self, "executor killed inside the Engine call");
                let before = self.receipt_rows();
                let parked = self.exec.kill_inside_engine_call(request, now).await;
                if parked && self.receipt_rows() <= before {
                    *self
                        .stats
                        .faults
                        .entry("of them: killed after a pruning insert")
                        .or_default() += 1;
                }
                self.start_executor("executor killed inside the Engine call");
                crate::checks::after_restart(self, "executor killed inside the Engine call");
                return Err(Failure {
                    error: Error(ErrorCode::Unavailable),
                    sent: true,
                });
            }
            Some(JobFault::EngineFailure { applied }) => {
                self.count_fault(JobFault::EngineFailure { applied });
                self.world.arm(crate::engine::Fault::Fail { applied });
            }
            _ => {}
        }
        let response = self.exec.handle(request, now).await;
        self.world.arm(crate::engine::Fault::None);
        if matches!(fault, Some(JobFault::AnswerLost | JobFault::LookupLost)) {
            self.count_fault(fault.unwrap());
            return Err(Failure {
                error: Error(ErrorCode::Unavailable),
                sent: true,
            });
        }
        let failed = |error| Failure { error, sent: true };
        if response.version != VERSION {
            return Err(failed(Error(ErrorCode::Unavailable)));
        }
        if let Some(error) = response.error {
            if LIFETIME.contains(&error) {
                self.violation(
                    crate::checks::REFUSALS,
                    format!("executor: lifetime refusal {error:?}"),
                );
            }
            return Err(failed(Error(error)));
        }
        if !response.ready {
            return Err(failed(Error(ErrorCode::Unavailable)));
        }
        if let Some(epoch) = response.epoch.clone() {
            self.db()
                .observe_executor_epoch(&epoch, now)
                .map_err(failed)?;
        }
        Ok(response)
    }
    /// Core's `operations::executor`.
    pub async fn executor(&mut self, request: Request) -> Result<Receipt> {
        self.exchange(request, None).await.map_err(|f| f.error)
    }
    pub fn count_fault(&mut self, fault: JobFault) {
        *self.stats.faults.entry(fault.name()).or_default() += 1;
    }
    /// Rows in the receipt store (both tables), read on a side connection.
    pub fn receipt_rows(&self) -> i64 {
        let path = self.exec.dir.join("receipts.sqlite");
        let Ok(conn) = crate::checks::reader(&path) else {
            return 0;
        };
        conn.query_row(
            "SELECT (SELECT count(*) FROM actions)+(SELECT count(*) FROM stack_actions)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }
}

use std::os::unix::fs::{DirBuilderExt, MetadataExt};
