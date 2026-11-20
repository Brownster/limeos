//! What the soak measures and asserts. Per-step checks record a violation as
//! they happen; the end of a run evaluates every check, prints the report and
//! fails on any check that failed, unless it is listed in
//! [`EXPECTED_FAILURES`] with its explanation (and fails if a listed one
//! passes, so a fixed defect cannot stay marked).
use crate::{
    sim::Sim,
    support::{DAY, Io, MINUTE, START, mib, slope},
    workload::{Profile, tick},
};
use rusqlite::{Connection, OpenFlags};
use std::{collections::BTreeMap, path::Path};

pub const REFUSALS: &str = "no lifetime or unexpected refusal";
pub const INVARIANTS: &str = "locks, ledger and retained history consistent after every restart";
pub const SIZE: &str = "database files under their bounds";
pub const BUDGETS: &str = "every table within its budget";
pub const FLAT: &str = "sizes and bounded tables flat across the last third";
pub const STARTUP: &str = "startup under its bounds";
pub const STARTUP_TREND_CORE: &str = "core startup work leaves at least five years";
pub const STARTUP_TREND_RECEIPTS: &str = "receipt-store startup work leaves at least five years";
pub const IDLE: &str = "an idle hour writes nothing";
pub const SETTLED: &str = "every job settled; none lost, duplicated or stranded";
pub const EFFECTS: &str = "every Engine effect ran at most once and matches its job";
pub const AUTHORITY: &str = "authority events all kept";
pub const PROVOKED: &str = "provoked refusals observed";
pub const PROJECTION: &str = "slow growth leaves at least five years";

/// Checks known to fail, with the defect that makes them fail and its
/// evidence. Both entries are one defect: the core store has no size bound.
const EXPECTED_FAILURES: &[(&str, &str)] = &[
    (
        PROJECTION,
        "core's per-job tables (jobs, job_resources, container_plans, container_results, \
         stack_plans, stack_results, job_resolutions) are never pruned: about 5 KiB per job, \
         about 50 KiB a day at the household rate over days 60 to 90, while events stay flat at \
         their budget; core.sqlite grows from 31 MiB (day 27) to 35 MiB (day 89) and passes its \
         80 MiB bound in about 2.5 years. Needs job-row retention (with their plans, results, \
         resources and completed resolutions) before core.sqlite is bounded",
    ),
    (
        STARTUP_TREND_CORE,
        "the same per-job growth: `Store::open` reads about 2.5 times the file (on the final \
         store quick_check reads 55 MiB, the foreign-key check 20 MiB and the audit count \
         4 MiB of a 35 MiB file), and its reads grow faster than the file: 75 MiB on day 27 and \
         93 MiB on day 89 while the file grew 4 MiB, about 0.3 MiB more each day; at that rate \
         an open reaches the 352 MiB read bound of a store at its file bound in about 2.4 years",
    ),
];

/// Core's file and WAL: the events table at its 131,072-row budget is about
/// 40 MiB of rows and indexes; the audit cap (twice that) is the hard stop.
pub const CORE_FILE_BOUND: u64 = 80 << 20;
pub const CORE_WAL_BOUND: u64 = 8 << 20;
/// Half the receipt store's own cap (`max_page_count` 4096 and the 16 MiB
/// file check in `ReceiptStore::open_at`): at the cap every execute fails.
pub const RECEIPT_FILE_BOUND: u64 = 8 << 20;
pub const RECEIPT_WAL_BOUND: u64 = 2 << 20;
/// Debug builds on a shared CI runner; a Pi in release is far faster.
pub const CORE_OPEN_BOUND_MS: u128 = 5_000;
/// Bytes an open may read per byte of database and WAL. `quick_check` reads
/// every page once, the foreign-key check reads every child table (the
/// events table, most of the file) again and the audit reconciliation
/// counts the events through an index: about 2.6 times the file, measured.
pub const STARTUP_READ_FACTOR: u64 = 4;
pub const RECEIPT_OPEN_BOUND_MS: u128 = 2_000;
/// `retention::EVENT_BUDGET / EVENT_BYTES`.
pub const EVENT_BUDGET: i64 = 131_072;

