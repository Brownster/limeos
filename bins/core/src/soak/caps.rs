//! The states normal traffic never reaches, built directly and checked in
//! seconds: the core store at and past its audit cap, a retention backlog far
//! larger than one batch, the receipt store at its row and size caps, and
//! core's open with the jobs a busy household keeps (5,120, the household
//! runs keep at most about 1,600).
//!
//! Separate tests rather than phases of the 90-day run: each state is built
//! in one step (rows written with SQL where only their number matters), so a
//! failure names the one state that broke, and the 90-day run keeps its
//! household shape and its run time. Each must open, prune in bounded
//! batches and keep writing, or refuse only where the stores document it:
//! core's startup when recovery needs room past the reserve and nothing can
//! be pruned; core's ordinary writes at the cap until a pass frees room; the
//! receipt store's last-resort row and file caps while nothing is prunable.
use super::{
    engine::{HOST, container_id},
    fixtures::{SESSION, add_events, core_store, value, writer},
    sim::{JobRecord, Kind, Sim, rule_limits, soak_dir},
    support::{DAY, HOUR, MINUTE, START, rfc3339},
};
use limeos_contracts::VERSION;
use limeos_domain::{
    ContainerAction, ContainerSnapshot, Error, ErrorCode, EventKind, JobState, Principal,
    RestartRequest, StackSnapshot,
};
use limeos_executor_container::limits;
use limeos_executor_protocol::Request;
use limeos_persistence::{AuditLevel, AuditUsage, LoginRecord, Store, test_support};
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Events at the audit cap, and in the reserve past it.
const CAP: i64 = AuditUsage::LIMIT / test_support::EVENT_BYTES;
const RESERVE: i64 = test_support::AUDIT_RESERVE / test_support::EVENT_BYTES;

fn container(name: &str) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", container_id(name)),
        image: format!("sha256:{}", limeos_identity::digest(name)),
        started_at: rfc3339(START - DAY, 0),
        running: true,
    }
}

/// Queues a container job and, with `claim`, claims it (`running`, as a
/// crash during dispatch leaves it). Returns its id.
fn job(store: &mut Store, admin: &Principal, name: &str, claim: bool) -> String {
    let snapshot = container(name);
    let planned = store
        .plan_container(admin, ContainerAction::Restart, &snapshot, START)
        .unwrap();
    let approval = store
        .approve_container(admin, &planned.plan.id, &planned.digest, START)
        .unwrap();
    let queued = store
        .queue_container(admin, name, &planned, &approval.token, &snapshot, START)
        .unwrap();
    if claim {
        let epoch = limeos_identity::opaque().unwrap();
        store.observe_executor_epoch(&epoch, START).unwrap();
        store
            .claim_container(admin, &queued.id, &snapshot, &epoch, START)
            .unwrap();
    }
    queued.id
}

fn record(store: &Store) -> LoginRecord {
    store.login_record("marc").unwrap().unwrap()
}

/// A login, as `Core::login_checked` writes it once the password passed.
fn login(store: &mut Store, token: &str, now: i64) -> limeos_domain::Result<()> {
    let digest = limeos_identity::digest(token);
    store
        .create_session(&record(store), None, &digest, &digest, now)
        .map(drop)
}

