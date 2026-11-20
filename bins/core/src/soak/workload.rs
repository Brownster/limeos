//! The household workload and the driver that runs it on the simulated clock.
//!
//! # Through core's own code
//!
//! Every step a person or core takes goes through the code core runs: the
//! dispatcher tick is `Core::dispatcher_pass` (resolution retries, stack and
//! container dispatch and recovery), container proposals, approvals, queueing,
//! cancellation, progress and dashboard polls and logouts are core's handlers
//! (`propose_action`, `approve_container`, `submit`, `cancel_restart`,
//! `restart_progress`, `observations`, `logout`, and their stack
//! counterparts), and the operator decides through `rpc_resolution_view` and
//! `rpc_resolve` (the root CLI's path). Two steps are not core's own: a
//! login is the session write `login_checked` makes once the password
//! workers have accepted the password (they are not driven), and a stack
//! proposal is `propose_stack`'s calls with its members taken from the
//! simulated Engine, because core takes them from an inventory cache that
//! judges freshness on its own system clock (`plan_stack`, checked against
//! the real `propose_stack` by `stack_proposals_match_core`).
//!
//! # Rates (normal days), and why
//!
//! Waking hours are 07:00 to 24:00; the house is quiet at night, which is
//! when the daily checkpoint (04:00) and the weekly idle window (02:00) run.
//!
//! | What | Rate | Why |
//! |---|---|---|
//! | Successful logins | 8 a day (4 per administrator) | Sessions last 8 h (`create_session`); a phone and a laptop each, twice a day |
//! | Explicit logouts | 40% of logins | The rest expire after 8 h |
//! | Failed logins | 3 a day (typos) + 1 unknown name | No durable write: `login_record` is a read; the password check that refuses them is core's and not driven |
//! | Dashboard polls | every 30 s for 15 min after a login | `Dashboard.tsx` refreshes every 30 s |
//! | Job progress polls | every 2 s for 30 s after queueing | `ContainerActions.tsx` / `StackActions.tsx` poll every 2 s |
//! | Container proposals | 10 a day over 18 containers | Restart 85% / stop 15% of running ones; a stopped one is started next |
//! | Of those: abandoned | 8% expire unapproved, 3% canceled, 2% approved too late, 2% approved and never queued | Exercises plan expiry (`PLAN_TTL_SECONDS` = 300) and the unqueued-plan cap |
//! | Stack actions | 2 a day over 4 Compose stacks | Restart 80% / stop 20%; a stack with stopped members is started next |
//! | Faults per dispatched job | about 6% in all (`arm_fault`) | Lost answers, executor and core deaths at each awkward step, Engine errors |
//! | Operator reviews | 08:30, 12:30, 18:30, 22:30 | Every `needs_intervention` job is resolved on fresh evidence |
//! | External restarts | 2 a day | An update tool or crash loop: plans go stale (`Conflict`) |
//! | Restarts | both daemons daily at 04:00 (measured), plus every fault above | |
//!
//! At these rates the shipped tables never reach their budgets within 90
//! days (about 100 events a day against 131,072; 12 receipts a day against
//! 1,024). Two bursts, each within what the code itself admits, saturate
//! them early, so the last two months run against full tables, the state a
//! Pi reaches after a year or two of normal use:
//!
//! - Day 20, 06:00 to 16:00: a runaway client logs in and out with valid
//!   credentials twice a second (two password workers, about half a second
//!   per check on a Pi, cap it near four a second): 144,000 events.
//! - Day 25, 08:00 to 20:00: a scripted client queues a container action
//!   every 40 s and a stack action every 45 s: 1,080 + 960 proposals, enough
//!   to overflow both receipt budgets (1,024 standalone, 256 stacks) although
//!   many of them meet a lock held by a job waiting for the operator.
use super::{
    checks,
    engine::{HOST, STACKS, container_id},
    sim::{CoreCrash, Flow, JobFault, JobRecord, Kind, Planned, Session, Sim, User, guarded},
    support::{DAY, HOUR, MINUTE, START},
};
use limeos_api::Backend;
use limeos_domain::{ContainerAction, DeclaredOutcome, ErrorCode, JobState};
use std::{collections::BTreeMap, time::Instant};

#[derive(Clone, Copy)]
pub struct Profile {
    pub name: &'static str,
    pub seed: u64,
    pub days: i64,
    /// (day, start hour, minutes) of the runaway-login burst.
    pub session_burst: (i64, i64, i64),
    /// (day, start hour, minutes) of the scripted action storm.
    pub action_burst: (i64, i64, i64),
    /// Weekday of the idle window (every day when 1).
    pub idle_every: i64,
    /// Trend checks need a steady last third.
    pub trends: bool,
    /// The last day core's store restarts (nightly and at awkward moments)
    /// begin; from then on it stays open, so nothing recomputes its counters,
    /// and startup is measured on copies. `None`: restarts throughout.
    pub core_restarts_until: Option<i64>,
}

/// What a flow does after its proposal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    Normal,
    Abandoned,
    Canceled,
    LateApproval,
    NeverQueued,
    /// Scripted: approved and queued within seconds.
    Scripted,
}

