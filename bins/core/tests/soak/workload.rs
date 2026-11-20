//! The household workload and the driver that runs it on the simulated clock.
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
use crate::{
    checks,
    engine::{HOST, STACKS, container_id},
    sim::{Flow, Halt, JobFault, JobRecord, Kind, Planned, Session, Sim, User},
    support::{DAY, HOUR, MINUTE, START},
};
use limeos_domain::{ContainerAction, ErrorCode, StackSnapshot};
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
    bootstrap(&mut sim);
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

/// Advances to the next thing due before `end` and does it. False at `end`.
async fn drive(sim: &mut Sim, agenda: &mut Agenda, end: i64) -> bool {
    let next = agenda
        .next_time()
        .min(sim.next_pass)
        .min(sim.last_retention + MINUTE)
        .min(sim.executor_back_at.unwrap_or(i64::MAX));
    if next >= end {
        sim.clock.set(end);
        return false;
    }
    sim.clock.set(next.max(sim.now()));
    let now = sim.now();
    if sim.executor_back_at.is_some_and(|at| at <= now) {
        sim.executor_back_at = None;
        sim.start_executor("executor back after a stop");
    }
    if now >= sim.last_retention + MINUTE {
        let at = Instant::now();
        sim.retention_pass();
        sim.stats.time("retention passes", at);
    }
    if now >= sim.next_pass {
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

/// One dispatcher pass, and when the next is due. Core ticks every 2 s; a
/// pass with no queued job cannot change anything until a job is queued or
/// a held job's recovery pause ends, so the harness skips the empty ticks in
/// between (up to `IDLE_TICK` while nothing is held; queueing a job brings
/// the next pass forward to the next second).
pub async fn tick(sim: &mut Sim) {
    sim.seen_queued = 0;
    sim.seen_held = 0;
    if let Err(Halt::CoreCrash) = sim.dispatcher_pass().await {
        sim.restart_core("core restart at an awkward moment");
    }
    let now = sim.now();
    sim.next_pass = if sim.seen_queued > 0 {
        now + 2
    } else if sim.seen_held > 0 {
        now + crate::sim::RECOVERY_PAUSE
    } else {
        now + IDLE_TICK
    };
}

pub fn bootstrap(sim: &mut Sim) {
    sim.start_executor("first start");
    sim.start_core("first start");
    let now = sim.now();
    let token = sim.rng.hex();
    let digest = limeos_identity::digest(&token);
    let issued = sim.db().issue_bootstrap(&digest, now);
    sim.judge("issue bootstrap", issued, &[]).unwrap();
    let enrolled = sim.db().enroll(
        &digest,
        "marc",
        "$argon2id$v=19$m=19456,t=2,p=1$soak$soak",
        now,
    );
    sim.judge("enroll administrator", enrolled, &[]).unwrap();
    // The second household administrator, imported from the old app.
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    let hash = fixtures[0]["hash"].as_str().unwrap().to_owned();
    let imported = sim.db().import_legacy_users(&[("anna".into(), hash)], now);
    sim.judge("import legacy administrator", imported, &[])
        .unwrap();
    for name in ["marc", "anna"] {
        let principal = sim.db().login_record(name).unwrap().unwrap().principal;
        sim.users.push(User {
            name,
            principal,
            session: None,
        });
    }
    *sim.stats.ops.entry("authority: expected").or_default() += 3;
}

/// Logs a user in through the store, as `Core::login_checked` does once the
/// password check passed.
fn login(sim: &mut Sim, user: usize) -> bool {
    let now = sim.now();
    let name = sim.users[user].name;
    let Some(Some(record)) = sim.judge(
        "login record",
        sim.store.as_ref().unwrap().login_record(name),
        &[],
    ) else {
        return false;
    };
    let token = sim.rng.hex();
    let csrf = limeos_identity::digest(&format!("csrf:{token}"));
    let session = Session { token, csrf };
    let created = sim.db().create_session(
        &record,
        None,
        &session.digest(),
        &session.csrf_digest(),
        now,
    );
    if sim.judge("login", created, &[]).is_none() {
        return false;
    }
    sim.users[user].principal = record.principal;
    sim.users[user].session = Some(session);
    true
}

/// The user's current principal through their session (`authenticate` with
/// the CSRF digest, as every mutating handler does); logs in again when the
/// session has expired or was evicted, which is a legitimate refusal.
fn session(sim: &mut Sim, user: usize, op: &'static str) -> Option<limeos_domain::Principal> {
    let now = sim.now();
    if let Some(session) = &sim.users[user].session {
        let result = sim.store.as_ref().unwrap().authenticate(
            &session.digest(),
            Some(&session.csrf_digest()),
            now,
        );
        match sim.judge(op, result, &[ErrorCode::Unauthenticated]) {
            Some(principal) => return Some(principal),
            None => sim.users[user].session = None,
        }
    }
    if !login(sim, user) {
        return None;
    }
    sim.users[user]
        .session
        .as_ref()
        .map(|_| sim.users[user].principal.clone())
}

pub fn arm_fault(sim: &mut Sim, job: &str) -> Option<JobFault> {
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
            sim.armed.insert(job.to_owned(), fault);
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
    let conn = checks::reader(&sim.core_path).expect("core side connection");
    let mut stmt = conn
        .prepare("SELECT resource FROM resource_locks UNION SELECT r.resource FROM job_resources r JOIN jobs j ON j.id=r.job WHERE j.state='queued'")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// `Core::propose_action`: session, fresh inspection, plan.
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
    let principal = session(sim, user, "propose container action: session")?;
    let resource = format!("container:{id}");
    let snapshot = match sim.inspect(&resource).await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            sim.judge::<()>(
                "propose container action: inspect",
                Err(e),
                &[ErrorCode::Unavailable],
            );
            return None;
        }
    };
    let now = sim.now();
    let planned = sim
        .db()
        .plan_container(&principal, operation, &snapshot, now);
    let planned = sim.judge("propose container action", planned, &[ErrorCode::Conflict])?;
    Some(start_flow(sim, user, Planned::Container(planned), fate))
}