/// The base of every cap case: a store with one session (to revoke at the
/// cap) and a job, `running` or only `queued`; then `events` audited events
/// in all, the bulk either prunable session events or pinned to the job
/// (its history is kept while it is unsettled). No other prunable row.
fn at_level(dir: &Path, events: i64, prunable: bool, running: bool) -> (PathBuf, String) {
    let (path, admin) = core_store(dir);
    let mut store = Store::open(&path).unwrap();
    login(&mut store, "kept-session", START).unwrap();
    let job = job(&mut store, &admin, "pihole", running);
    drop(store);
    // Only the job's history and the enrolment's authority events remain:
    // nothing prunable but what the case adds.
    let conn = writer(&path);
    conn.execute(
        "DELETE FROM events WHERE job IS NULL AND json_extract(kind,'$.kind') NOT IN ('bootstrap_issued','administrator_enrolled')",
        [],
    )
    .unwrap();
    drop(conn);
    let have = value(&path, "SELECT count(*) FROM events");
    let pinned = (!prunable).then_some(job.as_str());
    add_events(&path, &admin.id, pinned, SESSION, events - have, START);
    (path, job)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Level {
    AtCap,
    PastCap,
    ReserveFull,
}

/// One cap case, judged; returns what failed.
fn cap_case(level: Level, prunable: bool, running: bool) -> Vec<String> {
    let dir = soak_dir();
    let events = match level {
        Level::AtCap => CAP,
        Level::PastCap => CAP + RESERVE / 2,
        Level::ReserveFull => CAP + RESERVE,
    };
    let (path, job) = at_level(dir.path(), events, prunable, running);
    let case = format!(
        "{level:?}, {} history, {}",
        if prunable { "prunable" } else { "pinned" },
        if running {
            "a running job"
        } else {
            "no running job"
        }
    );
    let mut problems = Vec::new();
    let generation = value(&path, "SELECT generation FROM meta");
    // The one documented refusal: recovery needs room past the reserve and
    // nothing can be pruned. It must leave the store as it was.
    let refuses = level == Level::ReserveFull && running && !prunable;
    let mut store = match (Store::open(&path), refuses) {
        (Err(Error(ErrorCode::StateNotDurable)), true) => {
            let unchanged = value(&path, "SELECT count(*) FROM events") == events
                && value(&path, "SELECT generation FROM meta") == generation
                && value(&path, "SELECT count(*) FROM jobs WHERE state='running'") == 1;
            if !unchanged {
                problems.push(format!("{case}: the refused open changed the store"));
            }
            return problems;
        }
        (Ok(_), true) => {
            return vec![format!(
                "{case}: opened, though recovery has no room and nothing is prunable"
            )];
        }
        (Err(error), _) => {
            return vec![format!("{case}: the store refused to open: {error:?}")];
        }
        (Ok(store), false) => store,
    };
    if running && store.job_state(&job).unwrap() != "needs_intervention" {
        problems.push(format!("{case}: the interrupted job was not recovered"));
    }
    // Revocation always works, whatever the cap.
    let revoked = store.revoke_session(&limeos_identity::digest("kept-session"), START);
    if revoked.is_err() {
        problems.push(format!(
            "{case}: revocation refused at the cap: {revoked:?}"
        ));
    }
    let full = store.audit_usage().unwrap().level() == AuditLevel::Full;
    let first = login(&mut store, "first", START + MINUTE);
    match (full, &first) {
        (true, Err(Error(ErrorCode::StateNotDurable))) | (false, Ok(())) => {}
        _ => problems.push(format!(
            "{case}: an ordinary write at audit level {:?} answered {first:?}",
            store.audit_usage().unwrap().level()
        )),
    }
    // Retention, as the writer runs it: bounded batches, each followed by
    // an ordinary write, which must succeed once usage is below the cap and
    // be refused (`StateNotDurable`) while it is not.
    let (budget, batch) = rule_limits("events");
    let pinned = |path: &Path| {
        value(
            path,
            &format!("SELECT count(*) FROM events WHERE job='{job}'"),
        )
    };
    let kept = pinned(&path);
    let epoch = Instant::now();
    let mut passes = 0;
    for minute in 2.. {
        let now = START + minute * MINUTE;
        let at = epoch + Duration::from_secs((minute * MINUTE) as u64);
        let pass = test_support::maintain(&mut store, at, now).expect("a pass is due each minute");
        if !pass.failed.is_empty() {
            problems.push(format!(
                "{case}: a retention rule failed: {:?}",
                pass.failed
            ));
            break;
        }
        let removed = pass
            .removed
            .iter()
            .find(|(rule, _)| *rule == "events")
            .map_or(0, |(_, rows)| *rows);
        if removed > batch {
            problems.push(format!(
                "{case}: one batch removed {removed} events (limit {batch})"
            ));
        }
        if removed > 0 {
            passes += 1;
        }
        let level = store.audit_usage().unwrap().level();
        let write = login(&mut store, &format!("after pass {passes}"), now);
        match (level, &write) {
            (AuditLevel::Full, Err(Error(ErrorCode::StateNotDurable))) => {}
            (AuditLevel::Full, _) => problems.push(format!(
                "{case}: a write at the cap after pass {passes} answered {write:?}"
            )),
            (_, Ok(())) => {}
            (_, Err(error)) => {
                problems.push(format!(
                    "{case}: a login below the cap after pass {passes} was refused: {error:?}"
                ));
                break;
            }
        }
        // Done once a pass leaves nothing over budget, or frees nothing.
        if removed < batch || passes > 400 {
            break;
        }
    }
    if pinned(&path) != kept {
        problems.push(format!(
            "{case}: retention removed history pinned to an unsettled job"
        ));
    }
    let audited = store.audit_usage().unwrap().bytes / test_support::EVENT_BYTES;
    let rows = value(&path, "SELECT count(*) FROM events");
    if audited != rows {
        problems.push(format!(
            "{case}: the counter says {audited} events, the table holds {rows}"
        ));
    }
    if prunable {
        // Worked off down to the budget, in the passes that takes.
        let needed = (events - budget) / batch + 2;
        if rows > budget + 1 || passes > needed {
            problems.push(format!(
                "{case}: {passes} passes left {rows} events (budget {budget}, at most {needed} passes of {batch} expected)"
            ));
        }
    }
    problems
}

/// The core store at, past and full past its audit cap, with and without
/// prunable history, with and without a job needing a recovery event.
#[test]
#[ignore = "the cap states of the core store: in CI's soak job (about 20 s, up to 70 MiB)"]
fn soak_core_store_at_its_audit_cap() {
    let mut problems = Vec::new();
    for level in [Level::AtCap, Level::PastCap, Level::ReserveFull] {
        for prunable in [true, false] {
            for running in [false, true] {
                problems.extend(cap_case(level, prunable, running));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "core store at its cap:\n{}",
        problems.join("\n")
    );
}

/// Retention suspended during a burst of real writes, then resumed: every
/// pass removes exactly one batch until the backlog is gone, and the logins
/// between passes all succeed.
#[test]
fn soak_core_retention_works_off_a_backlog() {
    let dir = soak_dir();
    let (path, admin) = core_store(dir.path());
    let (budget, batch) = rule_limits("events");
    add_events(&path, &admin.id, None, SESSION, budget, START);
    let mut store = Store::open(&path).unwrap();
    // The burst: logins and logouts, two events each, no retention pass.
    let backlog = 12 * batch;
    for i in 0..backlog / 2 {
        let token = format!("burst-{i}");
        login(&mut store, &token, START + 1).unwrap();
        store
            .revoke_session(&limeos_identity::digest(&token), START + 1)
            .unwrap();
    }
    let epoch = Instant::now();
    let mut excess = store.audit_usage().unwrap().bytes / test_support::EVENT_BYTES - budget;
    assert!(excess >= backlog, "the burst built no backlog");
    let mut passes = 0;
    while excess > 0 {
        passes += 1;
        let at = epoch + Duration::from_secs((passes * MINUTE) as u64);
        let pass = test_support::maintain(&mut store, at, START + 1 + passes * MINUTE)
            .expect("a pass is due each minute");
        assert!(
            pass.failed.is_empty(),
            "a retention rule failed: {:?}",
            pass.failed
        );
        let removed: i64 = pass
            .removed
            .iter()
            .filter(|(rule, _)| *rule == "events")
            .map(|(_, rows)| rows)
            .sum();
        assert_eq!(
            removed,
            excess.min(batch),
            "pass {passes} removed {removed} events with {excess} over budget: each transaction removes one batch of at most {batch}"
        );
        excess -= removed;
        if excess > 0 {
            // An ordinary write between passes (its event joins the backlog).
            login(
                &mut store,
                &format!("between-{passes}"),
                START + passes * MINUTE,
            )
            .expect("a login while the backlog is worked off");
            excess += 1;
        }
        assert!(
            passes <= backlog / batch + 2,
            "the backlog is not shrinking"
        );
    }
    let rows = value(&path, "SELECT count(*) FROM events");
    assert_eq!(
        store.audit_usage().unwrap().bytes,
        rows * test_support::EVENT_BYTES,
        "the counter drifted from the table while working off the backlog"
    );
}

/// Rows of a receipt store filled directly: `rows` standalone `verified`
/// receipts expiring at `expires`, each with a receipt body of `body` bytes.
fn fill_receipts(dir: &Path, rows: i64, expires: i64, body: i64) {
    let conn = writer(&dir.join("receipts.sqlite"));
    conn.execute(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1) INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at,legacy) SELECT lower(hex(randomblob(32))),lower(hex(randomblob(32))),'container:'||lower(hex(randomblob(32))),'verified',hex(randomblob(?2)),'restart',?3,0 FROM n",
        rusqlite::params![rows, body / 2, expires],
    )
    .unwrap();
}

/// Fills a receipt store until SQLite refuses another row under the
/// store's own page cap (`limits::MAX_PAGES`): large rows, then small
/// ones, all expiring at `expires`.
fn fill_to_cap(dir: &Path, expires: i64) {
    let path = dir.join("receipts.sqlite");
    fill_receipts(dir, 3_600, expires, 2_900);
    let conn = writer(&path);
    conn.execute_batch(&format!("PRAGMA max_page_count={}", limits::MAX_PAGES))
        .unwrap();
    for body in [1_000, 200, 16] {
        while conn
            .execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at,legacy) VALUES(lower(hex(randomblob(32))),lower(hex(randomblob(32))),'container:'||lower(hex(randomblob(32))),'verified',hex(randomblob(?1)),'restart',?2,0)",
                rusqlite::params![body / 2, expires],
            )
            .is_ok()
        {}
    }
}