#[derive(Clone)]
pub enum Step {
    Login(usize),
    FailedLogin,
    UnknownLogin,
    Logout(usize),
    Dashboard {
        user: usize,
        left: u32,
    },
    Progress {
        job: String,
        left: u32,
    },
    ProposeContainer,
    ProposeStack,
    Approve(u64),
    Queue(u64),
    CancelPlan(u64),
    OperatorReview,
    ExternalRestart,
    Checkpoint,
    IdleWindow,
    /// Provokes the refusals every run must tell apart from lifetime ones:
    /// an approval after the plan expired, a session used after expiry.
    Provoke,
    UseStale,
    RunawayLogins {
        until: i64,
    },
    ScriptedContainer {
        until: i64,
    },
    ScriptedStack {
        until: i64,
    },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::Login(_) | Step::FailedLogin | Step::UnknownLogin | Step::Logout(_) => "sessions",
            Step::Dashboard { .. } | Step::Progress { .. } => "polls",
            Step::ProposeContainer
            | Step::ProposeStack
            | Step::Approve(_)
            | Step::Queue(_)
            | Step::CancelPlan(_) => "proposals",
            Step::OperatorReview => "operator reviews",
            Step::Provoke | Step::UseStale => "provocations",
            Step::ExternalRestart => "external restarts",
            Step::Checkpoint => "checkpoints and daily restarts",
            Step::IdleWindow => "idle windows",
            Step::RunawayLogins { .. } => "runaway-login burst",
            Step::ScriptedContainer { .. } | Step::ScriptedStack { .. } => "action storm",
        }
    }
}

#[derive(Default)]
pub struct Agenda {
    steps: BTreeMap<(i64, u64), Step>,
    seq: u64,
}
impl Agenda {
    pub fn at(&mut self, at: i64, step: Step) {
        self.seq += 1;
        self.steps.insert((at, self.seq), step);
    }
    fn next_time(&self) -> i64 {
        self.steps.keys().next().map_or(i64::MAX, |(at, _)| *at)
    }
    fn pop_due(&mut self, now: i64) -> Option<Step> {
        let (&key, _) = self.steps.iter().next()?;
        if key.0 > now {
            return None;
        }
        self.steps.remove(&key)
    }
}

/// The next waking-hours moment about `mean` seconds after `now`.
fn waking(sim: &mut Sim, now: i64, mean: i64) -> i64 {
    let at = now + sim.rng.around(mean);
    let hour = (at - START).rem_euclid(DAY) / HOUR;
    if hour < 7 {
        let day = (at - START).div_euclid(DAY);
        START + day * DAY + 7 * HOUR + sim.rng.below(3 * HOUR as u64) as i64
    } else {
        at
    }
}
fn day_at(day: i64, hour: i64, minute: i64) -> i64 {
    START + day * DAY + hour * HOUR + minute * MINUTE
}

pub async fn run(profile: Profile) -> Sim {
    let mut sim = Sim::new(profile.name, profile.seed, profile.core_restarts_until);
    let mut agenda = Agenda::default();
    bootstrap(&mut sim).await;
    let end = START + profile.days * DAY;
    for user in 0..sim.users.len() {
        let at = waking(&mut sim, START + 7 * HOUR, HOUR);
        agenda.at(at, Step::Login(user));
    }
    for step in [
        Step::FailedLogin,
        Step::UnknownLogin,
        Step::ProposeContainer,
        Step::ProposeStack,
        Step::ExternalRestart,
    ] {
        let at = waking(&mut sim, START + 8 * HOUR, 2 * HOUR);
        agenda.at(at, step);
    }
    for day in 0..profile.days {
        for (hour, minute) in [(8, 30), (12, 30), (18, 30), (22, 30)] {
            agenda.at(day_at(day, hour, minute), Step::OperatorReview);
        }
        agenda.at(day_at(day, 4, 0), Step::Checkpoint);
        if day % profile.idle_every == profile.idle_every - 1 {
            agenda.at(day_at(day, 2, 0), Step::IdleWindow);
        }
    }
    agenda.at(day_at(0, 9, 0), Step::Provoke);
    let (day, hour, minutes) = profile.session_burst;
    agenda.at(
        day_at(day, hour, 0),
        Step::RunawayLogins {
            until: day_at(day, hour, minutes),
        },
    );
    let (day, hour, minutes) = profile.action_burst;
    let until = day_at(day, hour, minutes);
    agenda.at(day_at(day, hour, 0), Step::ScriptedContainer { until });
    agenda.at(day_at(day, hour, 1), Step::ScriptedStack { until });

    while drive(&mut sim, &mut agenda, end).await {}
    drain(&mut sim, &mut agenda).await;
    sim
}

/// The next moment the driver acts: the next step, dispatcher tick or
/// executor return, and at least every simulated minute, so the store's
/// writer thread gets its once-a-minute retention gate as it would on the
/// system clock (it runs it on its own when the clock advances).
fn next_moment(sim: &Sim, agenda: &Agenda) -> i64 {
    agenda
        .next_time()
        .min(sim.next_pass)
        .min(sim.now() + MINUTE)
        .min(sim.executor_back_at.unwrap_or(i64::MAX))
}

/// Brings a stopped executor back when it is due.
fn executor_back(sim: &mut Sim) {
    if sim.executor_back_at.is_some_and(|at| at <= sim.now()) {
        sim.executor_back_at = None;
        sim.start_executor("executor back after a stop");
    }
}