/// `Core::propose_stack`: the members as the inventory cohort lists them,
/// authorized, checked against a fresh Engine listing, planned.
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
    let members = HOST
        .iter()
        .filter(|(_, p)| *p == Some(project))
        .map(|(n, _)| sim.world.snapshot(&container_id(n)).unwrap())
        .collect::<Vec<_>>();
    let operation = if members.iter().any(|m| !m.running) {
        ContainerAction::Start
    } else if sim.rng.per_mille(200) {
        ContainerAction::Stop
    } else {
        ContainerAction::Restart
    };
    let planned = StackSnapshot::new(project, members).unwrap();
    let principal = session(sim, user, "propose stack action: session")?;
    let authorized = sim
        .store
        .as_ref()
        .unwrap()
        .authorize_stack(&principal, project, &planned);
    sim.judge("propose stack action: authorize", authorized, &[])?;
    match sim.inspect_stack(project).await {
        Ok(current) if current == planned => {}
        Ok(_) => return None,
        Err(e) => {
            sim.judge::<()>(
                "propose stack action: inspect",
                Err(e),
                &[ErrorCode::Unavailable],
            );
            return None;
        }
    }
    let now = sim.now();
    let result = sim
        .db()
        .plan_stack(&principal, operation, project, &planned, now);
    let result = sim.judge("propose stack action", result, &[ErrorCode::Conflict])?;
    Some(start_flow(sim, user, Planned::Stack(result), fate))
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