fn receipts(sim: &Sim) -> (i64, u64) {
    let path = sim.exec.dir.join("receipts.sqlite");
    let rows = value(&path, "SELECT count(*) FROM actions");
    (rows, std::fs::metadata(&path).unwrap().len())
}

/// Plans, approves and queues a restart of container `name` as the first
/// administrator, through the store (the plan is the workload's business
/// elsewhere; here only the receipt store's state matters). Returns it.
async fn queued(sim: &mut Sim, name: &str, key: String) -> limeos_domain::RestartJob {
    let snapshot = sim.world.snapshot(&container_id(name)).unwrap();
    let admin = sim.users[0].principal.clone();
    let now = sim.now();
    sim.call(move |s| {
        let planned = s.plan_container(&admin, ContainerAction::Restart, &snapshot, now)?;
        let approval = s.approve_container(&admin, &planned.plan.id, &planned.digest, now)?;
        s.queue_container(&admin, &key, &planned, &approval.token, &snapshot, now)
    })
    .await
    .unwrap()
}

/// A container job through core's dispatcher and the executor: queued, then
/// one dispatcher pass. Returns the job's state after it.
async fn run_job(sim: &mut Sim, n: usize) -> String {
    let names = ["pihole", "syncthing", "uptime-kuma"];
    let job = queued(sim, names[n % names.len()], format!("cap-{n}")).await;
    sim.ledger.insert(
        job.id.clone(),
        JobRecord {
            kind: Kind::Container,
            operation: ContainerAction::Restart,
            targets: 1,
            fault: None,
            declared: None,
            user: 0,
            scripted: false,
        },
    );
    super::workload::tick(sim).await;
    let id = job.id;
    sim.call(move |s| s.job_state(&id)).await.unwrap()
}