/// Advances to the next thing due before `end` and does it. False at `end`.
async fn drive(sim: &mut Sim, agenda: &mut Agenda, end: i64) -> bool {
    let next = next_moment(sim, agenda);
    if next >= end {
        sim.advance(end);
        return false;
    }
    let at = Instant::now();
    sim.advance(next);
    sim.stats.time("clock and retention passes", at);
    executor_back(sim);
    if sim.now() >= sim.next_pass {
        let at = Instant::now();
        tick(sim).await;
        sim.stats.time("dispatcher passes", at);
    }
    while let Some(step) = agenda.pop_due(sim.now()) {
        let (name, at) = (step.name(), Instant::now());
        perform(sim, agenda, step, end).await;
        sim.stats.time(name, at);
    }
    true
}

/// The longest gap between two dispatcher passes while nothing is queued or
/// held. Core ticks every 2 s; such a pass only reads.
const IDLE_TICK: i64 = 5 * MINUTE;
/// Least time between two receipt lookups for one held job (core's
/// `operations::RECOVERY_PAUSE`).
const RECOVERY_PAUSE: i64 = 30;

/// One dispatcher tick, core's own (`Core::dispatcher_pass`), and when the
/// next is due. Core ticks every 2 s; a tick with nothing queued, held or
/// pending cannot change anything until a job is queued or a held job's
/// recovery pause ends, so the harness skips the empty ticks in between (up
/// to `IDLE_TICK` while nothing is held; queueing a job brings the next tick
/// forward to the next second).
pub async fn tick(sim: &mut Sim) {
    sim.stats.dispatcher_passes += 1;
    let at = Instant::now();
    let core = sim.core();
    let crash = sim.exec.crash.clone();
    let crashed = guarded(&crash, core.dispatcher_pass(&mut sim.dispatcher))
        .await
        .is_err();
    drop(core);
    if crashed {
        sim.restart_core("core restart at an awkward moment").await;
    }
    sim.absorb().await;
    sim.stats.time("dispatcher passes: core's pass", at);
    let (queued, held) = open_work(sim);
    let now = sim.now();
    sim.next_pass = if queued > 0 {
        now + 2
    } else if held > 0 {
        now + RECOVERY_PAUSE
    } else {
        now + IDLE_TICK
    };
}