async fn approve(sim: &mut Sim, flow: u64) -> bool {
    let Some(user) = sim.flows.get(&flow).map(|f| f.user) else {
        return false;
    };
    let Some(principal) = session(sim, user, "approve: session") else {
        return false;
    };
    let now = sim.now();
    let Flow { planned, .. } = &sim.flows[&flow];
    let (id, digest) = match planned {
        Planned::Container(p) => (p.plan.id.clone(), p.digest.clone()),
        Planned::Stack(p) => (p.plan.id.clone(), p.digest.clone()),
    };
    let stack = matches!(planned, Planned::Stack(_));
    let store = sim.db();
    let approval = if stack {
        store.approve_stack(&principal, &id, &digest, now)
    } else {
        store.approve_container(&principal, &id, &digest, now)
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

/// `Core::submit` / `Core::submit_stack`: replay check, fresh inspection,
/// queue; then the job is in the ledger and its fault (if any) is armed.
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
    let Some(principal) = session(sim, user, "queue job: session") else {
        return;
    };
    let legit = [
        ErrorCode::Expired,
        ErrorCode::Conflict,
        ErrorCode::Unavailable,
    ];
    let queued = match &planned {
        Planned::Container(proposal) => {
            let replay = sim
                .store
                .as_ref()
                .unwrap()
                .replay_container(&principal, &key, proposal, &approval);
            if sim
                .judge("queue job: replay check", replay, &legit)
                .is_none()
            {
                return;
            }
            let snapshot = match sim.inspect(&proposal.plan.expected.resource).await {
                Ok(s) => s,
                Err(e) => {
                    sim.judge::<()>("queue job: inspect", Err(e), &legit);
                    return;
                }
            };
            let now = sim.now();
            let job = sim
                .db()
                .queue_container(&principal, &key, proposal, &approval, &snapshot, now);
            sim.judge("queue container job", job, &legit)
                .map(|job| (job.id, Kind::Container, job.plan.operation, 1))
        }
        Planned::Stack(proposal) => {
            let replay = sim
                .store
                .as_ref()
                .unwrap()
                .replay_stack(&principal, &key, proposal, &approval);
            if sim
                .judge("queue job: replay check", replay, &legit)
                .is_none()
            {
                return;
            }
            let project = proposal.plan.expected.project().unwrap().to_owned();
            let snapshot = match sim.inspect_stack(&project).await {
                Ok(s) => s,
                Err(e) => {
                    sim.judge::<()>("queue job: inspect", Err(e), &legit);
                    return;
                }
            };
            let now = sim.now();
            let job = sim
                .db()
                .queue_stack(&principal, &key, proposal, &approval, &snapshot, now);
            sim.judge("queue stack job", job, &legit).map(|job| {
                let targets = job.plan.targets().len();
                (job.id, Kind::Stack, job.plan.operation, targets)
            })
        }
    };
    let Some((id, kind, operation, targets)) = queued else {
        return;
    };
    let fault = arm_fault(sim, &id);
    sim.ledger.insert(
        id.clone(),
        JobRecord {
            kind,
            operation,
            targets,
            fault,
            declared: None,
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
            if login(sim, user) {
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
            let record = sim.store.as_ref().unwrap().login_record(name);
            sim.judge("failed login: account read", record, &[]);
            again(sim, agenda, 17 * HOUR / 3, Step::FailedLogin);
        }
        Step::UnknownLogin => {
            let record = sim.store.as_ref().unwrap().login_record("nobody");
            if let Some(Some(_)) = sim.judge("failed login: account read", record, &[]) {
                sim.violation(checks::INVARIANTS, "an unknown name has an account".into());
            }
            again(sim, agenda, 17 * HOUR, Step::UnknownLogin);
        }
        Step::Logout(user) => {
            if let Some(session) = sim.users[user].session.take() {
                // `Database::revoke`: admission on the reader, then the write.
                let checked = sim.store.as_ref().unwrap().authenticate(
                    &session.digest(),
                    Some(&session.csrf_digest()),
                    now,
                );
                if sim
                    .judge("logout: admission", checked, &[ErrorCode::Unauthenticated])
                    .is_some()
                {
                    let revoked = sim.db().revoke_session(&session.digest(), now);
                    sim.judge("logout", revoked, &[]);
                }
            }
        }
        Step::Dashboard { user, left } => {
            // `Core::observations`: authenticate and read grants, no CSRF.
            let Some(session) = &sim.users[user].session else {
                return;
            };
            let store = sim.store.as_ref().unwrap();
            let read = store
                .authenticate(&session.digest(), None, now)
                .and_then(|p| store.grants(&p).map(|_| ()));
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
            // The page's 2 s progress poll (`restart_progress`).
            let state = sim.store.as_ref().unwrap().job_state(&job);
            sim.stats.reads += 1;
            if let Some(state) = sim.judge("job progress poll", state, &[]) {
                let terminal = !matches!(
                    state.as_str(),
                    "queued" | "running" | "verifying" | "outcome_unknown"
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
                if let Some(principal) = session(sim, user, "cancel plan: session") {
                    let result = match planned {
                        Planned::Container(p) => {
                            sim.db().cancel_container_plan(&principal, &p.plan.id, now)
                        }
                        Planned::Stack(p) => {
                            sim.db().cancel_stack_plan(&principal, &p.plan.id, now)
                        }
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
            if login(sim, 0) {
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
                let result = sim.store.as_ref().unwrap().authenticate(
                    &session.digest(),
                    Some(&session.csrf_digest()),
                    now,
                );
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
            crate::startup::core(sim);
            crate::startup::receipts(sim);
            // Both daemons restart every night (core only while its restarts
            // last): every restart is followed by the checks.
            sim.restart_executor("daily restart");
            if sim.core_restarts() {
                sim.restart_core("daily restart");
            }
        }
        Step::IdleWindow => checks::idle_window(sim).await,
        Step::RunawayLogins { until } => {
            for _ in 0..2 {
                if login(sim, 1) {
                    let session = sim.users[1].session.take().unwrap();
                    let revoked = sim.db().revoke_session(&session.digest(), now);
                    sim.judge("logout", revoked, &[]);
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
    let Some(Some(admin)) = sim.judge(
        "operator: record",
        sim.store.as_ref().unwrap().login_record("marc"),
        &[],
    ) else {
        return;
    };
    let admin = admin.principal;
    let listed = sim.store.as_ref().unwrap().needs_attention_jobs(&admin);
    let Some(jobs) = sim.judge("operator: needs-attention list", listed, &[]) else {
        return;
    };
    for job in jobs {
        match sim.resolve_as(admin.clone(), job.job).await {
            Ok(Some(finish)) => sim.note_finish(finish),
            Ok(None) => {}
            Err(Halt::CoreCrash) => {
                sim.restart_core("core restart between resolution request and executor");
                return;
            }
            Err(Halt::Error(_, e)) => {
                sim.judge::<()>(
                    "operator: resolve",
                    Err(e),
                    &[ErrorCode::Conflict, ErrorCode::Unavailable],
                );
            }
        }
    }
}

/// After the last day: no new work; dispatch, recover and resolve until
/// nothing is queued, held or pending (at most two days).
async fn drain(sim: &mut Sim, agenda: &mut Agenda) {
    let limit = sim.now() + 2 * DAY;
    let mut review = sim.now();
    while sim.now() < limit {
        let next = sim
            .next_pass
            .min(sim.last_retention + MINUTE)
            .min(review)
            .min(agenda.next_time())
            .min(sim.executor_back_at.unwrap_or(i64::MAX));
        sim.clock.set(next.max(sim.now()));
        let now = sim.now();
        if sim.executor_back_at.is_some_and(|at| at <= now) {
            sim.executor_back_at = None;
            sim.start_executor("executor back after a stop");
        }
        if now >= sim.last_retention + MINUTE {
            sim.retention_pass();
        }
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