/// The execute itself, as core sends it for a claimed job (through core's
/// own `exchange`): the executor's refusal, if any.
async fn execute(sim: &mut Sim, n: usize) -> Option<ErrorCode> {
    // A different container each time: the job is left claimed, holding
    // its lock in both stores.
    let job = queued(sim, HOST[n].0, format!("raw-{n}")).await;
    let core = sim.core();
    let epoch = core.receipt_epoch().await.unwrap();
    let (admin, now) = (sim.users[0].principal.clone(), sim.now());
    let snapshot = sim.world.snapshot(&container_id(HOST[n].0)).unwrap();
    let job = sim
        .call(move |s| s.claim_container(&admin, &job.id, &snapshot, &epoch, now))
        .await
        .unwrap();
    let request = Request::ExecuteContainer {
        version: VERSION,
        request: RestartRequest {
            action: job.id.clone(),
            plan_digest: limeos_identity::digest(&serde_json::to_string(&job.plan).unwrap()),
            plan: job.plan.clone(),
        },
    };
    let answer = core.exchange(request, Duration::from_secs(38)).await;
    sim.absorb().await;
    answer.err().map(|failure| failure.error.0)
}

/// A Sim whose receipt store was filled before the executor started.
async fn receipt_sim(name: &'static str, fill: impl FnOnce(&Path)) -> Sim {
    let mut sim = Sim::new(name, 0x5eed_0c47, None);
    sim.start_executor("creating the store");
    sim.exec.stop().await;
    fill(&sim.exec.dir.clone());
    super::workload::bootstrap(&mut sim).await;
    sim
}