/// Jobs queued, and jobs held or with a pending resolution request.
fn open_work(sim: &mut Sim) -> (i64, i64) {
    sim.side()
        .query_row(
            "SELECT (SELECT count(*) FROM jobs WHERE state='queued'),(SELECT count(*) FROM jobs WHERE state IN ('running','verifying','outcome_unknown','needs_intervention'))+(SELECT count(*) FROM job_resolutions WHERE confirmation IS NULL)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

pub async fn bootstrap(sim: &mut Sim) {
    sim.start_executor("first start");
    sim.start_core("first start").await;
    let now = sim.now();
    let token = sim.rng.hex();
    let digest = limeos_identity::digest(&token);
    let issued = {
        let digest = digest.clone();
        sim.call(move |s| s.issue_bootstrap(&digest, now)).await
    };
    sim.judge("issue bootstrap", issued, &[]).unwrap();
    let enrolled = sim
        .call(move |s| {
            s.enroll(
                &digest,
                "marc",
                "$argon2id$v=19$m=19456,t=2,p=1$soak$soak",
                now,
            )
        })
        .await;
    sim.judge("enroll administrator", enrolled, &[]).unwrap();
    // The second household administrator, imported from the old app.
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    let hash = fixtures[0]["hash"].as_str().unwrap().to_owned();
    let imported = sim
        .call(move |s| s.import_legacy_users(&[("anna".into(), hash)], now))
        .await;
    sim.judge("import legacy administrator", imported, &[])
        .unwrap();
    for name in ["marc", "anna"] {
        let principal = sim
            .call(move |s| s.login_record(name))
            .await
            .unwrap()
            .unwrap()
            .principal;
        sim.users.push(User {
            name,
            principal,
            session: None,
        });
    }
}

/// Logs a user in: the session write `Core::login_checked` makes once the
/// password workers accepted the password (they are not driven).
async fn login(sim: &mut Sim, user: usize) -> bool {
    let now = sim.now();
    let name = sim.users[user].name;
    let record = sim.call(move |s| s.login_record(name)).await;
    let Some(Some(record)) = sim.judge("login record", record, &[]) else {
        return false;
    };
    let token = sim.rng.hex();
    let csrf = limeos_identity::digest(&format!("csrf:{token}"));
    let session = Session { token, csrf };
    let principal = record.principal.clone();
    let (digest, csrf_digest) = (session.digest(), session.csrf_digest());
    let created = sim
        .call(move |s| s.create_session(&record, None, &digest, &csrf_digest, now))
        .await;
    if sim.judge("login", created, &[]).is_none() {
        return false;
    }
    sim.users[user].principal = principal;
    sim.users[user].session = Some(session);
    true
}

/// The user's live session (token and CSRF token), checked by core's own
/// `session` handler with its CSRF token as every mutating handler checks
/// it; logs in again when it has expired or was evicted, which is a
/// legitimate refusal.
async fn session(sim: &mut Sim, user: usize, op: &'static str) -> Option<(String, String)> {
    if let Some(session) = &sim.users[user].session {
        let (token, csrf) = (session.token.clone(), session.csrf.clone());
        let result = sim.core().session(token.clone(), Some(csrf.clone())).await;
        match sim.judge(op, result, &[ErrorCode::Unauthenticated]) {
            Some(view) => {
                sim.users[user].principal = view.principal;
                return Some((token, csrf));
            }
            None => sim.users[user].session = None,
        }
    }
    if !login(sim, user).await {
        return None;
    }
    let session = sim.users[user].session.as_ref()?;
    Some((session.token.clone(), session.csrf.clone()))
}

pub fn arm_fault(sim: &mut Sim, job: &str, resource: &str) -> Option<JobFault> {
    let roll = sim.rng.below(1000);
    let table = [
        (15, JobFault::AnswerLost),
        (8, JobFault::LostBeforeStore),
        (5, JobFault::ExecutorDownAfterEpoch),
        (5, JobFault::ExecutorKilledInEngineCall),
        (5, JobFault::EngineFailure { applied: true }),
        (5, JobFault::EngineFailure { applied: false }),
        (5, JobFault::CoreCrashAfterClaim),
        (5, JobFault::CoreCrashAfterExecute),
        (5, JobFault::CoreCrashAfterVerify),
        (5, JobFault::LookupLost),
    ];
    let mut edge = 0;
    for (per_mille, fault) in table {
        edge += per_mille;
        if roll < edge {
            if fault.restarts_core() && !sim.core_restarts() {
                return None;
            }
            sim.exec.arm(job, resource, fault);
            return Some(fault);
        }
    }
    None
}

fn fate(sim: &mut Sim) -> Fate {
    match sim.rng.below(1000) {
        0..80 => Fate::Abandoned,
        80..110 => Fate::Canceled,
        110..130 => Fate::LateApproval,
        130..150 => Fate::NeverQueued,
        _ => Fate::Normal,
    }
}

/// Resources the job list shows as busy: locked by a held job or claimed by
/// a queued one. People (and a sane script) pick something else; one time
/// in twenty they act anyway, and meet the lock.
fn busy(sim: &mut Sim) -> std::collections::HashSet<String> {
    if sim.rng.per_mille(50) {
        return Default::default();
    }
    let mut stmt = sim
        .side()
        .prepare_cached("SELECT resource FROM resource_locks UNION SELECT r.resource FROM job_resources r JOIN jobs j ON j.id=r.job WHERE j.state='queued'")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Core's `propose_action`: session, authorization, fresh inspection
/// through the executor, plan.
async fn propose_container(sim: &mut Sim, user: usize, fate: Fate) -> Option<u64> {
    let busy = busy(sim);
    let ids: Vec<String> = HOST
        .iter()
        .map(|(n, _)| container_id(n))
        .filter(|id| !busy.contains(&format!("container:{id}")))
        .collect();
    if ids.is_empty() {
        return None;
    }
    let id = sim.rng.pick(&ids).clone();
    let operation = if !sim.world.running(&id) {
        ContainerAction::Start
    } else if sim.rng.per_mille(150) {
        ContainerAction::Stop
    } else {
        ContainerAction::Restart
    };
    let (token, csrf) = session(sim, user, "propose container action: session").await?;
    let planned = sim
        .core()
        .propose_action(token, csrf, operation, format!("container:{id}"))
        .await;
    sim.absorb().await;
    let planned = sim.judge(
        "propose container action",
        planned,
        &[ErrorCode::Conflict, ErrorCode::Unavailable],
    )?;
    Some(start_flow(sim, user, Planned::Container(planned), fate))
}

/// A stack proposal: core's `propose_stack` but for where it takes the
/// members from. Core reads them from the storage reader's inventory cohort,
/// a cache (`limeos_observations::stacks`) that checks every cohort's
/// freshness (90 s) against its own system clock, which the simulated
/// months cannot satisfy and which has no seam. So the harness makes the
/// same calls in the same order with the members from the simulated Engine
/// (what that cohort would list), and `stack_proposals_match_core` runs the
/// real `propose_stack` beside this on the same Engine, at the system time,
/// and fails when what it writes, asks or plans differs.
async fn propose_stack(sim: &mut Sim, user: usize, fate: Fate) -> Option<u64> {
    let busy = busy(sim);
    let free: Vec<&str> = STACKS
        .iter()
        .copied()
        .filter(|project| {
            !busy.contains(&format!("stack:{project}"))
                && HOST
                    .iter()
                    .filter(|(_, p)| p == &Some(*project))
                    .all(|(n, _)| !busy.contains(&format!("container:{}", container_id(n))))
        })
        .collect();
    if free.is_empty() {
        return None;
    }
    let project = *sim.rng.pick(&free);
    let stopped = HOST
        .iter()
        .filter(|(_, p)| *p == Some(project))
        .any(|(n, _)| !sim.world.running(&container_id(n)));
    let operation = if stopped {
        ContainerAction::Start
    } else if sim.rng.per_mille(200) {
        ContainerAction::Stop
    } else {
        ContainerAction::Restart
    };
    let (token, csrf) = session(sim, user, "propose stack action: session").await?;
    let planned = plan_stack(sim, token, csrf, project, operation).await;
    sim.absorb().await;
    let planned = sim.judge(
        "propose stack action",
        planned,
        &[ErrorCode::Conflict, ErrorCode::Unavailable],
    )?;
    Some(start_flow(sim, user, Planned::Stack(planned), fate))
}

/// `Core::propose_stack` with the members from the simulated Engine (see
/// `propose_stack`): the stack as the cohort lists it, the same
/// authorization, fresh listing through core's `inspect_stack` and plan.
async fn plan_stack(
    sim: &Sim,
    token: String,
    csrf: String,
    project: &'static str,
    operation: ContainerAction,
) -> limeos_domain::Result<limeos_domain::PlannedStackAction> {
    let core = sim.core();
    let principal = core
        .session(token.clone(), Some(csrf.clone()))
        .await?
        .principal;
    let members = HOST
        .iter()
        .filter(|(_, p)| *p == Some(project))
        .map(|(n, _)| sim.world.snapshot(&container_id(n)).unwrap())
        .collect::<Vec<_>>();
    let planned = limeos_domain::StackSnapshot::new(project, members)?;
    planned.validate_for(operation)?;
    let (p, s) = (principal, planned.clone());
    core.db
        .call(move |store| store.authorize_stack(&p, project, &s))
        .await?;
    if core.inspect_stack(project).await? != planned {
        return Err(limeos_domain::Error(ErrorCode::Conflict));
    }
    let now = sim.now();
    let (digest, csrf) = (
        limeos_identity::digest(&token),
        limeos_identity::digest(&csrf),
    );
    core.db
        .call(move |s| {
            let principal = s.authenticate(&digest, Some(&csrf), now)?;
            s.plan_stack(&principal, operation, project, &planned, now)
        })
        .await
}

/// What a stack proposal did: executor requests, events written by kind,
/// and stack plans kept.
fn proposal_effects(sim: &mut Sim) -> (BTreeMap<&'static str, u64>, Vec<(String, i64)>, i64) {
    let requests = sim.exec.faults().requests.clone();
    let side = sim.side();
    let events = side
        .prepare("SELECT json_extract(kind,'$.kind'),count(*) FROM events GROUP BY 1 ORDER BY 1")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let plans = side
        .query_row("SELECT count(*) FROM stack_plans", [], |r| r.get(0))
        .unwrap();
    (requests, events, plans)
}

/// The stack-proposal shim against core's own `propose_stack`, on the same
/// Engine, at the system time (so the inventory cache accepts the cohort):
/// both must make the same executor requests, write the same events and
/// rows, and plan the same members and operation.
#[tokio::test(flavor = "current_thread")]
async fn stack_proposals_match_core() {
    let mut sim = Sim::new("stack-shim", 0x5eed_57ac, None);
    bootstrap(&mut sim).await;
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    sim.advance(wall);
    let (token, csrf) = session(&mut sim, 0, "session").await.unwrap();
    let mut seen = Vec::new();
    for (project, operation) in [
        ("home", ContainerAction::Restart),
        ("cloud", ContainerAction::Stop),
    ] {
        let before = proposal_effects(&mut sim);
        let core = sim.core();
        core.stacks
            .update(&sim.world.cohort(sim.now()))
            .expect("the inventory cohort is accepted");
        let real = core
            .propose_stack(token.clone(), csrf.clone(), project.into(), operation)
            .await
            .expect("core plans the stack action");
        let middle = proposal_effects(&mut sim);
        let shim = plan_stack(&sim, token.clone(), csrf.clone(), project, operation)
            .await
            .expect("the shim plans the stack action");
        let after = proposal_effects(&mut sim);
        let delta =
            |a: &(BTreeMap<&'static str, u64>, Vec<(String, i64)>, i64),
             b: &(BTreeMap<&'static str, u64>, Vec<(String, i64)>, i64)| {
                let requests: BTreeMap<&str, u64> =
                    b.0.iter()
                        .map(|(k, v)| (*k, v - a.0.get(k).copied().unwrap_or(0)))
                        .filter(|(_, v)| *v > 0)
                        .collect();
                let events: Vec<(String, i64)> =
                    b.1.iter()
                        .map(|(k, v)| {
                            let was = a.1.iter().find(|(j, _)| j == k).map_or(0, |(_, n)| *n);
                            (k.clone(), v - was)
                        })
                        .filter(|(_, v)| *v != 0)
                        .collect();
                (requests, events, b.2 - a.2)
            };
        let (core_did, shim_did) = (delta(&before, &middle), delta(&middle, &after));
        assert_eq!(
            core_did, shim_did,
            "core's propose_stack ({project}) asked, wrote and kept {core_did:?}; the soak's shim {shim_did:?}"
        );
        assert_eq!(
            (
                &real.plan.expected,
                real.plan.operation,
                &real.plan.grant_revision
            ),
            (
                &shim.plan.expected,
                shim.plan.operation,
                &shim.plan.grant_revision
            ),
            "core's propose_stack and the shim planned different stacks"
        );
        seen.push(core_did);
    }
    assert!(
        seen.iter()
            .all(|(requests, _, plans)| !requests.is_empty() && *plans == 1),
        "the comparison saw no work: {seen:?}"
    );
}

fn start_flow(sim: &mut Sim, user: usize, planned: Planned, fate: Fate) -> u64 {
    let id = sim.next_flow;
    sim.next_flow += 1;
    let key = format!("soak-{}", sim.rng.hex());
    sim.flows.insert(
        id,
        Flow {
            user,
            planned,
            approval: None,
            key,
            fate,
        },
    );
    id
}

/// Core's `approve_container` / `approve_stack_plan`.
async fn approve(sim: &mut Sim, flow: u64) -> bool {
    let Some(user) = sim.flows.get(&flow).map(|f| f.user) else {
        return false;
    };
    let Some((token, csrf)) = session(sim, user, "approve: session").await else {
        return false;
    };
    let core = sim.core();
    let approval = match &sim.flows[&flow].planned {
        Planned::Container(p) => {
            core.approve_container(token, csrf, p.plan.id.clone(), p.digest.clone())
                .await
        }
        Planned::Stack(p) => {
            core.approve_stack_plan(token, csrf, p.plan.id.clone(), p.digest.clone())
                .await
        }
    };
    // A plan approved too late is refused as expired, or, once a later
    // proposal has swept it away, as not found.
    match sim.judge(
        "approve plan",
        approval,
        &[ErrorCode::Expired, ErrorCode::NotFound],
    ) {
        Some(approval) => {
            sim.flows.get_mut(&flow).unwrap().approval = Some(approval.token);
            true
        }
        None => {
            sim.flows.remove(&flow);
            false
        }
    }
}

/// Core's `submit` / `submit_stack`: replay check, fresh inspection, queue;
/// then the job is in the ledger and its fault (if any) is armed.
async fn queue(sim: &mut Sim, agenda: &mut Agenda, flow: u64) {
    let Some(Flow {
        user,
        planned,
        approval: Some(approval),
        key,
        ..
    }) = sim.flows.remove(&flow)
    else {
        return;
    };
    let Some((token, csrf)) = session(sim, user, "queue job: session").await else {
        return;
    };
    let legit = [
        ErrorCode::Expired,
        ErrorCode::Conflict,
        ErrorCode::Unavailable,
    ];
    let core = sim.core();
    let queued = match planned {
        Planned::Container(proposal) => {
            let job = core
                .submit(
                    token,
                    csrf,
                    limeos_contracts::QueueRestartInput {
                        proposal,
                        approval,
                        idempotency_key: key,
                    },
                )
                .await;
            sim.absorb().await;
            sim.judge("queue container job", job, &legit).map(|job| {
                let resource = job.plan.expected.resource.clone();
                (job.id, Kind::Container, job.plan.operation, 1, resource)
            })
        }
        Planned::Stack(proposal) => {
            let job = core
                .submit_stack(
                    token,
                    csrf,
                    limeos_contracts::QueueStackInput {
                        proposal,
                        approval,
                        idempotency_key: key,
                    },
                )
                .await;
            sim.absorb().await;
            sim.judge("queue stack job", job, &legit).map(|job| {
                let targets = job.plan.targets().len();
                let project = job.plan.expected.project().unwrap_or_default().to_owned();
                (
                    job.id,
                    Kind::Stack,
                    job.plan.operation,
                    targets,
                    format!("stack:{project}"),
                )
            })
        }
    };
    let Some((id, kind, operation, targets, resource)) = queued else {
        return;
    };
    let fault = arm_fault(sim, &id, &resource);
    sim.ledger.insert(
        id.clone(),
        JobRecord {
            kind,
            operation,
            targets,
            fault,
            declared: None,
            user,
        },
    );
    sim.next_pass = sim.next_pass.min(sim.now() + 1);
    agenda.at(sim.now() + 2, Step::Progress { job: id, left: 15 });
}

async fn perform(sim: &mut Sim, agenda: &mut Agenda, step: Step, end: i64) {
    let now = sim.now();
    let again = |sim: &mut Sim, agenda: &mut Agenda, mean: i64, step: Step| {
        let at = waking(sim, now, mean);
        if at < end {
            agenda.at(at, step);
        }
    };
    match step {
        Step::Login(user) => {
            if login(sim, user).await {
                agenda.at(now + 30, Step::Dashboard { user, left: 30 });
                if sim.rng.per_mille(400) {
                    agenda.at(now + sim.rng.around(30 * MINUTE), Step::Logout(user));
                }
            }
            again(sim, agenda, 17 * HOUR / 4, Step::Login(user));
        }
        Step::FailedLogin => {
            // `Core::login_checked` reads the account, then the password
            // workers reject the password: the store sees only this read.
            // The refusal itself is core's, not driven here.
            let user = sim.rng.below(sim.users.len() as u64) as usize;
            let name = sim.users[user].name;
            let record = sim.call(move |s| s.login_record(name)).await;
            sim.judge("failed login: account read", record, &[]);
            again(sim, agenda, 17 * HOUR / 3, Step::FailedLogin);
        }
        Step::UnknownLogin => {
            let record = sim.call(|s| s.login_record("nobody")).await;
            if let Some(Some(_)) = sim.judge("failed login: account read", record, &[]) {
                sim.violation(checks::INVARIANTS, "an unknown name has an account".into());
            }
            again(sim, agenda, 17 * HOUR, Step::UnknownLogin);
        }
        Step::Logout(user) => {
            if let Some(session) = sim.users[user].session.take() {
                // Core's `logout`: admission on the query-only connection,
                // then the revocation lane.
                let result = sim.core().logout(session.token, session.csrf).await;
                sim.judge("logout", result, &[ErrorCode::Unauthenticated]);
            }
        }
        Step::Dashboard { user, left } => {
            // Core's `observations`: the session (no CSRF) and its grants.
            let Some(session) = &sim.users[user].session else {
                return;
            };
            let read = sim.core().observations(session.token.clone()).await;
            sim.stats.reads += 1;
            if sim
                .judge("dashboard poll", read, &[ErrorCode::Unauthenticated])
                .is_none()
            {
                sim.users[user].session = None;
            } else if left > 0 {
                agenda.at(
                    now + 30,
                    Step::Dashboard {
                        user,
                        left: left - 1,
                    },
                );
            }
        }
        Step::Progress { job, left } => {
            // The page's 2 s progress poll, with its owner's session.
            let Some(record) = sim.ledger.get(&job) else {
                return;
            };
            let (kind, user) = (record.kind, record.user);
            let Some(token) = sim.users[user].session.as_ref().map(|s| s.token.clone()) else {
                return;
            };
            let core = sim.core();
            let state = match kind {
                Kind::Container => core
                    .restart_progress(token, job.clone(), 0)
                    .await
                    .map(|p| p.job.state),
                Kind::Stack => core
                    .stack_progress(token, job.clone(), 0)
                    .await
                    .map(|p| p.job.state),
            };
            sim.stats.reads += 1;
            if let Some(state) =
                sim.judge("job progress poll", state, &[ErrorCode::Unauthenticated])
            {
                let terminal = !matches!(
                    state,
                    JobState::Queued
                        | JobState::Running
                        | JobState::Verifying
                        | JobState::OutcomeUnknown
                );
                if !terminal && left > 0 {
                    agenda.at(
                        now + 2,
                        Step::Progress {
                            job,
                            left: left - 1,
                        },
                    );
                }
            }
        }
        Step::ProposeContainer => {
            let user = sim.rng.below(sim.users.len() as u64) as usize;
            let fate = fate(sim);
            if let Some(flow) = propose_container(sim, user, fate).await {
                schedule_flow(sim, agenda, flow);
            }
            again(sim, agenda, 17 * HOUR / 10, Step::ProposeContainer);
        }
        Step::ProposeStack => {
            let user = sim.rng.below(sim.users.len() as u64) as usize;
            let fate = fate(sim);
            if let Some(flow) = propose_stack(sim, user, fate).await {
                schedule_flow(sim, agenda, flow);
            }
            again(sim, agenda, 17 * HOUR / 2, Step::ProposeStack);
        }
        Step::Approve(flow) => {
            if approve(sim, flow).await {
                match sim.flows.get(&flow).map(|f| f.fate) {
                    Some(Fate::NeverQueued) => {
                        sim.flows.remove(&flow);
                    }
                    Some(Fate::Scripted) => agenda.at(now + 3, Step::Queue(flow)),
                    _ => agenda.at(now + sim.rng.around(20), Step::Queue(flow)),
                }
            }
        }
        Step::Queue(flow) => queue(sim, agenda, flow).await,
        Step::CancelPlan(flow) => {
            if let Some(Flow { user, planned, .. }) = sim.flows.remove(&flow) {
                if let Some((token, csrf)) = session(sim, user, "cancel plan: session").await {
                    let core = sim.core();
                    let result = match planned {
                        Planned::Container(p) => {
                            core.cancel_restart(token, csrf, p.plan.id, true).await
                        }
                        Planned::Stack(p) => core.cancel_stack(token, csrf, p.plan.id, true).await,
                    };
                    sim.judge("cancel plan", result, &[]);
                }
            }
        }
        Step::OperatorReview => operator_review(sim).await,
        Step::Provoke => {
            if let Some(flow) = propose_container(sim, 0, Fate::LateApproval).await {
                schedule_flow(sim, agenda, flow);
            }
            if login(sim, 0).await {
                let session = sim.users[0].session.as_ref().unwrap();
                sim.stale = Some(Session {
                    token: session.token.clone(),
                    csrf: session.csrf.clone(),
                });
                agenda.at(now + 9 * HOUR, Step::UseStale);
            }
        }
        Step::UseStale => {
            if let Some(session) = sim.stale.take() {
                let result = sim.core().session(session.token, Some(session.csrf)).await;
                sim.judge(
                    "provoked: session used after it expired",
                    result,
                    &[ErrorCode::Unauthenticated],
                );
            }
        }
        Step::ExternalRestart => {
            let ids = sim.world.ids();
            let id = sim.rng.pick(&ids).clone();
            if sim.world.running(&id) {
                sim.world.external_restart(&id);
                *sim.stats
                    .faults
                    .entry("container restarted outside LimeOS")
                    .or_default() += 1;
            }
            again(sim, agenda, 17 * HOUR / 2, Step::ExternalRestart);
        }
        Step::Checkpoint => {
            checks::checkpoint(sim);
            // Startup, measured on copies of both stores every night: the
            // deterministic series the startup checks judge.
            super::startup::core(sim);
            super::startup::receipts(sim);
            // Both daemons restart every night (core only while its restarts
            // last): every restart is followed by the checks.
            sim.restart_executor("daily restart").await;
            if sim.core_restarts() {
                sim.restart_core("daily restart").await;
            }
        }
        Step::IdleWindow => checks::idle_window(sim).await,
        Step::RunawayLogins { until } => {
            for _ in 0..2 {
                if login(sim, 1).await {
                    let session = sim.users[1].session.take().unwrap();
                    let result = sim.core().logout(session.token, session.csrf).await;
                    sim.judge("logout", result, &[]);
                }
            }
            if now + 1 < until {
                agenda.at(now + 1, Step::RunawayLogins { until });
            }
        }
        Step::ScriptedContainer { until } => {
            if let Some(flow) = propose_container(sim, 0, Fate::Scripted).await {
                schedule_flow(sim, agenda, flow);
            }
            if now + 40 < until {
                agenda.at(now + 40, Step::ScriptedContainer { until });
            }
        }
        Step::ScriptedStack { until } => {
            if let Some(flow) = propose_stack(sim, 0, Fate::Scripted).await {
                schedule_flow(sim, agenda, flow);
            }
            if now + 45 < until {
                agenda.at(now + 45, Step::ScriptedStack { until });
            }
        }
    }
}

/// Schedules a new flow's next step by its fate.
fn schedule_flow(sim: &mut Sim, agenda: &mut Agenda, flow: u64) {
    let now = sim.now();
    match sim.flows[&flow].fate {
        Fate::Abandoned => {
            sim.flows.remove(&flow);
        }
        Fate::Canceled => agenda.at(now + sim.rng.around(60), Step::CancelPlan(flow)),
        Fate::LateApproval => agenda.at(now + 330, Step::Approve(flow)),
        Fate::Scripted => agenda.at(now + 3, Step::Approve(flow)),
        Fate::Normal | Fate::NeverQueued => {
            agenda.at(now + sim.rng.around(40), Step::Approve(flow))
        }
    }
}

/// The operator works through the needs-attention list.
pub async fn operator_review(sim: &mut Sim) {
    let record = sim.call(|s| s.login_record("marc")).await;
    let Some(Some(admin)) = sim.judge("operator: record", record, &[]) else {
        return;
    };
    let admin = admin.principal;
    let listed = sim.call(move |s| s.needs_attention_jobs(&admin)).await;
    let Some(jobs) = sim.judge("operator: needs-attention list", listed, &[]) else {
        return;
    };
    for job in jobs {
        if resolve(sim, job.job).await.is_err() {
            return;
        }
    }
}

/// The operator decides one job through core's own resolution path (the
/// root CLI's `rpc_resolution_view` and `rpc_resolve`): fresh evidence, the
/// declaration the evidence supports (on a second attempt, an accepted
/// divergence), then core's crash-safe request, executor, completion order.
/// One decision in ten, while core may restart, is interrupted by a core
/// restart between the recorded request and the executor; the retry loop
/// completes it after startup.
async fn resolve(sim: &mut Sim, job: String) -> Result<(), CoreCrash> {
    let view = sim
        .core()
        .rpc_resolution_view("marc".into(), job.clone())
        .await;
    sim.absorb().await;
    let Some(view) = sim.judge(
        "operator: evidence",
        view,
        &[ErrorCode::Conflict, ErrorCode::Unavailable],
    ) else {
        return Ok(());
    };
    let awaiting = matches!(
        view.state,
        JobState::NeedsIntervention | JobState::Running | JobState::Verifying
    );
    let Some(digest) = view.evidence_digest.clone() else {
        return Ok(());
    };
    if view.resolution.is_some() || view.pending.is_some() || !awaiting {
        return Ok(());
    }
    let attempt = {
        let count = sim.attempts.entry(job.clone()).or_default();
        *count += 1;
        *count
    };
    let declared = if attempt == 1 && view.allowed.contains(&DeclaredOutcome::VerifiedDone) {
        DeclaredOutcome::VerifiedDone
    } else if attempt == 1 && view.allowed.contains(&DeclaredOutcome::NotDone) {
        DeclaredOutcome::NotDone
    } else {
        DeclaredOutcome::AcceptedDivergence
    };
    let reason = match declared {
        DeclaredOutcome::AcceptedDivergence => {
            "checked the containers by hand; accepting their state"
        }
        _ => "checked by the soak operator",
    };
    if let Some(record) = sim.ledger.get_mut(&job) {
        record.declared = Some(declared);
    }
    if sim.core_restarts() && sim.rng.per_mille(100) {
        sim.exec.arm(&job, "", JobFault::ResolutionInterrupted);
    }
    let core = sim.core();
    let crash = sim.exec.crash.clone();
    let decided = guarded(
        &crash,
        core.rpc_resolve("marc".into(), job, declared, reason.into(), digest),
    )
    .await;
    drop(core);
    match decided {
        Err(CoreCrash) => {
            sim.restart_core("core restart between resolution request and executor")
                .await;
            sim.absorb().await;
            return Err(CoreCrash);
        }
        Ok(Ok(response)) => sim.stats.op(&format!("resolution: {:?}", response.status)),
        Ok(Err(e)) => {
            sim.judge::<()>(
                "operator: resolve",
                Err(e),
                &[ErrorCode::Conflict, ErrorCode::Unavailable],
            );
        }
    }
    sim.absorb().await;
    Ok(())
}

/// After the last day: no new work; dispatch, recover and resolve until
/// nothing is queued, held or pending (at most two days).
async fn drain(sim: &mut Sim, agenda: &mut Agenda) {
    let limit = sim.now() + 2 * DAY;
    let mut review = sim.now();
    while sim.now() < limit {
        let next = next_moment(sim, agenda).min(review);
        sim.advance(next);
        executor_back(sim);
        let now = sim.now();
        if now >= sim.next_pass {
            tick(sim).await;
        }
        // Only reads and the steps of work already started.
        while let Some(step) = agenda.pop_due(sim.now()) {
            if matches!(
                step,
                Step::Approve(_)
                    | Step::Queue(_)
                    | Step::Progress { .. }
                    | Step::Logout(_)
                    | Step::Dashboard { .. }
            ) {
                perform(sim, agenda, step, limit).await;
            }
        }
        if now >= review {
            operator_review(sim).await;
            review = now + 2 * HOUR;
            if checks::settled(sim) {
                break;
            }
        }
    }
}