/// A table's budget.
#[derive(Clone, Copy)]
pub enum Budget {
    /// At most this many rows, at every checkpoint.
    Rows(i64),
    /// Meant to grow slowly with use, and why. Reported with its growth rate
    /// and the time until the core file reaches its bound at that rate.
    Grows(&'static str),
}

const PER_JOB: &str = "one row per job (or per queued plan, result, resource or completed resolution): \
     job rows and their plans and results are kept as the snapshot of each job when its event history \
     is pruned (`retention.rs`: the job row itself is never removed)";

/// Every core table: walking `sqlite_master` makes a table added later fail
/// the soak until it is given a budget here.
fn core_budget(table: &str) -> Option<Budget> {
    use Budget::*;
    Some(match table {
        // The retention budget, plus one batch of arrivals past it.
        "events" => Rows(EVENT_BUDGET + 2048),
        // 16 per user (`create_session` evicts the oldest), two users.
        "sessions" => Rows(32),
        "users" => Rows(2),
        "grants" => Rows(10),
        "meta" => Rows(1),
        "bootstrap" => Rows(1),
        // One per locked resource: 18 containers and 4 stacks.
        "resource_locks" => Rows(22),
        "jobs" | "job_resources" | "container_plans" | "container_results" | "stack_plans"
        | "stack_results" | "job_resolutions" => Grows(PER_JOB),
        // Never written in the shipped scope (no producer, no task tokens,
        // no storage or compose effects, legacy tables migrated empty).
        "task_tokens"
        | "budgets"
        | "reservations"
        | "resources"
        | "restart_plans"
        | "restart_results"
        | "compose_plans"
        | "storage_plans"
        | "storage_target_plans"
        | "storage_target_results"
        | "backup_preparation_plans"
        | "storage_mount_plans"
        | "storage_mount_results"
        | "automation_schedules"
        | "automation_occurrences"
        | "automation_streams"
        | "automation_observation_results"
        | "automation_alerts"
        | "automation_notifications"
        | "automation_health_policies"
        | "automation_poll_origins" => Rows(0),
        _ => return None,
    })
}

/// Every receipt-store table.
fn receipt_budget(table: &str) -> Option<Budget> {
    use Budget::*;
    Some(match table {
        // 1,024 standalone receipts kept, each of 256 stacks keeps up to 16
        // member rows (4 stacks here: at most 6 members), plus one prune
        // batch and the rows still holding locks.
        "actions" => Rows(1024 + 256 * 6 + 64 + 32),
        "stack_actions" => Rows(256 + 64 + 8),
        "retention" => Rows(1),
        "pruned_legacy" => Rows(0),
        _ => return None,
    })
}

#[derive(Clone, Debug, Default)]
pub struct StoreStat {
    /// Rows and bytes (tables with their indexes, from `dbstat`) per table.
    pub tables: BTreeMap<String, (i64, i64)>,
    pub file: u64,
    pub wal: u64,
}
impl StoreStat {
    fn rows(&self, table: &str) -> i64 {
        self.tables.get(table).map_or(0, |t| t.0)
    }
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub at: i64,
    pub core: StoreStat,
    pub receipts: StoreStat,
}

/// A read-only side connection: it can neither write nor checkpoint.
pub fn reader(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn stat(path: &Path) -> StoreStat {
    let conn = reader(path).expect("side connection");
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let bytes: BTreeMap<String, i64> = conn
        .prepare("SELECT m.tbl_name,sum(d.pgsize) FROM dbstat d JOIN sqlite_master m ON m.name=d.name GROUP BY m.tbl_name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let tables = names
        .into_iter()
        .map(|name| {
            let rows: i64 = conn
                .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
                    r.get(0)
                })
                .unwrap();
            let size = bytes.get(&name).copied().unwrap_or(0);
            (name, (rows, size))
        })
        .collect();
    let len = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    StoreStat {
        tables,
        file: len(path),
        wal: len(Path::new(&wal)),
    }
}

/// Reads of the startup checks' biggest steps, each on a fresh read-only
/// connection to the store as it is: to explain an expensive open.
pub fn probe_open(path: &Path) -> String {
    let steps = [
        ("quick_check", "PRAGMA quick_check"),
        (
            "foreign_key_check",
            "SELECT count(*) FROM pragma_foreign_key_check",
        ),
        ("count(events)", "SELECT count(*) FROM events"),
    ];
    steps
        .iter()
        .map(|(name, sql)| {
            let conn = reader(path).expect("side connection");
            let before = Io::now();
            let _: rusqlite::types::Value = conn.query_row(sql, [], |r| r.get(0)).unwrap();
            format!("{name} {:.1} MiB", mib(Io::now().since(before).rchar))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn receipts_path(sim: &Sim) -> std::path::PathBuf {
    sim.exec.dir.join("receipts.sqlite")
}

/// The daily checkpoint (04:00, the house asleep): sizes, row counts against
/// budgets, and the invariants. Both daemons then restart, measured.
pub fn checkpoint(sim: &mut Sim) {
    let sample = Sample {
        at: sim.now(),
        core: stat(&sim.core_path),
        receipts: stat(&receipts_path(sim)),
    };
    let mut problems = Vec::new();
    for (name, stat, file_bound, wal_bound) in [
        ("core.sqlite", &sample.core, CORE_FILE_BOUND, CORE_WAL_BOUND),
        (
            "receipts.sqlite",
            &sample.receipts,
            RECEIPT_FILE_BOUND,
            RECEIPT_WAL_BOUND,
        ),
    ] {
        if stat.file > file_bound || stat.wal > wal_bound {
            problems.push((
                SIZE,
                format!(
                    "{name}: file {:.1} MiB (bound {:.0}), WAL {:.1} MiB (bound {:.0})",
                    mib(stat.file),
                    mib(file_bound),
                    mib(stat.wal),
                    mib(wal_bound)
                ),
            ));
        }
    }
    let jobs = sample.core.rows("jobs");
    for (store, stat, budget) in [
        (
            "core",
            &sample.core,
            core_budget as fn(&str) -> Option<Budget>,
        ),
        ("receipts", &sample.receipts, receipt_budget),
    ] {
        for (table, (rows, _)) in &stat.tables {
            match budget(table) {
                None => problems.push((
                    BUDGETS,
                    format!(
                        "{store}.{table} ({rows} rows) has no budget: give it one in checks.rs"
                    ),
                )),
                Some(Budget::Rows(limit)) if *rows > limit => problems.push((
                    BUDGETS,
                    format!("{store}.{table}: {rows} rows over its budget of {limit}"),
                )),
                // A per-job table may hold at most a bounded number of rows
                // per job: 1 + 2 stack and up to 16 member resources.
                Some(Budget::Grows(_)) if table != "jobs" && *rows > 20 * jobs + 64 => problems
                    .push((
                        BUDGETS,
                        format!("{store}.{table}: {rows} rows for {jobs} jobs"),
                    )),
                _ => {}
            }
        }
    }
    let unqueued: i64 = reader(&sim.core_path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM container_plans WHERE job IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if unqueued > 200 {
        problems.push((
            BUDGETS,
            format!("core: {unqueued} unqueued container plans kept"),
        ));
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
    sim.samples.push(sample);
    invariants(sim, "daily checkpoint");
}

/// After every restart: the stores opened (else the run has already
/// stopped), and the invariants hold.
pub fn after_restart(sim: &mut Sim, reason: &'static str) {
    invariants(sim, reason);
}

const HELD: &str = "('running','verifying','outcome_unknown','needs_intervention')";
const ACTIVE: &str = "('prepared','effect_accepted','outcome_unknown')";

/// Locks agree with job states; every job core holds is one the workload
/// queued and none is missing; nothing the retention rules promise to keep
/// is gone; the executor holds no lock for a job core has released.
fn invariants(sim: &mut Sim, when: &'static str) {
    let core = reader(&sim.core_path).expect("core side connection");
    let count = |sql: &str| -> i64 { core.query_row(sql, [], |r| r.get(0)).unwrap() };
    let mut problems = Vec::new();
    let orphan = count(&format!(
        "SELECT count(*) FROM resource_locks l JOIN jobs j ON j.id=l.job WHERE j.state NOT IN {HELD}"
    ));
    let unlocked = count(&format!(
        "SELECT count(*) FROM jobs j WHERE j.state IN {HELD} AND NOT EXISTS(SELECT 1 FROM resource_locks l WHERE l.job=j.id AND l.resource=j.resource)"
    ));
    if orphan + unlocked > 0 {
        problems.push(format!(
            "{orphan} locks of released jobs, {unlocked} held jobs without their lock"
        ));
    }
    // Retention's promises: every unsettled job, and every job with an open
    // resolution request, keeps its whole history (its `job_queued` event,
    // which retention removes first).
    let unkept = count(
        r#"SELECT count(*) FROM jobs j WHERE (j.state NOT IN ('succeeded','failed','canceled','precondition_changed','resolved') OR EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NULL)) AND NOT EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND e.kind='{"kind":"job_queued"}')"#,
    );
    if unkept > 0 {
        problems.push(format!(
            "{unkept} unsettled or contested jobs lost history retention must keep"
        ));
    }
    let states: BTreeMap<String, String> = core
        .prepare("SELECT id,state FROM jobs")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let unknown = states
        .keys()
        .filter(|id| !sim.ledger.contains_key(*id))
        .count();
    let lost = sim
        .ledger
        .keys()
        .filter(|id| !states.contains_key(*id))
        .count();
    if unknown + lost > 0 {
        problems.push(format!(
            "{lost} queued jobs missing from the store, {unknown} jobs nobody queued"
        ));
    }
    if let Ok(receipts) = reader(&receipts_path(sim)) {
        let active = |sql: String| -> Vec<String> {
            receipts
                .prepare(&sql)
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .unwrap_or_default()
        };
        let mut locked = active(format!(
            "SELECT a.action FROM actions a WHERE a.state IN {ACTIVE} AND NOT EXISTS(SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)"
        ));
        locked.extend(active(format!(
            "SELECT action FROM stack_actions WHERE state IN {ACTIVE}"
        )));
        for action in locked {
            match states.get(&action) {
                Some(state) if HELD.contains(&format!("'{state}'")) => {}
                state => problems.push(format!(
                    "the executor holds a lock for job {} that core has as {state:?}",
                    &action[..12]
                )),
            }
        }
    }
    for problem in problems {
        sim.violation(INVARIANTS, format!("{when}: {problem}"));
    }
}

/// Nothing queued, held or pending.
pub fn settled(sim: &Sim) -> bool {
    let core = reader(&sim.core_path).expect("core side connection");
    let open: i64 = core
        .query_row(
            "SELECT (SELECT count(*) FROM jobs WHERE state IN ('queued','running','verifying','outcome_unknown','needs_intervention'))+(SELECT count(*) FROM job_resolutions WHERE confirmation IS NULL)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    open == 0
}

/// An hour of the night with nothing to do: once retention has nothing
/// left to remove, dispatcher passes (recovery polls of any held job
/// included) and retention passes must not write a byte to either store.
pub async fn idle_window(sim: &mut Sim) {
    for _ in 0..240 {
        sim.clock.set(sim.now() + MINUTE);
        let removed = sim.retention_pass();
        tick(sim).await;
        if removed == 0 {
            break;
        }
    }
    let restarts: u64 = sim.stats.restarts.values().sum();
    let start = sim.now();
    let before = Io::now();
    for _ in 0..60 {
        sim.clock.set(sim.now() + MINUTE);
        sim.retention_pass();
        tick(sim).await;
    }
    let io = Io::now().since(before);
    if sim.stats.restarts.values().sum::<u64>() != restarts || sim.executor_back_at.is_some() {
        // A fault armed on a held job fired: not an idle hour.
        return;
    }
    sim.idle.push((start, io));
    if io.syscw != 0 || io.wchar != 0 {
        sim.violation(
            IDLE,
            format!(
                "{} write calls, {} bytes written in an idle hour",
                io.syscw, io.wchar
            ),
        );
    }
}

fn third(samples: &[(f64, f64)], part: usize) -> &[(f64, f64)] {
    let n = samples.len();
    &samples[n * part / 3..n * (part + 1) / 3]
}
fn mean(points: &[(f64, f64)]) -> f64 {
    points.iter().map(|p| p.1).sum::<f64>() / points.len().max(1) as f64
}

/// The end of a run: the final checks, the report, and the verdict.
pub fn finish(sim: &mut Sim, profile: &Profile) {
    final_state(sim);
    if profile.trends {
        trends(sim);
    }
    startup_bounds(sim);
    report(sim, profile);
    verdict(sim, profile);
}

fn final_state(sim: &mut Sim) {
    let core = reader(&sim.core_path).expect("core side connection");
    let states: BTreeMap<String, String> = core
        .prepare("SELECT id,state FROM jobs")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let open: i64 = core
        .query_row(
            "SELECT count(*) FROM job_resolutions WHERE confirmation IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut problems = Vec::new();
    if open > 0 {
        problems.push((SETTLED, format!("{open} resolution requests still open")));
    }
    for (id, record) in &sim.ledger {
        let state = states.get(id).map(String::as_str).unwrap_or("missing");
        let effects = sim.exec.effects.get(id).copied().unwrap_or(0) as usize;
        if !matches!(
            state,
            "succeeded" | "failed" | "canceled" | "precondition_changed" | "resolved"
        ) {
            problems.push((
                SETTLED,
                format!(
                    "job {} ({:?} {:?}) left {state}",
                    &id[..12],
                    record.kind,
                    record.fault
                ),
            ));
        }
        let wrong = match state {
            "succeeded" => effects != record.targets,
            "precondition_changed" | "canceled" => effects != 0,
            "resolved" if record.declared == Some(limeos_domain::DeclaredOutcome::NotDone) => {
                effects != 0
            }
            _ => effects > record.targets,
        };
        if wrong {
            problems.push((
                EFFECTS,
                format!(
                    "job {} ({:?} {:?}, fault {:?}) is {state} after {effects} Engine effects for {} targets",
                    &id[..12],
                    record.kind,
                    record.operation,
                    record.fault,
                    record.targets
                ),
            ));
        }
    }
    // Authority events: far below their budget (4,096), so every one the
    // run caused must still be there.
    let authority: i64 = core
        .query_row(
            "SELECT count(*) FROM events WHERE json_extract(kind,'$.kind') IN ('bootstrap_issued','administrator_enrolled','legacy_identity_imported','grants_revised','job_resolution_requested','job_resolution_withdrawn','job_resolution_superseded')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let expected: u64 = [
        "authority: expected",
        "resolution requested",
        "authority: withdrawn",
        "authority: superseded",
    ]
    .iter()
    .map(|op| sim.stats.ops.get(op).copied().unwrap_or(0))
    .sum();
    if authority != expected as i64 {
        problems.push((
            AUTHORITY,
            format!("{authority} authority events kept, {expected} written"),
        ));
    }
    // The workload provokes these on purpose; each must have been seen, so
    // the run proves legitimate refusals are told apart from lifetime ones.
    for (op, code) in [
        ("login with a wrong password", "Unauthenticated"),
        ("login with an unknown name", "Unauthenticated"),
        ("approve plan", "Expired"),
        ("provoked: session used after it expired", "Unauthenticated"),
    ] {
        if !sim.stats.refused.contains_key(&(op, code.to_owned())) {
            problems.push((PROVOKED, format!("{op}: never refused with {code}")));
        }
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
}

/// The last third of a full run against the middle third.
fn trends(sim: &mut Sim) {
    let samples = sim.samples.clone();
    let day = |s: &Sample| (s.at - START) as f64 / DAY as f64;
    let mut problems = Vec::new();
    for (store, pick, budget) in [
        (
            "core",
            (|s: &Sample| &s.core) as fn(&Sample) -> &StoreStat,
            core_budget as fn(&str) -> Option<Budget>,
        ),
        ("receipts", |s: &Sample| &s.receipts, receipt_budget),
    ] {
        let last = &samples[samples.len() * 2 / 3..];
        let span = day(last.last().unwrap()) - day(&last[0]);
        let tables: Vec<String> = pick(last.last().unwrap()).tables.keys().cloned().collect();
        let (mut bounded, mut growing) = (Vec::new(), Vec::new());
        for s in last {
            let (mut b, mut g) = (0.0, 0.0);
            for (table, (_, bytes)) in &pick(s).tables {
                match budget(table) {
                    Some(Budget::Grows(_)) => g += *bytes as f64,
                    _ => b += *bytes as f64,
                }
            }
            bounded.push((day(s), b));
            growing.push((day(s), g));
        }
        // Bounded content must be flat: its fitted growth over the last
        // third under 2% of its size (or 256 KiB).
        let grown = slope(&bounded) * span;
        if grown > (mean(&bounded) * 0.02).max(256.0 * 1024.0) {
            problems.push((
                FLAT,
                format!(
                    "{store}: bounded tables grew {:.2} MiB over the last third",
                    grown / 1048576.0
                ),
            ));
        }
        // The file may grow only by what the allowlisted tables added.
        let file: Vec<(f64, f64)> = last.iter().map(|s| (day(s), pick(s).file as f64)).collect();
        let file_grown = file.last().unwrap().1 - file[0].1;
        let allowed = (growing.last().unwrap().1 - growing[0].1).max(0.0) + 1024.0 * 1024.0;
        if file_grown > allowed {
            problems.push((FLAT, format!("{store}: file grew {:.2} MiB over the last third, {:.2} MiB explained by per-job tables", file_grown / 1048576.0, allowed / 1048576.0)));
        }
        // Every bounded table by name: rows flat within 10% of its budget.
        for table in tables {
            let Some(Budget::Rows(limit)) = budget(&table) else {
                continue;
            };
            let rows: Vec<(f64, f64)> = last
                .iter()
                .map(|s| (day(s), pick(s).rows(&table) as f64))
                .collect();
            let grown = slope(&rows) * span;
            if grown > limit as f64 * 0.10 + 8.0 {
                problems.push((
                    FLAT,
                    format!(
                        "{store}.{table}: grew {grown:.0} rows over the last third (budget {limit})"
                    ),
                ));
            }
        }
    }
    // Slow growth: years until the core file reaches its bound at the rate
    // the per-job tables grew over the last third.
    let last = &samples[samples.len() * 2 / 3..];
    let span = day(last.last().unwrap()) - day(&last[0]);
    let grown = |s: &Sample| -> f64 {
        s.core
            .tables
            .iter()
            .filter(|(t, _)| matches!(core_budget(t), Some(Budget::Grows(_))))
            .map(|(_, (_, b))| *b as f64)
            .sum()
    };
    let per_day = (grown(last.last().unwrap()) - grown(&last[0])) / span;
    let room = CORE_FILE_BOUND as f64 - last.last().unwrap().core.file as f64;
    let years = if per_day > 0.0 {
        room / per_day / 365.0
    } else {
        f64::INFINITY
    };
    sim.projection = Some((per_day, years));
    if years < 5.0 {
        problems.push((PROJECTION, format!("per-job tables grow {:.0} KiB a day: core.sqlite reaches {:.0} MiB in {years:.1} years", per_day / 1024.0, mib(CORE_FILE_BOUND))));
    }
    // Startup work: the bytes the daily opens read, fitted over the last
    // third and projected to the read bound of an open of a store at its
    // file bound. Work counters are deterministic, unlike CI wall time.
    for (store, starts, file_bound, check) in [
        (
            "core",
            &sim.core_starts,
            CORE_FILE_BOUND + CORE_WAL_BOUND,
            STARTUP_TREND_CORE,
        ),
        (
            "receipts",
            &sim.executor_starts,
            RECEIPT_FILE_BOUND + RECEIPT_WAL_BOUND,
            STARTUP_TREND_RECEIPTS,
        ),
    ] {
        let points: Vec<(f64, f64)> = starts
            .iter()
            .filter(|s| (s.at - START) % DAY == 4 * 3600)
            .map(|s| ((s.at - START) as f64 / DAY as f64, s.opened.io.rchar as f64))
            .collect();
        let last = third(&points, 2);
        let per_day = slope(last);
        let now = last.last().map_or(0.0, |p| p.1);
        let cap = (STARTUP_READ_FACTOR * file_bound) as f64;
        let years = if per_day > 0.0 {
            (cap - now) / per_day / 365.0
        } else {
            f64::INFINITY
        };
        sim.startup_projection.push((store, per_day, years));
        if years < 5.0 {
            problems.push((
                check,
                format!(
                    "{store} opens read {:.2} MiB more each day over the last third ({:.1} MiB now): {:.0} MiB, the read bound at the file bound, in {years:.1} years",
                    per_day / 1048576.0,
                    mib(now as u64),
                    mib(cap as u64)
                ),
            ));
        }
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
}

/// Every measured open within bounds: work linear in the file it opens
/// (`quick_check` reads every page once), and wall time.
fn startup_bounds(sim: &mut Sim) {
    let mut problems = Vec::new();
    for (store, starts, bound) in [
        ("core", &sim.core_starts, CORE_OPEN_BOUND_MS),
        ("receipts", &sim.executor_starts, RECEIPT_OPEN_BOUND_MS),
    ] {
        for s in starts {
            let ms = s.opened.elapsed.as_millis();
            let reads = STARTUP_READ_FACTOR * (s.file + s.wal) + (1 << 20);
            if ms > bound || s.opened.io.rchar > reads {
                problems.push(format!(
                    "{store} open at day {:.2} ({}): {ms} ms (bound {bound}), {:.1} MiB read for a {:.1} MiB file and {:.1} MiB WAL (bound {:.1} MiB)",
                    (s.at - START) as f64 / DAY as f64,
                    s.reason,
                    mib(s.opened.io.rchar),
                    mib(s.file),
                    mib(s.wal),
                    mib(reads)
                ));
            }
        }
    }
    for problem in problems {
        sim.violation(STARTUP, problem);
    }
}

/// Bytes read per byte of file and WAL over a run's opens of one store:
/// minimum, median and maximum, with the reason of the maximum.
fn read_ratios(starts: &[crate::sim::Startup]) -> String {
    let mut ratios: Vec<(f64, &str)> = starts
        .iter()
        .filter(|s| s.file > 1 << 20)
        .map(|s| (s.opened.io.rchar as f64 / (s.file + s.wal) as f64, s.reason))
        .collect();
    if ratios.is_empty() {
        return "no open of a file over 1 MiB".into();
    }
    ratios.sort_by(|a, b| a.0.total_cmp(&b.0));
    let last = ratios[ratios.len() - 1];
    format!(
        "{:.2} / {:.2} / {:.2} (the largest: {})",
        ratios[0].0,
        ratios[ratios.len() / 2].0,
        last.0,
        last.1
    )
}

fn report(sim: &Sim, profile: &Profile) {
    let mut out = String::new();
    let p = &mut out;
    use std::fmt::Write;
    let _ = writeln!(
        p,
        "\n=== soak {}: {} simulated days in {:.1} s ===",
        profile.name,
        profile.days,
        sim.started.elapsed().as_secs_f64()
    );
    let total_ops: u64 = sim.stats.ops.values().sum();
    let _ = writeln!(
        p,
        "operations {total_ops}, dispatcher passes {}, retention passes {}, reads {}, peak events {}, jobs {}",
        sim.stats.dispatcher_passes,
        sim.stats.retention_passes,
        sim.stats.reads,
        sim.stats.peak_events,
        sim.ledger.len()
    );
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    if let Ok(core) = reader(&sim.core_path) {
        let rows: Vec<(String, String, i64)> = core
            .prepare(
                "SELECT json_extract(intent,'$.operation'),state,count(*) FROM jobs GROUP BY 1,2",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (op, state, n) in rows {
            outcomes.insert(format!("{op} {state}"), n as u64);
        }
    }
    let _ = writeln!(p, "job outcomes: {outcomes:?}");
    let _ = writeln!(
        p,
        "wall time by part (s): {:?}",
        sim.stats
            .timing
            .iter()
            .map(|(k, v)| (*k, (v.as_secs_f64() * 10.0).round() / 10.0))
            .collect::<Vec<_>>()
    );
    if let Ok(receipts) = reader(&receipts_path(sim)) {
        let (watermark, actions, stacks): (i64, i64, i64) = receipts
            .query_row(
                "SELECT (SELECT watermark FROM retention),(SELECT count(*) FROM actions),(SELECT count(*) FROM stack_actions)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let executed = sim.exec.effects.len();
        let _ = writeln!(
            p,
            "receipt store: {actions} action rows and {stacks} stack rows kept of {executed} executes; prune watermark at day {:.2}",
            (watermark - START) as f64 / DAY as f64
        );
    }
    let _ = writeln!(p, "faults: {:?}", sim.stats.faults);
    let _ = writeln!(p, "restarts: {:?}", sim.stats.restarts);
    let _ = writeln!(p, "retention removed: {:?}", sim.stats.retention_removed);
    let _ = writeln!(p, "legitimate refusals: {:?}", sim.stats.refused);
    let _ = writeln!(p, "operations: {:?}", sim.stats.ops);
    let _ = writeln!(
        p,
        "day | core MiB (WAL) | events | jobs | sessions | receipts MiB (WAL) | actions | stacks"
    );
    let step = (sim.samples.len() / 10).max(1);
    for s in sim.samples.iter().step_by(step).chain(sim.samples.last()) {
        let _ = writeln!(
            p,
            "{:>3} | {:>6.1} ({:.1}) | {:>6} | {:>5} | {:>3} | {:>5.2} ({:.2}) | {:>5} | {:>4}",
            (s.at - START) / DAY,
            mib(s.core.file),
            mib(s.core.wal),
            s.core.rows("events"),
            s.core.rows("jobs"),
            s.core.rows("sessions"),
            mib(s.receipts.file),
            mib(s.receipts.wal),
            s.receipts.rows("actions"),
            s.receipts.rows("stack_actions"),
        );
    }
    let _ = writeln!(
        p,
        "day | core open: ms, MiB read, read calls | receipts open: ms, KiB read, read calls"
    );
    let daily = |starts: &[crate::sim::Startup]| -> Vec<crate::sim::Startup> {
        starts
            .iter()
            .filter(|s| (s.at - START) % DAY == 4 * 3600)
            .copied()
            .collect()
    };
    let (core, receipts) = (daily(&sim.core_starts), daily(&sim.executor_starts));
    let step = (core.len() / 10).max(1);
    for (c, r) in core
        .iter()
        .zip(&receipts)
        .step_by(step)
        .chain(core.last().zip(receipts.last()))
    {
        let _ = writeln!(
            p,
            "{:>3} | {:>4} ms {:>6.1} MiB {:>6} | {:>4} ms {:>6.0} KiB {:>5}",
            (c.at - START) / DAY,
            c.opened.elapsed.as_millis(),
            mib(c.opened.io.rchar),
            c.opened.io.syscr,
            r.opened.elapsed.as_millis(),
            r.opened.io.rchar as f64 / 1024.0,
            r.opened.io.syscr,
        );
    }
    let _ = writeln!(
        p,
        "bytes read per byte of file at open, min / median / max: core {}; receipts {}",
        read_ratios(&sim.core_starts),
        read_ratios(&sim.executor_starts)
    );
    let _ = writeln!(
        p,
        "core startup reads by step, on the final store: {}",
        probe_open(&sim.core_path)
    );
    for line in &sim.stats.probes {
        let _ = writeln!(p, "expensive open: {line}");
    }
    let worst = |starts: &[crate::sim::Startup]| {
        starts
            .iter()
            .map(|s| s.opened.elapsed.as_millis())
            .max()
            .unwrap_or(0)
    };
    let _ = writeln!(
        p,
        "slowest open: core {} ms, receipts {} ms",
        worst(&sim.core_starts),
        worst(&sim.executor_starts)
    );
    let _ = writeln!(
        p,
        "idle hours measured: {} (write calls in each: {:?})",
        sim.idle.len(),
        sim.idle.iter().map(|(_, io)| io.syscw).collect::<Vec<_>>()
    );
    if let Some(last) = sim.samples.last() {
        let _ = writeln!(p, "per-job tables at the end (rows, MiB):");
        for (table, (rows, bytes)) in &last.core.tables {
            if let Some(Budget::Grows(why)) = core_budget(table) {
                let _ = writeln!(p, "  {table}: {rows}, {:.2}", mib(*bytes as u64));
                if table == "jobs" {
                    let _ = writeln!(p, "    why they grow: {why}");
                }
            }
        }
    }
    for (store, per_day, years) in &sim.startup_projection {
        let _ = writeln!(
            p,
            "{store} startup reads over the last third: {:+.0} KiB/day; the read bound in {years:.0} years at that rate",
            per_day / 1024.0
        );
    }
    if let Some((per_day, years)) = sim.projection {
        let _ = writeln!(
            p,
            "per-job growth over the last third: {:.1} KiB/day; core.sqlite reaches {:.0} MiB in {years:.0} years at that rate",
            per_day / 1024.0,
            mib(CORE_FILE_BOUND)
        );
    }
    eprint!("{out}");
}

fn verdict(sim: &Sim, profile: &Profile) {
    let mut failed: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for (check, line) in &sim.stats.violations {
        failed.entry(check).or_default().push(line);
    }
    let mut checks = vec![
        REFUSALS, INVARIANTS, SIZE, BUDGETS, STARTUP, IDLE, SETTLED, EFFECTS, AUTHORITY, PROVOKED,
    ];
    if profile.trends {
        checks.extend([FLAT, STARTUP_TREND_CORE, STARTUP_TREND_RECEIPTS, PROJECTION]);
    }
    let mut verdict = Vec::new();
    for check in checks {
        let expected = EXPECTED_FAILURES.iter().find(|(c, _)| *c == check);
        match (failed.get(check), expected) {
            (None, None) => eprintln!("PASS  {check}"),
            (Some(lines), Some((_, why))) => {
                eprintln!("XFAIL {check}: {why} ({} failures, first: {})", lines.len(), lines[0]);
            }
            (None, Some(_)) => verdict.push(format!("{check}: listed as an expected failure but passed; remove it from EXPECTED_FAILURES")),
            (Some(lines), None) => {
                eprintln!("FAIL  {check} ({} failures)", lines.len());
                for line in lines.iter().take(20) {
                    eprintln!("      {line}");
                }
                verdict.push(format!("{check}: {} failures, first: {}", lines.len(), lines[0]));
            }
        }
    }
    assert!(
        verdict.is_empty(),
        "soak {} failed:\n{}",
        profile.name,
        verdict.join("\n")
    );
}