/// The receipt store at its row cap. With every row prunable, each execute
/// prunes at most one batch and succeeds, until the backlog is worked off.
/// With none prunable (too young), the execute is refused `Overloaded`, the
/// documented last resort, and succeeds once the rows have aged.
#[tokio::test(flavor = "current_thread")]
async fn soak_receipt_store_at_its_caps() {
    let mut problems = Vec::new();
    let row_cap = i64::from(limits::MAX_ACTION_ROWS);
    let (retained, prune_batch) = (
        i64::from(limits::RETAINED_ACTIONS),
        i64::from(limits::PRUNE_BATCH),
    );
    // Row cap, prunable: expired well past the retention margin.
    let mut sim = receipt_sim("receipts-row-cap-prunable", |dir| {
        fill_receipts(dir, row_cap, START - 2 * DAY, 64)
    })
    .await;
    let mut previous = receipts(&sim).0;
    for n in 0.. {
        sim.advance(sim.now() + MINUTE);
        let state = run_job(&mut sim, n).await;
        let rows = receipts(&sim).0;
        if state != "succeeded" {
            problems.push(format!(
                "row cap, prunable: job {n} ended {state} with {previous} rows"
            ));
            break;
        }
        if previous + 1 - rows > prune_batch {
            problems.push(format!(
                "row cap, prunable: one insert pruned {} rows (limit {prune_batch})",
                previous + 1 - rows
            ));
        }
        if rows <= retained + 1 {
            break;
        }
        if rows >= previous || n > 200 {
            problems.push(format!(
                "row cap, prunable: stuck at {rows} rows after {n} jobs"
            ));
            break;
        }
        previous = rows;
    }
    // Row cap, nothing prunable: the plans expire in ten minutes.
    let mut sim = receipt_sim("receipts-row-cap-young", |dir| {
        fill_receipts(dir, row_cap, START + 600, 64)
    })
    .await;
    sim.advance(START + MINUTE);
    match execute(&mut sim, 0).await {
        Some(ErrorCode::Overloaded) => {}
        other => problems.push(format!(
            "row cap, nothing prunable: the execute answered {other:?}, not the documented Overloaded"
        )),
    }
    sim.advance(START + 600 + HOUR + MINUTE);
    if let Some(code) = execute(&mut sim, 1).await {
        problems.push(format!(
            "row cap, aged rows: the execute was still refused: {code:?}"
        ));
    }
    // File cap: a store at its 4096 pages, every row prunable, keeps
    // executing; with none prunable the insert fails `StateNotDurable`.
    for (name, expires) in [("prunable", START - 2 * DAY), ("young", START + 600)] {
        let mut sim = receipt_sim("receipts-file-cap", |dir| {
            fill_to_cap(dir, expires);
        })
        .await;
        let (_, before) = receipts(&sim);
        sim.advance(START + MINUTE);
        let answers: Vec<Option<ErrorCode>> = {
            let mut answers = Vec::new();
            for n in 0..16 {
                answers.push(execute(&mut sim, n).await);
                sim.advance(sim.now() + MINUTE);
            }
            answers
        };
        let (_, after) = receipts(&sim);
        let refused: Vec<_> = answers.iter().flatten().collect();
        match name {
            "prunable" if !refused.is_empty() => problems.push(format!(
                "file cap, prunable: executes refused {refused:?} in a {:.1} MiB store",
                super::support::mib(before)
            )),
            "young" if !refused.iter().all(|c| **c == ErrorCode::StateNotDurable) || refused.is_empty() => {
                problems.push(format!(
                    "file cap, nothing prunable: answers {answers:?}, not the documented StateNotDurable once full"
                ))
            }
            _ => {}
        }
        if after > limits::MAX_FILE_BYTES {
            problems.push(format!(
                "file cap, {name}: the file grew to {:.1} MiB, past the 16 MiB its open refuses",
                super::support::mib(after)
            ));
        }
        // It still opens.
        sim.exec.stop().await;
        if let Err(error) = sim.exec.start() {
            problems.push(format!(
                "file cap, {name}: the store no longer opens: {error:?}"
            ));
        }
    }
    // Past the file cap, the open refuses, as documented.
    let dir = soak_dir();
    let store_dir = dir.path().join("receipts");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&store_dir)
        .unwrap();
    drop(limeos_executor_container::ReceiptStore::open(&store_dir).unwrap());
    fill_receipts(&store_dir, 5_800, START - 2 * DAY, 2_900);
    if std::fs::metadata(store_dir.join("receipts.sqlite"))
        .unwrap()
        .len()
        <= limits::MAX_FILE_BYTES
    {
        problems.push("file cap: the oversized fixture is not past 16 MiB".into());
    }
    match limeos_executor_container::ReceiptStore::open(&store_dir) {
        Err(Error(ErrorCode::StateNotDurable)) => {}
        other => problems.push(format!(
            "past the file cap: the open answered {:?}, not the documented StateNotDurable",
            other.map(|_| ())
        )),
    }
    assert!(
        problems.is_empty(),
        "receipt store at its caps:\n{}",
        problems.join("\n")
    );
}

