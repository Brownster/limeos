//! What the soak measures and asserts. Per-step checks record a violation as
//! they happen; the end of a run evaluates every check, prints the report and
//! fails on any check that failed, unless it is listed in
//! [`EXPECTED_FAILURES`] with its explanation (and fails if a listed one
//! passes, so a fixed defect cannot stay marked).
use super::{
    sim::Sim,
    startup::{COPY, pages},
    support::{DAY, MINUTE, START, mib, slope},
    workload::{Profile, tick},
};
use limeos_executor_container::limits;
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
pub const BATCHES: &str = "every retention batch within its rule's limit";
pub const ACCOUNTING: &str = "audit counter and retained rows match what retention promises";
pub const EXPLAINED: &str = "core startup reads nothing its checks do not explain";
pub const MOUNT_PROOF: &str = "the storage-mount proof check reads little without storage mounts";
pub const STARTUP_PER_JOB: &str =
    "core startup work grows only with the history tables (events and per-job)";
pub const CLOSURES: &str = "recovery closes every absent action it asks about";
pub const LOST_ANSWERS: &str = "every lost executor answer is logged once, as unavailable";
pub const LOGS: &str = "core logs only the warnings it is expected to";
pub const HELD_OPEN: &str = "a job's history stays while its resolution request is open";
pub const EPOCHS: &str =
    "no job dispatched before a receipt-store change is released on a closure alone";

/// A warning or error core may log while the soak drives it: its message,
/// exactly as core writes it, whether it is an error, the codes it may carry
/// (none: it carries no code), and why the soak provokes it. Anything else
/// core logs fails the run (`LOGS`), as does a lifetime refusal whatever its
/// message (`REFUSALS`).
pub struct ExpectedLog {
    pub message: &'static str,
    pub error: bool,
    pub codes: &'static [&'static str],
    pub why: &'static str,
}

/// Core's dispatcher's warning for a job whose pass failed (logged at a
/// job's 1st, 2nd, 4th... failure in a row), container and stack.
pub const PENDING: [&str; 2] = [
    "container job remains pending or requires reconciliation",
    "stack job remains pending or requires reconciliation",
];

/// Every warning core logs in the soak.
pub const EXPECTED_LOGS: &[ExpectedLog] = &[
    ExpectedLog {
        message: PENDING[0],
        error: false,
        codes: &["Conflict", "Unavailable"],
        why: "Conflict: a queued job meets a lock its resource's held job keeps (the workload acts \
              on busy resources one time in twenty, and the storm on locked ones); Unavailable: \
              the executor is down (stopped between epoch and execute, or dead in the pass) or a \
              recovery lookup's answer was lost (counted, `lost_lookups`)",
    },
    ExpectedLog {
        message: PENDING[1],
        error: false,
        codes: &["Conflict", "Unavailable"],
        why: "the same, for stack jobs",
    },
    ExpectedLog {
        message: EXECUTE_FAILED,
        error: false,
        codes: &["Unavailable"],
        why: "an execute answer the executor never wrote (lost, or the process dead with the \
              request unread or inside the Engine call): the job is held for reconciliation \
              (counted exactly, `lost_answers`)",
    },
    ExpectedLog {
        message: "job resolution stays pending",
        error: false,
        codes: &["Unavailable"],
        why: "the held-open job's recorded decision, which the executor refuses to resolve until \
              the harness releases it (`release_held`)",
    },
    ExpectedLog {
        message: BEYOND_HISTORY,
        error: false,
        codes: &[],
        why: "only within a plan's lifetime after the executor's receipt store was replaced \
              (`JobFault::ReceiptStoreLost`): the new store cannot prove an action of an older \
              plan never ran, so it refuses it before any row (counted exactly, `lost_answers`; \
              any other `BeyondReceiptHistory` is a lifetime refusal)",
    },
    ExpectedLog {
        message: "the container executor could not be reached; the execute request was never sent, and the job is released without effect",
        error: false,
        codes: &[],
        why: "the executor stopped between answering `ReceiptEpoch` and the execute (its socket \
              gone): the claimed job is released as refused",
    },
];

/// The entry of `EXPECTED_LOGS` that admits `entry`, if any.
pub fn expected_log(entry: &super::sim::Logged) -> Option<&'static ExpectedLog> {
    EXPECTED_LOGS.iter().find(|expected| {
        expected.message == entry.message
            && expected.error == entry.error
            && match &entry.code {
                None => expected.codes.is_empty(),
                Some(code) => expected.codes.contains(&code.as_str()),
            }
    })
}

/// Core's message for an execute the executor refused before any row.
pub const BEYOND_HISTORY: &str = "executor refused the action: its plan expires within pruned receipt history, so the executor cannot tell whether an action with it already ran; check the system clock (nothing was recorded or run, and the job is settled as refused)";

/// Core's message for an execute that failed after it was written: the one
/// site every lost execute answer reaches, logged unconditionally.
pub const EXECUTE_FAILED: &str = "execute call failed; the job is held for reconciliation";

/// A check known to fail, the defect that makes it fail, and what is known
/// about that defect: each pin names a measure the run records (`measures`)
/// and the range it must fall in. A failure outside a pin is a new failure;
/// a listed check that passes fails the run too, so a fixed or changed
/// defect cannot stay marked.
pub struct Known {
    pub check: &'static str,
    pub why: &'static str,
    pub pins: &'static [(&'static str, f64, f64)],
}

/// The defects the soak has found, pinned to their cause. None now: the
/// per-job tables' growth (and the startup reads it drove) is bounded by
/// job-row retention, and the storage-mount proof check reads a job's own
/// events (`docs/soak.md`, "Fixed").
pub const EXPECTED_FAILURES: &[Known] = &[];

/// The most pages the storage-mount proof check may read cold on a copy of a
/// store with no storage mount.
pub const PROOF_READS: u64 = 64;

/// Measures the pins read, recorded by the trend checks.
pub const PER_JOB_DAY: &str = "per-job tables' growth over the last third, KiB a day";
pub const PER_JOB_JOB: &str = "per-job tables' bytes per job over the last third, KiB";
pub const PER_JOB_YEARS: &str = "years until core.sqlite reaches its bound at that rate";
pub const STARTUP_DAY: &str = "core startup page reads (but the proof check) added a night";
pub const STARTUP_YEARS: &str = "years until a core open reaches its bound at that rate";
pub const PROOF_BY_KIND: &str = "the proof check looks events up by kind (1 = on every copy)";
pub const PROOF_FILES: &str = "the proof check's largest read, in files' worth of pages";

/// Core's file and WAL: the events table at its 131,072-row budget is about
/// 40 MiB of rows and indexes; the audit cap (twice that) is the hard stop.
pub const CORE_FILE_BOUND: u64 = 80 << 20;
pub const CORE_WAL_BOUND: u64 = 8 << 20;
/// Half the receipt store's own page cap (`limits::MAX_PAGES`, below its
/// `MAX_FILE_BYTES` file check): at the cap every execute fails.
pub const RECEIPT_FILE_BOUND: u64 = limits::MAX_PAGES as u64 * PAGE / 2;
pub const RECEIPT_WAL_BOUND: u64 = 2 << 20;
/// Debug builds on a shared CI runner; a Pi in release is far faster.
pub const CORE_OPEN_BOUND_MS: u128 = 5_000;
/// SQLite's page size in both stores.
pub const PAGE: u64 = 4096;
/// Page reads an open on a copy may make, per page of database and WAL,
/// in thousandths, and a fixed allowance: see `startup_page_bound`. Core
/// measured about 2.4 (quick_check 1.55, the foreign-key check 0.55, six
/// scans of the jobs table, the audit count) beyond its proof check, and up
/// to 2.8 while that check evicts the cache; the receipt store about 1.0.
pub const CORE_PAGE_FACTOR: u64 = 3200;
pub const RECEIPT_PAGE_FACTOR: u64 = 2000;
pub const PAGE_SLACK: u64 = 256;
/// The page reads an open may make beyond what its known statements read
/// cold (see `startup::KNOWN`): its own small statements, schema reads and
/// writes, measured at 1.5% to 2.2% of the statements' work (but the proof
/// check's), plus a fixed allowance. One more scan of the events table
/// (about 7% of that work) does not fit.
pub const EXPLAIN_PERCENT: u64 = 3;
/// Page reads an open of a crash image may make per committed WAL frame it
/// replays, beyond what its statements read cold on a consistent copy:
/// recovery reads each frame once to rebuild the wal-index, and the
/// statements then read replayed pages from the WAL.
pub const REPLAY_READS: u64 = 2;
pub const EXPLAIN_SLACK: u64 = 256;
/// The most page reads an open on a copy of `bytes` may make.
pub fn startup_page_bound(store: &str, bytes: u64) -> u64 {
    let factor = if store == "core" {
        CORE_PAGE_FACTOR
    } else {
        RECEIPT_PAGE_FACTOR
    };
    bytes / PAGE * factor / 1000 + PAGE_SLACK
}
pub const RECEIPT_OPEN_BOUND_MS: u128 = 2_000;

