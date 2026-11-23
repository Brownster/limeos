//! The container executor as a process. The real `ReceiptStore`, opened as
//! the daemon opens it (`open`, which reads no clock), answers every
//! request through the daemon's own request handling: the ceiling's
//! `validate`, then `limeos_executor::container_requests::answer` (the store
//! behind its `try_lock`, the Engine behind the `Engine` trait) and its
//! `refusal` answer. It is served on a real Unix socket with the protocol's
//! framing, so core's real dispatcher, recovery and resolution code reaches
//! it exactly as it reaches the daemon. The harness adds only the accept
//! loop (the daemon's peer check is replaced by the ceiling's own) and the
//! faults, decided per request from the job it names:
//!
//! - an answer lost (handled, then the connection closes unanswered): an
//!   execute's, or an execute's and then the first recovery lookup's;
//! - the process dying with a request unread, inside the Engine call (the
//!   `prepared` row committed), or between answering `ReceiptEpoch` and the
//!   execute (the socket goes, so core's connect is refused);
//! - the Engine failing, before or after acting;
//! - core dying around a request: before the execute is sent, after its
//!   answer, after the verify, before a resolution reaches the executor. The
//!   executor signals [`Crash`] before it would answer and closes the
//!   connection; the harness, which polls that signal ahead of core's own
//!   work, drops core's work at exactly that point and restarts core.
use super::{
    engine::{Docker, Fault, HOST, STACKS, World, container_id},
    sim::{JobFault, LIFETIME},
    support::{Clock, Io},
};
use limeos_contracts::VERSION;
use limeos_domain::{Error, Result};
use limeos_executor::container_requests::{ContainerRole, answer, refusal};
use limeos_executor_container::{ContainerCeiling, ReceiptStore};
use limeos_executor_protocol::{Ceiling, Request};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::Notify,
    task::JoinHandle,
};

/// One store open: what it read and how long it took.
#[derive(Clone, Copy, Debug)]
pub struct Opened {
    pub io: Io,
    pub elapsed: Duration,
}

