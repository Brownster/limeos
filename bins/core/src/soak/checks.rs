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
pub const CLOCK: &str = "every time core stores is on the simulated clock";
pub const BUDGET: &str =
    "the executor's effect budget refuses only the action storm, which it does refuse";

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
        message: RATE_LIMITED,
        error: false,
        codes: &[],
        why: "the action storm's scripted jobs spend a container's executor effect budget \
              (counted exactly, `lost_answers`, and only in the storm, `BUDGET`): the job is \
              settled as refused",
    },
    ExpectedLog {
        message: limeos_executor::container_requests::REFUSAL_LINE,
        error: false,
        codes: &[],
        why: "the executor's journal line for each refusal row of its action log, at most one \
              per container an hour (`BUDGET`)",
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

/// Core's message for an execute the executor's effect budget refused.
pub const RATE_LIMITED: &str = "executor refused the action: a container it would act on has spent its executor effect budget; nothing was recorded or run, and the job is settled as refused with the time the budget frees; read the executor's action log if no one here asked for that many effects";

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
/// 22 MiB of rows and indexes filled by the login burst's small rows, and
/// would be about 60 MiB of the household's rows at the bytes of pages each
/// row kept takes once job-row retention has removed its jobs' events
/// around it (`flatness` asserts that end stays within this bound); the
/// audit cap is the hard stop on rows.
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
/// in thousandths, and a fixed allowance: see `startup_page_bound`. Core's
/// statements but the proof check read 2.14 to 2.79 cold: `quick_check`
/// 1.45 to 1.63 and the foreign-key check 0.44 to 0.65 (both more per page
/// as the share of table pages grows), the one walk of `jobs` with its
/// resource-set probes up to 0.35, the index accounting about 0.06, the
/// audit count 0.07 to 0.17; the low end at the household runs' few hundred
/// jobs, the high end at 5,113 retained jobs
/// (`startup_at_high_retained_job_count`,
/// `caps::soak_core_startup_at_high_retained_job_count`). The household
/// copies read at most 2.27 to 2.42 a page on four seeds of each 90-day
/// profile (24 November), so 3.0 fails a further full pass over the store
/// (1.0 a page) on every one of them; it was 3.2 while eight owners' checks
/// each walked `jobs` (2.4 to 3.0 in the household runs, 3.83 at 5,113
/// jobs).
///
/// The receipt store checks through a cache much smaller than a grown store
/// (768 KiB), so `quick_check` reads what no cache keeps: one walk of every
/// B-tree page, for the structure, and a second of every table's pages, for
/// its NOT NULL and CHECK constraints. That is at most 2 reads per page, 1.0
/// plus the share of the file in tables (about 0.86 measured, receipts being
/// most of it); the open's other statements (schema, watermark, counts) read
/// a few pages more. 2.2 holds that with room for new tables, and fails a
/// further full pass over the store (1.87 plus 1.0). With the executor's
/// action log (`effect_log`, `effect_days`; PR-30) the soak's copies read
/// 1.82 to 1.85 per page on both profiles, and a store with saturated
/// receipts and a full log of the longest rows 1.84 (20,984 KiB read for
/// 11,388 KiB, `measure_the_open_reads_of_a_full_store`), so a further full
/// pass would still fail.
pub const CORE_PAGE_FACTOR: u64 = 3000;
pub const RECEIPT_PAGE_FACTOR: u64 = 2200;
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
     a settled job is removed with all of them 30 days after it settled, but for the newest of \
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
        // The action log keeps its newest rows; its totals their newest days.
        "effect_log" => Rows(limits::LOG_ROWS),
        "effect_days" => Rows(limits::DAYS_KEPT),
        _ => return None,
    })
}