/// A table's budget.
#[derive(Clone, Copy)]
pub enum Budget {
    /// At most this many rows, at every checkpoint.
    Rows(i64),
    /// Bounded by job-row retention, by age and per-family floors rather than
    /// a row count, and why: flat over the last third with every other
    /// bounded table, no settled job kept a day past what the rule allows
    /// (`checkpoint`), and their growth reported and projected to the file
    /// bound.
    Retained(&'static str),
}

const PER_JOB: &str = "one row per job (or per queued plan, result, resource or completed resolution): \
     a settled job is removed with all of them 30 days after it last changed, but for the newest of \
     each family and jobs an open request or an authority event keeps (`retention/jobs.rs`)";

/// Every core table: walking `sqlite_master` makes a table added later fail
/// the soak until it is given a budget here.
fn core_budget(table: &str) -> Option<Budget> {
    use Budget::*;
    Some(match table {
        // The retention budget, plus one batch of arrivals past it.
        "events" => {
            let (budget, batch) = super::sim::rule_limits("events");
            Rows(budget + batch)
        }
        // 16 per user (`create_session` evicts the oldest), two users.
        "sessions" => Rows(32),
        "users" => Rows(2),
        "grants" => Rows(10),
        "meta" => Rows(1),
        "bootstrap" => Rows(1),
        // One per locked resource: 18 containers and 4 stacks.
        "resource_locks" => Rows(22),
        "jobs" | "job_resources" | "container_plans" | "container_results" | "stack_plans"
        | "stack_results" | "job_resolutions" => Retained(PER_JOB),
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

/// Every receipt-store table, from the store's own limits.
fn receipt_budget(table: &str) -> Option<Budget> {
    use Budget::*;
    let (actions, stacks, batch) = (
        i64::from(limits::RETAINED_ACTIONS),
        i64::from(limits::RETAINED_STACKS),
        i64::from(limits::PRUNE_BATCH),
    );
    // The most members one stack here has (the media stack).
    let members = super::engine::STACKS
        .iter()
        .map(|s| {
            super::engine::HOST
                .iter()
                .filter(|(_, p)| *p == Some(*s))
                .count()
        })
        .max()
        .unwrap_or(0) as i64;
    Some(match table {
        // The standalone receipts kept, each kept stack's member rows, plus
        // one prune batch and the rows still holding locks (one per
        // container, one per stack).
        "actions" => Rows(actions + stacks * members + batch + super::engine::HOST.len() as i64),
        "stack_actions" => Rows(stacks + batch + super::engine::STACKS.len() as i64),
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
                Some(Budget::Retained(_)) if table != "jobs" && *rows > 20 * jobs + 64 => problems
                    .push((
                        BUDGETS,
                        format!("{store}.{table}: {rows} rows for {jobs} jobs"),
                    )),
                _ => {}
            }
        }
    }
    let overdue = overdue_jobs(&reader(&sim.core_path).unwrap(), sample.at);
    if overdue > 0 {
        problems.push((
            BUDGETS,
            format!(
                "core: {overdue} settled jobs kept a day past job-row retention's horizon and floor, with nothing that keeps them"
            ),
        ));
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
    let authority = authority_events(&reader(&sim.core_path).unwrap());
    if authority < sim.authority_seen {
        sim.violation(
            AUTHORITY,
            format!(
                "{authority} authority events, {} at the last checkpoint",
                sim.authority_seen
            ),
        );
    }
    sim.authority_seen = sim.authority_seen.max(authority);
    invariants(sim, "daily checkpoint");
    core_accounting(sim, "daily checkpoint");
    receipt_accounting(sim, "daily checkpoint");
}

/// Settled container and stack jobs that job-row retention should have
/// removed a day ago (`retention/jobs.rs`, rules 1 to 6, read independently
/// of its query): unchanged since a day before the horizon, past the newest
/// `JOB_FLOOR` of their family, with no open resolution request and no
/// authority event. A day is 1,440 passes of up to 64 jobs.
fn overdue_jobs(conn: &Connection, now: i64) -> i64 {
    let cutoff = now - limeos_persistence::test_support::JOB_HORIZON - DAY;
    let floor = limeos_persistence::test_support::JOB_FLOOR;
    let mut overdue = 0;
    for family in [
        "('container_restart','container_start','container_stop')",
        "('stack_restart','stack_start','stack_stop')",
    ] {
        let settled = format!(
            "j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') IN {family}"
        );
        overdue += conn
            .query_row(
                &format!(
                    "WITH kept AS (SELECT j.id FROM jobs j WHERE {settled} ORDER BY j.rowid DESC LIMIT ?2) \
                     SELECT count(*) FROM jobs j WHERE {settled} AND j.id NOT IN kept AND j.deadline<?1 \
                     AND NOT EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND (e.created>=?1 OR json_extract(e.kind,'$.kind') GLOB 'job_resolution_*')) \
                     AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND (r.confirmation IS NULL OR r.completed>=?1)) \
                     AND NOT EXISTS(SELECT 1 FROM container_results x WHERE x.job=j.id AND x.recorded>=?1) \
                     AND NOT EXISTS(SELECT 1 FROM stack_results x WHERE x.job=j.id AND x.recorded>=?1)"
                ),
                rusqlite::params![cutoff, floor],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
    }
    overdue
}

/// The core store's audit accounting, read on a side connection: before
/// every restart (where `Store::open` would recompute the counter and hide
/// a drift) and at every checkpoint. The counter is exactly `EVENT_BYTES`
/// per event row, and retention never takes the table below what it keeps:
/// once the run has written as many events as the events rule keeps, at
/// least that many remain (less only what the authority rule removed and
/// what job-row retention removed with its jobs).
pub fn core_accounting(sim: &mut Sim, when: &str) {
    let (bytes, rows): (i64, i64) = reader(&sim.core_path)
        .unwrap()
        .query_row(
            "SELECT audit_bytes,(SELECT count(*) FROM events) FROM meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let per_event = limeos_persistence::test_support::EVENT_BYTES;
    let mut problems = Vec::new();
    if bytes != rows * per_event {
        problems.push(format!(
            "{when}: meta.audit_bytes is {bytes}, but {rows} events take {} ({per_event} each): the audit counter drifted by {} events",
            rows * per_event,
            (bytes - rows * per_event) / per_event
        ));
    }
    let removed = |rule: &str| sim.stats.retention_removed.get(rule).copied().unwrap_or(0);
    let (budget, _) = super::sim::rule_limits("events");
    let jobs = sim.stats.job_events;
    let written = rows + removed("events") + removed("authority events") + jobs;
    let floor = budget.min(written) - removed("authority events") - jobs;
    if rows < floor {
        problems.push(format!(
            "{when}: {rows} events kept of {written} written, below the {budget} the events rule keeps: retention over-pruned by {}",
            floor - rows
        ));
    }
    for problem in problems {
        sim.violation(ACCOUNTING, problem);
    }
}

/// The receipt store's retention, read on a side connection at every
/// checkpoint and before every executor restart: standalone and stack
/// receipts never fall below what retention keeps once reached; the prune
/// watermark never decreases, and never passes the oldest expiry the rule
/// may prune (`now - RECEIPT_RETAIN_AFTER_EXPIRY`) unless it is the store's
/// creation floor.
pub fn receipt_accounting(sim: &mut Sim, when: &str) {
    let Ok(conn) = reader(&receipts_path(sim)) else {
        return;
    };
    let Ok((standalone, stacks, watermark)) = conn.query_row(
        "SELECT (SELECT count(*) FROM actions a WHERE NOT EXISTS(SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)),(SELECT count(*) FROM stack_actions),(SELECT watermark FROM retention WHERE id=1)",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    ) else {
        return;
    };
    let mut problems = Vec::new();
    let reached = &mut sim.receipts_reached;
    for (what, rows, kept, seen) in [
        (
            "standalone receipts",
            standalone,
            i64::from(limits::RETAINED_ACTIONS),
            &mut reached.0,
        ),
        (
            "stack receipts",
            stacks,
            i64::from(limits::RETAINED_STACKS),
            &mut reached.1,
        ),
    ] {
        *seen |= rows >= kept;
        if *seen && rows < kept {
            problems.push(format!(
                "{when}: {rows} {what} kept, below the {kept} retention keeps once reached: over-pruned"
            ));
        }
    }
    let creation = sim.receipt_creation;
    let cutoff = sim.now() - limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY;
    if watermark < sim.last_watermark {
        problems.push(format!(
            "{when}: the prune watermark went back from day {:.3} to day {:.3}",
            (sim.last_watermark - START) as f64 / DAY as f64,
            (watermark - START) as f64 / DAY as f64
        ));
    }
    if watermark > cutoff && watermark != creation {
        problems.push(format!(
            "{when}: the prune watermark (day {:.3}) is past the youngest expiry retention may prune (day {:.3}): a receipt was pruned too early",
            (watermark - START) as f64 / DAY as f64,
            (cutoff - START) as f64 / DAY as f64
        ));
    }
    sim.last_watermark = sim.last_watermark.max(watermark);
    for problem in problems {
        sim.violation(ACCOUNTING, problem);
    }
}

/// Authority events in core's store, read on a side connection.
fn authority_events(core: &Connection) -> i64 {
    core.query_row(
        "SELECT count(*) FROM events WHERE json_extract(kind,'$.kind') IN ('bootstrap_issued','administrator_enrolled','legacy_identity_imported','grants_revised','job_resolution_requested','job_resolution_withdrawn','job_resolution_superseded')",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// After every restart: the stores opened (else the run has already
/// stopped), and the invariants hold.
pub fn after_restart(sim: &mut Sim, reason: &'static str) {
    invariants(sim, reason);
}

const HELD: &str = "('running','verifying','outcome_unknown','needs_intervention')";
const SETTLED_STATES: [&str; 5] = [
    "succeeded",
    "failed",
    "canceled",
    "precondition_changed",
    "resolved",
];
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
        .filter(|id| !states.contains_key(*id) && !sim.pruned.contains(*id))
        .count();
    if unknown + lost > 0 {
        problems.push(format!(
            "{lost} queued jobs missing from the store (and not removed by retention), {unknown} jobs nobody queued"
        ));
    }
    // A job retention removed is gone for good, and was settled when last
    // read (every checkpoint and restart reads every job; the horizon is 30
    // days).
    for id in &sim.pruned {
        let last = sim.last_state.get(id).map(String::as_str);
        if states.contains_key(id) || !last.is_some_and(|s| SETTLED_STATES.contains(&s)) {
            problems.push(format!(
                "retention removed job {} ({:?} when last read), and the store has it as {:?}",
                &id[..12],
                last,
                states.get(id)
            ));
        }
    }
    sim.last_state
        .extend(states.iter().map(|(id, state)| (id.clone(), state.clone())));
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

/// Every file of both stores that a write changes (database and WAL), with
/// its size and modification time: any write to it moves the latter.
fn store_files(sim: &Sim) -> Vec<(String, Option<(u64, std::time::SystemTime)>)> {
    let receipts = receipts_path(sim);
    [sim.core_path.clone(), receipts]
        .into_iter()
        .flat_map(|path| {
            let mut wal = path.as_os_str().to_owned();
            wal.push("-wal");
            [path, std::path::PathBuf::from(wal)]
        })
        .map(|path| {
            let stamp = std::fs::metadata(&path)
                .ok()
                .and_then(|m| Some((m.len(), m.modified().ok()?)));
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                stamp,
            )
        })
        .collect()
}

/// An hour of the night with nothing to do: once retention has nothing
/// left to remove, dispatcher passes (recovery polls of any held job
/// included) and the writer thread's own retention passes must not write a
/// byte to either store: no file of either store (database or WAL) may
/// change. (Thread write counters are no measure here: core's writer thread
/// wakes the test thread's runtime with an eventfd write for every answer,
/// and recovery polls write to the executor's socket.)
pub async fn idle_window(sim: &mut Sim) {
    sim.idle_scheduled += 1;
    let started = sim.now();
    for _ in 0..240 {
        let removed = sim.advance(sim.now() + MINUTE);
        tick(sim).await;
        if removed == 0 {
            break;
        }
    }
    let restarts: u64 = sim.stats.restarts.values().sum();
    let start = sim.now();
    let before = store_files(sim);
    for _ in 0..60 {
        sim.advance(sim.now() + MINUTE);
        tick(sim).await;
    }
    let after = store_files(sim);
    if sim.stats.restarts.values().sum::<u64>() != restarts || sim.executor_back_at.is_some() {
        // A fault armed on a held job fired: not an idle hour. Recorded,
        // so a run that skips too many fails instead of passing unmeasured.
        sim.idle_skipped.push((
            started,
            "a fault armed on a held job restarted a daemon".into(),
        ));
        return;
    }
    let changed: Vec<String> = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b.1 != a.1)
        .map(|(b, _)| b.0.clone())
        .collect();
    if !changed.is_empty() {
        sim.violation(
            IDLE,
            format!("an idle hour wrote to {}", changed.join(", ")),
        );
    }
    sim.idle.push((start, changed));
}

/// Idle hours: every scheduled one measured or skipped for a recorded
/// reason, and at least three in four measured.
fn idle_coverage(sim: &mut Sim) {
    let (scheduled, measured) = (sim.idle_scheduled as usize, sim.idle.len());
    if measured + sim.idle_skipped.len() != scheduled
        || measured * 4 < scheduled * 3
        || measured == 0
    {
        let skipped: Vec<String> = sim
            .idle_skipped
            .iter()
            .map(|(at, why)| format!("day {}: {why}", (at - START) / DAY))
            .collect();
        sim.violation(
            IDLE,
            format!("{measured} of {scheduled} idle hours measured (at least three in four needed); skipped: {skipped:?}"),
        );
    }
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
    idle_coverage(sim);
    lost_answers(sim);
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
    let effects = sim.exec.faults().effects.clone();
    let unclosed = sim.exec.faults().unclosed.clone();
    problems.extend(unclosed.into_iter().map(|line| (CLOSURES, line)));
    for (id, record) in &sim.ledger {
        // A job retention removed, as the invariants last read it.
        let state = states
            .get(id)
            .or_else(|| {
                sim.pruned
                    .contains(id)
                    .then(|| sim.last_state.get(id))
                    .flatten()
            })
            .map(String::as_str)
            .unwrap_or("missing");
        let effects = effects.get(id).copied().unwrap_or(0) as usize;
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
    // A job core held when the executor's receipt store changed (a new
    // epoch at every executor start, receipts lost with a replaced store)
    // may have been executed under the old store: a closure from the new
    // one proves nothing about it, so it must never be released on one
    // (core marks it `executor_store_changed` and holds it for an operator).
    let closed = sim.exec.faults().closed_jobs.clone();
    let mut exposed_closed = 0;
    for id in sim.exposed.intersection(&closed) {
        exposed_closed += 1;
        if states.get(id).map(String::as_str) == Some("precondition_changed") {
            problems.push((
                EPOCHS,
                format!(
                    "job {} was held when the receipt store changed, and released on a closure from the new store",
                    &id[..12]
                ),
            ));
        }
    }
    sim.measures
        .insert(EXPOSED_CLOSED, f64::from(exposed_closed));
    let replaced = sim
        .faults()
        .iter()
        .filter(|(name, _)| name.contains("receipt store replaced"))
        .map(|(_, n)| n)
        .sum::<u64>();
    if !sim.replacements.is_empty() || (replaced > 0 && exposed_closed == 0) {
        problems.push((
            EPOCHS,
            format!(
                "{} receipt-store replacements never happened, {replaced} did, and {exposed_closed} jobs held across a store change got a closure: the check saw nothing",
                sim.replacements.len()
            ),
        ));
    }
    // Authority events: far below their budget (4,096), so retention must
    // never have removed one (it is the only thing that deletes events), and
    // their count never went down between checkpoints.
    let pruned = sim
        .stats
        .retention_removed
        .get("authority events")
        .copied()
        .unwrap_or(0);
    let authority = authority_events(&core);
    if pruned != 0 || authority < sim.authority_seen {
        problems.push((
            AUTHORITY,
            format!(
                "{authority} authority events kept ({} at the last checkpoint); retention removed {pruned}",
                sim.authority_seen
            ),
        ));
    }
    // The workload provokes these on purpose; each must have been seen, so
    // the run proves legitimate refusals are told apart from lifetime ones.
    // (Failed logins are not among them: their refusal is the password
    // check's, in core, which the soak does not drive.)
    for (op, code) in [
        ("approve plan", "Expired"),
        ("provoked: session used after it expired", "Unauthenticated"),
    ] {
        if !sim
            .stats
            .refused
            .contains_key(&(op.to_owned(), code.to_owned()))
        {
            problems.push((PROVOKED, format!("{op}: never refused with {code}")));
        }
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
}

/// Every execute whose answer the executor never wrote (it closed the
/// connection with the request read: the answer lost, or the process dead
/// with the request unread or inside the Engine call) is logged by core
/// exactly once, at its execute site, with the transport code
/// `Unavailable`; and every other code core logs there is a refusal the
/// executor answered with that code, one for one. No code core logs anywhere
/// is `InvalidInput` (`EXPECTED_LOGS` admits it nowhere).
fn lost_answers(sim: &mut Sim) {
    let (lost, refused) = {
        let faults = sim.exec.faults();
        (faults.lost.clone(), faults.refused.clone())
    };
    let executes = ["execute", "execute stack"];
    let mut expected: BTreeMap<String, u64> = BTreeMap::new();
    for kind in executes {
        *expected.entry("Unavailable".into()).or_default() += lost.get(kind).copied().unwrap_or(0);
    }
    // A refusal before any row has its own message (`BEYOND_HISTORY`).
    let mut beyond = 0;
    for ((kind, code), count) in &refused {
        if executes.contains(kind) {
            if code == "BeyondReceiptHistory" {
                beyond += count;
            } else {
                *expected.entry(code.clone()).or_default() += count;
            }
        }
    }
    expected.retain(|_, count| *count > 0);
    let mut logged: BTreeMap<String, u64> = BTreeMap::new();
    let mut logged_beyond = 0;
    for ((_, message, code), count) in &sim.logged {
        if message == EXECUTE_FAILED {
            *logged
                .entry(code.clone().unwrap_or_else(|| "none".into()))
                .or_default() += count;
        } else if message == BEYOND_HISTORY {
            logged_beyond += count;
        }
    }
    if logged_beyond != beyond {
        sim.violation(
            LOST_ANSWERS,
            format!(
                "core logged {logged_beyond} executes refused before any row; the executor refused {beyond}"
            ),
        );
    }
    let total: u64 = lost.values().sum();
    sim.measures.insert(
        LOST_EXECUTES,
        expected.get("Unavailable").copied().unwrap_or(0) as f64,
    );
    sim.measures.insert(LOST_TOTAL, total as f64);
    sim.measures
        .insert(LOST_LOOKUPS, sim.stats.lookups_checked as f64);
    let lookups = {
        let (containers, stacks) = sim.exec.lost_lookups();
        containers + stacks
    };
    if lookups != sim.stats.lookups_checked + sim.stats.lookups_unchecked {
        sim.violation(
            LOST_ANSWERS,
            format!(
                "{lookups} recovery-lookup answers lost, {} of them in dispatcher passes",
                sim.stats.lookups_checked + sim.stats.lookups_unchecked
            ),
        );
    }
    if logged != expected {
        sim.violation(
            LOST_ANSWERS,
            format!(
                "core logged {logged:?} at its execute site; the executor lost {lost:?} answers and refused {refused:?}, so {expected:?} were due"
            ),
        );
    }
}
pub const LOST_EXECUTES: &str =
    "execute answers the executor never wrote (each logged as Unavailable)";
pub const LOST_LOOKUPS: &str =
    "recovery-lookup answers the executor never wrote, checked one for one";

/// After a dispatcher pass, with the receipt-lookup answers the executor
/// never wrote in it (`lost`: container, stack). When the executor ran
/// throughout the pass (`checked`), nothing but a lost answer makes core's
/// dispatcher log a job's failure with the transport code `Unavailable`
/// (`PENDING`), so:
/// - every lost container lookup (a held job's recovery lookup) is logged so
///   once, one for one: core's log damping (a job's 1st, 2nd, 4th... failure
///   in a row) cannot hide it, since the lookup follows a pass the job went
///   through (its execute, answered or not, resets the count), so it is the
///   job's first failure, or its second after a dead executor's refusal;
/// - a lost stack lookup is the one `release_if_unsent` makes right after a
///   failed execute, which holds the job for the operator without a log, so
///   the stack site logs none.
pub fn lost_lookups(sim: &mut Sim, lost: (u64, u64), checked: bool) {
    let (containers, stacks) = sim.batch_pending_unavailable;
    if !checked {
        sim.stats.lookups_unchecked += lost.0 + lost.1;
        return;
    }
    sim.stats.lookups_checked += lost.0 + lost.1;
    if containers != lost.0 || stacks != 0 {
        sim.violation(
            LOST_ANSWERS,
            format!(
                "a dispatcher pass with the executor up lost {} container and {} stack receipt-lookup answers, and core logged {containers} container and {stacks} stack jobs as pending with Unavailable ({} and none due)",
                lost.0, lost.1, lost.0
            ),
        );
    }
}
pub const LOST_TOTAL: &str = "answers the executor never wrote, every kind";
pub const HELD_PRUNED: &str =
    "events pruned around the held-open job while its request stayed open";
pub const EXPOSED_CLOSED: &str =
    "jobs held across a receipt-store change that the new store answered with a closure";

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
        // Every table: the per-job tables, bounded by job-row retention,
        // with the rest.
        let bounded: Vec<(f64, f64)> = last
            .iter()
            .map(|s| {
                let bytes = pick(s).tables.values().map(|(_, b)| *b as f64).sum();
                (day(s), bytes)
            })
            .collect();
        // Rows a table under its row budget regained, at its bytes a row:
        // the events table refills to its budget once job-row retention has
        // removed settled jobs' history with them, and stops there.
        let refill = refilled(pick(&last[0]), pick(last.last().unwrap()), budget);
        // Bounded content must be flat: its fitted growth over the last
        // third under 2% of its size (or 256 KiB), beyond that refill.
        let grown = slope(&bounded) * span;
        if grown > (mean(&bounded) * 0.02).max(256.0 * 1024.0) + refill {
            problems.push((
                FLAT,
                format!(
                    "{store}: bounded tables grew {:.2} MiB over the last third, {:.2} MiB of it tables refilling to their row budgets",
                    grown / 1048576.0,
                    refill / 1048576.0
                ),
            ));
        }
        // The file may grow by at most 1 MiB over the last third beyond that
        // refill: every table is bounded.
        let file: Vec<(f64, f64)> = last.iter().map(|s| (day(s), pick(s).file as f64)).collect();
        let file_grown = file.last().unwrap().1 - file[0].1;
        let allowed = 1024.0 * 1024.0 + refill;
        if file_grown > allowed {
            problems.push((
                FLAT,
                format!(
                    "{store}: file grew {:.2} MiB over the last third, more than {:.2} MiB",
                    file_grown / 1048576.0,
                    allowed / 1048576.0
                ),
            ));
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
    // Slow growth: the per-job tables over the last third, per day and per
    // job, and the years until the core file reaches its bound at that rate.
    let last = &samples[samples.len() * 2 / 3..];
    let (first, final_) = (&last[0], last.last().unwrap());
    let span = day(final_) - day(first);
    let per_day = (per_job_bytes(final_) - per_job_bytes(first)) / span;
    let jobs = (final_.core.rows("jobs") - first.core.rows("jobs")) as f64;
    let per_job = (per_job_bytes(final_) - per_job_bytes(first)) / jobs.max(1.0);
    let room = CORE_FILE_BOUND as f64 - final_.core.file as f64;
    let years = years_until(room, per_day);
    sim.projection = Some((per_day, years));
    sim.measures.insert(PER_JOB_DAY, per_day / 1024.0);
    sim.measures.insert(PER_JOB_JOB, per_job / 1024.0);
    sim.measures.insert(PER_JOB_YEARS, years);
    if years < 5.0 {
        problems.push((PROJECTION, format!("per-job tables grow {:.0} KiB a day ({:.1} KiB a job): core.sqlite reaches {:.0} MiB in {years:.1} years", per_day / 1024.0, per_job / 1024.0, mib(CORE_FILE_BOUND))));
    }
    // Core startup on the nightly copies: the cold page reads of its known
    // statements but the proof check (a defect of its own, `MOUNT_PROOF`,
    // whose cache eviction would otherwise blur this series; the open is
    // tied to the sum by `EXPLAINED`), against the history tables' growth on
    // the same nights (the per-job tables, and the events table refilling to
    // its budget after job-row retention removed history with its jobs), over
    // the last third. Growth is taken between the
    // medians of the two halves of the last third.
    let nights: Vec<(f64, f64, f64)> = sim
        .core_starts
        .iter()
        .filter_map(|c| {
            let trace = c.trace.as_ref()?;
            let sample = samples.iter().find(|s| s.at == c.at)?;
            Some((
                (c.at - START) as f64 / DAY as f64,
                trace.cold_but_defect() as f64,
                history_bytes(sample) / PAGE as f64,
            ))
        })
        .collect();
    let tail = &nights[nights.len() * 2 / 3..];
    let reads: Vec<(f64, f64)> = tail.iter().map(|n| (n.0, n.1)).collect();
    let history_pages: Vec<(f64, f64)> = tail.iter().map(|n| (n.0, n.2)).collect();
    let startup_per_day = half_slope(&reads);
    let history_per_day = half_slope(&history_pages);
    let cap = startup_page_bound("core", CORE_FILE_BOUND + CORE_WAL_BOUND) as f64;
    let now = reads.last().map_or(0.0, |p| p.1);
    let startup_years = years_until(cap - now, startup_per_day);
    sim.startup_projection
        .push(("core", startup_per_day, startup_years));
    sim.measures.insert(STARTUP_DAY, startup_per_day);
    sim.measures.insert(STARTUP_YEARS, startup_years);
    if startup_years < 5.0 {
        problems.push((
            STARTUP_TREND_CORE,
            format!(
                "core opens read {startup_per_day:.0} more pages each night over the last third ({now:.0} now): {cap:.0}, the bound at the file bound, in {startup_years:.1} years"
            ),
        ));
    }
    if history_per_day > 0.0 {
        let ratio = startup_per_day / history_per_day;
        sim.measures.insert(STARTUP_RATIO, ratio);
        if ratio > STARTUP_PER_JOB_MAX {
            problems.push((
                STARTUP_PER_JOB,
                format!(
                    "core opens read {startup_per_day:.0} more pages a night while the history tables gain {history_per_day:.1}: {ratio:.1} reads per page, more than the {STARTUP_PER_JOB_MAX} their growth explains"
                ),
            ));
        }
    }
    // The receipt store's opens. Every one of its tables is bounded by a row
    // budget (asserted at every checkpoint), so its reads can grow only
    // while its rows approach those budgets: the stack receipts kept turn
    // over at two stack actions a day, slower than the run, and their member
    // rows follow the mix of stacks acted on. Growth is projected to the
    // bound, but no further than the reads of a store whose tables are full.
    let receipts: Vec<(f64, f64)> = sim
        .executor_starts
        .iter()
        .filter(|s| s.reason == COPY)
        .map(|s| ((s.at - START) as f64 / DAY as f64, pages(s) as f64))
        .collect();
    let tail = &receipts[receipts.len() * 2 / 3..];
    let per_day = half_slope(tail);
    let cap = startup_page_bound("receipts", RECEIPT_FILE_BOUND + RECEIPT_WAL_BOUND) as f64;
    let now = tail.last().map_or(0.0, |p| p.1);
    let full = samples.last().map_or(f64::MAX, |last| {
        let (mut rows, mut budget) = (0, 0);
        for (table, (count, _)) in &last.receipts.tables {
            if let Some(Budget::Rows(limit)) = receipt_budget(table) {
                rows += count;
                budget += limit;
            }
        }
        now * budget as f64 / rows.max(1) as f64
    });
    let years = if full < cap {
        99.0
    } else {
        years_until(cap - now, per_day)
    };
    sim.measures.insert(RECEIPTS_FULL, full);
    sim.startup_projection.push(("receipts", per_day, years));
    if years < 5.0 {
        problems.push((
            STARTUP_TREND_RECEIPTS,
            format!(
                "receipt-store opens read {per_day:.1} more pages each night over the last third ({now:.0} now, about {full:.0} with every table full): {cap:.0}, the bound at the file bound, in {years:.1} years"
            ),
        ));
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
}

/// Bytes the tables of `to` with a row budget gained over `from` by rows
/// they regained while still within that budget (the budget check holds
/// them to it at every checkpoint), at their bytes a row in `to`.
fn refilled(from: &StoreStat, to: &StoreStat, budget: fn(&str) -> Option<Budget>) -> f64 {
    to.tables
        .iter()
        .filter_map(|(table, (rows, bytes))| {
            let Some(Budget::Rows(limit)) = budget(table) else {
                return None;
            };
            let gained = rows - from.rows(table);
            (gained > 0 && *rows <= limit && *rows > 0)
                .then(|| gained as f64 * *bytes as f64 / *rows as f64)
        })
        .sum()
}

/// Bytes of core's history tables in a sample: the events table (bounded
/// by its row budget) and the per-job tables (by job-row retention).
fn history_bytes(s: &Sample) -> f64 {
    per_job_bytes(s) + s.core.tables.get("events").map_or(0, |t| t.1) as f64
}

/// Bytes of core's per-job tables in a sample.
fn per_job_bytes(s: &Sample) -> f64 {
    s.core
        .tables
        .iter()
        .filter(|(t, _)| matches!(core_budget(t), Some(Budget::Retained(_))))
        .map(|(_, (_, b))| *b as f64)
        .sum()
}

fn years_until(room: f64, per_day: f64) -> f64 {
    if per_day > 0.0 {
        (room / per_day / 365.0).min(99.0)
    } else {
        99.0
    }
}

/// Growth per day between the medians of the two halves of `points`.
fn half_slope(points: &[(f64, f64)]) -> f64 {
    let median = |part: &[(f64, f64)], pick: fn(&(f64, f64)) -> f64| -> f64 {
        let mut values: Vec<f64> = part.iter().map(pick).collect();
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    if points.len() < 4 {
        return 0.0;
    }
    let (a, b) = points.split_at(points.len() / 2);
    let dx = median(b, |p| p.0) - median(a, |p| p.0);
    if dx <= 0.0 {
        0.0
    } else {
        (median(b, |p| p.1) - median(a, |p| p.1)) / dx
    }
}

/// Startup page reads per page the history tables gain, at most: measured
/// about 5 for per-job pages (quick_check and the foreign-key check read
/// each new page, and six statements scan the jobs table), fewer for events
/// pages; twice that is still the history component, more is growth from
/// something else.
pub const STARTUP_PER_JOB_MAX: f64 = 10.0;
pub const STARTUP_RATIO: &str = "core startup page reads per history-table page gained";
pub const RECEIPTS_FULL: &str = "receipt-store open page reads with every table at its budget";

/// Every measured open within bounds. On each copy of core's store, from the
/// open's own statement trace: every page read is inside one of the
/// statements startup executes, but a few (`EXPLAINED`), no statement scans
/// the events table itself (`EXPLAINED`), the open reads no more than its
/// statements read cold, and those statements, but the pinned proof check,
/// read a bounded number of pages per page of file (`STARTUP`); the proof
/// check itself is judged on its own (`MOUNT_PROOF`). The wall time of every
/// open, real or on a copy, is machine-dependent and guards against the
/// catastrophic only.
fn startup_bounds(sim: &mut Sim) {
    let mut problems = Vec::new();
    for (store, starts, bound) in [
        ("core", &sim.core_starts, CORE_OPEN_BOUND_MS),
        ("receipts", &sim.executor_starts, RECEIPT_OPEN_BOUND_MS),
    ] {
        for s in starts {
            let ms = s.opened.elapsed.as_millis();
            let day = (s.at - START) as f64 / DAY as f64;
            let file_pages = (s.file + s.wal) / PAGE;
            if ms > bound {
                problems.push((
                    STARTUP,
                    format!(
                        "{store} open at day {day:.2} ({}): {ms} ms (bound {bound})",
                        s.reason
                    ),
                ));
            }
            if s.reason != COPY && s.reason != super::startup::CRASH {
                continue;
            }
            // Core: its statements but the proof check, cold; the receipt
            // store: the open.
            let (work, what) = match &s.trace {
                Some(trace) => (
                    trace.cold_but_defect(),
                    "its startup statements but the proof check read",
                ),
                None => (pages(s), "the open read"),
            };
            let limit = startup_page_bound(store, s.file + s.wal);
            if work > limit {
                problems.push((
                    STARTUP,
                    format!(
                        "{store} open on a copy at day {day:.2}: {what} {work} pages for {file_pages} pages of file (bound {limit})"
                    ),
                ));
            }
            let Some(trace) = &s.trace else { continue };
            problems.extend(scan_problems(trace, s.replay).into_iter().map(|problem| {
                (
                    EXPLAINED,
                    format!("core open on a copy at day {day:.2}: {problem}"),
                )
            }));
            let cold = trace.cold();
            let allowed = cold
                + trace.cold_but_defect() * EXPLAIN_PERCENT / 100
                + EXPLAIN_SLACK
                + s.replay * REPLAY_READS;
            if pages(s) > allowed {
                problems.push((
                    EXPLAINED,
                    format!(
                        "core open on a copy at day {day:.2}: {} page reads, {} more than its statements read cold ({cold})",
                        pages(s),
                        pages(s) - cold
                    ),
                ));
            }
            // The household has no storage mount: the proof check, driven
            // from the mount plans, reads a few pages, whatever the jobs
            // and events (it read 6.5 files' worth while it looked events
            // up by kind for every job, and up to an eighth of the file
            // while it visited every settled job).
            for proof in trace.statements.iter().filter(|s| s.proof()) {
                let reads = proof.cold.unwrap_or(proof.reads);
                if proof.events_by_kind() || reads > PROOF_READS {
                    problems.push((
                        MOUNT_PROOF,
                        format!(
                            "day {day:.2}: the storage-mount proof check reads {reads} pages cold ({:.2} files' worth){}",
                            reads as f64 / file_pages.max(1) as f64,
                            if proof.events_by_kind() {
                                ", looking events up by kind"
                            } else {
                                ""
                            }
                        ),
                    ));
                }
            }
        }
    }
    // The pins: the proof check, found in the open by its own text, looks
    // events up by kind on every copy, and its largest cold read.
    let copies: Vec<(&super::startup::Trace, u64)> = sim
        .core_starts
        .iter()
        .filter_map(|s| Some((s.trace.as_ref()?, (s.file + s.wal) / PAGE)))
        .filter(|(_, file)| *file > 256)
        .collect();
    if !copies.is_empty() {
        let by_kind = copies.iter().all(|(t, _)| t.defect().is_some());
        let worst = copies
            .iter()
            .filter_map(|(t, file)| {
                let proof = t.statements.iter().find(|s| s.proof())?;
                Some(proof.cold.unwrap_or(proof.reads) as f64 / *file as f64)
            })
            .fold(0.0, f64::max);
        sim.measures
            .insert(PROOF_BY_KIND, if by_kind { 1.0 } else { 0.0 });
        sim.measures.insert(PROOF_FILES, worst);
    }
    for (check, problem) in problems {
        sim.violation(check, problem);
    }
}

/// Tables whose rows grow with history: the events table (bounded by
/// retention, but the store's largest) and every per-job table.
pub fn history_table(table: &str) -> bool {
    table == "events" || matches!(core_budget(table), Some(Budget::Retained(_)))
}

/// A startup statement allowed to walk tables that grow with history whole,
/// known by a fragment of its own text: the tables it may walk, whether that
/// includes the events table itself (not only an index of it), the most
/// full-scan steps it may make per retained row of those tables, and why.
pub struct ScanAllowance {
    pub marker: &'static str,
    pub tables: &'static [&'static str],
    pub events_table: bool,
    pub steps_per_row: u64,
    pub why: &'static str,
}

/// Every startup statement that walks a table growing with history, found by
/// reading every statement's bytecode on every copy. All but the first three
/// are startup checks over the per-job tables, one walk of each listed table
/// (one full-scan step per row), which grow with the pinned per-job defect.
pub const SCAN_ALLOWANCES: &[ScanAllowance] = &[
    ScanAllowance {
        marker: "PRAGMA quick_check",
        tables: &[
            "events",
            "jobs",
            "job_resources",
            "container_plans",
            "container_results",
            "stack_plans",
            "stack_results",
            "job_resolutions",
        ],
        events_table: true,
        steps_per_row: 0,
        why: "the integrity check reads every page of the file by design (its page reads are \
              bounded per page of file by `startup under its bounds`); it makes no full-scan \
              steps",
    },
    ScanAllowance {
        marker: "PRAGMA foreign_key_check",
        tables: &[
            "events",
            "jobs",
            "job_resources",
            "container_plans",
            "container_results",
            "stack_plans",
            "stack_results",
            "job_resolutions",
        ],
        events_table: true,
        steps_per_row: 0,
        why: "the foreign-key check, which the lock and resource invariants run through \
              `pragma_foreign_key_check` as a statement of its own, walks every table that has \
              a foreign key by design, the events table included (bounded by retention); it \
              makes no full-scan steps, and its page reads are inside the invariants \
              statement's cold run, so in the startup trend",
    },
    ScanAllowance {
        marker: "(SELECT count(*) FROM events)",
        tables: &["events"],
        events_table: false,
        steps_per_row: 0,
        why: "the audit reconciliation counts the events table through its smallest index \
              (`Count`, no full-scan steps); the events table is bounded by retention",
    },
    ScanAllowance {
        marker: "SELECT EXISTS( SELECT 1 FROM jobs j WHERE NOT EXISTS( SELECT 1 FROM job_resources r",
        tables: &["jobs", "job_resolutions"],
        events_table: false,
        steps_per_row: 1,
        why: "the lock and resource invariants walk every job",
    },
    ScanAllowance {
        marker: "SELECT id,principal,intent,digest,resource,revision,deadline FROM jobs WHERE json_extract(intent,'$.operation')",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the storage target and mount checks list their jobs by operation, a walk of jobs",
    },
    ScanAllowance {
        marker: "SELECT EXISTS(SELECT 1 FROM backup_preparation_plans",
        tables: &["jobs", "job_resources"],
        events_table: false,
        steps_per_row: 1,
        why: "the backup-preparation check walks jobs and job resources once each",
    },
    ScanAllowance {
        marker: "SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='backup_prepare_private",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the backup-preparation check walks jobs by operation",
    },
    ScanAllowance {
        marker: "SELECT EXISTS(SELECT 1 FROM automation_occurrences o JOIN automation_schedules s",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the automation check walks jobs by operation",
    },
    ScanAllowance {
        marker: "SELECT j.id,j.principal,j.idempotency,j.intent,j.digest,j.resource,j.state,j.revision,j.deadline,o.s",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the automation job listing walks jobs by operation",
    },
    ScanAllowance {
        marker: "UPDATE jobs SET state='needs_intervention' WHERE state IN ('running','verifying')",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 0,
        why: "startup recovery: its lock-release trigger programs can walk jobs' indexes, but \
              the statement itself searches `jobs_state` (no full-scan steps)",
    },
];

/// Full-scan steps any startup statement may make beyond its allowance and
/// its walks of bounded tables (each bounded table's rows, once per run): a
/// walk's own last step and a statement's rounding, measured at most 2.
pub const SMALL_SCAN_STEPS: u64 = 16;

/// Rows a table walked one full-scan step a row (an allowance's
/// `steps_per_row` 1) must hold before an open that counted no step at all
/// is taken to have lost its counters, not to have had nothing to walk.
pub const MEASURED_WALK: u64 = 64;

/// Page reads an open may make outside every statement it executes
/// (preparing statements, reading the schema): measured 16 on every copy of
/// every run (`outside`), the schema's pages; twice that.
pub const OUTSIDE_READS: u64 = 32;

/// The most VM steps the statements of one open (but the pinned proof check)
/// may take: `VM_PER_ROW` per row retained in any table, plus `VM_FIXED`
/// (the schema reads and checks of an empty store): linear in what the store
/// holds. Measured (see `docs/soak.md`): see the report's
/// "VM steps per retained row".
pub const VM_PER_ROW: u64 = 64;
pub const VM_FIXED: u64 = 200_000;

/// What a traced open's statements do that its checks do not allow: judged
/// on every b-tree each statement's program walks whole (`startup::Scan`),
/// under any alias or `NOT INDEXED`, and on the full-scan steps it made.
/// - No statement walks the events table itself unless its allowance says
///   so (only the integrity check).
/// - Only a statement in `SCAN_ALLOWANCES` walks a table growing with
///   history (or an index of one), and only the tables listed for it.
/// - Each statement's full-scan steps stay within its listed tables' rows
///   times its steps per row, plus its walks of bounded tables (their rows,
///   once per run), plus `SMALL_SCAN_STEPS`.
/// - The open's VM steps, but the pinned proof check's, stay linear in the
///   rows the store holds (`VM_PER_ROW`, `VM_FIXED`).
/// - Every statement's bytecode was read (a statement whose walks are
///   unknown is not taken to walk nothing), and the counters were: a store
///   with history to walk whose open made no VM step or no full-scan step
///   at all was not measured.
pub fn scan_problems(trace: &super::startup::Trace, replay: u64) -> Vec<String> {
    let mut problems = Vec::new();
    for statement in &trace.statements {
        let text = statement.text();
        let allowance = SCAN_ALLOWANCES.iter().find(|a| text.contains(a.marker));
        if let Some(why) = &statement.unread {
            problems.push(format!(
                "a startup statement's bytecode could not be read ({why}), so its walks are unjudged: {}",
                statement.short()
            ));
        }
        for scan in &statement.scans {
            if scan.table == "events" && !scan.index && !allowance.is_some_and(|a| a.events_table) {
                problems.push(format!(
                    "a startup statement walks the events table itself ({} on {}; {} full-scan steps, {} page reads in the open): {}",
                    scan.op,
                    scan.btree,
                    statement.full_scans,
                    statement.reads,
                    statement.short()
                ));
            } else if history_table(&scan.table)
                && !allowance.is_some_and(|a| a.tables.contains(&scan.table.as_str()))
            {
                problems.push(format!(
                    "a startup statement walks {} (a table growing with history) whole through {} ({}) without an allowance: {}",
                    scan.table,
                    scan.btree,
                    scan.op,
                    statement.short()
                ));
            }
        }
        let history = allowance.map_or(0, |a| {
            a.tables.iter().map(|t| trace.rows_of(t)).sum::<u64>() * a.steps_per_row
        });
        let bounded: u64 = statement
            .walked()
            .into_iter()
            .filter(|t| !history_table(t))
            .map(|t| trace.rows_of(t))
            .sum::<u64>()
            * statement.runs.max(1);
        let allowed = history + bounded + SMALL_SCAN_STEPS;
        if statement.full_scans > allowed {
            problems.push(format!(
                "a startup statement made {} full-scan steps, more than the {allowed} allowed it ({history} for tables growing with history, {bounded} for bounded tables): {}",
                statement.full_scans,
                statement.short()
            ));
        }
    }
    let walked_rows: u64 = trace
        .statements
        .iter()
        .flat_map(|statement| {
            let text = statement.text();
            let per_row = SCAN_ALLOWANCES
                .iter()
                .find(|a| text.contains(a.marker))
                .map_or(0, |a| a.steps_per_row);
            statement
                .scans
                .iter()
                .filter(move |scan| per_row > 0 && !scan.index && history_table(&scan.table))
                .map(|scan| scan.table.clone())
        })
        .map(|table| trace.rows_of(&table))
        .max()
        .unwrap_or(0);
    let vm_total: u64 = trace.statements.iter().map(|s| s.vm_steps).sum();
    let full_total: u64 = trace.statements.iter().map(|s| s.full_scans).sum();
    if walked_rows >= MEASURED_WALK && (vm_total == 0 || full_total == 0) {
        problems.push(format!(
            "its statements' counters read {vm_total} VM steps and {full_total} full-scan steps, although their bytecode walks a table of {walked_rows} rows one step a row: the counters were not measured"
        ));
    }
    let rows: u64 = trace.rows.values().sum();
    let vm = trace.vm_steps_but_defect();
    let vm_bound = rows * VM_PER_ROW + VM_FIXED;
    if vm > vm_bound {
        problems.push(format!(
            "its statements but the proof check took {vm} VM steps for {rows} retained rows, more than the {vm_bound} linear in them ({VM_PER_ROW} a row and {VM_FIXED})"
        ));
    }
    let outside = OUTSIDE_READS + replay * REPLAY_READS;
    if trace.outside > outside {
        problems.push(format!(
            "{} page reads outside every statement it executed (at most {outside}: {OUTSIDE_READS}, and {REPLAY_READS} for each of {replay} WAL frames to replay)",
            trace.outside
        ));
    }
    problems
}

/// Page reads per page of file and WAL over a run's opens on copies:
/// minimum, median and maximum.
fn page_ratios(starts: &[super::sim::Startup]) -> String {
    let mut ratios: Vec<f64> = starts
        .iter()
        .filter(|s| s.reason == COPY && s.file > 1 << 20)
        .map(|s| pages(s) as f64 / ((s.file + s.wal) / PAGE) as f64)
        .collect();
    if ratios.is_empty() {
        return "no copy over 1 MiB".into();
    }
    ratios.sort_by(f64::total_cmp);
    format!(
        "{:.3} / {:.3} / {:.3}",
        ratios[0],
        ratios[ratios.len() / 2],
        ratios[ratios.len() - 1]
    )
}

fn report(sim: &Sim, profile: &Profile) {
    let mut out = String::new();
    let p = &mut out;
    use std::fmt::Write;
    let _ = writeln!(
        p,
        "\n=== soak {}: {} simulated days in {:.1} s, stores in {} ===",
        profile.name,
        profile.days,
        sim.started.elapsed().as_secs_f64(),
        sim._dir
            .path()
            .parent()
            .map_or("?".into(), |p| p.display().to_string())
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
        let executed = sim.exec.faults().effects.len();
        let _ = writeln!(
            p,
            "receipt store: {actions} action rows and {stacks} stack rows kept of {executed} executes; prune watermark at day {:.2}",
            (watermark - START) as f64 / DAY as f64
        );
    }
    let _ = writeln!(p, "faults: {:?}", sim.faults());
    {
        let faults = sim.exec.faults();
        let _ = writeln!(
            p,
            "executor requests: {:?}; recovery lookups answered with a closure: {}",
            faults.requests, faults.closures
        );
    }
    let _ = writeln!(
        p,
        "core's warnings and errors (site, message, code: count):"
    );
    for ((site, message, code), count) in &sim.logged {
        let why = EXPECTED_LOGS
            .iter()
            .find(|e| e.message == message)
            .map_or("not expected", |e| e.why);
        let _ = writeln!(
            p,
            "  {count:>6} {site}: {message} ({code:?}); expected: {why}"
        );
    }
    {
        let faults = sim.exec.faults();
        let _ = writeln!(
            p,
            "executor: answers never written by kind {:?}; refusals by kind and code {:?}",
            faults.lost, faults.refused
        );
    }
    let _ = writeln!(p, "restarts: {:?}", sim.stats.restarts);
    let _ = writeln!(
        p,
        "retention removed: {:?}; largest batch per rule: {:?}",
        sim.stats.retention_removed, sim.stats.largest_batch
    );
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
        "day | core open on a copy: page reads, MiB of file, ms, MiB read | receipts: page reads, KiB of file, ms"
    );
    let copies = |starts: &[super::sim::Startup]| -> Vec<super::sim::Startup> {
        starts
            .iter()
            .filter(|s| s.reason == COPY)
            .cloned()
            .collect()
    };
    let (core, receipts) = (copies(&sim.core_starts), copies(&sim.executor_starts));
    let step = (core.len() / 10).max(1);
    for (c, r) in core
        .iter()
        .zip(&receipts)
        .step_by(step)
        .chain(core.last().zip(receipts.last()))
    {
        let _ = writeln!(
            p,
            "{:>3} | {:>6} {:>6.1} {:>5} ms {:>6.1} | {:>5} {:>6.0} {:>4} ms",
            (c.at - START) / DAY,
            pages(c),
            mib(c.file + c.wal),
            c.opened.elapsed.as_millis(),
            mib(c.opened.io.rchar),
            pages(r),
            (r.file + r.wal) as f64 / 1024.0,
            r.opened.elapsed.as_millis(),
        );
    }
    let _ = writeln!(
        p,
        "page reads per page of file at open on a copy, min / median / max: core {}; receipts {}",
        page_ratios(&sim.core_starts),
        page_ratios(&sim.executor_starts)
    );
    if let Some(trace) = sim.core_starts.iter().rev().find_map(|s| s.trace.as_ref()) {
        let mut statements: Vec<&super::startup::Statement> = trace.statements.iter().collect();
        statements.sort_by_key(|s| std::cmp::Reverse(s.cold.unwrap_or(0).max(s.reads)));
        let _ = writeln!(
            p,
            "core startup on the last copy: {} statements, {} page reads in the open ({} outside every statement), {} cold; the largest:",
            trace.statements.len(),
            trace.reads(),
            trace.outside,
            trace.cold()
        );
        for s in statements.iter().take(10) {
            let _ = writeln!(
                p,
                "  {:>6} in the open, {:>6} cold, {:>9} VM steps, {:>7} full-scan steps, {} runs: {}",
                s.reads,
                s.cold.map_or("-".into(), |c| c.to_string()),
                s.vm_steps,
                s.full_scans,
                s.runs,
                s.short()
            );
        }
        let _ = writeln!(
            p,
            "  every statement walking a b-tree whole (VM steps, full-scan steps: walks):"
        );
        for s in &trace.statements {
            if s.scans.is_empty() && s.full_scans == 0 {
                continue;
            }
            let walks: Vec<String> = s
                .scans
                .iter()
                .map(|w| format!("{} {}", w.op, w.btree))
                .collect();
            let text = s.text();
            let allowed = SCAN_ALLOWANCES
                .iter()
                .find(|a| text.contains(a.marker))
                .map_or("bounded tables only", |a| a.why);
            let _ = writeln!(
                p,
                "    {:>9} {:>7}: {walks:?}: {} (allowed: {allowed})",
                s.vm_steps,
                s.full_scans,
                s.short()
            );
        }
        let rows: u64 = trace.rows.values().sum();
        let _ = writeln!(
            p,
            "  {rows} rows retained; VM steps but the proof check's: {} ({:.1} a row); rows: {:?}",
            trace.vm_steps_but_defect(),
            trace.vm_steps_but_defect() as f64 / rows.max(1) as f64,
            trace.rows
        );
        if let Some(defect) = trace.defect() {
            let _ = writeln!(p, "  the pinned proof check's plan: {:?}", defect.plan);
        }
    }
    let _ = writeln!(
        p,
        "core opens on crash images (day, WAL frames replayed, MiB of WAL, page reads in the open, outside every statement, statements cold):"
    );
    for c in sim
        .core_starts
        .iter()
        .filter(|s| s.reason == super::startup::CRASH)
    {
        let Some(trace) = &c.trace else { continue };
        let _ = writeln!(
            p,
            "  {:>6.2} {:>5} {:>5.2} {:>6} {:>5} {:>6}",
            (c.at - START) as f64 / DAY as f64,
            c.replay,
            mib(c.wal),
            pages(c),
            trace.outside,
            trace.cold()
        );
    }
    let worst = |starts: &[super::sim::Startup]| {
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
        "idle hours: {} scheduled, {} measured (files written in each: {:?}), skipped: {:?}",
        sim.idle_scheduled,
        sim.idle.len(),
        sim.idle
            .iter()
            .map(|(_, files)| files.len())
            .collect::<Vec<_>>(),
        sim.idle_skipped
    );
    if let Some(last) = sim.samples.last() {
        let _ = writeln!(p, "per-job tables at the end (rows, MiB):");
        for (table, (rows, bytes)) in &last.core.tables {
            if let Some(Budget::Retained(why)) = core_budget(table) {
                let _ = writeln!(p, "  {table}: {rows}, {:.2}", mib(*bytes as u64));
                if table == "jobs" {
                    let _ = writeln!(p, "    kept: {why}");
                }
            }
        }
    }
    for (store, per_day, years) in &sim.startup_projection {
        let _ = writeln!(
            p,
            "{store} startup page reads over the last third: {per_day:+.0}/day; the bound in {years:.0} years at that rate"
        );
    }
    let outside: Vec<u64> = sim
        .core_starts
        .iter()
        .filter_map(|s| Some(s.trace.as_ref()?.outside))
        .collect();
    let beyond: Vec<i64> = sim
        .core_starts
        .iter()
        .filter_map(|s| Some(pages(s) as i64 - s.trace.as_ref()?.cold() as i64))
        .collect();
    let proofs: Vec<u64> = sim
        .core_starts
        .iter()
        .filter_map(|s| s.trace.as_ref()?.defect()?.cold)
        .collect();
    let _ = writeln!(
        p,
        "core opens on copies: page reads outside every statement {:?} .. {:?}; in the open beyond the statements cold {:?} .. {:?}; the proof check cold {:?} .. {:?}",
        outside.iter().min(),
        outside.iter().max(),
        beyond.iter().min(),
        beyond.iter().max(),
        proofs.iter().min(),
        proofs.iter().max()
    );
    let _ = writeln!(p, "measures: {:#?}", sim.measures);
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
        REFUSALS,
        INVARIANTS,
        SIZE,
        BUDGETS,
        STARTUP,
        EXPLAINED,
        MOUNT_PROOF,
        IDLE,
        SETTLED,
        EFFECTS,
        AUTHORITY,
        PROVOKED,
        BATCHES,
        ACCOUNTING,
        CLOSURES,
        LOST_ANSWERS,
        LOGS,
        EPOCHS,
    ];
    if profile.held_open.is_some() {
        checks.push(HELD_OPEN);
    }
    if profile.trends {
        checks.extend([
            FLAT,
            STARTUP_PER_JOB,
            STARTUP_TREND_CORE,
            STARTUP_TREND_RECEIPTS,
            PROJECTION,
        ]);
    }
    let mut verdict = Vec::new();
    for check in checks {
        let known = EXPECTED_FAILURES.iter().find(|k| k.check == check);
        match (failed.get(check), known) {
            (None, None) => eprintln!("PASS  {check}"),
            (Some(lines), Some(known)) => {
                // The failure must be the known defect: every pin in range.
                let outside: Vec<String> = known
                    .pins
                    .iter()
                    // A short run never reaches the proof check's jump.
                    .filter(|(measure, _, _)| profile.trends || *measure != PROOF_FILES)
                    .filter_map(|(measure, low, high)| {
                        let value = sim.measures.get(measure).copied();
                        match value {
                            Some(v) if (*low..=*high).contains(&v) => None,
                            _ => Some(format!("{measure} = {value:?}, pinned to {low}..={high}")),
                        }
                    })
                    .collect();
                if outside.is_empty() {
                    eprintln!(
                        "XFAIL {check}: {} ({} failures, first: {})",
                        known.why,
                        lines.len(),
                        lines[0]
                    );
                } else {
                    eprintln!("FAIL  {check}: a known failure outside what is known: {outside:?}");
                    verdict.push(format!(
                        "{check}: failed, but not as its known defect: {}; first failure: {}",
                        outside.join("; "),
                        lines[0]
                    ));
                }
            }
            (None, Some(_)) => verdict.push(format!(
                "{check}: listed as an expected failure but passed; remove it from EXPECTED_FAILURES"
            )),
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