/// Jobs job-row retention keeps at 170 a day for its 30 days: the operating
/// point `startup_at_high_retained_job_count` reaches in 40 simulated days
/// (5,102 to 5,116 retained), built here in seconds.
const RETAINED_JOBS: i64 = 5_120;

/// A core store holding `jobs` settled jobs, as the store of a busy
/// household looks once job-row retention has kept 30 days of them: half
/// container restarts and half restarts of a four-container stack (at
/// 5,113 jobs the restart scenario kept 2,610 container and 2,503 stack
/// jobs, the stack jobs with 4.9 resources each), each with its plan,
/// result, resource set and four events (queued, running, verifying,
/// succeeded), and session events up to the events budget. One job of each
/// kind goes through the store (plan, approval, queue); its rows are the
/// pattern for the rest, written with SQL: startup walks jobs and their
/// resource sets, it does not interpret container or stack jobs. The ids
/// are digests, as random as real ones (an index probed in their order
/// finds no locality).
fn retained_jobs_store(dir: &Path, jobs: i64) -> PathBuf {
    let (path, admin) = core_store(dir);
    let mut store = Store::open(&path).unwrap();
    let container_job = job(&mut store, &admin, "pihole", false);
    let members = ["jellyfin", "sonarr", "radarr", "prowlarr"]
        .iter()
        .map(|name| container(name))
        .collect();
    let stack = StackSnapshot::new("media", members).unwrap();
    let proposal = store
        .plan_stack(
            &admin,
            ContainerAction::Restart,
            "registered",
            &stack,
            START,
        )
        .unwrap();
    let approval = store
        .approve_stack(&admin, &proposal.plan.id, &proposal.digest, START)
        .unwrap();
    let stack_job = store
        .queue_stack(&admin, "stack", &proposal, &approval.token, &stack, START)
        .unwrap()
        .id;
    drop(store);
    let mut conn = writer(&path);
    let tx = conn.transaction().unwrap();
    let kinds: Vec<String> = [
        EventKind::JobQueued,
        EventKind::JobTransition {
            state: JobState::Running,
        },
        EventKind::JobTransition {
            state: JobState::Verifying,
        },
        EventKind::JobTransition {
            state: JobState::Succeeded,
        },
    ]
    .iter()
    .map(|kind| serde_json::to_string(kind).unwrap())
    .collect();
    {
        let mut job_row = tx
            .prepare(
                "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
                 SELECT ?2,principal,?3,intent,digest,resource,'queued',generation,revision,deadline FROM jobs WHERE id=?1",
            )
            .unwrap();
        // The resources beyond the primary one (which the insert trigger
        // adds), while the copy is queued, as the queue path adds them.
        let mut resources = tx
            .prepare(
                "INSERT INTO job_resources(job,resource) SELECT ?2,r.resource FROM job_resources r JOIN jobs j ON j.id=r.job WHERE r.job=?1 AND r.resource<>j.resource",
            )
            .unwrap();
        let mut copies = Vec::new();
        for table in ["container", "stack"] {
            let plan = tx
                .prepare(&format!(
                    "INSERT INTO {table}_plans(id,principal,body,digest,revision,expires,approval_digest,job,canceled)
                     SELECT ?2,principal,body,digest,revision,expires,approval_digest,?3,0 FROM {table}_plans WHERE job=?1"
                ))
                .unwrap();
            let result = tx
                .prepare(&format!(
                    "INSERT INTO {table}_results(job,receipt,verification,recorded)
                     SELECT ?2,body,NULL,?3 FROM {table}_plans WHERE job=?1"
                ))
                .unwrap();
            copies.push((plan, result));
        }
        let mut event = tx
            .prepare("INSERT INTO events(principal,job,kind,created) VALUES(?,?,?,?)")
            .unwrap();
        for n in 0..jobs {
            let (template, (plan, result)) = if n % 2 == 0 {
                (&container_job, &mut copies[0])
            } else {
                (&stack_job, &mut copies[1])
            };
            let id = limeos_identity::digest(&format!("retained job {n}"));
            let at = START + n * 30 * DAY / jobs;
            job_row
                .execute(rusqlite::params![template, id, format!("retained-{n}")])
                .unwrap();
            resources.execute(rusqlite::params![template, id]).unwrap();
            plan.execute(rusqlite::params![
                template,
                limeos_identity::digest(&format!("retained plan {n}")),
                id
            ])
            .unwrap();
            result.execute(rusqlite::params![template, id, at]).unwrap();
            for (step, kind) in kinds.iter().enumerate() {
                event
                    .execute(rusqlite::params![admin.id, id, kind, at + step as i64])
                    .unwrap();
            }
        }
    }
    tx.execute(
        "UPDATE jobs SET state='succeeded' WHERE idempotency LIKE 'retained-%'",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    drop(conn);
    let (budget, _) = rule_limits("events");
    let have = value(&path, "SELECT count(*) FROM events");
    add_events(&path, &admin.id, None, SESSION, budget - have, START);
    path
}

/// Core's open at the high retained-job operating point, on every CI run:
/// within the startup bound per page of file, every statement's walks
/// allowed and explained, as on every nightly copy of the household runs.
/// Since 24 November startup walks `jobs` once (`JOB_WALK` in
/// `limeos_persistence`'s `resources.rs`); the eight owners' walks it
/// replaced read 3.83 pages per page of file here, past the bound.
#[test]
#[ignore = "core's open at 5,120 retained jobs: in CI's soak job (a few seconds, about 60 MiB)"]
fn soak_core_startup_at_high_retained_job_count() {
    let dir = soak_dir();
    let path = retained_jobs_store(dir.path(), RETAINED_JOBS);
    let copy = dir.path().join("startup-copy");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&copy)
        .unwrap();
    let (start, refused) = super::startup::core_copy(&path, &copy, START);
    assert_eq!(refused, None, "the store refused to open");
    let trace = start.trace.as_ref().expect("the open's trace");
    let jobs = trace.rows_of("jobs");
    let pages = (start.file + start.wal) / super::checks::PAGE;
    eprintln!(
        "{jobs} jobs, {pages} pages of file and WAL, {} cold startup reads, {:.2} per page (bound factor {:.1}), {} ms",
        trace.cold_but_defect(),
        trace.cold_but_defect() as f64 / pages as f64,
        super::checks::CORE_PAGE_FACTOR as f64 / 1000.0,
        start.opened.elapsed.as_millis()
    );
    let mut statements: Vec<_> = trace.statements.iter().collect();
    statements.sort_by_key(|s| std::cmp::Reverse(s.cold.unwrap_or(s.reads)));
    for statement in statements.iter().take(6) {
        eprintln!(
            "  {:>7} cold, {:>9} VM steps, {:>6} full-scan steps: {}",
            statement.cold.unwrap_or(statement.reads),
            statement.vm_steps,
            statement.full_scans,
            statement.short()
        );
    }
    assert!(jobs >= RETAINED_JOBS as u64, "the store holds {jobs} jobs");
    let problems = super::checks::start_problems("core", &start, super::checks::CORE_OPEN_BOUND_MS);
    assert!(
        problems.is_empty(),
        "core's open at {jobs} retained jobs:\n{}",
        problems
            .iter()
            .map(|(check, problem)| format!("{check}: {problem}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