/// Core's death at a point only the executor sees, signalled before the
/// executor would answer (or instead of answering).
#[derive(Default)]
pub struct Crash {
    flag: AtomicBool,
    notify: Notify,
}
impl Crash {
    fn signal(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
    /// Ready once signalled.
    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.flag.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
    pub fn clear(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// The executor process died on its own (a fault); the harness restarts it,
/// at once or at `back_at`.
#[derive(Clone, Copy, Debug)]
pub struct Death {
    pub reason: &'static str,
    pub back_at: Option<i64>,
    /// Its receipt store is replaced before it starts again: `Some(older)`.
    pub replace: Option<bool>,
}

/// The faults armed per job, and what the executor saw and did.
#[derive(Default)]
pub struct Faults {
    pub armed: HashMap<String, JobFault>,
    /// The resource each armed job's dispatch inspects (`container:…`, or
    /// `stack:<project>`), so the `ReceiptEpoch` that follows its inspection
    /// can be told apart.
    pub resources: HashMap<String, String>,
    last_inspected: Option<String>,
    /// Whether core may restart now: a core-death fault due after core's
    /// restarts ended is dropped.
    pub core_restarts: bool,
    /// Faults that fired, by name.
    pub fired: BTreeMap<&'static str, u64>,
    /// Engine effects each execute caused, by action, over the whole run.
    pub effects: HashMap<String, u64>,
    /// Lifetime refusals the executor answered.
    pub violations: Vec<String>,
    /// Receipt lookups whose absence was left open (or that did not ask to
    /// close one): recovery's closure missing.
    pub unclosed: Vec<String>,
    /// Absent actions closed for a recovery lookup.
    pub closures: u64,
    /// Connections closed with the request read and no answer written: the
    /// answer lost, or the process dead with the request unread or inside
    /// the Engine (core's own deaths drop core's side first).
    pub dropped: u64,
    /// The same, by the kind of request whose answer core never got.
    pub lost: BTreeMap<&'static str, u64>,
    /// Requests answered with a refusal, by kind and code.
    pub refused: BTreeMap<(&'static str, String), u64>,
    /// Requests served, by kind.
    pub requests: BTreeMap<&'static str, u64>,
    /// Times the process died (or stopped) on its own.
    pub deaths: u64,
    /// Jobs whose recovery lookup the executor answered with a closure.
    pub closed_jobs: std::collections::HashSet<String>,
    /// Jobs whose recovery lookups it refuses (`Conflict`), and whose
    /// resolution it refuses (`Unavailable`), while held open.
    pub held_lookups: std::collections::HashSet<String>,
    pub held_resolutions: std::collections::HashSet<String>,
    died: Option<Death>,
}

/// What to do with a request before handling it.
enum Before {
    Handle,
    /// The process dies with it unread.
    DieUnread,
    /// Core dies first: the request is never handled.
    CoreDies,
    /// Refused with this code, unhandled.
    Refuse(limeos_domain::ErrorCode),
    /// The next Engine effect fails (or hangs: the process dies inside it).
    Engine(Fault),
}
/// What to do with the answer.
enum After {
    Write,
    /// Handled; the answer never reaches core.
    Lose,
    /// Handled; core dies before it reads the answer.
    CoreDies,
    /// Answered; then the process stops until `back_at`.
    StopUntil(i64),
    /// Handled; the answer is lost and the process dies, to come back with
    /// its receipt store replaced (`Some(older)`).
    LoseAndReplace(bool),
}

fn kind(request: &Request) -> &'static str {
    match request {
        Request::Inspect { .. } => "inspect",
        Request::InspectStack { .. } => "inspect stack",
        Request::ReceiptEpoch { .. } => "receipt epoch",
        Request::ExecuteContainer { .. } => "execute",
        Request::ExecuteStack { .. } => "execute stack",
        Request::RestartReceipt { .. } => "receipt lookup",
        Request::StackReceipt { .. } => "stack receipt lookup",
        Request::VerifyRestart { .. } => "verify",
        Request::VerifyStack { .. } => "verify stack",
        Request::ResolveAction { .. } => "resolve",
        _ => "other",
    }
}

impl Faults {
    fn take(&mut self, action: &str, wanted: impl Fn(JobFault) -> bool) -> Option<JobFault> {
        let fault = *self.armed.get(action)?;
        if !wanted(fault) {
            return None;
        }
        self.armed.remove(action);
        Some(fault)
    }
    fn fired(&mut self, fault: JobFault) {
        *self.fired.entry(fault.name()).or_default() += 1;
    }
    /// A core-death fault fires only while core may restart.
    fn core_dies(&mut self, fault: JobFault) -> bool {
        if self.core_restarts {
            self.fired(fault);
        }
        self.core_restarts
    }
    fn decide(&mut self, request: &Request, now: i64) -> (Before, After) {
        *self.requests.entry(kind(request)).or_default() += 1;
        let (mut before, mut after) = (Before::Handle, After::Write);
        match request {
            Request::Inspect { resource, .. } => self.last_inspected = Some(resource.clone()),
            Request::InspectStack { project, .. } => {
                self.last_inspected = Some(format!("stack:{project}"));
            }
            Request::ReceiptEpoch { .. } => {
                let inspected = self.last_inspected.take();
                let job = self
                    .armed
                    .iter()
                    .find(|(job, fault)| {
                        **fault == JobFault::ExecutorDownAfterEpoch
                            && self.resources.get(*job) == inspected.as_ref()
                    })
                    .map(|(job, _)| job.clone());
                if let Some(job) = job {
                    self.armed.remove(&job);
                    self.fired(JobFault::ExecutorDownAfterEpoch);
                    after = After::StopUntil(now + 20);
                }
            }
            Request::ExecuteContainer { request, .. } | Request::Restart { request, .. } => {
                (before, after) = self.execute(&request.action);
            }
            Request::ExecuteStack { request, .. } => {
                (before, after) = self.execute(&request.action)
            }
            Request::VerifyRestart { action, .. } | Request::VerifyStack { action, .. } => {
                if let Some(fault) = self.take(action, |f| f == JobFault::CoreCrashAfterVerify)
                    && self.core_dies(fault)
                {
                    after = After::CoreDies;
                }
            }
            Request::RestartReceipt { action, .. } | Request::StackReceipt { action, .. } => {
                if self.held_lookups.contains(action) {
                    before = Before::Refuse(limeos_domain::ErrorCode::Conflict);
                } else if let Some(fault) = self.take(action, |f| f == JobFault::LookupLost) {
                    self.fired(fault);
                    after = After::Lose;
                }
            }
            Request::ResolveAction { action, .. } => {
                if let Some(fault) = self.take(action, |f| f == JobFault::ResolutionInterrupted)
                    && self.core_dies(fault)
                {
                    before = Before::CoreDies;
                } else if self.held_resolutions.contains(action) {
                    before = Before::Refuse(limeos_domain::ErrorCode::Unavailable);
                }
            }
            _ => {}
        }
        (before, after)
    }
    fn execute(&mut self, action: &str) -> (Before, After) {
        // Left armed for the first recovery lookup, which loses its answer too.
        if self.armed.get(action) == Some(&JobFault::LookupLost) {
            return (Before::Handle, After::Lose);
        }
        let Some(fault) = self.take(action, |f| {
            matches!(
                f,
                JobFault::AnswerLost
                    | JobFault::LostBeforeStore
                    | JobFault::ExecutorKilledInEngineCall
                    | JobFault::EngineFailure { .. }
                    | JobFault::CoreCrashAfterClaim
                    | JobFault::CoreCrashAfterExecute
                    | JobFault::ReceiptStoreLost { .. }
                    | JobFault::HeldOpen
            )
        }) else {
            return (Before::Handle, After::Write);
        };
        match fault {
            JobFault::AnswerLost => {
                self.fired(fault);
                (Before::Handle, After::Lose)
            }
            JobFault::ReceiptStoreLost { older } => {
                self.fired(fault);
                (Before::Handle, After::LoseAndReplace(older))
            }
            JobFault::HeldOpen => {
                self.fired(fault);
                self.held_lookups.insert(action.to_owned());
                self.held_resolutions.insert(action.to_owned());
                (Before::Handle, After::Lose)
            }
            JobFault::LostBeforeStore => {
                self.fired(fault);
                (Before::DieUnread, After::Write)
            }
            JobFault::ExecutorKilledInEngineCall => {
                self.fired(fault);
                (Before::Engine(Fault::Hang), After::Write)
            }
            JobFault::EngineFailure { applied } => {
                self.fired(fault);
                (Before::Engine(Fault::Fail { applied }), After::Write)
            }
            JobFault::CoreCrashAfterClaim if self.core_dies(fault) => {
                (Before::CoreDies, After::Write)
            }
            JobFault::CoreCrashAfterExecute if self.core_dies(fault) => {
                (Before::Handle, After::CoreDies)
            }
            // Armed before core's restarts ended, due after: dropped.
            _ => (Before::Handle, After::Write),
        }
    }
}

/// One running executor process.
struct Serving {
    role: ContainerRole<Docker>,
    ceiling: Arc<Ceiling>,
    restart_ceiling: Arc<ContainerCeiling>,
    world: Arc<World>,
    clock: Clock,
    faults: Arc<Mutex<Faults>>,
    crash: Arc<Crash>,
    socket: PathBuf,
    dir: PathBuf,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    accept: Mutex<Option<tokio::task::AbortHandle>>,
}
impl Serving {
    fn faults(&self) -> std::sync::MutexGuard<'_, Faults> {
        self.faults.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The process dies: it accepts nothing more and its socket goes, so a
    /// connect is refused (the request never leaves core).
    fn die(&self, reason: &'static str, back_at: Option<i64>) {
        self.die_replacing(reason, back_at, None);
    }
    fn die_replacing(&self, reason: &'static str, back_at: Option<i64>, replace: Option<bool>) {
        {
            let mut faults = self.faults();
            faults.died = Some(Death {
                reason,
                back_at,
                replace,
            });
            faults.deaths += 1;
        }
        if let Some(accept) = self.accept.lock().unwrap().take() {
            accept.abort();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn accept(serving: Arc<Serving>, listener: UnixListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let task = tokio::spawn(connection(serving.clone(), stream));
        serving.tasks.lock().unwrap().push(task);
    }
}

/// One request, as the daemon serves it, with the fault armed for it.
async fn connection(serving: Arc<Serving>, mut stream: UnixStream) {
    let Ok(peer) = stream.peer_cred() else {
        return;
    };
    let Ok(Ok(request)) = tokio::time::timeout(
        limeos_contracts::RPC_DEADLINE,
        limeos_contracts::read_frame::<Request>(&mut stream),
    )
    .await
    else {
        return;
    };
    let now = serving.clock.now();
    let what = kind(&request);
    let lost = |faults: &mut Faults| {
        faults.dropped += 1;
        *faults.lost.entry(what).or_default() += 1;
    };
    let (before, after) = serving.faults().decide(&request, now);
    let world = &serving.world;
    let hang = match before {
        Before::DieUnread => {
            lost(&mut serving.faults());
            serving.die("executor crash with an execute unread", None);
            return;
        }
        Before::CoreDies => {
            serving.crash.signal();
            return;
        }
        Before::Engine(fault) => {
            world.arm(fault);
            fault == Fault::Hang
        }
        Before::Refuse(code) => {
            *serving
                .faults()
                .refused
                .entry((what, format!("{code:?}")))
                .or_default() += 1;
            let _ = tokio::time::timeout(
                limeos_contracts::RPC_DEADLINE,
                limeos_contracts::write_frame(&mut stream, &refusal(limeos_domain::Error(code))),
            )
            .await;
            return;
        }
        Before::Handle => false,
    };
    let action = match &request {
        Request::ExecuteContainer { request, .. } => Some(request.action.clone()),
        Request::ExecuteStack { request, .. } => Some(request.action.clone()),
        _ => None,
    };
    // A recovery lookup, and whether core asked to close an absent action.
    let lookup = match &request {
        Request::RestartReceipt {
            action,
            close_if_absent,
            ..
        }
        | Request::StackReceipt {
            action,
            close_if_absent,
            ..
        } => Some((action.clone(), *close_if_absent)),
        _ => None,
    };
    let effects = world.effects();
    let rows = hang.then(|| receipt_rows(&serving.dir));
    let clock = serving.clock.clone();
    let read_clock = move || clock.now();
    let response = match serving.ceiling.validate(peer.uid(), &request) {
        Err(error) => refusal(error),
        Ok(mut receipt) => {
            let outcome = {
                let handled = answer(
                    &serving.role,
                    &serving.restart_ceiling,
                    peer.uid(),
                    request,
                    &mut receipt,
                    &read_clock,
                );
                tokio::pin!(handled);
                if hang {
                    tokio::select! {
                        biased;
                        result = &mut handled => Some(result),
                        () = world.hung.notified() => None,
                    }
                } else {
                    Some(handled.await)
                }
            };
            match outcome {
                Some(Ok(())) => receipt,
                Some(Err(error)) => refusal(error),
                None => {
                    // Parked inside the Engine call, its `prepared` row (and
                    // any pruning that insert did) committed: the process is
                    // killed there.
                    world.arm(Fault::None);
                    if rows.is_some_and(|before| receipt_rows(&serving.dir) <= before) {
                        *serving
                            .faults()
                            .fired
                            .entry("of them: killed after a pruning insert")
                            .or_default() += 1;
                    }
                    lost(&mut serving.faults());
                    serving.die("executor killed inside the Engine call", None);
                    return;
                }
            }
        }
    };
    // A request refused before the Engine leaves the fault armed.
    world.arm(Fault::None);
    {
        let mut faults = serving.faults();
        if let Some(action) = action {
            *faults.effects.entry(action).or_default() += world.effects() - effects;
        }
        if let Some(code) = response.error {
            *faults
                .refused
                .entry((what, format!("{code:?}")))
                .or_default() += 1;
            // `BeyondReceiptHistory` included: the soak never steps the clock
            // back, and a replaced store has pruned nothing it could refuse by.
            if LIFETIME.contains(&code) {
                faults
                    .violations
                    .push(format!("executor: lifetime refusal {code:?}"));
            }
        }
        if let Some((action, close)) = lookup
            && response.error.is_none()
        {
            if !close {
                faults.unclosed.push(format!(
                    "a lookup for job {} did not ask to close an absent action",
                    &action[..12.min(action.len())]
                ));
            } else if response.absence.is_some() && !response.closed {
                faults.unclosed.push(format!(
                    "the lookup for job {} answered {:?} without closing the action",
                    &action[..12.min(action.len())],
                    response.absence
                ));
            } else if response.closed {
                faults.closures += 1;
                faults.closed_jobs.insert(action.clone());
            }
        }
    }
    match after {
        After::Lose => {
            lost(&mut serving.faults());
            return;
        }
        After::LoseAndReplace(older) => {
            lost(&mut serving.faults());
            let reason = if older {
                "receipt store replaced by an older copy"
            } else {
                "receipt store replaced by an empty one"
            };
            serving.die_replacing(reason, None, Some(older));
            return;
        }
        After::CoreDies => {
            serving.crash.signal();
            return;
        }
        After::Write | After::StopUntil(_) => {}
    }
    let _ = tokio::time::timeout(
        limeos_contracts::RPC_DEADLINE,
        limeos_contracts::write_frame(&mut stream, &response),
    )
    .await;
    if let After::StopUntil(back_at) = after {
        serving.die("executor stopped between epoch and execute", Some(back_at));
    }
}

/// Rows in the receipt store (both tables), read on a side connection.
pub fn receipt_rows(dir: &Path) -> i64 {
    let Ok(conn) = super::checks::reader(&dir.join("receipts.sqlite")) else {
        return 0;
    };
    conn.query_row(
        "SELECT (SELECT count(*) FROM actions)+(SELECT count(*) FROM stack_actions)",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

pub struct Executor {
    /// The receipt store's directory.
    pub dir: PathBuf,
    /// The socket core connects to.
    pub socket: PathBuf,
    ceiling: Arc<Ceiling>,
    restart_ceiling: Arc<ContainerCeiling>,
    pub world: Arc<World>,
    clock: Clock,
    pub faults: Arc<Mutex<Faults>>,
    pub crash: Arc<Crash>,
    running: Option<(Arc<Serving>, JoinHandle<()>)>,
}

impl Executor {
    pub fn new(dir: &Path, socket: PathBuf, world: Arc<World>, clock: Clock, uid: u32) -> Self {
        let managed_containers: Vec<String> = HOST.iter().map(|(n, _)| container_id(n)).collect();
        let managed_stacks: Vec<String> = STACKS.iter().map(|s| (*s).to_owned()).collect();
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: uid,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: true,
            allow_restart: true,
            allow_start: true,
            allow_stop: true,
            allow_container_logs: true,
            allow_stack: true,
            managed_containers,
            managed_stacks,
        };
        assert!(ceiling.configuration_valid());
        // As the daemon derives it from its ceiling.
        let restart_ceiling = limeos_executor::ceiling::container_ceiling(&ceiling);
        Self {
            dir: dir.to_path_buf(),
            socket,
            ceiling: Arc::new(ceiling),
            restart_ceiling: Arc::new(restart_ceiling),
            world,
            clock,
            faults: Arc::default(),
            crash: Arc::default(),
            running: None,
        }
    }
    pub fn faults(&self) -> std::sync::MutexGuard<'_, Faults> {
        self.faults.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Arms `fault` for `job`, whose dispatch inspects `resource`.
    pub fn arm(&self, job: &str, resource: &str, fault: JobFault) {
        let mut faults = self.faults();
        faults.armed.insert(job.to_owned(), fault);
        faults.resources.insert(job.to_owned(), resource.to_owned());
    }
    /// Whether the process is running (listening on its socket).
    pub fn running(&self) -> bool {
        self.running.is_some()
    }
    /// Receipt-lookup answers it never wrote so far: container, stack.
    pub fn lost_lookups(&self) -> (u64, u64) {
        let faults = self.faults();
        let lost = |kind| faults.lost.get(kind).copied().unwrap_or(0);
        (lost("receipt lookup"), lost("stack receipt lookup"))
    }
    /// The process died on its own since the last call.
    pub fn take_death(&self) -> Option<Death> {
        self.faults().died.take()
    }
    /// Starts the process: opens (or creates) the receipt store, as the
    /// daemon does at startup, measured, and listens. A new open draws a new
    /// epoch.
    pub fn start(&mut self) -> Result<Opened> {
        assert!(self.running.is_none(), "one executor at a time");
        let (io, at) = (Io::now(), Instant::now());
        let store = ReceiptStore::open(&self.dir)?;
        let opened = Opened {
            io: Io::now().since(io),
            elapsed: at.elapsed(),
        };
        let epoch: Arc<str> = store.epoch().into();
        let _ = std::fs::remove_file(&self.socket);
        let listener = UnixListener::bind(&self.socket)
            .map_err(|_| Error(limeos_domain::ErrorCode::Unavailable))?;
        let serving = Arc::new(Serving {
            role: ContainerRole {
                engine: Docker(self.world.clone()),
                receipts: Some(Arc::new(tokio::sync::Mutex::new(store))),
                epoch: Some(epoch),
                docker: true,
            },
            ceiling: self.ceiling.clone(),
            restart_ceiling: self.restart_ceiling.clone(),
            world: self.world.clone(),
            clock: self.clock.clone(),
            faults: self.faults.clone(),
            crash: self.crash.clone(),
            socket: self.socket.clone(),
            dir: self.dir.clone(),
            tasks: Mutex::default(),
            accept: Mutex::default(),
        });
        let task = tokio::spawn(accept(serving.clone(), listener));
        *serving.accept.lock().unwrap() = Some(task.abort_handle());
        self.running = Some((serving, task));
        Ok(opened)
    }
    /// Stops the process: everything in flight is lost, the store closes.
    pub async fn stop(&mut self) {
        let Some((serving, accept)) = self.running.take() else {
            return;
        };
        accept.abort();
        let _ = accept.await;
        let tasks = std::mem::take(&mut *serving.tasks.lock().unwrap());
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        let store = serving.role.receipts.clone().expect("a receipt store");
        drop(serving);
        assert_eq!(
            Arc::strong_count(&store),
            1,
            "the stopped executor's receipt store is still in use"
        );
        drop(store);
        let _ = std::fs::remove_file(&self.socket);
    }
}