#[derive(Clone, Debug, Default)]
pub struct StoreStat {
    /// Rows and bytes (tables with their indexes, from `dbstat`) per table.
    pub tables: BTreeMap<String, (i64, i64)>,
    /// Bytes of each b-tree (a table's own or one of its indexes, from
    /// `dbstat`), with the table it belongs to.
    pub btrees: BTreeMap<String, (String, i64)>,
    /// Core's events table (left empty for the receipt store): the newest
    /// cursor, the bytes of the leaf cells each of its b-trees was given for
    /// the rows appended since the first checkpoint (`appended_cells`,
    /// counted at the checkpoint after their append, of the rows still
    /// there), and the rows the events and authority rules had removed by
    /// this checkpoint (the writer's passes, as absorbed).
    pub cursor: i64,
    pub appended: BTreeMap<String, i64>,
    pub pruned: i64,
    pub file: u64,
    pub wal: u64,
}
impl StoreStat {
    fn rows(&self, table: &str) -> i64 {
        self.tables.get(table).map_or(0, |t| t.0)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub at: i64,
    pub core: StoreStat,
    pub receipts: StoreStat,
    /// Settled container and stack jobs job-row retention may remove at
    /// this moment, by the soak's own query (`overdue_jobs` with no day's
    /// allowance): every other job is one the rule keeps.
    pub removable: i64,
    /// Rows of each per-job table that belong to those removable jobs.
    pub removable_rows: BTreeMap<String, i64>,
    /// Settled container and stack jobs kept with no result row (refused
    /// before any effect: a stale plan, a closure, the effect budget): the
    /// small jobs whose replacement by full-size ones the per-job model
    /// explains table by table.
    pub kept_without_result: i64,
    /// Of those, jobs the executor's effect budget refused.
    pub kept_budget_refused: i64,
    /// Unqueued plans (no job) past their expiry, per plan table: they own
    /// no job and explain no bytes.
    pub expired_unqueued: BTreeMap<String, i64>,
    /// Unqueued plans that expired before the newest plan of their table was
    /// created, which removes every expired one (`plan_container`,
    /// `plan_stack`): a removal that did not happen.
    pub stale_unqueued: i64,
    /// Jobs owning more rows of some per-job table than one job can.
    pub jobs_over_cap: i64,
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
    let btrees: BTreeMap<String, (String, i64)> = conn
        .prepare("SELECT d.name,m.tbl_name,sum(d.pgsize) FROM dbstat d JOIN sqlite_master m ON m.name=d.name GROUP BY d.name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
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
        btrees,
        file: len(path),
        wal: len(Path::new(&wal)),
        ..StoreStat::default()
    }
}

/// The events b-trees `appended_cells` knows: the table, keyed by `cursor`,
/// and each index with its key columns (`schema.sql`, `migration-v17.sql`).
/// An events index added later fails the flatness check until it is given
/// its key here.
const EVENT_BTREES: [&str; 5] = [
    "events",
    "events_scope",
    "events_job",
    "events_kind",
    "events_notification",
];

/// Bytes of a SQLite varint holding `v`.
fn varint_len(v: i64) -> i64 {
    let mut v = v as u64;
    let mut n = 1;
    while v >= 0x80 && n < 9 {
        v >>= 7;
        n += 1;
    }
    n
}

/// Bytes of the record SQLite writes for `values` (file format 4): its
/// header (its own length and one serial type per value) and its body.
fn record_len(values: &[&rusqlite::types::Value]) -> i64 {
    use rusqlite::types::Value;
    let (mut types, mut body) = (0, 0);
    for value in values {
        let (serial, bytes) = match value {
            Value::Null => (0, 0),
            Value::Integer(0 | 1) => (8, 0),
            Value::Integer(i) => {
                // Serial types 1 to 6: 1, 2, 3, 4, 6 and 8 bytes.
                match i.unsigned_abs() {
                    0..0x80 => (1, 1),
                    0x80..0x8000 => (2, 2),
                    0x8000..0x80_0000 => (3, 3),
                    0x80_0000..0x8000_0000 => (4, 4),
                    0x8000_0000..0x8000_0000_0000 => (5, 6),
                    _ => (6, 8),
                }
            }
            Value::Real(_) => (7, 8),
            Value::Text(t) => (13 + 2 * t.len() as i64, t.len() as i64),
            Value::Blob(b) => (12 + 2 * b.len() as i64, b.len() as i64),
        };
        types += varint_len(serial);
        body += bytes;
    }
    let header = types + 1;
    header + varint_len(header) - 1 + body
}

/// The bytes of leaf cells (with their cell pointers) each events b-tree
/// holds for the rows after `cursor` through `through`: a table cell is the payload's length,
/// the rowid and the record (`cursor` is the rowid, so the record holds
/// NULL for it); an index cell the payload's length and the record of its
/// key columns and the rowid. The partial notification index has a cell
/// only for notification events.
fn appended_cells(conn: &Connection, cursor: i64, through: i64) -> BTreeMap<String, i64> {
    use rusqlite::types::Value;
    let mut cells: BTreeMap<String, i64> =
        EVENT_BTREES.iter().map(|b| ((*b).to_owned(), 0)).collect();
    let mut stmt = conn
        .prepare(
            "SELECT cursor,principal,job,kind,created,json_extract(kind,'$.kind'),json_extract(kind,'$.notification') FROM events WHERE cursor>? AND cursor<=? ORDER BY cursor",
        )
        .unwrap();
    let mut rows = stmt.query([cursor, through]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let value = |i: usize| row.get::<_, Value>(i).unwrap();
        let (rowid, principal, job, kind, created, by_kind, notification) = (
            value(0),
            value(1),
            value(2),
            value(3),
            value(4),
            value(5),
            value(6),
        );
        let Value::Integer(id) = rowid else {
            unreachable!("cursor is the rowid")
        };
        let table = record_len(&[&Value::Null, &principal, &job, &kind, &created]);
        *cells.get_mut("events").unwrap() += varint_len(table) + varint_len(id) + table + 2;
        let mut index = |name: &str, key: &Value| {
            let record = record_len(&[key, &rowid]);
            *cells.get_mut(name).unwrap() += varint_len(record) + record + 2;
        };
        index("events_scope", &principal);
        index("events_job", &job);
        index("events_kind", &by_kind);
        if notification != Value::Null {
            index("events_notification", &notification);
        }
    }
    cells
}

fn receipts_path(sim: &Sim) -> std::path::PathBuf {
    sim.exec.dir.join("receipts.sqlite")
}

/// The daily checkpoint (04:00, the house asleep): sizes, row counts against
/// budgets, and the invariants. Both daemons then restart, measured.
pub fn checkpoint(sim: &mut Sim) {
    let at = sim.now();
    sim.absorb_passes();
    let mut core = stat(&sim.core_path);
    {
        let conn = reader(&sim.core_path).unwrap();
        let since = sim
            .samples
            .last()
            .map(|s| (s.core.cursor, s.core.appended.clone()));
        let (after, mut appended) = since.unwrap_or_default();
        core.cursor = conn
            .query_row("SELECT coalesce(max(cursor),0) FROM events", [], |r| {
                r.get(0)
            })
            .unwrap();
        for (btree, bytes) in appended_cells(&conn, after, core.cursor) {
            *appended.entry(btree).or_default() += bytes;
        }
        core.appended = appended;
        core.pruned = ["events", "authority events"]
            .iter()
            .map(|rule| sim.stats.retention_removed.get(rule).copied().unwrap_or(0))
            .sum();
    }
    let sample = Sample {
        at,
        core,
        receipts: stat(&receipts_path(sim)),
        removable: overdue_jobs(&reader(&sim.core_path).unwrap(), at, 0),
        removable_rows: removable_rows(&reader(&sim.core_path).unwrap(), at),
        kept_without_result: kept_without_result(&reader(&sim.core_path).unwrap()),
        kept_budget_refused: kept_budget_refused(&reader(&sim.core_path).unwrap()),
        expired_unqueued: unqueued_plans(&reader(&sim.core_path).unwrap(), at).0,
        stale_unqueued: unqueued_plans(&reader(&sim.core_path).unwrap(), at).1,
        jobs_over_cap: jobs_over_cap(&reader(&sim.core_path).unwrap()),
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
    if sample.stale_unqueued > 0 {
        problems.push((
            BUDGETS,
            format!(
                "core: {} unqueued plans expired before a newer plan of their table was created, and remain",
                sample.stale_unqueued
            ),
        ));
    }
    if sample.jobs_over_cap > 0 {
        problems.push((
            BUDGETS,
            format!(
                "core: {} jobs own more rows of a per-job table than one job can",
                sample.jobs_over_cap
            ),
        ));
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
    let overdue = overdue_jobs(&reader(&sim.core_path).unwrap(), sample.at, DAY);
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
    // Every time core stores is the simulated clock's (its writer's, by
    // which job-row retention ages jobs): none after now. Startup recovery's
    // events once took the system clock, months ahead, which kept every job
    // they name past retention for the whole run.
    let future: Vec<(String, i64)> = reader(&sim.core_path)
        .unwrap()
        .prepare(
            "SELECT 'events.created',count(*) FROM events WHERE created>?1 \
             UNION ALL SELECT 'jobs.settled',count(*) FROM jobs WHERE settled>?1 \
             UNION ALL SELECT 'container_results.recorded',count(*) FROM container_results WHERE recorded>?1 \
             UNION ALL SELECT 'stack_results.recorded',count(*) FROM stack_results WHERE recorded>?1 \
             UNION ALL SELECT 'job_resolutions.completed',count(*) FROM job_resolutions WHERE completed>?1",
        )
        .unwrap()
        .query_map([at], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (column, count) in future {
        if count > 0 {
            let detail: String = if column == "events.created" {
                reader(&sim.core_path)
                    .unwrap()
                    .prepare("SELECT json_extract(kind,'$.kind'),count(*),max(created)-?1 FROM events WHERE created>?1 GROUP BY 1")
                    .unwrap()
                    .query_map([at], |r| {
                        Ok(format!(
                            " {}: {} (up to {} s ahead)",
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?
                        ))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
                    .join(",")
            } else {
                String::new()
            };
            problems.push((
                CLOCK,
                format!("core: {count} rows with {column} after the simulated now{detail}"),
            ));
        }
    }
    // The jobs startup recovery marked, to report what job-row retention
    // did with them.
    let recovered: Vec<String> = reader(&sim.core_path)
        .unwrap()
        .prepare("SELECT DISTINCT job FROM events WHERE json_extract(kind,'$.kind')='job_recovery_required' AND job IS NOT NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    sim.recovered.extend(recovered);
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
/// of its query): settled, and unchanged since, a day before the horizon, past the newest
/// `JOB_FLOOR` of their family (by settlement), with no open resolution
/// request and no authority event. A day is 1,440 passes of up to 64 jobs,
/// and 24 starts of the rule's scan over the jobs a reference kept. With
/// `allowance` 0, the jobs the rule may remove right now.
const FAMILIES: [&str; 2] = [
    "('container_restart','container_start','container_stop')",
    "('stack_restart','stack_start','stack_stop')",
];
/// The jobs of `family` job-row retention may remove at `?1` (the cutoff:
/// the horizon, less any allowance) with the floor `?2`, as a `SELECT j.id`.
fn overdue_sql(family: &str) -> String {
    let settled = format!(
        "j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') IN {family}"
    );
    format!(
        "SELECT j.id FROM jobs j WHERE {settled} AND j.id NOT IN (SELECT j.id FROM jobs j WHERE {settled} ORDER BY j.settled DESC,j.rowid DESC LIMIT ?2) AND j.settled<?1 AND j.deadline<?1 \
         AND NOT EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND (e.created>=?1 OR json_extract(e.kind,'$.kind') GLOB 'job_resolution_*')) \
         AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND (r.confirmation IS NULL OR r.completed>=?1)) \
         AND NOT EXISTS(SELECT 1 FROM container_results x WHERE x.job=j.id AND x.recorded>=?1) \
         AND NOT EXISTS(SELECT 1 FROM stack_results x WHERE x.job=j.id AND x.recorded>=?1)"
    )
}
fn overdue_jobs(conn: &Connection, now: i64, allowance: i64) -> i64 {
    let cutoff = now - limeos_persistence::test_support::JOB_HORIZON - allowance;
    let floor = limeos_persistence::test_support::JOB_FLOOR;
    FAMILIES
        .iter()
        .map(|family| {
            conn.query_row(
                &format!("SELECT count(*) FROM ({})", overdue_sql(family)),
                rusqlite::params![cutoff, floor],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        })
        .sum()
}
/// Rows of each per-job table belonging to the jobs `overdue_jobs` counts
/// (no allowance) at `now`.
fn removable_rows(conn: &Connection, now: i64) -> BTreeMap<String, i64> {
    let cutoff = now - limeos_persistence::test_support::JOB_HORIZON;
    let floor = limeos_persistence::test_support::JOB_FLOOR;
    let removable = FAMILIES.map(overdue_sql).join(" UNION ");
    let mut rows = BTreeMap::new();
    for table in [
        "jobs",
        "job_resources",
        "container_plans",
        "container_results",
        "stack_plans",
        "stack_results",
        "job_resolutions",
    ] {
        let column = if table == "jobs" { "id" } else { "job" };
        let count = conn
            .query_row(
                &format!("WITH removable(id) AS ({removable}) SELECT count(*) FROM {table} WHERE {column} IN removable"),
                rusqlite::params![cutoff, floor],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        rows.insert(table.to_owned(), count);
    }
    rows
}
/// Settled container and stack jobs refused by the effect budget.
fn kept_budget_refused(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT count(DISTINCT e.job) FROM events e JOIN jobs j ON j.id=e.job WHERE json_extract(e.kind,'$.kind')='executor_budget_spent'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
/// The plan tables whose unqueued rows (`job IS NULL`) are removed once
/// expired, when the next plan of the table is created.
const PLAN_TABLES: [&str; 2] = ["container_plans", "stack_plans"];
/// Per plan table, unqueued plans expired at `now`; and in all, those that
/// expired before the newest plan of their table was created (its creation
/// removes every expired one, and a plan's expiry is its creation plus the
/// plan lifetime).
fn unqueued_plans(conn: &Connection, now: i64) -> (BTreeMap<String, i64>, i64) {
    let mut expired = BTreeMap::new();
    let mut stale = 0;
    for table in PLAN_TABLES {
        let (gone, left): (i64, i64) = conn
            .query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM {table} WHERE job IS NULL AND expires<=?1), \
                     (SELECT count(*) FROM {table} WHERE job IS NULL AND expires<=(SELECT max(expires) FROM {table})-?2)"
                ),
                rusqlite::params![now, limeos_domain::PLAN_TTL_SECONDS],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        expired.insert(table.to_owned(), gone);
        stale += left;
    }
    (expired, stale)
}
/// The most rows of each per-job table one job can own: a stack job's
/// resources are the stack and each member; one plan, result and resolution.
fn rows_per_job(table: &str) -> i64 {
    match table {
        "job_resources" => 1 + limeos_domain::MAX_STACK_MEMBERS as i64,
        _ => 1,
    }
}
/// Jobs owning more rows of a per-job table than `rows_per_job` allows.
fn jobs_over_cap(conn: &Connection) -> i64 {
    [
        "job_resources",
        "container_plans",
        "container_results",
        "stack_plans",
        "stack_results",
        "job_resolutions",
    ]
    .iter()
    .map(|table| {
        conn.query_row(
            &format!(
                "SELECT count(*) FROM (SELECT job FROM {table} WHERE job IS NOT NULL GROUP BY job HAVING count(*)>?1)"
            ),
            [rows_per_job(table)],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    })
    .sum()
}
/// Settled container and stack jobs with no result row.
fn kept_without_result(conn: &Connection) -> i64 {
    conn.query_row(
        &format!(
            "SELECT count(*) FROM jobs j WHERE j.state IN ('succeeded','failed','canceled','precondition_changed','resolved') AND json_extract(j.intent,'$.operation') IN {} \
             AND NOT EXISTS(SELECT 1 FROM container_results x WHERE x.job=j.id) AND NOT EXISTS(SELECT 1 FROM stack_results x WHERE x.job=j.id)",
            "('container_restart','container_start','container_stop','stack_restart','stack_start','stack_stop')"
        ),
        [],
        |r| r.get(0),
    )
    .unwrap()
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
/// may prune (`now - RECEIPT_RETAIN_AFTER_EXPIRY`): it is only ever an
/// expiry that was really pruned, never a clock reading.
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
    let cutoff = sim.now() - limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY;
    if watermark < sim.last_watermark {
        problems.push(format!(
            "{when}: the prune watermark went back from {} to {}",
            watermark_day(sim.last_watermark),
            watermark_day(watermark)
        ));
    }
    if watermark > cutoff {
        problems.push(format!(
            "{when}: the prune watermark ({}) is past the youngest expiry retention may prune (day {:.3}): a receipt was pruned too early",
            watermark_day(watermark),
            (cutoff - START) as f64 / DAY as f64
        ));
    }
    sim.last_watermark = sim.last_watermark.max(watermark);
    for problem in problems {
        sim.violation(ACCOUNTING, problem);
    }
}

/// A prune watermark as a simulated day, or "nothing pruned" for a store that
/// has pruned nothing yet (its watermark below every expiry).
fn watermark_day(watermark: i64) -> String {
    if watermark == i64::MIN {
        "nothing pruned".into()
    } else {
        format!("day {:.3}", (watermark - START) as f64 / DAY as f64)
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
    effect_budget(sim, profile);
    if profile.trends {
        trends(sim);
    }
    startup_bounds(sim);
    report(sim, profile);
    verdict(sim, profile);
}

/// The executor's effect budget (ADR 0013) against the household: nothing
/// but the action storm meets it. Refusals are answered from the storm's
/// first minute until a day after its last day's last proposal, the daily window in
/// which the storm's charges still count (a household job on a container
/// the storm spent that evening is refused too), plus an hour for the
/// storm's queue to drain; never otherwise. A storm long enough to spend a
/// day's budget (six hours of a stack action every 45 s charge each stacked
/// container about 25 effects an hour) is refused at least once.
fn effect_budget(sim: &mut Sim, profile: &Profile) {
    let (day, hour, minutes) = profile.action_burst;
    let start = START + day * DAY + hour * 3600;
    let storm_end = start + (profile.storm_days - 1) * DAY + minutes * MINUTE + 3600;
    let end = storm_end + limeos_executor_container::limit::DAILY.seconds;
    let times = sim.exec.faults().rate_limited.clone();
    let outside: Vec<_> = times.iter().filter(|&&at| at < start || at > end).collect();
    if let Some(first) = outside.first() {
        sim.violation(
            BUDGET,
            format!(
                "{} executes refused by the effect budget outside the action storm, the first on day {} at {:02}:{:02}",
                outside.len(),
                (**first - START) / DAY,
                (**first - START).rem_euclid(DAY) / 3600,
                (**first - START).rem_euclid(3600) / 60
            ),
        );
    }
    // A storm long enough to spend a day's budget, hammered by a script that
    // reads no job history, sees its own jobs settled as refused by the
    // budget again and again (counted from the jobs' progress, as its client
    // sees them; a refusal whose answer was lost settles otherwise and is
    // not counted).
    let hammered = sim.careless_until;
    let while_hammering = sim
        .stats
        .budget_refused_seen
        .iter()
        .filter(|&&at| at < hammered + 3600)
        .count();
    if minutes >= 360 && day < profile.days && while_hammering < HAMMERED_AT_LEAST {
        sim.violation(
            BUDGET,
            format!(
                "the {minutes}-minute action storm saw {while_hammering} of its jobs settled as refused by the effect budget, fewer than {HAMMERED_AT_LEAST}"
            ),
        );
    }
    sim.measures.insert(BUDGET_HAMMERED, while_hammering as f64);
    sim.measures.insert(BUDGET_REFUSALS, times.len() as f64);
    sim.measures.insert(
        BUDGET_AFTER,
        times.iter().filter(|&&at| at > storm_end).count() as f64,
    );
}
/// The storm's jobs a day of hammering must see settled as refused by the
/// budget, at least (a day of it met about 800 refusals).
const HAMMERED_AT_LEAST: usize = 100;
pub const BUDGET_HAMMERED: &str =
    "the storm's own jobs its client saw settled as refused by the budget while it hammered";
pub const BUDGET_AFTER: &str =
    "of them, household jobs refused within a day of the storm (its charges still count)";
pub const BUDGET_REFUSALS: &str = "executes refused by the executor's effect budget";

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
    // A refusal before any row has its own message (`BEYOND_HISTORY`,
    // `RATE_LIMITED`).
    let (mut beyond, mut limited) = (0, 0);
    for ((kind, code), count) in &refused {
        if executes.contains(kind) {
            if code == "BeyondReceiptHistory" {
                beyond += count;
            } else if code == "RateLimited" {
                limited += count;
            } else {
                *expected.entry(code.clone()).or_default() += count;
            }
        }
    }
    expected.retain(|_, count| *count > 0);
    let mut logged: BTreeMap<String, u64> = BTreeMap::new();
    let (mut logged_beyond, mut logged_limited) = (0, 0);
    for ((_, message, code), count) in &sim.logged {
        if message == EXECUTE_FAILED {
            *logged
                .entry(code.clone().unwrap_or_else(|| "none".into()))
                .or_default() += count;
        } else if message == BEYOND_HISTORY {
            logged_beyond += count;
        } else if message == RATE_LIMITED {
            logged_limited += count;
        }
    }
    if logged_limited != limited {
        sim.violation(
            LOST_ANSWERS,
            format!(
                "core logged {logged_limited} executes refused by the effect budget; the executor refused {limited}"
            ),
        );
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
    for (store, pick, budget, file_bound) in [
        (
            "core",
            (|s: &Sample| &s.core) as fn(&Sample) -> &StoreStat,
            core_budget as fn(&str) -> Option<Budget>,
            CORE_FILE_BOUND,
        ),
        (
            "receipts",
            |s: &Sample| &s.receipts,
            receipt_budget,
            RECEIPT_FILE_BOUND,
        ),
    ] {
        let last = &samples[samples.len() * 2 / 3..];
        let span = day(last.last().unwrap()) - day(&last[0]);
        let tables: Vec<String> = pick(last.last().unwrap()).tables.keys().cloned().collect();
        let points: Vec<(f64, &StoreStat)> = last.iter().map(|s| (day(s), pick(s))).collect();
        let flat = flatness(store, &points, file_bound);
        for tree in &flat.events {
            sim.flat_lines.push(format!(
                "{store}: {} grew {:.0} KiB over the last third; the rows appended to it explain at most {:.0} KiB",
                tree.name,
                tree.grown / 1024.0,
                tree.ceiling / 1024.0
            ));
        }
        if let Some(refill) = &flat.refill {
            sim.measures.insert(EVENTS_PER_ROW, refill.per_row);
            sim.measures
                .insert(EVENTS_AT_BUDGET, refill.at_budget / 1048576.0);
        }
        if let Some(why) = &flat.unprojected {
            sim.flat_lines.push(format!(
                "{store}: the events table's end is not projected: {why}"
            ));
        }
        problems.extend(flat.problems.into_iter().map(|p| (FLAT, p)));
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
    // The per-job tables on their own, beside the whole store's flatness
    // (whose 2% of 30 MiB would hide a slow leak): `per_job_model`, from the
    // first checkpoint of the last third with at most a tenth more jobs than
    // the last (job-row retention works off the action storm's jobs, of
    // another size, in its first days).
    let settled_in = last
        .iter()
        .position(|s| s.core.rows("jobs") * 10 <= final_.core.rows("jobs") * 11)
        .unwrap_or(0)
        .min(last.len().saturating_sub(8));
    let tail: Vec<&Sample> = last[settled_in..].iter().collect();
    let model = per_job_model(&tail);
    sim.measures.insert(PER_JOB_NET, model.net / 1024.0);
    sim.measures
        .insert(PER_JOB_NET_JOBS, model.jobs_only / 1024.0);
    sim.measures.insert(PER_JOB_ROWS, model.rows_excess);
    let (start, end) = (tail[0], tail[tail.len() - 1]);
    sim.measures
        .insert(WITHOUT_RESULT_START, start.kept_without_result as f64);
    sim.measures
        .insert(WITHOUT_RESULT_END, final_.kept_without_result as f64);
    sim.measures
        .insert(BUDGET_REFUSED_START, start.kept_budget_refused as f64);
    sim.measures
        .insert(BUDGET_REFUSED_END, end.kept_budget_refused as f64);
    problems.extend(model.problems.into_iter().map(|p| (FLAT, p)));
    // Core startup on the nightly copies: the cold page reads of its known
    // statements but the proof check (a defect of its own, `MOUNT_PROOF`,
    // whose cache eviction would otherwise blur this series; the open is
    // tied to the sum by `EXPLAINED`), against the history tables' growth on
    // the same nights (the per-job tables, and the events table refilling to
    // its budget after job-row retention removed history with its jobs), over
    // the last third. Growth is taken between the
    // medians of the two halves of the last third.
    let nights: Vec<(f64, f64, f64, f64)> = sim
        .core_starts
        .iter()
        .filter_map(|c| {
            let trace = c.trace.as_ref()?;
            let sample = samples.iter().find(|s| s.at == c.at)?;
            Some((
                (c.at - START) as f64 / DAY as f64,
                trace.cold_but_defect() as f64,
                per_job_bytes(sample) / PAGE as f64,
                events_bytes(sample) / PAGE as f64,
            ))
        })
        .collect();
    let tail = &nights[nights.len() * 2 / 3..];
    let reads: Vec<(f64, f64)> = tail.iter().map(|n| (n.0, n.1)).collect();
    let per_job_pages: Vec<(f64, f64)> = tail.iter().map(|n| (n.0, n.2)).collect();
    let events_pages: Vec<(f64, f64)> = tail.iter().map(|n| (n.0, n.3)).collect();
    let startup_per_day = half_slope(&reads);
    let per_job_per_day = half_slope(&per_job_pages);
    let events_per_day = half_slope(&events_pages);
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
    // Each history table's growth explains reads at its own rate: the
    // per-job tables', `STARTUP_PER_JOB_MAX` a page, and the events
    // table's (refilling to its budget), `STARTUP_PER_EVENT_MAX`; growth
    // beyond both (and `STARTUP_SLACK`) is something else.
    let explained = STARTUP_PER_JOB_MAX * per_job_per_day.max(0.0)
        + STARTUP_PER_EVENT_MAX * events_per_day.max(0.0);
    sim.measures.insert(STARTUP_PER_JOB_PAGES, per_job_per_day);
    sim.measures.insert(STARTUP_EVENT_PAGES, events_per_day);
    if startup_per_day > explained + STARTUP_SLACK {
        problems.push((
            STARTUP_PER_JOB,
            format!(
                "core opens read {startup_per_day:.0} more pages a night while the per-job tables gain {per_job_per_day:.1} pages and the events table {events_per_day:.1}: their growth explains {explained:.0} ({STARTUP_PER_JOB_MAX} and {STARTUP_PER_EVENT_MAX} reads a page) and {STARTUP_SLACK} more"
            ),
        ));
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

/// The events table regaining rows toward its budget over the last third.
#[derive(Debug)]
struct Refill {
    /// Bytes its b-trees gained per row it kept: their fitted growth over
    /// the fitted growth of its rows.
    per_row: f64,
    /// Every other b-tree's bytes at the end, with the events table at its
    /// row budget at `per_row` bytes a row (or its average a row now, if
    /// that is more).
    at_budget: f64,
}

/// One events b-tree over the last third: its fitted growth, and the most
/// the rows appended to it explain.
#[derive(Debug)]
struct EventsTree {
    name: String,
    grown: f64,
    ceiling: f64,
}

/// What the flatness check found in one store over the last third.
#[derive(Debug)]
struct Flat {
    problems: Vec<String>,
    events: Vec<EventsTree>,
    refill: Option<Refill>,
    /// Why the events table's end was not projected, if it was not.
    unprojected: Option<String>,
}

/// Rows the events table must regain over the last third (fitted) for its
/// bytes a row kept to be measured (the runs regain about 1,200).
const REFILL_MIN_ROWS: f64 = 256.0;
/// The fitted growth a b-tree outside the events table may have over the
/// last third beyond 2% of its size. The per-job tables' b-trees grow while
/// their families refill toward job-row retention's floor (`stack_results`
/// 142 KiB in both traced runs of 23 November, its rows from 87 to 131 of
/// the 128 newest stack jobs and those kept besides; every other per-job
/// b-tree under 20 KiB): 192 KiB. Every other core b-tree measured under
/// 3 KiB: 32 KiB, eight pages. The receipt store's tables are still filling
/// toward their row budgets (0.21 MiB over the third in all): 256 KiB.
const PER_JOB_BTREE_FLOOR: f64 = 192.0 * 1024.0;
const BTREE_FLOOR: f64 = 32.0 * 1024.0;
const RECEIPT_BTREE_FLOOR: f64 = 256.0 * 1024.0;
/// The least fill of the pages an appended events row's cells go to: the
/// table appends at its right edge, where SQLite packs pages full
/// (measured 97 to 99%); an index within its key ranges (`events_scope`
/// measured 86%, the others lower than their cells because they reuse
/// holes). Beside them, interior pages: `APPEND_SLACK`.
const TABLE_FILL: f64 = 0.90;
const INDEX_FILL: f64 = 0.75;
const APPEND_SLACK: f64 = 16.0 * 1024.0;
/// How much larger the cells of a row appended in the second half of the
/// third may be than of one appended in the first (measured 0.2% apart).
const ROW_SIZE_DRIFT: f64 = 1.20;

/// The least fitted growth a b-tree of `table` in `store` may have, beyond
/// 2% of its size.
fn btree_floor(store: &str, table: &str) -> f64 {
    if store == "receipts" {
        RECEIPT_BTREE_FLOOR
    } else if matches!(core_budget(table), Some(Budget::Retained(_))) {
        PER_JOB_BTREE_FLOOR
    } else {
        BTREE_FLOOR
    }
}

/// Sizes flat over the last third (`points`: day and the store's stat at
/// each checkpoint), every b-tree on its own, so that one table or index
/// growing cannot hide behind another shrinking.
///
/// Every b-tree outside the events table: fitted growth under 2% of its size
/// or its floor (`btree_floor`).
///
/// The events table's b-trees, each against what the rows appended to it
/// explain: at most the leaf cells it was given for them (`appended_cells`,
/// exact from the rows), at `TABLE_FILL` or `INDEX_FILL`, plus
/// `APPEND_SLACK`. Nothing else is credited: the holes job-row retention
/// leaves when it removes a job's events are taken as never reused (a job's
/// events lie between the job-less events of their days, logins and plans,
/// whose pages stay; rows are appended at the end of the table and of each
/// key range, and only `events_job`, keyed by random job IDs, refills its
/// holes). Measured on the restart scenario over days 60 to 89: the table's
/// rows of days 30 to 59 at 15 a page and 57% full, of days 60 to 89 (their
/// jobs' events still there) at 24 and 97%, the login burst's at 54 and
/// 98%; 2,247 rows appended for 1,213 kept; the table grew 364 KiB for 369
/// KiB of cells, `events_scope` 182 for 157, `events_kind` 39 for 49 and
/// `events_job` -11 to -22 for 96. The ceiling is absolute, so a page leak
/// is caught whatever the table's size; it ignores what pruning frees, so
/// while the events rule prunes it is loose by that much.
///
/// The cells of a row appended in the second half of the third at most
/// `ROW_SIZE_DRIFT` times those of one appended in the first: rows that
/// grow are a leak the cell ceiling would credit.
///
/// While the table regains rows (the events and authority rules removed
/// nothing over the third, by the writer's passes, and its fitted rows rose
/// by at least `REFILL_MIN_ROWS`), where its growth ends: every other b-tree
/// as it is at the end, with the events table at its row budget at the
/// bytes it gained per row kept (or its average now, if more), within the
/// file bound. That growth is bounded (the row budget caps the rows, and
/// SQLite rebalances a page that deletes leave less than about a third
/// full, roots and the newest pages aside) but goes on until the burst's
/// rows have all been replaced, years at household rates.
fn flatness(store: &str, points: &[(f64, &StoreStat)], file_bound: u64) -> Flat {
    let mut problems = Vec::new();
    let (first, last) = (points[0], points[points.len() - 1]);
    let span = last.0 - first.0;
    let mut names: Vec<(String, String)> = Vec::new();
    for (_, s) in points {
        for (name, (table, _)) in &s.btrees {
            if !names.iter().any(|(n, _)| n == name) {
                names.push((name.clone(), table.clone()));
            }
        }
    }
    names.sort();
    let series = |pick: &dyn Fn(&StoreStat) -> f64| -> Vec<(f64, f64)> {
        points.iter().map(|(day, s)| (*day, pick(s))).collect()
    };
    let bytes_of = |s: &StoreStat, name: &str| s.btrees.get(name).map_or(0, |b| b.1) as f64;
    let mut events = Vec::new();
    let mut ceilings = 0.0;
    for (name, table) in &names {
        let bytes = series(&|s| bytes_of(s, name));
        let grown = slope(&bytes) * span;
        if table != "events" {
            let allowed = (mean(&bytes) * 0.02).max(btree_floor(store, table));
            if grown > allowed {
                problems.push(format!(
                    "{store}: {name} (of {table}) grew {:.0} KiB over the last third, more than {:.0} KiB",
                    grown / 1024.0,
                    allowed / 1024.0
                ));
            }
            continue;
        }
        if !EVENT_BTREES.contains(&name.as_str()) {
            problems.push(format!(
                "{store}: events b-tree {name} has no model of its cells: give it its key in `appended_cells`"
            ));
            continue;
        }
        let appended = series(&|s| s.appended.get(name).copied().unwrap_or(0) as f64);
        let fill = if name == "events" {
            TABLE_FILL
        } else {
            INDEX_FILL
        };
        let ceiling = (slope(&appended) * span).max(0.0) / fill + APPEND_SLACK;
        ceilings += ceiling;
        if grown > ceiling {
            problems.push(format!(
                "{store}: {name} grew {:.0} KiB over the last third, more than the {:.0} KiB the rows appended to it explain",
                grown / 1024.0,
                ceiling / 1024.0
            ));
        }
        events.push(EventsTree {
            name: name.clone(),
            grown,
            ceiling,
        });
    }
    if !events.is_empty() {
        let cells = |s: &StoreStat| s.appended.values().sum::<i64>() as f64;
        let mid = points[points.len() / 2].1;
        let per_row = |from: &StoreStat, to: &StoreStat| {
            let rows = (to.cursor - from.cursor) as f64;
            (rows >= 64.0).then(|| (cells(to) - cells(from)) / rows)
        };
        if let (Some(early), Some(late)) = (per_row(first.1, mid), per_row(mid, last.1))
            && late > early * ROW_SIZE_DRIFT
        {
            problems.push(format!(
                "{store}: rows appended to the events table in the second half of the last third take {late:.0} bytes of cells each, {early:.0} in the first"
            ));
        }
    }
    let mut refill = None;
    let mut unprojected = None;
    if let Some(Budget::Rows(limit)) = (!events.is_empty())
        .then(|| core_budget("events"))
        .flatten()
    {
        let rows = series(&|s| s.rows("events") as f64);
        let gain = slope(&rows) * span;
        let total = |s: &StoreStat| events.iter().map(|t| bytes_of(s, &t.name)).sum::<f64>();
        if last.1.pruned != first.1.pruned {
            unprojected = Some(format!(
                "the events and authority rules removed {} rows over the third",
                last.1.pruned - first.1.pruned
            ));
        } else if gain < REFILL_MIN_ROWS {
            unprojected = Some(format!(
                "its fitted rows rose by {gain:.0} over the third, fewer than {REFILL_MIN_ROWS:.0}"
            ));
        } else {
            let per_row = slope(&series(&total)) / slope(&rows);
            let average = total(last.1) / (last.1.rows("events") as f64).max(1.0);
            let others: f64 = names
                .iter()
                .filter(|(_, table)| table != "events")
                .map(|(name, _)| bytes_of(last.1, name))
                .sum();
            let at_budget = others + limit as f64 * per_row.max(average);
            if at_budget > file_bound as f64 {
                problems.push(format!(
                    "{store}: the events table took {per_row:.0} bytes of pages a row it kept over the last third ({gain:.0} rows regained): at its budget of {limit} rows, with every other table as it is, the tables hold {:.1} MiB, past the {:.0} MiB file bound",
                    at_budget / 1048576.0,
                    mib(file_bound)
                ));
            }
            refill = Some(Refill { per_row, at_budget });
        }
    }
    // The file may grow by at most 1 MiB over the last third beyond what the
    // events b-trees' cell ceilings allow (between the third's ends): every
    // table is bounded.
    let file_grown = last.1.file as f64 - first.1.file as f64;
    let allowed = 1024.0 * 1024.0 + ceilings;
    if file_grown > allowed {
        problems.push(format!(
            "{store}: file grew {:.2} MiB over the last third, more than {:.2} MiB",
            file_grown / 1048576.0,
            allowed / 1048576.0
        ));
    }
    Flat {
        problems,
        events,
        refill,
        unprojected,
    }
}

/// Bytes of core's events table (with its indexes) in a sample.
fn events_bytes(s: &Sample) -> f64 {
    s.core.tables.get("events").map_or(0, |t| t.1) as f64
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

/// Startup page reads per page the per-job tables gain, at most: measured
/// 3.26 between 257 and 5,113 retained jobs (`quick_check` and the
/// foreign-key check read each new page, and the one walk of `jobs` and the
/// index accounting their share of them; 5.1 while eight checks each walked
/// `jobs`, when this was 10); twice that is still the history component,
/// more is growth from something else. On four seeds of each 90-day profile
/// the events table's growth alone explained the last third's (the per-job
/// tables gained -0.4 to 2.4 pages a night), with 8.6 to 17.7 reads a night
/// to spare.
pub const STARTUP_PER_JOB_MAX: f64 = 6.5;
/// Startup page reads per page the events table (with its indexes) gains,
/// at most: `quick_check` and the foreign-key check read each new page
/// once, and no other startup statement walks it (`EXPLAINED`); twice that.
pub const STARTUP_PER_EVENT_MAX: f64 = 4.0;
/// Reads a night beyond what the history tables explain that are taken as
/// noise between the medians of the two halves.
pub const STARTUP_SLACK: f64 = 4.0;
pub const STARTUP_PER_JOB_PAGES: &str = "per-job tables' pages gained a night over the last third";
pub const STARTUP_EVENT_PAGES: &str = "events table's pages gained a night over the last third";
/// The most the per-job tables may grow over the last third (once the storm's
/// jobs are gone) beyond the jobs job-row retention keeps: 64 KiB, two KiB a
/// day over 30 days, a dozen jobs' worth.
pub const PER_JOB_FLAT: f64 = 64.0 * 1024.0;
pub const EVENTS_PER_ROW: &str =
    "events table's bytes of pages a row it kept while it regained rows over the last third";
pub const EVENTS_AT_BUDGET: &str =
    "core's tables with the events table at its budget at that rate, MiB";
pub const PER_JOB_NET: &str = "per-job tables' fitted growth over the last third beyond the rows of the jobs retention keeps, table by table, KiB";
pub const PER_JOB_NET_JOBS: &str =
    "the same counted by jobs alone, every job at the average size (information), KiB";
pub const WITHOUT_RESULT_START: &str =
    "settled jobs without a result row at the start of the per-job tail";
pub const WITHOUT_RESULT_END: &str = "settled jobs without a result row at the end";
pub const BUDGET_REFUSED_START: &str =
    "jobs refused by the effect budget kept at the start of the per-job tail";
pub const BUDGET_REFUSED_END: &str = "jobs refused by the effect budget kept at the end";
pub const PER_JOB_ROWS: &str = "per-job tables' rows of retained jobs grown over the last third beyond what the retained jobs and their gained results explain";
/// Rows the per-job tables may gain beyond what the retained jobs explain
/// (at the starting rows a job, and one result row for each resultless job
/// that left): stack jobs of different sizes replacing each other move it
/// by a few rows; a dozen jobs' worth.
pub const PER_JOB_ROWS_FLAT: f64 = 64.0;

/// What `per_job_model` finds over the tail of the last third.
struct PerJob {
    /// Fitted growth of the per-job tables' bytes beyond what each table's
    /// rows of retained jobs explain at its own bytes a row.
    net: f64,
    /// The same counted by jobs alone (information).
    jobs_only: f64,
    /// Fitted growth of the retained rows beyond what the retained jobs
    /// explain.
    rows_excess: f64,
    problems: Vec<String>,
}

/// The per-job tables over `tail`, which nothing else can absorb a leak in:
///
/// - Bytes: each per-job table is explained by its own rows, at its own
///   bytes a row, since jobs are not one size (a job refused before any
///   effect has no result row, a stack job's plan and result are larger) and
///   retention's per-family floor keeps the newest 128 settled jobs of each
///   family whatever their age, so small refused jobs replaced one for one
///   by full-size ones grow the tables with the job count flat. Only rows
///   that are accounted for explain bytes: each table's rows less those of
///   the jobs the rule may remove now and, for a plan table, less unqueued
///   plans already expired. Fitted growth under `PER_JOB_FLAT`.
/// - Rows: those retained rows in all may grow only as the retained jobs do
///   (at the starting rows a job) plus one result row for each resultless
///   job that left; fitted growth beyond that under `PER_JOB_ROWS_FLAT`. A
///   table whose rows per job grow, or rows owned by no job, fail here.
/// - No job owns more rows of a table than one job can (`rows_per_job`), and
///   no unqueued plan outlives the next plan creation that removes it.
fn per_job_model(tail: &[&Sample]) -> PerJob {
    let mut problems = Vec::new();
    let start = tail[0];
    let day = |s: &Sample| (s.at - START) as f64 / DAY as f64;
    let span = day(tail[tail.len() - 1]) - day(start);
    let kept = |s: &Sample| (s.core.rows("jobs") - s.removable) as f64;
    let kept_rows = |s: &Sample, table: &str| {
        (s.core.rows(table)
            - s.removable_rows.get(table).copied().unwrap_or(0)
            - s.expired_unqueued.get(table).copied().unwrap_or(0)) as f64
    };
    let per_job_tables = |s: &Sample| -> Vec<(String, i64, i64)> {
        s.core
            .tables
            .iter()
            .filter(|(t, _)| matches!(core_budget(t), Some(Budget::Retained(_))))
            .map(|(t, (rows, bytes))| (t.clone(), *rows, *bytes))
            .collect()
    };
    let net: Vec<(f64, f64)> = tail
        .iter()
        .map(|s| {
            let net: f64 = per_job_tables(s)
                .iter()
                .map(|(t, rows, bytes)| {
                    let per_row = *bytes as f64 / (*rows as f64).max(1.0);
                    *bytes as f64 - (kept_rows(s, t) - kept_rows(start, t)) * per_row
                })
                .sum();
            (day(s), net)
        })
        .collect();
    let jobs_only: Vec<(f64, f64)> = tail
        .iter()
        .map(|s| {
            let bytes = per_job_bytes(s);
            let per_job = bytes / (s.core.rows("jobs") as f64).max(1.0);
            (day(s), bytes - (kept(s) - kept(start)) * per_job)
        })
        .collect();
    let retained = |s: &Sample| -> f64 {
        per_job_tables(s)
            .iter()
            .map(|(t, _, _)| kept_rows(s, t))
            .sum()
    };
    let per_job = retained(start) / kept(start).max(1.0);
    let rows: Vec<(f64, f64)> = tail
        .iter()
        .map(|s| {
            let explained = retained(start)
                + (kept(s) - kept(start)) * per_job
                + (start.kept_without_result - s.kept_without_result) as f64;
            (day(s), retained(s) - explained)
        })
        .collect();
    let grown = |points: &[(f64, f64)]| slope(points) * span;
    let model = PerJob {
        net: grown(&net),
        jobs_only: grown(&jobs_only),
        rows_excess: grown(&rows),
        problems: Vec::new(),
    };
    if model.net > PER_JOB_FLAT {
        problems.push(format!(
            "core: per-job tables grew {:.0} KiB from day {:.0} to {:.0} beyond the rows of the jobs job-row retention keeps (at most {:.0} KiB)",
            model.net / 1024.0,
            day(start),
            day(tail[tail.len() - 1]),
            PER_JOB_FLAT / 1024.0
        ));
    }
    if model.rows_excess > PER_JOB_ROWS_FLAT {
        problems.push(format!(
            "core: per-job tables' retained rows grew {:.0} beyond what the retained jobs and their gained results explain (at most {PER_JOB_ROWS_FLAT})",
            model.rows_excess
        ));
    }
    if let Some(s) = tail.iter().find(|s| s.stale_unqueued > 0) {
        problems.push(format!(
            "core: {} unqueued plans expired before a newer plan of their table was created, and remain (day {:.1})",
            s.stale_unqueued,
            day(s)
        ));
    }
    if let Some(s) = tail.iter().find(|s| s.jobs_over_cap > 0) {
        problems.push(format!(
            "core: {} jobs own more rows of a per-job table than one job can (day {:.1})",
            s.jobs_over_cap,
            day(s)
        ));
    }
    PerJob { problems, ..model }
}
pub const RECEIPTS_FULL: &str = "receipt-store open page reads with every table at its budget";

/// What `startup_bounds` finds wrong with one measured open of `store`
/// (`core` or `receipts`) whose wall time is bounded by `bound`.
pub fn start_problems(
    store: &str,
    s: &super::sim::Startup,
    bound: u128,
) -> Vec<(&'static str, String)> {
    let mut problems = Vec::new();
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
        return problems;
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
    let Some(trace) = &s.trace else {
        return problems;
    };
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
    // The household has no storage mount: the proof check, driven from the
    // mount plans, reads a few pages, whatever the jobs and events (it read
    // 6.5 files' worth while it looked events up by kind for every job, and
    // up to an eighth of the file while it visited every settled job).
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
    problems
}

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
            problems.extend(start_problems(store, s, bound));
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
/// reading every statement's bytecode on every copy. After the first three
/// (whole-file checks and the audit count): one walk of `jobs`, the
/// accounting's reads of four indexes of `jobs` (each alone, covering) and
/// of the `job_resources` table, and the lock invariants' walk of
/// `job_resolutions`, one full-scan step per row. Until 24 November eight
/// owners' checks each walked `jobs` (and one `job_resources`) to find the
/// jobs of their operation; the walk now lists them, and a statement that
/// walks `jobs` again fails here.
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
        marker: "SELECT j.id,j.rowid,j.principal,j.idempotency,j.state,j.deadline,j.resource,json_extract(j.intent,'$.operation')",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "startup's one walk of every job: each job's resource set (its primary resource, at \
              most 128), and the jobs of the operations an owner's check reads, which those \
              checks then read through the primary key",
    },
    ScanAllowance {
        marker: "SELECT id,rowid FROM jobs INDEXED BY sqlite_autoindex_jobs_1",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the accounting of the jobs primary key's index against the walk (an entry a job)",
    },
    ScanAllowance {
        marker: "SELECT principal,idempotency,rowid FROM jobs INDEXED BY sqlite_autoindex_jobs_2",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the accounting of the jobs idempotency key's index against the walk",
    },
    ScanAllowance {
        marker: "SELECT state,deadline,rowid FROM jobs INDEXED BY jobs_state",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the accounting of jobs_state against the walk",
    },
    ScanAllowance {
        marker: "SELECT resource,rowid FROM jobs INDEXED BY jobs_resource_lock",
        tables: &["jobs"],
        events_table: false,
        steps_per_row: 1,
        why: "the accounting of the partial jobs_resource_lock against the walk (a lock-holding job \
              an entry)",
    },
    ScanAllowance {
        marker: "SELECT r.job,r.resource,r.rowid FROM job_resources r NOT INDEXED",
        tables: &["job_resources"],
        events_table: false,
        steps_per_row: 1,
        why: "the unsettled jobs' resource rows read from the table, for the accounting of \
              job_resources' primary key",
    },
    ScanAllowance {
        marker: "PRAGMA integrity_check(job_resolutions)",
        tables: &["job_resolutions"],
        events_table: false,
        steps_per_row: 1,
        why: "the resolutions checked against their indexes (one per resolved or resolving job)",
    },
    ScanAllowance {
        marker: "SELECT EXISTS( SELECT 1 FROM job_resources r JOIN jobs j ON j.id=r.job LEFT JOIN resource_locks l",
        tables: &["job_resolutions"],
        events_table: false,
        steps_per_row: 1,
        why: "the lock invariants walk the resolutions (one per resolved or resolving job); \
              they reach jobs only through `jobs_state` and the primary key",
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
            "receipt store: {actions} action rows and {stacks} stack rows kept of {executed} executes; prune watermark: {}",
            watermark_day(watermark)
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
    if !sim.recovered.is_empty() {
        let core = reader(&sim.core_path).unwrap();
        let now = sim.now();
        let (mut removed, mut authority, mut recent, mut open) = (0, 0, 0, 0);
        for job in &sim.recovered {
            if sim.pruned.contains(job) {
                removed += 1;
                continue;
            }
            let (settled, resolution): (Option<i64>, bool) = core
                .query_row(
                    "SELECT settled,EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND json_extract(e.kind,'$.kind') GLOB 'job_resolution_*') FROM jobs j WHERE id=?",
                    [job],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap_or((None, false));
            match (settled, resolution) {
                (_, true) => authority += 1,
                (Some(at), false) if at >= now - limeos_persistence::test_support::JOB_HORIZON => {
                    recent += 1
                }
                _ => open += 1,
            }
        }
        let _ = writeln!(
            p,
            "jobs startup recovery marked: {}; removed by job-row retention {removed}; kept: {authority} for an authority event, {recent} settled within the horizon, {open} otherwise (a family's floor, or unsettled)",
            sim.recovered.len()
        );
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
    for line in &sim.flat_lines {
        let _ = writeln!(p, "{line}");
    }
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
        CLOCK,
        BUDGET,
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

/// The flatness model on the restart scenario's last third as measured
/// (days 60 to 89 of `soak_ninety_days_restarts`, the two traced runs of 23
/// November: every b-tree's bytes and the cells appended to the events
/// b-trees), and the same third with a leak or a boundary case put in.
/// For `startup_at_high_retained_job_count`: core's open on each nightly
/// copy against the jobs it held, the cold page reads of its statements
/// (but the pinned proof check) per page of file and WAL against the bound's
/// factor, the open's wall time, and the statements reading most at the
/// worst ratio. Returns the most jobs a copy held.
pub fn report_startup_scaling(sim: &Sim) -> u64 {
    let mut most_jobs = 0;
    let mut worst: Option<(f64, &super::sim::Startup)> = None;
    eprintln!(
        "day | jobs | pages of file+WAL | cold reads | per page (bound factor {:.1}) | ms",
        CORE_PAGE_FACTOR as f64 / 1000.0
    );
    for start in &sim.core_starts {
        let Some(trace) = &start.trace else { continue };
        let jobs = trace.rows.get("jobs").copied().unwrap_or(0);
        most_jobs = most_jobs.max(jobs);
        let pages = ((start.file + start.wal) / PAGE).max(1);
        let ratio = trace.cold_but_defect() as f64 / pages as f64;
        eprintln!(
            "{:>5.2} | {jobs:>5} | {pages:>6} | {:>6} | {ratio:.2} | {}",
            (start.at - START) as f64 / DAY as f64,
            trace.cold_but_defect(),
            start.opened.elapsed.as_millis()
        );
        if worst.is_none_or(|(r, _)| ratio > r) {
            worst = Some((ratio, start));
        }
    }
    if let Some((ratio, start)) = worst
        && let Some(trace) = &start.trace
    {
        eprintln!("worst ratio {ratio:.2}; its statements by cold reads:");
        let mut statements: Vec<_> = trace.statements.iter().collect();
        statements.sort_by_key(|s| std::cmp::Reverse(s.cold.unwrap_or(s.reads)));
        for statement in statements.iter().take(10) {
            eprintln!(
                "  {:>7} cold, {:>9} VM steps, {:>6} full-scan steps: {}",
                statement.cold.unwrap_or(statement.reads),
                statement.vm_steps,
                statement.full_scans,
                statement.sql.chars().take(110).collect::<String>()
            );
        }
    }
    most_jobs
}

#[cfg(test)]
mod flatness_model {
    use super::*;

    const KIB: f64 = 1024.0;
    const LEAK: f64 = 512.0 * KIB;

    /// One b-tree: its table, its bytes on day 60 and its growth over the
    /// third, and for an events b-tree the cells appended to it.
    struct Tree {
        name: &'static str,
        table: &'static str,
        start: f64,
        grown: f64,
        appended: f64,
    }
    const fn tree(name: &'static str, table: &'static str, start: f64, grown: f64) -> Tree {
        Tree {
            name,
            table,
            start: start * KIB,
            grown: grown * KIB,
            appended: 0.0,
        }
    }
    const fn events(name: &'static str, start: f64, grown: f64, appended: f64) -> Tree {
        Tree {
            appended: appended * KIB,
            ..tree(name, "events", start, grown)
        }
    }
    /// The measured third (trace 1; trace 2 differs only in `events_job`
    /// and the random-keyed autoindexes, by a few KiB).
    fn measured() -> Vec<Tree> {
        vec![
            events("events", 10_268.0, 364.1, 369.1),
            events("events_scope", 6_316.0, 182.5, 157.4),
            events("events_kind", 3_868.0, 39.1, 48.8),
            events("events_job", 2_036.0, -10.7, 96.0),
            tree("stack_results", "stack_results", 276.0, 141.7),
            tree("jobs", "jobs", 568.0, 15.4),
            tree("job_resources", "job_resources", 152.0, 18.5),
            tree(
                "sqlite_autoindex_job_resources_1",
                "job_resources",
                220.0,
                -46.9,
            ),
            tree("container_plans", "container_plans", 280.0, 13.1),
            tree("sqlite_autoindex_jobs_1", "jobs", 64.0, -4.2),
            tree("sessions", "sessions", 8.0, 0.0),
            tree("users", "users", 8.0, 0.0),
        ]
    }

    /// A third: `trees` growing linearly (with `shape(t)`, t from 0 to 1, of
    /// their growth by day t), the events table's rows by `rows(t)`, 77.5
    /// rows appended a day, and the rules' removals by `pruned(t)`.
    struct Third {
        trees: Vec<Tree>,
        rows: fn(f64) -> i64,
        pruned: fn(f64) -> i64,
        shape: fn(f64) -> f64,
        /// The cells a row appended takes, by t (1 = as measured).
        row_size: fn(f64) -> f64,
    }
    fn refilling(t: f64) -> i64 {
        128_568 + (1_213.0 * t) as i64
    }
    fn none(_: f64) -> i64 {
        0
    }
    fn linear(t: f64) -> f64 {
        t
    }
    fn same(_: f64) -> f64 {
        1.0
    }
    impl Third {
        fn measured() -> Self {
            Third {
                trees: measured(),
                rows: refilling,
                pruned: none,
                shape: linear,
                row_size: same,
            }
        }
        fn with(mut self, name: &str, extra: f64) -> Self {
            self.trees
                .iter_mut()
                .find(|t| t.name == name)
                .unwrap()
                .grown += extra;
            self
        }
        fn stats(&self) -> Vec<StoreStat> {
            (0..30)
                .map(|day| {
                    let t = day as f64 / 29.0;
                    let mut stat = StoreStat {
                        file: 30_183_424,
                        cursor: 153_280 + (2_247.0 * t) as i64,
                        pruned: (self.pruned)(t),
                        ..StoreStat::default()
                    };
                    for tree in &self.trees {
                        let bytes = tree.start + tree.grown * (self.shape)(t);
                        stat.btrees
                            .insert(tree.name.to_owned(), (tree.table.to_owned(), bytes as i64));
                        let entry = stat.tables.entry(tree.table.to_owned()).or_default();
                        entry.1 += bytes as i64;
                        if tree.table == "events" {
                            // Integral of the row size over the third so far.
                            let steps = 64;
                            let cells: f64 = (0..steps)
                                .map(|i| (self.row_size)(t * (i as f64 + 0.5) / steps as f64))
                                .sum::<f64>()
                                * t
                                / steps as f64;
                            stat.appended.insert(
                                tree.name.to_owned(),
                                (5_000_000.0 + tree.appended * cells) as i64,
                            );
                        }
                    }
                    stat.tables.get_mut("events").unwrap().0 = (self.rows)(t);
                    stat.tables.get_mut("sessions").unwrap().0 = 3;
                    stat
                })
                .collect()
        }
        fn judge(&self) -> Flat {
            let stats = self.stats();
            let points: Vec<(f64, &StoreStat)> = stats
                .iter()
                .enumerate()
                .map(|(i, s)| (60.0 + i as f64, s))
                .collect();
            flatness("core", &points, CORE_FILE_BOUND)
        }
    }
    fn fails(flat: &Flat, needle: &str) -> bool {
        flat.problems.iter().any(|p| p.contains(needle))
    }

    #[test]
    fn the_measured_third_is_flat_and_its_end_within_the_bound() {
        let flat = Third::measured().judge();
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        let refill = flat.refill.expect("the events table regained rows");
        assert!((440.0..520.0).contains(&refill.per_row), "{refill:?}");
        assert!(refill.at_budget < 66.0 * 1048576.0, "{refill:?}");
        // The margins the events model leaves, b-tree by b-tree.
        for tree in &flat.events {
            assert!(tree.ceiling - tree.grown > 40.0 * KIB, "{tree:?}");
        }
    }

    #[test]
    fn a_leak_in_a_table_fails_whatever_the_others_release() {
        // One table growing while another shrinks: each is judged alone.
        let flat = Third::measured()
            .with("sessions", 300.0 * KIB)
            .with("job_resources", -100.0 * KIB)
            .judge();
        assert!(
            fails(&flat, "core: sessions (of sessions) grew"),
            "{:?}",
            flat.problems
        );
        let flat = Third::measured().with("stack_results", 120.0 * KIB).judge();
        assert!(fails(&flat, "core: stack_results"), "{:?}", flat.problems);
    }

    #[test]
    fn a_leak_in_an_index_fails_whatever_its_table_releases() {
        let flat = Third::measured()
            .with("sqlite_autoindex_jobs_1", 256.0 * KIB)
            .with("jobs", -256.0 * KIB)
            .judge();
        assert!(
            fails(&flat, "core: sqlite_autoindex_jobs_1 (of jobs) grew"),
            "{:?}",
            flat.problems
        );
    }

    #[test]
    fn a_leak_in_each_events_btree_fails() {
        for (name, leak) in [
            ("events", 72.0 * KIB),
            ("events_scope", 72.0 * KIB),
            ("events_kind", 72.0 * KIB),
            ("events_job", 192.0 * KIB),
        ] {
            let flat = Third::measured().with(name, leak).judge();
            assert!(
                fails(&flat, &format!("core: {name} grew")),
                "{name}: {:?}",
                flat.problems
            );
        }
    }

    #[test]
    fn a_leak_fails_however_many_rows_the_table_regains() {
        // A larger refill makes the projection forgiving; the cell ceiling
        // does not depend on it.
        let mut third = Third::measured().with("events", LEAK);
        third.rows = |t| 128_568 + (3_000.0 * t) as i64;
        let flat = third.judge();
        assert!(fails(&flat, "core: events grew"), "{:?}", flat.problems);
    }

    #[test]
    fn a_leak_at_the_steady_state_size_fails() {
        // The events b-trees at about 60 MiB (the household's rows at their
        // fill), refilling and at their budget with the events rule pruning.
        let scaled = || {
            let mut third = Third::measured();
            for tree in third.trees.iter_mut().filter(|t| t.table == "events") {
                tree.start *= 2.7;
            }
            third
        };
        let flat = scaled().judge();
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        let flat = scaled().with("events", LEAK).judge();
        assert!(fails(&flat, "core: events grew"), "{:?}", flat.problems);
        let mut third = scaled();
        third.rows = |_| 131_072;
        third.pruned = |t| (1_218.0 * t) as i64;
        for tree in third.trees.iter_mut().filter(|t| t.table == "events") {
            tree.grown = 0.0;
        }
        let flat = third.judge();
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        assert!(flat.refill.is_none() && flat.unprojected.is_some());
        // At its budget the ceiling ignores what pruning frees: a leak
        // beyond what the appended rows explain fails, a smaller one passes.
        let flat = Third { ..third }.with("events", 2.0 * LEAK).judge();
        assert!(fails(&flat, "core: events grew"), "{:?}", flat.problems);
    }

    #[test]
    fn rows_that_grow_fail() {
        let mut third = Third::measured();
        third.row_size = |t| 1.0 + t;
        let flat = third.judge();
        assert!(fails(&flat, "in the second half"), "{:?}", flat.problems);
    }

    #[test]
    fn a_plateau_then_a_gain_projects_a_finite_end() {
        let mut third = Third::measured();
        third.rows = |t| 128_568 + (2_426.0 * (t - 0.5).max(0.0)) as i64;
        third.shape = |t| (2.0 * (t - 0.5)).max(0.0);
        let flat = third.judge();
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        let refill = flat.refill.expect("measured");
        assert!(
            refill.at_budget.is_finite() && refill.per_row.is_finite(),
            "{refill:?}"
        );
    }

    #[test]
    fn falling_rows_project_nothing() {
        let mut third = Third::measured();
        third.rows = |t| 128_568 - (1_213.0 * t) as i64;
        let flat = third.judge();
        assert!(flat.refill.is_none(), "{:?}", flat.refill);
        assert!(flat.unprojected.unwrap().contains("fitted rows rose by -"));
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
    }

    #[test]
    fn falling_bytes_project_at_the_average_row() {
        let mut third = Third::measured();
        for tree in third.trees.iter_mut().filter(|t| t.table == "events") {
            tree.grown = -tree.grown.abs();
        }
        let flat = third.judge();
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        let refill = flat.refill.expect("measured");
        assert!(refill.per_row < 0.0, "{refill:?}");
        // 22 MiB of events over 129,781 rows, at the 132,096-row budget.
        assert!(refill.at_budget > 20.0 * 1048576.0, "{refill:?}");
    }

    #[test]
    fn a_refill_that_reaches_the_budget_is_projected_until_pruning_starts() {
        let mut third = Third::measured();
        third.rows = |t| (128_568 + (5_000.0 * t) as i64).min(131_072);
        let flat = third.judge();
        assert!(
            flat.problems.is_empty() && flat.refill.is_some(),
            "{flat:?}"
        );
        third.pruned = |t| if t > 0.9 { 64 } else { 0 };
        let flat = third.judge();
        assert!(flat.refill.is_none(), "{flat:?}");
        assert!(flat.unprojected.unwrap().contains("removed 64 rows"));
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
    }

    #[test]
    fn an_events_index_without_a_model_fails() {
        let mut third = Third::measured();
        third.trees.push(events("events_new", 64.0, 0.0, 0.0));
        let flat = third.judge();
        assert!(
            fails(&flat, "events b-tree events_new has no model"),
            "{:?}",
            flat.problems
        );
    }
}

#[cfg(test)]
mod per_job_model_tests {
    use super::*;

    /// A tail of 30 daily samples: `kept` retained jobs, each owning a job
    /// row, two resource rows and a plan row, with a result row unless it is
    /// one of `resultless(t)`; plus `extra(t)` rows of `table` (owned by no
    /// retained job) and `unqueued(t)` expired unqueued stack plans, the stale
    /// ones `stale(t)`. Every row 200 bytes, a result 1,000.
    struct Tail {
        kept: fn(f64) -> i64,
        resultless: fn(f64) -> i64,
        extra: (&'static str, fn(f64) -> i64),
        unqueued: fn(f64) -> i64,
        stale: fn(f64) -> i64,
    }
    fn flat(_: f64) -> i64 {
        400
    }
    fn zero(_: f64) -> i64 {
        0
    }
    impl Tail {
        fn steady() -> Self {
            Tail {
                kept: flat,
                resultless: zero,
                extra: ("job_resources", zero),
                unqueued: zero,
                stale: zero,
            }
        }
        fn samples(&self) -> Vec<Sample> {
            (0..30)
                .map(|day| {
                    let t = day as f64 / 29.0;
                    let jobs = (self.kept)(t);
                    let results = jobs - (self.resultless)(t);
                    let unqueued = (self.unqueued)(t);
                    let mut rows: BTreeMap<&str, i64> = [
                        ("jobs", jobs),
                        ("job_resources", 2 * jobs),
                        ("stack_plans", jobs + unqueued),
                        ("stack_results", results),
                    ]
                    .into();
                    *rows.get_mut(self.extra.0).unwrap() += (self.extra.1)(t);
                    let mut core = StoreStat::default();
                    for (table, n) in rows {
                        let size = if table == "stack_results" { 1000 } else { 200 };
                        core.tables.insert(table.to_owned(), (n, n * size));
                    }
                    Sample {
                        at: START + (60 + day) * DAY,
                        core,
                        kept_without_result: (self.resultless)(t),
                        expired_unqueued: [("stack_plans".to_owned(), unqueued)].into(),
                        stale_unqueued: (self.stale)(t),
                        ..Sample::default()
                    }
                })
                .collect()
        }
        fn judge(&self) -> PerJob {
            let samples = self.samples();
            let tail: Vec<&Sample> = samples.iter().collect();
            per_job_model(&tail)
        }
    }

    #[test]
    fn steady_jobs_and_a_composition_transient_pass() {
        let model = Tail::steady().judge();
        assert!(model.problems.is_empty(), "{:?}", model.problems);
        // Sixty resultless jobs replaced one for one by full-size ones: 60
        // results (60 KB) gained with the job count flat. Counted by jobs
        // alone it reads as growth; table by table it is explained.
        let model = Tail {
            resultless: |t| 60 - (60.0 * t) as i64,
            ..Tail::steady()
        }
        .judge();
        assert!(model.problems.is_empty(), "{:?}", model.problems);
        assert!(model.jobs_only > 40_000.0, "{}", model.jobs_only);
        assert!(model.net.abs() < 1_000.0, "{}", model.net);
    }

    #[test]
    fn expired_unqueued_plans_that_accumulate_fail() {
        // A plan cleanup that stopped: expired unqueued plans pile up. They
        // explain no bytes, and they outlive the newer plans' creation.
        let model = Tail {
            unqueued: |t| (400.0 * t) as i64,
            stale: |t| (400.0 * t) as i64,
            ..Tail::steady()
        }
        .judge();
        assert!(
            model.problems.iter().any(|p| p.contains("unqueued plans")),
            "{:?}",
            model.problems
        );
        assert!(model.net > PER_JOB_FLAT, "{}", model.net);
    }

    #[test]
    fn rows_per_job_that_grow_fail_though_each_table_explains_its_bytes() {
        // Resource rows grow by 300 with the retained jobs flat: the bytes
        // model credits each row at its own size, the rows model does not.
        let model = Tail {
            extra: ("job_resources", |t| (300.0 * t) as i64),
            ..Tail::steady()
        }
        .judge();
        assert!(
            model
                .problems
                .iter()
                .any(|p| p.contains("retained rows grew")),
            "{:?}",
            model.problems
        );
    }
}
